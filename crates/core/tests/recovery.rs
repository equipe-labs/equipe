//! A ticket whose run stopped without a hand-in, recovered on the same
//! ticket, branch and worktree — driven the way screens and engines drive it,
//! from the reported stale-ticket cases: a turn that ended without a board
//! call, a daemon restart, a review send-back before the stall, an approved
//! add that unblocks the stalled work, the PM trying to force it on its own,
//! the same approval sent twice, and an approved recovery waiting across a
//! restart. Each case checks that no ticket was added, and that the branch,
//! worktree, commit and uncommitted files are what they were. Stand-in
//! engines play the members; nothing reaches a network.
//!
//! The minute tick's lease check is run as of a later time
//! ([`Handle::check_leases_at`]) instead of waiting minutes.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{
    Access, Command, Engine, Event, ExecutionState, Recovery, RunView, SessionSpec, SessionState, Snapshot, Ticket, TicketStatus, stall_reason,
};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-recovery-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// A stand-in `claude`, signed in. Each ticket turn's prompt is kept in
/// `prompts.log`, and the process and folder it ran in in `runs.log`; it
/// writes nothing where it runs. While `hang` exists beside it, a turn goes
/// on until it is stopped; otherwise it answers in a line and ends — without
/// a board call, as the reported stale runs did.
struct Engine0 {
    bin: PathBuf,
}

impl Engine0 {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("bin");
        let bin = dir.join("claude");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then set -- -p "$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")')" "$@"; fi
