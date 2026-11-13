//! A run's workspace is made off the daemon's loop: a ticket is reserved for
//! its member at once — its run's generation and lease, the run's session
//! open and counted — and its worktree is made in the background; the run's
//! first turn starts once that is ready. Driven the way screens and engines
//! drive it, with a stand-in `claude` and a stand-in git that is slow, or
//! fails, at `git worktree add` for a repository that asks it to.
//!
//! What each test pins:
//! - the loop keeps answering while a worktree takes its time, and the run
//!   shows as preparing meanwhile;
//! - a worktree that comes after its run ended (cancelled, its member gone)
//!   starts nothing, and goes; the ticket starts afresh for whoever takes it;
//! - a worktree that can't be made lets the reservation go, and the ticket
//!   goes back as a start that failed always did;
//! - runs still preparing count against the lanes' limits;
//! - a daemon stopped while a run prepared puts its ticket back on the board.
//!
//! This file is a process of its own: its git is the stand-in for every test.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant};

use equipe_core::protocol::lanes::Parallel;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Engine, Event, ExecutionState, SessionSpec, SessionState, Snapshot, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::{squads, worktree};
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-start-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    // Written beside, then copied in: a script being written can't be run ("text file busy").
    let draft = path.with_extension("draft");
    std::fs::write(&draft, text).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(path).status().unwrap().success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The stand-in git, for every test of this process: each call goes down in
/// `<repo>.log`; `worktree add` sleeps the seconds `<repo>.slow` says, and
/// fails where `<repo>.fail` is.
fn fake_git() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let bin = scratch("git").join("git");
        // `git -C <dir> …`: the directory is the second argument.
        write_script(
            &bin,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$2.log\" 2>/dev/null\ncase \"$*\" in *'worktree add'*)\n  [ -f \"$2.fail\" ] && { echo 'fatal: no room left' >&2; exit 128; }\n  [ -f \"$2.slow\" ] && sleep \"$(cat \"$2.slow\")\";;\nesac\nexec git \"$@\"\n",
        );
        worktree::use_git(bin);
    });
}

/// A stand-in `claude`, signed in, whose ticket turns go on until stopped.
/// Each ticket turn's prompt is kept in `prompts.log`.
struct Engine0 {
    bin: PathBuf,
}

