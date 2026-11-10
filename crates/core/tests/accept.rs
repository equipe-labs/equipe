//! Handed-in work the person accepts in words, driven the way a screen and an
//! engine drive it: the PM's `propose_accept` (or a screen's reading of the
//! person's own words) in, the card out as an event — and nothing accepted
//! until the person's own command confirms it, which runs what the board's
//! buttons send. No tool accepts or merges, and nothing in the daemon does but
//! the person's commands.

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::accept::{AcceptProposal, Confirmed, FROM_WORDS, Land, Verdict};
use equipe_core::protocol::rpc::{RunKey, ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-accept-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Windows canonicalization adds the verbatim `\\?\` prefix, which Git for
    // Windows does not accept as a `worktree add` destination.
    #[cfg(windows)]
    return dir;
    #[cfg(not(windows))]
    dir.canonicalize().unwrap()
}

/// Write an executable script without holding it open for writing (see tests/runtime.rs).
#[cfg(unix)]
fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let draft = path.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(path).status().unwrap().success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude`, signed in: a ticket's turn takes a moment and writes
/// `HERO.md` where it runs; every turn answers in a line.
fn claude() -> PathBuf {
    #[cfg(unix)]
    let bin = scratch("bin").join("claude");
    #[cfg(windows)]
    let bin = scratch("bin").join("claude.exe");
    #[cfg(unix)]
    {
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\ncase \"$*\" in *\"You're on T-\"*) sleep 1; echo \"made by the squad\" > HERO.md;; esac\n{answer}\n"
    );
    write_script(&bin, &script);
    }
    #[cfg(windows)]
    {
        // A native fake keeps TP-5 independent of cmd.exe quoting. The real
        // npm `.cmd` launchers are exercised separately by the Windows job.
        let source = bin.with_extension("rs");
        std::fs::write(
            &source,
            r##"fn main() {
    if std::env::args().nth(1).as_deref() == Some("auth") {
        println!(r#"{{"loggedIn":true,"authMethod":"claude.ai"}}"#);
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    if std::fs::metadata(".git").is_ok_and(|m| m.is_file()) {
        std::fs::write("HERO.md", "made by the squad\n").unwrap();
    }
    println!(r#"{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}"#);
    println!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}"#);
    println!(r#"{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":3,"output_tokens":1}}}}"#);
}
"##,
        )
        .unwrap();
        let built = Process::new("rustc").args(["--edition=2024", "-o"]).arg(&bin).arg(&source).status().unwrap();
        assert!(built.success(), "build the Windows fake engine");
    }
    bin
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

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        // No Codex CLI either: the person's own must never run in a test.
        codex_binary: scratch("missing").join("codex-does-not-exist").to_string_lossy().into_owned(),
        persist: false,
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

fn member(cwd: &Path, role: &str) -> SessionSpec {
    SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// A tool call from the run of `ticket`, as its board server makes it.
async fn tool_on(handle: &Handle, member: &str, ticket: u32, name: &str, arguments: serde_json::Value) -> ToolResult {
    let snapshot = handle.snapshot("acme").await;
    let run = snapshot.sessions.iter().find_map(|s| s.run.filter(|r| r.ticket == ticket).map(|r| RunKey { session: s.session, ticket, generation: r.generation }));
    handle.tool(ToolCall { run, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

fn proposed(events: &[Event]) -> AcceptProposal {
    events.iter().find_map(|e| if let Event::AcceptProposed { proposal, .. } = e { Some(proposal.clone()) } else { None }).expect("proposed")
}

async fn status(handle: &Handle, id: u32) -> TicketStatus {
    handle.snapshot("acme").await.tickets.iter().find(|t| t.id == id).map(|t| t.status).expect("on the board")
}

/// A squad of a PM and a frontend on one repository: T-1 handed in, in
/// review, and T-2 waiting on it.
async fn handed_in() -> (Handle, UnboundedReceiver<Event>, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (handle, mut events) = runtime::start(config(&claude(), &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Pricing", "body": "-", "owner": "frontend", "depends_on": [1] })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    (handle, events, repo)
}

#[tokio::test]
async fn the_pm_puts_it_on_a_card_and_only_the_persons_confirmation_accepts_and_merges() {
    let (handle, mut events, repo) = handed_in().await;

    // A member doesn't put acceptance to the person: the tool is the PM's.
    let refused = tool(&handle, "frontend", "propose_accept", json!({ "verdict": "accept", "tickets": [1] })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{refused:?}");
    // Checked before anything waits.
    let odd = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [99] })).await;
    assert!(odd.is_error && odd.text.contains("there is no T-99"), "{odd:?}");

    // "전부 수락, 머지까지": the card goes up, T-2 (not handed in) left out, said why.
    let said = tool(&handle, "pm", "propose_accept", json!({ "verdict": "merge", "tickets": [1, 2, 1], "summary": "Hero" })).await;
    assert!(!said.is_error && said.text.starts_with("Proposed — the person confirms on a card: accepting and merging T-1. Confirmed, T-2 can start."), "{said:?}");
    assert!(said.text.contains("Left out: T-2 is todo, not handed in yet.") && said.text.contains("Don't explain buttons"), "{said:?}");
    let card = proposed(&until(&mut events, |e| matches!(e, Event::AcceptProposed { .. })).await);
    assert_eq!((card.verdict, card.tickets.as_slice(), card.by.as_str(), card.session), (Verdict::Merge, &[1][..], "pm", 1));
    // Proposed is not accepted: still in review, and a screen attaching sees the card.
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    assert_eq!(handle.snapshot("acme").await.accepts, std::slice::from_ref(&card));
    // The same again, from the PM or from the person's words on a screen: one card.
    let again = tool(&handle, "pm", "propose_accept", json!({ "verdict": "merge", "all": true })).await;
    assert!(!again.is_error && again.text.starts_with("Already on a card in front of the person: T-1"), "{again:?}");
    handle.send(Command::ProposeAccept { squad: "acme".into(), session: 1, verdict: Verdict::Merge, tickets: vec![1], note: String::new() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(handle.snapshot("acme").await.accepts.len(), 1);

    // The person confirms, merging as their screen's way says: what A and the merge send.
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: card.id, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Merge) }], said: None });
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::AcceptClosed { id, confirmed: true, tickets, .. } if *id == card.id && tickets == &[1])), "{seen:#?}");
    assert_eq!(git(&repo, &["show", "main:HERO.md"]), "made by the squad", "on main");
    // And T-2, waiting on it, started as soon as it was accepted.
    assert!(seen.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 2, .. })), "{seen:#?}");
    assert!(handle.snapshot("acme").await.accepts.is_empty());
    // Confirmed again from another screen: nothing more.
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: card.id, tickets: vec![Confirmed::only(1)], said: None });
    handle.send(Command::DeclineAccept { squad: "acme".into(), id: card.id });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Done);
    handle.shutdown().await;
}