case "$*" in *"You're on T-"*|*"equipe restarted while you worked"*) printf '%s\n<<end>>\n' "$*" >> '{log}'; echo "$$ $(pwd -P)" >> '{runs}';; esac
echo '{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}'
if [ -e '{hang}' ]; then exec sleep 30; fi
echo '{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}'
"#,
            log = dir.join("prompts.log").display(),
            runs = dir.join("runs.log").display(),
            hang = dir.join("hang").display(),
        );
        let draft = dir.join("claude.draft");
        std::fs::write(&draft, script).unwrap();
        assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Engine0 { bin }
    }

    fn hang(&self, on: bool) {
        let marker = self.bin.with_file_name("hang");
        if on {
            std::fs::write(marker, "").unwrap();
        } else {
            let _ = std::fs::remove_file(marker);
        }
    }

    fn prompts(&self) -> Vec<String> {
        let log = std::fs::read_to_string(self.bin.with_file_name("prompts.log")).unwrap_or_default();
        log.split("<<end>>").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
    }

    /// The ticket turns that ran: `(process id, folder)`, oldest first.
    fn runs(&self) -> Vec<(String, PathBuf)> {
        let log = std::fs::read_to_string(self.bin.with_file_name("runs.log")).unwrap_or_default();
        log.lines().filter_map(|l| l.split_once(' ')).map(|(pid, dir)| (pid.to_owned(), PathBuf::from(dir))).collect()
    }

    async fn prompt_with(&self, wanted: &str) -> String {
        for _ in 0..400 {
            if let Some(p) = self.prompts().into_iter().rev().find(|p| p.contains(wanted)) {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no ticket prompt said {wanted:?}: {:#?}", self.prompts());
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// A home with the squad `acme` on a fresh repository, and a Codex login
/// made up for the test (a Codex member here has no Codex CLI).
fn squad() -> (PathBuf, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let login = Credentials {
        provider: "openai".into(),
        method: AuthMethod::OauthSubscription,
        access: "test-only".into(),
        refresh: None,
        expires: Some(4_000_000_000),
        account_id: None,
        residency: None,
    };
    CredentialStore::new(&home).save(&login).unwrap();
    (home, repo)
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI: the person's own must never run in a test.
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
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

/// Everything that happens in the next `wait`.
async fn within(rx: &mut UnboundedReceiver<Event>, wait: Duration) -> Vec<Event> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(wait, async {
        while let Some(event) = rx.recv().await {
            seen.push(event);
        }
    })
    .await;
    seen
}

fn member(cwd: &Path, role: &str, engine: Option<Engine>) -> SessionSpec {
    // A Codex member names its model: nothing asks Codex's catalog.
    let model = (engine == Some(Engine::Codex)).then(|| "gpt-test".to_owned());
    // Codex edits only through its CLI, which isn't here: a Codex member reads.
    let access = if engine == Some(Engine::Codex) { Access::ReadOnly } else { Access::EditFiles };
    SessionSpec { engine, model, access, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn ticket(handle: &Handle, id: u32) -> Ticket {
    handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == id).expect("on the board")
}

/// The ticket ids on the board, in order: a recovery never adds one.
fn ids(snapshot: &Snapshot) -> Vec<u32> {
    let mut ids: Vec<u32> = snapshot.tickets.iter().map(|t| t.id).collect();
    ids.sort();
    ids
}

/// Wait until member `id`'s run is idle with its ticket still in hand: its
/// turn ended without a hand-in or a wait.
async fn idle_on_its_ticket(handle: &Handle, id: u64) {
    for _ in 0..200 {
        let snapshot = handle.snapshot("acme").await;
        if snapshot.sessions.iter().any(|s| s.run.is_some_and(|r| r.of == id) && s.state == SessionState::Ready && s.work.is_some()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("session {id} never sat idle on its ticket: {:#?}", handle.snapshot("acme").await.sessions);
}

fn execution(t: &Ticket) -> equipe_core::protocol::TicketExecution {
    *t.execution.clone().expect("the daemon keeps the ticket's run")
}

/// What the person's work-in-progress looks like on disk: the branch's
/// commit, and each file's bytes, the uncommitted and untracked included.
fn checkout(worktree: &Path) -> (String, Vec<(String, Vec<u8>)>) {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(worktree)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name() != ".git")
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect();
    files.sort();
    (git(worktree, &["rev-parse", "HEAD"]), files)
}

/// The board's and the sidebar's reading of a ticket's run, from one
/// snapshot: the member's state as the sidebar shows it, fed to the same
/// computation the board uses.
fn run_seen(snapshot: &Snapshot, id: u32) -> Option<RunView> {
    let t = snapshot.tickets.iter().find(|t| t.id == id)?;
    let working = t.assignee.as_deref().filter(|name| snapshot.sessions.iter().any(|s| s.member.as_ref().is_some_and(|m| m.name == *name))).map(|name| {
        snapshot.sessions.iter().any(|s| {
            s.run.is_some_and(|r| r.ticket == id) && s.member.as_ref().is_some_and(|m| m.name == name) && s.state == SessionState::Working && s.work.as_ref().is_some_and(|w| w.ticket == id)
        })
    });
    t.run_view(now(), working)
}

/// A squad of a PM and a frontend on Claude, T-1 picked up and its turn
/// ended without a board call; uncommitted and untracked work in its worktree.
async fn idle_frontend(engine: &Engine0, home: &Path, repo: &Path) -> (Handle, UnboundedReceiver<Event>, PathBuf) {
    let (handle, mut events) = runtime::start(config(&engine.bin, home));
    add(&handle, &mut events, 1, member(repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(repo, "frontend", Some(Engine::Claude))).await;
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "Build the hero.", "owner": "frontend" })).await;
    assert!(!made.is_error, "{made:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    idle_on_its_ticket(&handle, 2).await;
    let worktree = ticket(&handle, 1).await.worktree.expect("its own worktree");
    std::fs::write(worktree.join("README.md"), "hello\nhalf-done change\n").unwrap();
    std::fs::write(worktree.join("notes.txt"), "untracked notes\n").unwrap();
    (handle, events, worktree)
}

/// TP-1, TP-3, TP-4, TP-5, TP-8, TP-10, TP-11, TP-12.
#[tokio::test]
async fn a_run_that_ended_without_a_hand_in_stalls_after_its_lease_and_the_person_resumes_it_in_place() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events, worktree) = idle_frontend(&engine, &home, &repo).await;
    let before = handle.snapshot("acme").await;
    let first = ticket(&handle, 1).await;
    let first_run = execution(&first);
    assert_eq!((first_run.state, first_run.generation), (ExecutionState::Active, 1));
    // Its member idle while the ticket says active: every screen reads paused.
    assert_eq!(run_seen(&before, 1), Some(RunView::Paused));
    let disk = checkout(&worktree);
    assert_eq!(git(&worktree, &["status", "--porcelain"]).lines().count(), 2, "one changed, one untracked");

    // Within its lease, nothing is called stale.
    handle.check_leases_at(now() + 10).await;
    assert_eq!(execution(&ticket(&handle, 1).await).state, ExecutionState::Active);
    // Past it, with no turn running it: stalled — and only that.
    handle.check_leases_at(now() + 1_000).await;
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled && x.dirty.is_some()))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::WorkEnded { of: Some(2), ticket: 1, .. })), "its member lets go: {seen:#?}");
    let stalled = ticket(&handle, 1).await;
    let run = execution(&stalled);
    assert_eq!((stalled.status, stalled.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend")), "the same ticket, in progress");
    assert_eq!(run.reason.as_deref(), Some(stall_reason::LEASE_EXPIRED));
    assert_eq!(run.head.as_deref(), Some(git(&worktree, &["rev-parse", "--short", "HEAD"]).as_str()), "the card shows the commit kept");
    assert_eq!(run.dirty, Some(2), "and the uncommitted files kept");
    let after_stall = handle.snapshot("acme").await;
    assert_eq!(run_seen(&after_stall, 1), Some(RunView::Stalled), "board and sidebar agree");
    assert!(after_stall.sessions.iter().any(|s| s.session == 2 && s.work.is_none()), "the sidebar shows it free");
    assert!(!after_stall.sessions.iter().any(|s| s.run.is_some_and(|r| r.of == 2)), "its run is over");
    assert_eq!(checkout(&worktree), disk, "nothing committed, reset or cleaned by the stall");
    let told = |e: &Event| matches!(e, Event::Notice { text, .. } if text.contains("T-1's run stopped without a hand-in"));
    if !seen.iter().any(told) {
        until(&mut events, told).await;
    }

    // The person resumes it: the same member, a fresh runtime session, at once.
    let runs_before = engine.runs().len();
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1:a".into(), how: Recovery::Resume });
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    let prompt = engine.prompt_with("brought it back").await;
    assert!(prompt.contains("Same branch and worktree, nothing reset, committed or cleaned"), "{prompt}");
    for _ in 0..100 {
        if engine.runs().len() > runs_before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let runs = engine.runs();
    let (old, new) = (&runs[runs_before - 1], runs.last().unwrap());
    assert_ne!(old.0, new.0, "a new engine process, not a no-op");
    assert_eq!(new.1, worktree.canonicalize().unwrap(), "in the same worktree");
    let resumed = ticket(&handle, 1).await;
    let run = execution(&resumed);
    assert_eq!((run.state, run.generation), (ExecutionState::Active, 2), "a new generation");
    assert_ne!(run.session, first_run.session, "a new runtime session on the same member and engine");
    assert_eq!((resumed.id, resumed.branch.clone(), resumed.worktree.clone()), (1, first.branch.clone(), first.worktree.clone()));
    assert_eq!(checkout(&worktree), disk, "the commit and every file as they were");

    // The same approval again — a double click, a retry: one recovery.
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1:a".into(), how: Recovery::Resume });
    let again = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("already brought back"))).await;
    assert!(!again.iter().any(|e| matches!(e, Event::WorkStarted { .. })), "{again:#?}");
    assert_eq!(execution(&ticket(&handle, 1).await).generation, 2);
    assert_eq!(ids(&handle.snapshot("acme").await), ids(&before), "no recovery ticket");
    handle.shutdown().await;
}