impl Engine0 {
    fn new() -> Self {
        let dir = scratch("bin");
        let bin = dir.join("claude");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then set -- -p "$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")')" "$@"; fi
case "$*" in *"You're on T-"*|*"equipe restarted while you worked"*) printf '%s\n<<end>>\n' "$*" >> '{log}';; esac
echo '{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}'
exec sleep 30
"#,
            log = dir.join("prompts.log").display(),
        );
        write_script(&bin, &script);
        Engine0 { bin }
    }

    fn prompts(&self) -> Vec<String> {
        // Lossy: engines appending big prompts at once can cut a multi-byte character in two.
        let log = String::from_utf8_lossy(&std::fs::read(self.bin.with_file_name("prompts.log")).unwrap_or_default()).into_owned();
        log.split("<<end>>").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A home with the squad `acme` on a fresh repository.
fn squad() -> (PathBuf, PathBuf) {
    let repo = scratch("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    (home, repo)
}

/// `git worktree add` for `repo` takes `secs` seconds (0: none).
fn slow(repo: &Path, secs: u32) {
    let marker = repo.with_extension("slow");
    if secs == 0 {
        let _ = std::fs::remove_file(marker);
    } else {
        std::fs::write(marker, secs.to_string()).unwrap();
    }
}

/// `git worktree add` for `repo` fails, or no longer does.
fn failing(repo: &Path, on: bool) {
    let marker = repo.with_extension("fail");
    if on {
        std::fs::write(marker, "").unwrap();
    } else {
        let _ = std::fs::remove_file(marker);
    }
}

fn git_calls(repo: &Path, with: &str) -> usize {
    std::fs::read_to_string(repo.with_extension("log")).unwrap_or_default().lines().filter(|l| l.contains(with)).count()
}

/// Once git has been asked `with` for `repo` `count` times.
async fn git_asked(repo: &Path, with: &str, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while git_calls(repo, with) < count {
        assert!(Instant::now() < deadline, "git was never asked {with:?}: {}", std::fs::read_to_string(repo.with_extension("log")).unwrap_or_default());
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        design_path: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
        nemotron: None,
        watch_disk: false,
    }
}

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let wait = async {
        while let Some(event) = rx.recv().await {
            let stop = done(&event);
            seen.push(event);
            if stop {
                return;
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(30), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

/// What comes within `secs`, however much.
async fn within(rx: &mut UnboundedReceiver<Event>, secs: f64) -> Vec<Event> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs_f64(secs), async {
        while let Some(event) = rx.recv().await {
            seen.push(event);
        }
    })
    .await;
    seen
}

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::EditFiles, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn ticket_for(handle: &Handle, owner: &str, title: &str) {
    let made = tool(handle, "pm", "create_ticket", json!({ "title": title, "body": "-", "owner": owner, "kind": "code" })).await;
    assert!(!made.is_error, "{made:?}");
}

/// The runs a snapshot lists, as `(member session, ticket, run session, generation)`, by ticket.
fn runs(snapshot: &Snapshot) -> Vec<(u64, u32, u64, u64)> {
    let mut runs: Vec<(u64, u32, u64, u64)> = snapshot.sessions.iter().filter_map(|s| s.run.map(|r| (r.of, r.ticket, s.session, r.generation))).collect();
    runs.sort_by_key(|r| r.1);
    runs
}

fn started_on(event: &Event, id: u32) -> bool {
    matches!(event, Event::WorkStarted { ticket, .. } if *ticket == id)
}

/// The worktree folders made for ticket `id` under `home`.
fn worktrees_of(home: &Path, id: u32) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for repo in std::fs::read_dir(home.join("worktrees")).into_iter().flatten().flatten() {
        for dir in std::fs::read_dir(repo.path()).into_iter().flatten().flatten() {
            // Only folders: the stand-in git's logs lie beside them.
            if dir.file_type().is_ok_and(|t| t.is_dir()) && dir.file_name().to_string_lossy().starts_with(&format!("t{id}-")) {
                found.push(dir.path());
            }
        }
    }
    found
}

#[tokio::test]
async fn the_loop_answers_while_a_worktree_takes_its_time_and_the_run_shows_preparing() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    slow(&repo, 4);
    let began = Instant::now();
    ticket_for(&handle, "frontend", "Hero").await;
    git_asked(&repo, "worktree add", 1).await;

    // Meanwhile every screen is answered at once.
    for _ in 0..5 {
        let asked = Instant::now();
        let snapshot = handle.snapshot("acme").await;
        assert!(asked.elapsed() < Duration::from_secs(1), "a snapshot waited {:?} on a worktree", asked.elapsed());
        let runs = runs(&snapshot);
        assert_eq!(runs.iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(2, 1, 1)], "reserved for frontend, generation 1");
        let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
        assert_eq!((ticket.status, ticket.assignee.as_deref(), ticket.worktree.as_ref()), (TicketStatus::InProgress, Some("frontend"), None));
        let run = ticket.execution.as_ref().unwrap();
        assert_eq!((run.state, run.session, run.generation), (ExecutionState::Active, runs[0].2, 1));
        let session = snapshot.sessions.iter().find(|s| s.session == runs[0].2).unwrap();
        assert_eq!(session.state, SessionState::Working, "shown at work, preparing");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(engine.prompts().is_empty(), "no turn before its workspace");

    let seen = until(&mut events, |e| started_on(e, 1)).await;
    assert!(began.elapsed() >= Duration::from_secs(3), "the run started once git was done");
    let run = runs(&handle.snapshot("acme").await)[0].2;
    assert!(
        seen.iter().any(|e| matches!(e, Event::Notice { session, text, .. } if *session == run && text == "Preparing workspace…")),
        "the run says it prepares its workspace: {seen:#?}"
    );
    let at = seen.iter().position(|e| matches!(e, Event::RunOpened { ticket: 1, .. })).expect("the run opened");
    assert!(at < seen.len() - 1, "before its work started");
    let snapshot = handle.snapshot("acme").await;
    let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
    let path = ticket.worktree.clone().expect("its worktree");
    assert!(path.join("README.md").exists());
    assert_eq!(ticket.execution.as_ref().and_then(|e| e.reason.clone()), None, "started: no longer starting");
    for _ in 0..100 {
        if !engine.prompts().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(engine.prompts().iter().filter(|p| p.contains("You're on T-1")).count(), 1, "its first turn, once");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_worktree_that_comes_after_its_ticket_was_cancelled_starts_nothing_and_goes() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    slow(&repo, 3);
    ticket_for(&handle, "frontend", "Hero").await;
    git_asked(&repo, "worktree add", 1).await;
    handle.send(Command::Cancel { squad: "acme".into(), ticket: 1 });
    let mut seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Cancelled)).await;
    // The worktree comes, too late: it goes with its branch.
    git_asked(&repo, "worktree remove", 1).await;
    git_asked(&repo, "branch -q -D equipe/frontend-t1-", 1).await;
    seen.extend(within(&mut events, 0.5).await);
    assert!(!seen.iter().any(|e| started_on(e, 1)), "nothing started: {seen:#?}");
    let snapshot = handle.snapshot("acme").await;
    assert!(runs(&snapshot).is_empty());
    let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
    assert_eq!(ticket.status, TicketStatus::Cancelled);
    let run = ticket.execution.as_ref().unwrap();
    assert_eq!((run.state, run.generation, run.reason.as_deref()), (ExecutionState::Stopped, 1, Some("cancelled")));
    assert!(worktrees_of(&home, 1).is_empty(), "{:?}", worktrees_of(&home, 1));
    assert!(git(&repo, &["branch", "--list", "equipe/frontend-t1-*"]).is_empty());
    assert!(engine.prompts().is_empty(), "no turn ran");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_worktree_that_comes_after_its_member_left_goes_and_the_ticket_starts_afresh_for_another() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    add(&handle, &mut events, 3, member(&repo, "frontend")).await;
    slow(&repo, 3);
    ticket_for(&handle, "frontend", "Hero").await;
    git_asked(&repo, "worktree add", 1).await;
    let first = runs(&handle.snapshot("acme").await);
    assert_eq!(first.iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(2, 1, 1)]);
    slow(&repo, 0);
    // frontend leaves while its worktree is made: frontend-2 takes the
    // ticket only once that job is back, in a worktree of its own.
    handle.send(Command::Close { session: 2 });
    let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, of: Some(3), .. })).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::WorkStarted { of: Some(2), .. })), "the run that left never started: {seen:#?}");
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(3, 1, 2)], "frontend-2's, generation 2");
    let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
    assert_eq!((ticket.status, ticket.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend-2")));
    let path = ticket.worktree.clone().unwrap();
    assert!(path.file_name().unwrap().to_string_lossy().ends_with("-2"), "{}", path.display());
    // The one made for frontend went, with its branch.
    git_asked(&repo, "worktree remove", 1).await;
    for _ in 0..100 {
        if worktrees_of(&home, 1).len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(worktrees_of(&home, 1), std::slice::from_ref(&path));
    assert_eq!(git(&repo, &["branch", "--list", "equipe/frontend-t1-*"]), "", "frontend's branch went");
    assert_eq!(git(&repo, &["worktree", "list", "--porcelain"]).lines().filter(|l| l.starts_with("worktree ")).count(), 2, "the repo and frontend-2's");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_worktree_that_cant_be_made_lets_its_reservation_go_and_the_ticket_goes_back() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    failing(&repo, true);
    ticket_for(&handle, "frontend", "Hero").await;
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Proposed)).await;
    assert!(!seen.iter().any(|e| started_on(e, 1)));
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { .. })), "the run closed: {seen:#?}");
    let snapshot = handle.snapshot("acme").await;
    assert!(runs(&snapshot).is_empty(), "its lane is free");
    let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
    assert!(ticket.could_not_start(), "{:?}", ticket.note);
    assert!(ticket.note.as_deref().unwrap_or_default().contains("no room left"), "{:?}", ticket.note);
    assert_eq!(ticket.assignee, None);
    let run = ticket.execution.as_ref().unwrap();
    assert_eq!((run.state, run.reason.as_deref()), (ExecutionState::Stopped, Some("could_not_start")));
    assert!(engine.prompts().is_empty());
    // The lane it held takes the next ticket.
    failing(&repo, false);
    ticket_for(&handle, "frontend", "Footer").await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, of: Some(2), .. })).await;
    handle.shutdown().await;
}