#[tokio::test]
async fn the_pm_accepts_its_teams_work_itself_unless_the_person_keeps_that_for_themselves() {
    let (handle, mut events, repo) = handed_in().await;
    // A member doesn't accept its own or a teammate's work.
    let refused = tool(&handle, "frontend", "accept_work", json!({ "tickets": [1], "checked": "looks fine" })).await;
    assert!(refused.is_error && refused.text.contains("Only the PM"), "{refused:?}");
    // The PM says what it checked, or nothing is accepted.
    let bare = tool(&handle, "pm", "accept_work", json!({ "tickets": [1] })).await;
    assert!(bare.is_error && bare.text.contains("`checked`"), "{bare:?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);

    // Handed in, the PM is woken to look at it — not left until someone speaks to it.
    let woken = until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. } | Event::TurnEnded { session: 1, .. })).await;
    assert!(woken.iter().any(|e| matches!(e, Event::TurnEnded { session: 1, .. })), "the PM took a turn on it: {woken:#?}");
    // Checked and accepted: done, merged into main, and what waited on it starts.
    let ok = tool(&handle, "pm", "accept_work", json!({ "tickets": [1, 2], "checked": "Read the diff: the hero renders, nothing else touched." })).await;
    assert!(!ok.is_error && ok.text.contains("Accepted T-1") && ok.text.contains("T-2"), "{ok:?}");
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert_eq!(git(&repo, &["show", "main:HERO.md"]), "made by the squad", "on main");
    assert!(seen.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 2, .. })), "{seen:#?}");
    assert!(handle.snapshot("acme").await.pm_accepts, "the PM accepts unless the person says otherwise");

    // A member's proposal is simply done: on the board, its own, no sorting.
    let put = tool(&handle, "frontend", "create_ticket", json!({ "title": "Footer links", "body": "-" })).await;
    assert!(!put.is_error && put.text.contains("is on the board"), "{put:?}");
    let footer = handle.snapshot("acme").await.tickets.into_iter().find(|t| t.title == "Footer links").unwrap();
    assert_eq!((footer.status == TicketStatus::Proposed, footer.owner.as_deref()), (false, Some("frontend")));
    // The person keeps acceptance for themselves: the PM only puts it on a card.
    handle.send(Command::SetPmAccepts { squad: "acme".into(), on: false });
    until(&mut events, |e| matches!(e, Event::PmAcceptsChanged { on: false, .. })).await;
    // Its own proposal runs beside T-2 now: the hand-in says which run it is.
    let handed = tool_on(&handle, "frontend", 2, "submit_for_review", json!({ "summary": "Pricing done.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 2 && ticket.status == TicketStatus::Review)).await;
    let kept = tool(&handle, "pm", "accept_work", json!({ "tickets": [2], "checked": "fine" })).await;
    assert!(kept.is_error && kept.text.contains("propose_accept"), "{kept:?}");
    assert_eq!(status(&handle, 2).await, TicketStatus::Review);
    handle.shutdown().await;
}

#[tokio::test]
async fn the_persons_words_raise_the_card_a_no_keeps_it_as_it_is_and_the_board_settles_it() {
    let (handle, mut events, _repo) = handed_in().await;

    // A screen read "T-1 수락" in the person's message: the card, in that conversation, at once.
    handle.send(Command::ProposeAccept { squad: "acme".into(), session: 1, verdict: Verdict::Accept, tickets: vec![1], note: String::new() });
    let card = proposed(&until(&mut events, |e| matches!(e, Event::AcceptProposed { .. })).await);
    assert_eq!((card.by.as_str(), card.session, card.tickets.as_slice()), (FROM_WORDS, 1, &[1][..]));
    // The PM, hearing the same words a moment later, raises nothing twice.
    let pm = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [1] })).await;
    assert!(!pm.is_error && pm.text.starts_with("Already on a card"), "{pm:?}");

    // The person says no: nothing changes, the PM hears it, and it doesn't come back at once.
    handle.send(Command::DeclineAccept { squad: "acme".into(), id: card.id });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::AcceptClosed { id, confirmed: false, .. } if *id == card.id)), "{seen:#?}");
    assert!(matches!(seen.last(), Some(Event::Notice { text, .. }) if text == "You didn't accept T-1 — it stays in review"), "{seen:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    let again = tool(&handle, "pm", "propose_accept", json!({ "verdict": "accept", "tickets": [1] })).await;
    assert!(again.is_error && again.text.contains("said no to that very card"), "{again:?}");

    // Sending back needs the person's words; then accepted on the board instead, the card is moot.
    let bare = tool(&handle, "pm", "propose_accept", json!({ "verdict": "changes", "tickets": [1] })).await;
    assert!(bare.is_error && bare.text.contains("`note`"), "{bare:?}");
    tool(&handle, "pm", "propose_accept", json!({ "verdict": "changes", "tickets": [1], "note": "Bigger button" })).await;
    let back = proposed(&until(&mut events, |e| matches!(e, Event::AcceptProposed { .. })).await);
    handle.send(Command::Accept { squad: "acme".into(), ticket: 1 });
    let seen = until(&mut events, |e| matches!(e, Event::AcceptClosed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::AcceptClosed { id, confirmed: false, .. }) if *id == back.id), "{seen:#?}");
    // Confirmed late, from a screen that hadn't heard: nothing is sent back.
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: back.id, tickets: vec![Confirmed::only(1)], said: None });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Done);
    handle.shutdown().await;
}