/// TP-8: a turn still running is never stalled, however long it takes.
#[tokio::test]
async fn a_turn_still_running_is_never_stalled_however_late_the_check() {
    let engine = Engine0::new();
    engine.hang(true);
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Long build", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    engine.prompt_with("You're on T-1").await;
    handle.check_leases_at(now() + 100_000).await;
    let later = within(&mut events, Duration::from_millis(500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::WorkEnded { .. })), "{later:#?}");
    let t = ticket(&handle, 1).await;
    assert_eq!(execution(&t).state, ExecutionState::Active);
    assert_eq!(run_seen(&handle.snapshot("acme").await, 1), Some(RunView::Active));
    handle.shutdown().await;
}

/// TP-9: the PM can't bring stalled work back on its own, by any route; the
/// run stays stalled until the person's command.
#[tokio::test]
async fn the_pm_cannot_force_a_requeue_and_nothing_changes_without_the_person() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events, _worktree) = idle_frontend(&engine, &home, &repo).await;
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    let stalled = ticket(&handle, 1).await;

    // A recovery is the person's command only: it never passes the door from a member's process.
    assert!(equipe_core::permissions::person_only(&Command::Requeue { squad: "acme".into(), ticket: 1, operation: "x".into(), how: Recovery::Requeue }));
    // What the door does with one that came from the PM's own process: refuses, and tells the person.
    handle.refused_person_only(Some("acme".into()));
    // Through its tools: moving work in progress is the person's.
    let forced = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "status": "todo" })).await;
    assert!(forced.is_error && forced.text.contains("the person's to change"), "{forced:?}");
    let later = within(&mut events, Duration::from_millis(800)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::WorkStarted { .. })), "nothing started: {later:#?}");
    assert_eq!(ticket(&handle, 1).await, stalled, "the ticket and its run unchanged");
    handle.shutdown().await;
}