#[tokio::test]
async fn runs_still_preparing_count_against_the_lanes_limit_on_max() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    handle.send(Command::SetParallel { squad: "acme".into(), parallel: Parallel::max(2, 16) });
    until(&mut events, |e| matches!(e, Event::ParallelChanged { parallel, .. } if parallel.is_max())).await;
    slow(&repo, 3);
    for n in 1..=4 {
        ticket_for(&handle, "frontend", &format!("Page {n}")).await;
    }
    git_asked(&repo, "worktree add", 1).await;
    slow(&repo, 0);
    // Every ticket asked for a dispatch; two went, both still preparing.
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(), [(2, 1), (2, 2)], "never past the cap");
    assert_eq!(snapshot.tickets.iter().filter(|t| t.status == TicketStatus::Todo).count(), 2);
    assert!(snapshot.tickets.iter().all(|t| t.worktree.is_none()), "none ready yet");
    // In the board's order, one folder's worktrees one at a time.
    let mut order = Vec::new();
    for _ in 0..2 {
        let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { .. })).await;
        if let Some(Event::WorkStarted { ticket, .. }) = seen.last() {
            order.push(*ticket);
        }
    }
    assert_eq!(order, [1, 2]);
    within(&mut events, 0.5).await;
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).len(), 2, "still two once they run");
    assert_eq!(git_calls(&repo, "worktree add"), 2, "only the two that run had worktrees made");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_run_still_preparing_when_the_daemon_stops_goes_back_on_the_board_and_starts_afresh() {
    fake_git();
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    slow(&repo, 3);
    ticket_for(&handle, "frontend", "Hero").await;
    git_asked(&repo, "worktree add", 1).await;
    assert_eq!(runs(&handle.snapshot("acme").await).len(), 1);
    handle.shutdown().await;
    drop(events);
    slow(&repo, 0);

    // Back up: nothing carries a run that never ran on; its ticket goes back
    // on the board and starts afresh, under generation 2, as a first start.
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, of: Some(2), .. })).await;
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(runs(&snapshot).iter().map(|r| (r.0, r.1, r.3)).collect::<Vec<_>>(), [(2, 1, 2)]);
    let ticket = snapshot.tickets.iter().find(|t| t.id == 1).unwrap();
    assert_eq!((ticket.status, ticket.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend")));
    let run = ticket.execution.as_ref().unwrap();
    assert_eq!((run.state, run.generation, run.reason.as_deref()), (ExecutionState::Active, 2, None));
    assert!(ticket.worktree.as_ref().is_some_and(|p| p.to_string_lossy().ends_with("-2")), "{:?}", ticket.worktree);
    for _ in 0..100 {
        if !engine.prompts().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let prompts = engine.prompts();
    assert_eq!(prompts.iter().filter(|p| p.contains("You're on T-1")).count(), 1, "{prompts:#?}");
    assert!(!prompts.iter().any(|p| p.contains("equipe restarted while you worked")), "it never ran: nothing to carry on");
    handle.shutdown().await;
}