// ── the project's checks, run on the merged result before the PM's acceptance lands ──

/// [`handed_in`], on a repository whose main declares `checks` in
/// `.equipe/checks.json`.
async fn handed_in_checked(checks: &str) -> (Handle, UnboundedReceiver<Event>, PathBuf) {
    let repo = repo();
    std::fs::create_dir_all(repo.join(".equipe")).unwrap();
    std::fs::write(repo.join(".equipe/checks.json"), checks).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "declare the checks"]);
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (handle, mut events) = runtime::start(config(&claude(), &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done, tests pass.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    // Main moves on meanwhile: the branch is behind it, as T-578's was.
    std::fs::write(repo.join("LATER.md"), "later\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "later on main"]);
    (handle, events, repo)
}

fn on_main(repo: &Path, file: &str) -> bool {
    Process::new("git").arg("-C").arg(repo).args(["cat-file", "-e", &format!("main:{file}")]).stderr(std::process::Stdio::null()).status().unwrap().success()
}

#[cfg(unix)]
#[tokio::test]
async fn checks_that_pass_on_the_merged_result_accept_and_merge_it() {
    // The command sees the merge: the branch's file and main's later one together.
    let (handle, mut events, repo) = handed_in_checked(r#"{ "commands": ["test -f HERO.md && test -f LATER.md"] }"#).await;
    let main_before = git(&repo, &["rev-parse", "main"]);
    let ok = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Read the diff." })).await;
    assert!(!ok.is_error && ok.text.starts_with("Checking T-1 first"), "{ok:?}");
    // Not accepted on the PM's word: in review until the checks pass.
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    let again = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "again" })).await;
    assert!(again.text.contains("checks are running already") || again.text.contains("accepted already"), "{again:?}");
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("once the project's checks passed"))), "{seen:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Done);
    assert!(on_main(&repo, "HERO.md") && on_main(&repo, "LATER.md"));
    // What landed is the merge that was checked: main's old tip and the branch, as its parents.
    assert_eq!(git(&repo, &["rev-parse", "main^1"]), main_before);
    handle.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn failing_checks_send_it_back_with_the_output_and_leave_main_untouched() {
    let (handle, mut events, repo) = handed_in_checked(r#"{ "commands": ["echo the hero breaks the build; test ! -f HERO.md"] }"#).await;
    let main_before = git(&repo, &["rev-parse", "main"]);
    let ok = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Looks right." })).await;
    assert!(!ok.is_error && ok.text.contains("Checking T-1"), "{ok:?}");
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Todo)).await;
    let back = seen.iter().rev().find_map(|e| if let Event::TicketChanged { ticket, .. } = e { (ticket.id == 1).then(|| ticket.clone()) } else { None }).unwrap();
    let note = back.note.unwrap_or_default();
    assert!(note.starts_with("Reopened by the PM: equipe ran the project's checks"), "{note}");
    assert!(note.contains("test ! -f HERO.md") && note.contains("exit code 1") && note.contains("the hero breaks the build"), "{note}");
    assert_eq!(back.owner.as_deref(), Some("frontend"), "back to its member");
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before, "main untouched");
    assert!(!on_main(&repo, "HERO.md"));
    handle.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn main_failing_its_own_checks_keeps_the_work_in_review_rather_than_blaming_it() {
    let (handle, mut events, repo) = handed_in_checked(r#"{ "commands": ["false"] }"#).await;
    let main_before = git(&repo, &["rev-parse", "main"]);
    tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Fine." })).await;
    let seen = until(&mut events, |e| matches!(e, Event::Error { message, .. } if message.contains("Main fails the project's own checks"))).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status != TicketStatus::Review)), "{seen:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_checks_file_that_cant_be_read_merges_nothing() {
    let (handle, _events, repo) = handed_in_checked(r#"{ "command": ["typo"] }"#).await;
    let main_before = git(&repo, &["rev-parse", "main"]);
    let held = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Fine." })).await;
    assert!(held.text.starts_with("Nothing accepted.") && held.text.contains(".equipe/checks.json isn't valid"), "{held:?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
    handle.shutdown().await;
}

#[tokio::test]
async fn with_no_checks_declared_it_merges_as_before_and_says_none_ran() {
    let (handle, mut events, repo) = handed_in().await;
    let ok = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Read the diff." })).await;
    assert!(!ok.is_error && ok.text.starts_with("Accepted T-1") && ok.text.contains("No project checks ran for T-1"), "{ok:?}");
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { text, .. } if text.contains("No project checks ran") && text.contains(".equipe/checks.json"))), "{seen:#?}");
    assert!(on_main(&repo, "HERO.md"));
    handle.shutdown().await;
}