/// TP-6: work the PM sent back from review, stalled before it was handed in
/// again, comes back with that latest reason and its open blockers first —
/// not with the older hand-in, and not with a note the recovery wrote over it.
#[tokio::test]
async fn a_requeue_after_a_review_send_back_leads_with_the_latest_reason_and_blockers() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "Build the hero.", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done and complete.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    let back = tool(&handle, "pm", "comment", json!({ "id": 1, "text": "The header overlaps the logo at 760px.", "review": "changes" })).await;
    assert!(!back.is_error, "{back:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    idle_on_its_ticket(&handle, 2).await;
    // A blocker the person left open, still on the ticket.
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let mut open = journal.ticket(1).unwrap().unwrap();
    open.remaining = vec!["The staging API token".into()];
    journal.save_ticket(&open).unwrap();
    let note = open.note.clone().expect("the send-back note");
    assert!(note.starts_with("Reopened by the PM: "), "{note}");

    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    let prompts_before = engine.prompts().len();
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1:b".into(), how: Recovery::Requeue });
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    let prompt = engine.prompt_with("brought it back").await;
    assert!(engine.prompts().len() > prompts_before);
    let reason = prompt.find("The PM reviewed your hand-in and sent it back: The header overlaps the logo at 760px.").expect(&prompt);
    let blockers = prompt.find("The latest unresolved blockers take priority:\n- The staging API token").expect(&prompt);
    let recovered = prompt.find("brought it back").unwrap();
    assert!(reason < blockers && blockers < recovered, "the reason, then the blockers, then the recovery: {prompt}");
    assert!(prompt.contains("supersedes any older message saying the work was complete"), "{prompt}");
    assert!(!prompt.contains("Requeued:"), "{prompt}");
    assert_eq!(ticket(&handle, 1).await.note.as_deref(), Some(note.as_str()), "the note is kept");
    handle.shutdown().await;
}

/// TP-7: an approved add that unblocks stalled work hands that ticket to the
/// new member — owner and recovery together — on the same branch and
/// worktree; work a member still on the team owns by name is refused before
/// it reaches the person.
#[tokio::test]
async fn an_approved_add_hands_its_roles_stalled_ticket_to_the_new_member_but_never_a_members_own() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events, worktree) = idle_frontend(&engine, &home, &repo).await;
    let disk = checkout(&worktree);
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    let stalled = ticket(&handle, 1).await;
    let before = ids(&handle.snapshot("acme").await);

    // Owned by name by frontend-2, still on the team: a new frontend would never take it.
    add(&handle, &mut events, 3, member(&repo, "frontend", Some(Engine::Claude))).await;
    let own = tool(&handle, "pm", "create_ticket", json!({ "title": "Its own", "body": "-", "owner": "frontend-2" })).await;
    assert!(!own.is_error, "{own:?}");
    let refused = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "frontend", "reason": "T-2 waits", "unblocks": [2], "asked": true })).await;
    assert!(refused.is_error && refused.text.contains("T-2 is owned by frontend-2 by name"), "{refused:?}");
    assert!(handle.snapshot("acme").await.team_changes.is_empty(), "nothing waits for the person");

    // The role's stalled work: proposed, approved, handed over.
    let said = tool(&handle, "pm", "propose_team_change", json!({ "change": "add", "role": "frontend", "reason": "T-1 stalled", "unblocks": [1], "asked": true })).await;
    assert!(!said.is_error, "{said:?}");
    let change = handle.snapshot("acme").await.team_changes.first().cloned().expect("waits for the person");
    handle.send(Command::ApproveTeamChange { squad: "acme".into(), id: change.id });
    let seen = until(&mut events, |e| matches!(e, Event::TeamChangeClosed { .. })).await;
    let joined = seen
        .iter()
        .find_map(|e| match e {
            Event::SessionStarted { session, member: Some(m), .. } if m.name == "frontend-3" => Some(*session),
            _ => None,
        })
        .expect("frontend-3 joined");
    assert!(seen.iter().any(|e| matches!(e, Event::WorkStarted { of: Some(session), ticket: 1, .. } if *session == joined)), "it takes T-1 at once: {seen:#?}");
    let handed = ticket(&handle, 1).await;
    assert_eq!((handed.owner.as_deref(), handed.assignee.as_deref()), (Some("frontend-3"), Some("frontend-3")));
    assert_eq!((handed.branch.clone(), handed.worktree.clone()), (stalled.branch.clone(), stalled.worktree.clone()), "the same branch and worktree");
    assert_eq!((execution(&handed).state, execution(&handed).generation), (ExecutionState::Active, 2));
    assert_eq!(checkout(&worktree), disk);
    let mut after = ids(&handle.snapshot("acme").await);
    after.retain(|id| *id != 2);
    assert_eq!(after, before, "no recovery ticket");
    handle.shutdown().await;
}

