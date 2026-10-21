//! Handed-in work the person accepts in words, driven the way a screen and an
//! engine drive it: the PM's `propose_accept` (or a screen's reading of the
//! person's own words) in, the card out as an event — and nothing accepted
//! until the person's own command confirms it, which runs what the board's
//! buttons send. No tool accepts or merges, and nothing in the daemon does but
//! the person's commands.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::accept::{AcceptProposal, Confirmed, FROM_WORDS, Land, Verdict};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
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
    dir.canonicalize().unwrap()
}

/// Write an executable script without holding it open for writing (see tests/runtime.rs).
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
    let bin = scratch("bin").join("claude");
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\ncase \"$*\" in *\"You're on T-\"*) sleep 1; echo \"made by the squad\" > HERO.md;; esac\n{answer}\n"
    );
    write_script(&bin, &script);
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
        codex_binary: "/nonexistent/codex".into(),
        persist: false,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::En,
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
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
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
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done." })).await;
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
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: card.id, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Merge) }] });
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::AcceptClosed { id, confirmed: true, tickets, .. } if *id == card.id && tickets == &[1])), "{seen:#?}");
    assert_eq!(git(&repo, &["show", "main:HERO.md"]), "made by the squad", "on main");
    // And T-2, waiting on it, started as soon as it was accepted.
    assert!(seen.iter().any(|e| matches!(e, Event::WorkStarted { ticket: 2, .. })), "{seen:#?}");
    assert!(handle.snapshot("acme").await.accepts.is_empty());
    // Confirmed again from another screen: nothing more.
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: card.id, tickets: vec![Confirmed::only(1)] });
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

    // The person keeps acceptance for themselves: the PM only puts it on a card.
    handle.send(Command::SetPmAccepts { squad: "acme".into(), on: false });
    until(&mut events, |e| matches!(e, Event::PmAcceptsChanged { on: false, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Pricing done." })).await;
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
    handle.send(Command::ConfirmAccept { squad: "acme".into(), id: back.id, tickets: vec![Confirmed::only(1)] });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Done);
    handle.shutdown().await;
}

/// The guarantee, in the code itself: the daemon accepts a ticket only for
/// [`Command::Accept`] (or the person's own move to done on the board), merges
/// only for [`Command::Merge`], sends back only for [`Command::Reopen`] (or
/// their move back) — and the only other place those commands are sent is the
/// person's [`Command::ConfirmAccept`]. No tool, and no reading of anyone's
/// words, reaches any of it.
#[test]
fn nothing_accepts_or_merges_but_the_persons_commands() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("runtime.rs")];
    files.extend(std::fs::read_dir(src.join("runtime")).unwrap().flatten().map(|e| e.path()));
    files.sort();
    let watched = ["self.accept(", "self.merge(", "self.reopen(", "end_work(id, WorkEnd::Accepted)", "TicketStatus::Done)", "Command::Accept {", "Command::Merge {", "Command::Reopen {", "confirm_accept("];
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
            "accept.rs: Box::pin(self.accept(squad, ticket)).await;",
            "accept.rs: self.merge(squad, ticket, push);",
            "accept.rs: pub(super) async fn confirm_accept(&mut self, squad: &str, id: u64, picked: Vec<Confirmed>) {",
            "accept.rs: Box::pin(self.handle(Command::Reopen { squad, ticket, note: proposal.note.clone() })).await;",
            "accept.rs: Box::pin(self.handle(Command::Accept { squad: squad.clone(), ticket })).await;",
            "accept.rs: Box::pin(self.handle(Command::Merge { squad, ticket, push: land == Land::Push })).await;",
            "runtime.rs: Command::Accept { squad, ticket } => self.accept(&squad, ticket).await,",
            "runtime.rs: Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),",
            "runtime.rs: Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,",
            "runtime.rs: Command::ConfirmAccept { squad, id, tickets } => self.confirm_accept(&squad, id, tickets).await,",
            "runtime.rs: self.end_work(id, WorkEnd::Accepted);",
            "runtime.rs: None => self.close_ticket(squad, found, TicketStatus::Done),",
            "tickets.rs: board::Move::Accept => self.accept(squad, id).await,",
            "tickets.rs: board::Move::SendBack => self.reopen(squad, id, String::new()).await,",
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