/// The guarantee, in the code itself: the daemon accepts a ticket only for
/// [`Command::Accept`] (or the person's own move to done on the board), merges
/// only for [`Command::Merge`], sends back only for [`Command::Reopen`] (or
/// their move back) — and the only other places those commands are sent are
/// the person's [`Command::ConfirmAccept`] and the person's own words in a
/// [`Command::Say`] (`by_words`: low risk after its undo window, the rest only
/// read back and confirmed by their next own message, in "all by words"). No
/// tool, and no reading of a member's words, reaches any of it.
#[test]
fn nothing_accepts_or_merges_but_the_persons_commands() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("runtime.rs"), src.join("by_words.rs")];
    files.extend(std::fs::read_dir(src.join("runtime")).unwrap().flatten().map(|e| e.path()));
    files.sort();
    let watched = ["self.accept(", "self.merge(", "worktree::land(", "self.reopen(", "end_work(id, WorkEnd::Accepted)", "TicketStatus::Done)", "Command::Accept {", "Command::Merge {", "Command::Reopen {", "confirm_accept("];
    let mut found: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        // Only the daemon's own code: its tests send what screens send.
        let text = text.split("#[cfg(test)]").next().unwrap_or_default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            if watched.iter().any(|w| line.contains(w)) {
                found.push(format!("{name}: {line}"));
            }
        }
    }
    found.sort();
    let mut expected = vec![
            // The PM's `accept_work`, only while the person lets the PM accept (`SetPmAccepts`).
            "accept.rs: Box::pin(self.accept(squad, ticket, crate::telemetry::Accepter::Pm)).await;",
            "accept.rs: self.merge(squad, ticket, push);",
            // The same, once the project's checks passed on the very merge that lands (`gate.rs`).
            "gate.rs: Box::pin(self.accept(squad, ticket, crate::telemetry::Accepter::Pm)).await;",
            "gate.rs: None => self.merge(squad, ticket, gate.push),",
            "gate.rs: let result = worktree::land(&repo, &merge).map(|main| (main, push.then(|| worktree::push_main(&repo).map(|_| ()))));",
            "accept.rs: pub(super) async fn confirm_accept(&mut self, squad: &str, id: u64, picked: Vec<Confirmed>, said: Option<equipe_protocol::by_words::Said>) {",
            "accept.rs: Box::pin(self.handle(Command::Reopen { squad, ticket, note: proposal.note.clone() })).await;",
            "accept.rs: Box::pin(self.handle(Command::Accept { squad: squad.clone(), ticket })).await;",
            "accept.rs: Box::pin(self.handle(Command::Merge { squad, ticket, push: land == Land::Push })).await;",
            // The PM's `change_setting` merge: always on a card, done by the daemon on the person's tap.
            "controls.rs: Change::Merge { ticket, push } => Command::Merge { squad: squad_name, ticket, push },",
            "runtime.rs: Command::Accept { squad, ticket } => self.accept(&squad, ticket, crate::telemetry::Accepter::Person).await,",
            "runtime.rs: Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),",
            "runtime.rs: Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,",
            "runtime.rs: Command::ConfirmAccept { squad, id, tickets, said } => self.confirm_accept(&squad, id, tickets, said).await,",
            "runtime.rs: self.end_work(id, WorkEnd::Accepted);",
            "runtime.rs: None => self.close_ticket(squad, found, TicketStatus::Done),",
            "tickets.rs: board::Move::Accept => self.accept(squad, id, crate::telemetry::Accepter::Person).await,",
            "tickets.rs: board::Move::SendBack => self.reopen(squad, id, String::new()).await,",
            // The person's own words (`Say`), as their card's buttons would: run by `runtime/words.rs` only.
            "by_words.rs: Verb::Accept => vec![Command::Accept { squad, ticket }],",
            "by_words.rs: out.push(Command::Accept { squad: squad.clone(), ticket });",
            "by_words.rs: out.push(Command::Merge { squad, ticket, push });",
    ];
    expected.sort();
    assert_eq!(found, expected, "accepting, merging and sending back run only on the person's commands — and the PM's accept_work they allow");
    // And no member has a tool that accepts, merges or confirms — the PM accepts only with `accept_work`.
    for is_pm in [true, false] {
        for tool in equipe_core::board::tools(is_pm) {
            let name = tool["name"].as_str().unwrap();
            assert!(!name.contains("merge") && !name.contains("confirm") && (!name.contains("accept") || name == "propose_accept" || (is_pm && name == "accept_work")), "{name}");
        }
    }
}