/// TP-2: a daemon restart carries on the runs it can and stalls only the
/// dead one — here a Codex member with no Codex CLI to resume on.
#[tokio::test]
async fn a_restart_carries_on_what_it_can_and_stalls_only_what_nothing_runs() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    add(&handle, &mut events, 3, member(&repo, "qa", Some(Engine::Codex))).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Check the release", "body": "-", "owner": "qa" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    idle_on_its_ticket(&handle, 2).await;
    idle_on_its_ticket(&handle, 3).await;
    let before = handle.snapshot("acme").await;
    for id in [1, 2] {
        assert_eq!(execution(&ticket(&handle, id).await).state, ExecutionState::Active);
    }
    handle.shutdown().await;
    drop(events);

    let (handle, _events) = runtime::start(config(&engine.bin, &home));
    engine.prompt_with("equipe restarted while you worked on T-1").await;
    let after = handle.snapshot("acme").await;
    let carried = after.tickets.iter().find(|t| t.id == 1).unwrap();
    let dead = after.tickets.iter().find(|t| t.id == 2).unwrap();
    assert_eq!((execution(carried).state, execution(carried).generation), (ExecutionState::Active, 2), "carried on under a new generation");
    assert_eq!((execution(dead).state, execution(dead).reason.as_deref()), (ExecutionState::Stalled, Some(stall_reason::NOT_RESUMED)));
    assert_eq!((dead.status, dead.assignee.as_deref()), (TicketStatus::InProgress, Some("qa")), "the same ticket, kept");
    let runs_of = |name: &str| -> Vec<u32> { after.sessions.iter().filter(|s| s.run.is_some() && s.member.as_ref().is_some_and(|m| m.name == name)).filter_map(|s| s.work.as_ref().map(|w| w.ticket)).collect() };
    assert_eq!((runs_of("frontend"), runs_of("qa")), (vec![1], vec![]));
    assert_eq!(run_seen(&after, 2), Some(RunView::Stalled));
    assert_eq!(ids(&after), ids(&before), "no recovery ticket");

    // TP-3: the same Codex member and model, resumed — not a no-op: a fresh
    // runtime session and a new turn on the same ticket.
    let mut events = handle_events(&handle).await;
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 2, operation: "card:2:a".into(), how: Recovery::Resume });
    let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 2, .. })).await;
    let session = seen.iter().find_map(|e| if let Event::WorkStarted { session, ticket: 2, .. } = e { Some(*session) } else { None }).unwrap();
    let snapshot = handle.snapshot("acme").await;
    let qa = snapshot.sessions.iter().find(|s| s.session == session).unwrap();
    assert_eq!((qa.member.as_ref().map(|m| m.name.as_str()), qa.engine, qa.model.as_deref()), (Some("qa"), Engine::Codex, Some("gpt-test")));
    let run = execution(snapshot.tickets.iter().find(|t| t.id == 2).unwrap());
    assert_eq!((run.state, run.generation), (ExecutionState::Active, execution(dead).generation + 1));
    assert_ne!(run.session, execution(dead).session);
    handle.shutdown().await;
}

/// A screen attached now: its events from here on.
async fn handle_events(handle: &Handle) -> UnboundedReceiver<Event> {
    handle.subscribe("acme").await.expect("the squad").1
}

/// An approved recovery queued while its member was busy survives a restart
/// and starts exactly once, before the member's unstarted work; replayed
/// after the restart, it changes nothing.
#[tokio::test]
async fn an_approved_recovery_waiting_across_a_restart_starts_exactly_once() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events, worktree) = idle_frontend(&engine, &home, &repo).await;
    let disk = checkout(&worktree);
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    // Free again, the member takes its next tickets — as many as it runs at
    // once ([`equipe_core::lanes::LANES_PER_MEMBER`]) — and is busy with them.
    engine.hang(true);
    for title in ["Footer", "Header", "Sidebar"] {
        tool(&handle, "pm", "create_ticket", json!({ "title": title, "body": "-", "owner": "frontend" })).await;
    }
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 4, .. })).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Later", "body": "-", "owner": "frontend" })).await;
    // Resumed for it while it's busy: queued.
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1:c".into(), how: Recovery::Resume });
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Todo)).await;
    let queued = execution(&ticket(&handle, 1).await);
    assert_eq!(queued.state, ExecutionState::Queued);
    assert_eq!(run_seen(&handle.snapshot("acme").await, 1), Some(RunView::Queued));
    let before = ids(&handle.snapshot("acme").await);
    handle.shutdown().await;
    drop(events);

    // The restart carries T-2 to T-4 on; once their runs stall, the member
    // has room and takes the recovery first — before T-5, which it hasn't started.
    engine.hang(false);
    let (handle, _events) = runtime::start(config(&engine.bin, &home));
    engine.prompt_with("equipe restarted while you worked on T-2").await;
    idle_on_its_ticket(&handle, 2).await;
    let mut screen = handle_events(&handle).await;
    handle.check_leases_at(now() + 1_000).await;
    let seen = until(&mut screen, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 5, .. })), "the recovery goes first: {seen:#?}");
    let run = execution(&ticket(&handle, 1).await);
    assert_eq!((run.state, run.generation, run.operation.as_str()), (ExecutionState::Active, queued.generation, "card:1:c"), "the run it reserved, once");
    assert_eq!(checkout(&worktree), disk);
    // Replayed after the restart: nothing more.
    handle.send(Command::Requeue { squad: "acme".into(), ticket: 1, operation: "card:1:c".into(), how: Recovery::Resume });
    until(&mut screen, |e| matches!(e, Event::Notice { text, .. } if text.contains("already brought back"))).await;
    assert_eq!(execution(&ticket(&handle, 1).await).generation, queued.generation);
    assert_eq!(ids(&handle.snapshot("acme").await), before, "no recovery ticket");
    handle.shutdown().await;
}

/// A ticket as an older equipe, or the board before now, left it: written
/// straight into the squad's journal, its kind a task unless changed.
fn plain(title: &str, status: TicketStatus, owner: &str, assignee: Option<&str>, depends_on: &[u32]) -> Ticket {
    Ticket {
        id: 0,
        title: title.into(),
        body: "-".into(),
        status,
        owner: Some(owner.into()),
        assignee: assignee.map(Into::into),
        project: None,
        depends_on: depends_on.to_vec(),
        branch: None,
        worktree: None,
        execution: None,
        merged: None,
        off_main: None,
        design_qa: None,
        plan: None,
        kind: equipe_core::protocol::TicketKind::Task,
        created_by: "pm".into(),
        note: None,
        asks: vec![],
        remaining: Vec::new(),
        images: Vec::new(),
        updated_at: None,
        priority: Default::default(),
        rank: 0,
    }
}

fn put(home: &Path, ticket: &Ticket) -> u32 {
    Journal::for_squad(home, "acme").unwrap().create_ticket(ticket).unwrap()
}

/// What the PM's screen was told, among `seen`.
fn told(seen: &[Event], wanted: &str) -> bool {
    seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains(wanted)))
}

/// A numbered member closed with work in every state. What it held in
/// progress is requeued for the frontend still on the team, on the same
/// branch and worktree, its commit and files kept; what it handed in stays
/// in review for the role, and a send-back goes to the role; waiting and
/// to-do work loses its name. Finished work, and work naming someone else,
/// stay as they were; the PM hears what moved.
#[tokio::test]
async fn closing_a_numbered_member_hands_what_still_names_it_to_its_role() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    add(&handle, &mut events, 3, member(&repo, "frontend", Some(Engine::Claude))).await;
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "Build the hero.", "owner": "frontend-2" })).await;
    assert!(!made.is_error, "{made:?}");
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(3), ticket: 1, .. })).await;
    idle_on_its_ticket(&handle, 3).await;
    let worktree = ticket(&handle, 1).await.worktree.expect("its own worktree");
    std::fs::write(worktree.join("hero.txt"), "the hero, committed\n").unwrap();
    git(&worktree, &["add", "-A"]);
    git(&worktree, &["commit", "-q", "-m", "hero so far"]);
    std::fs::write(worktree.join("notes.txt"), "untracked notes\n").unwrap();
    let disk = checkout(&worktree);
    handle.check_leases_at(now() + 1_000).await;
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.execution.as_ref().is_some_and(|x| x.state == ExecutionState::Stalled))).await;
    let stalled = ticket(&handle, 1).await;
    assert_eq!((stalled.status, stalled.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend-2")));

    let review = put(&home, &plain("Footer", TicketStatus::Review, "frontend-2", Some("frontend-2"), &[]));
    let waiting = put(&home, &plain("Login", TicketStatus::Waiting, "frontend", Some("frontend-2"), &[]));
    let todo = put(&home, &plain("Menu", TicketStatus::Todo, "frontend-2", None, &[review]));
    let done = put(&home, &plain("Logo", TicketStatus::Done, "frontend-2", Some("frontend-2"), &[]));
    let other = put(&home, &plain("Search", TicketStatus::Todo, "frontend-3", None, &[review]));

    handle.send(Command::Close { session: 3 });
    let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket: 1, .. })).await;
    assert!(told(&seen, "frontend-2 left: T-1, T-2, T-3, T-4 went to frontend"), "{seen:#?}");

    // In progress in its hands: requeued for frontend, nothing lost.
    let taken = ticket(&handle, 1).await;
    let run = execution(&taken);
    assert_eq!((taken.status, taken.owner.as_deref(), taken.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend"), Some("frontend")));
    assert_eq!((taken.branch.clone(), taken.worktree.clone()), (stalled.branch.clone(), stalled.worktree.clone()), "the same branch and worktree");
    assert!(run.operation.starts_with("member-left:frontend-2:1:"), "{run:?}");
    assert_eq!(run.generation, execution(&stalled).generation + 1);
    assert_eq!(checkout(&worktree), disk, "its commit and every file as they were");
    let branch = stalled.branch.clone().expect("a branch");
    assert!(git(&repo, &["log", "--format=%s", &branch]).contains("hero so far"), "the committed work survives");
    engine.prompt_with("brought it back").await;

    // In review: still in review, the role's — accepted or sent back by it.
    let in_review = ticket(&handle, review).await;
    assert_eq!((in_review.status, in_review.owner.as_deref(), in_review.assignee.as_deref()), (TicketStatus::Review, Some("frontend"), None));
    // Waiting, and to do: the name gone, the state kept.
    let w = ticket(&handle, waiting).await;
    assert_eq!((w.status, w.owner.as_deref(), w.assignee.as_deref()), (TicketStatus::Waiting, Some("frontend"), None));
    let t = ticket(&handle, todo).await;
    assert_eq!((t.status, t.owner.as_deref(), t.assignee.as_deref()), (TicketStatus::Todo, Some("frontend"), None));
    // Finished work, and someone else's, untouched.
    let d = ticket(&handle, done).await;
    assert_eq!((d.status, d.owner.as_deref(), d.assignee.as_deref()), (TicketStatus::Done, Some("frontend-2"), Some("frontend-2")));
    assert_eq!(ticket(&handle, other).await.owner.as_deref(), Some("frontend-3"));

    // A send-back from review reaches a member still on the team.
    let back = tool(&handle, "pm", "comment", json!({ "id": review, "text": "The footer overlaps at 760px.", "review": "changes" })).await;
    assert!(!back.is_error && back.text.contains(&format!("Sent T-{review} back to frontend ")), "{back:?}");
    handle.shutdown().await;
}

/// Members closed before equipe handed their work on: at the daemon's
/// start, what still names them goes to their roles the same way — and
/// only once, however often it starts again.
#[tokio::test]
async fn at_start_work_still_naming_a_member_that_left_goes_to_its_role_once() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    add(&handle, &mut events, 3, member(&repo, "frontend", Some(Engine::Claude))).await;
    handle.shutdown().await;
    drop(events);

    // As an older equipe left it: frontend-2 closed, its work still naming it.
    Journal::for_squad(&home, "acme").unwrap().close_session(3).unwrap();
    let review = put(&home, &plain("Footer", TicketStatus::Review, "frontend-2", Some("frontend-2"), &[]));
    let held = put(&home, &Ticket { kind: equipe_core::protocol::TicketKind::Code, project: Some(repo.clone()), ..plain("Hero", TicketStatus::InProgress, "frontend-2", Some("frontend-2"), &[]) });
    let todo = put(&home, &plain("Menu", TicketStatus::Todo, "frontend-2", None, &[review]));
    let waiting = put(&home, &plain("Login", TicketStatus::Waiting, "frontend", Some("frontend-2"), &[]));

    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), ticket, .. } if *ticket == held)).await;
    assert!(told(&seen, "frontend-2 left: T-2, T-1, T-3, T-4 went to frontend"), "{seen:#?}");
    let h = ticket(&handle, held).await;
    assert_eq!((h.status, h.owner.as_deref(), h.assignee.as_deref()), (TicketStatus::InProgress, Some("frontend"), Some("frontend")));
    let r = ticket(&handle, review).await;
    assert_eq!((r.status, r.owner.as_deref(), r.assignee.as_deref()), (TicketStatus::Review, Some("frontend"), None));
    let t = ticket(&handle, todo).await;
    assert_eq!((t.status, t.owner.as_deref(), t.assignee.as_deref()), (TicketStatus::Todo, Some("frontend"), None));
    let w = ticket(&handle, waiting).await;
    assert_eq!((w.status, w.owner.as_deref(), w.assignee.as_deref()), (TicketStatus::Waiting, Some("frontend"), None));
    handle.shutdown().await;
    drop(events);

    // Started again: nothing names frontend-2 any more, so nothing moves.
    let record = |id: u32| Journal::for_squad(&home, "acme").unwrap().ticket_history(id).unwrap();
    let before: Vec<_> = [review, held, todo, waiting].map(record).to_vec();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    let later = within(&mut events, Duration::from_millis(800)).await;
    assert!(!told(&later, "went to frontend"), "{later:#?}");
    for (i, id) in [review, held, todo, waiting].into_iter().enumerate() {
        let moved = |h: &[(String, String, String)]| h.iter().filter(|(_, _, text)| text.contains("frontend-2 left the squad")).count();
        assert_eq!(moved(&record(id)), moved(&before[i]), "T-{id} moved once");
        assert_eq!(moved(&before[i]), 1, "T-{id} moved");
    }
    handle.shutdown().await;
}

/// With no member of its role left, what a member that left still names
/// stays as it is, and the PM is told.
#[tokio::test]
async fn nothing_moves_when_no_member_of_the_role_is_left() {
    let engine = Engine0::new();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&engine.bin, &home));
    add(&handle, &mut events, 1, member(&repo, "pm", None)).await;
    add(&handle, &mut events, 2, member(&repo, "frontend", Some(Engine::Claude))).await;
    add(&handle, &mut events, 3, member(&repo, "frontend", Some(Engine::Claude))).await;
    handle.send(Command::Close { session: 2 });
    until(&mut events, |e| matches!(e, Event::SessionClosed { session: 2 })).await;
    let review = put(&home, &plain("Footer", TicketStatus::Review, "frontend-2", Some("frontend-2"), &[]));
    let todo = put(&home, &plain("Menu", TicketStatus::Todo, "frontend-2", None, &[review]));
    let before = [ticket(&handle, review).await, ticket(&handle, todo).await];

    handle.send(Command::Close { session: 3 });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("no frontend is left"))).await;
    assert!(told(&seen, "frontend-2 left and no frontend is left on the team: T-1, T-2 wait for one"), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::TicketChanged { .. })), "{seen:#?}");
    assert_eq!([ticket(&handle, review).await, ticket(&handle, todo).await], before);
    handle.shutdown().await;
}
