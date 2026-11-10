//! Work in review on its way back to its member, driven the way a screen and
//! an engine drive it: the PM's `comment` on handed-in work sends it back or
//! only notes it — said which, never a change request its member doesn't
//! hear; a branch only behind main is accepted and merged, and only a real
//! conflict goes back; and change requests once left as plain comments go
//! back to their members once, when the daemon starts. Nothing here accepts
//! or merges on a member's word.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::Journal;
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
    let dir = std::env::temp_dir().join(format!("equipe-review-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`, signed in: a ticket's turn takes a moment and writes
/// `HERO.md` where it runs, its prompt kept beside it ([`prompts`]); every
/// turn answers in a line.
fn claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let log = prompts(&bin);
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\nif [ \"$2\" = --input-format ] && IFS= read -r line; then set -- -p \"$(printf '%s' \"$line\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"message\"][\"content\"], end=\"\")')\" \"$@\"; fi\ncase \"$*\" in *\"You're on T-\"*) printf '%s\\n<<end>>\\n' \"$*\" >> '{}'; sleep 1; echo \"made by the squad\" > HERO.md;; esac\n{answer}\n",
        log.display()
    );
    let draft = bin.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
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

/// A commit on main in the person's checkout: main moves on.
fn commit_on_main(repo: &Path, file: &str, text: &str) {
    std::fs::write(repo.join(file), text).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", &format!("main: {file}")]);
}

fn config(claude: &Path, home: &Path, persist: bool) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: scratch("missing").join("codex-does-not-exist").to_string_lossy().into_owned(),
        persist,
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

async fn status(handle: &Handle, id: u32) -> TicketStatus {
    handle.snapshot("acme").await.tickets.iter().find(|t| t.id == id).map(|t| t.status).expect("on the board")
}

/// T-1's record: `(by, kind, text)`, oldest first.
fn record(home: &Path, id: u32) -> Vec<(String, String, String)> {
    Journal::for_squad(home, "acme").unwrap().ticket_history(id).unwrap()
}

fn sent_back(home: &Path, id: u32) -> usize {
    record(home, id).iter().filter(|(_, kind, _)| kind == "sent back").count()
}

/// A squad of a PM and a frontend on one repository, T-1 handed in and in
/// review; with `persist`, what it does is kept for a restart.
async fn handed_in(persist: bool) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let bin = claude();
    let (handle, mut events) = runtime::start(config(&bin, &home, persist));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend" })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done.", "changes_screens": false })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    (handle, events, repo, home, bin)
}

/// Where the stand-in `claude` at `bin` keeps the ticket prompts it was given.
fn prompts(bin: &Path) -> PathBuf {
    bin.with_file_name("prompts.log")
}

/// The ticket prompt `bin` was given that says `wanted`, once it was.
async fn ticket_prompt(bin: &Path, wanted: &str) -> String {
    for _ in 0..300 {
        let log = std::fs::read_to_string(prompts(bin)).unwrap_or_default();
        if let Some(found) = log.split("<<end>>").find(|p| p.contains(wanted)) {
            return found.to_owned();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no ticket prompt said {wanted:?}: {}", std::fs::read_to_string(prompts(bin)).unwrap_or_default());
}

#[tokio::test]
async fn the_pms_change_request_sends_the_work_back_to_its_member_and_a_note_leaves_it_in_review() {
    let (handle, mut events, _repo, home, bin) = handed_in(false).await;

    // Said without `review`: refused, and nothing is noted — the PM must say which.
    let unsaid = tool(&handle, "pm", "comment", json!({ "id": 1, "text": "The header overlaps the logo at 760px." })).await;
    assert!(unsaid.is_error && unsaid.text.contains("say `review`") && unsaid.text.contains("never reaches frontend"), "{unsaid:?}");
    assert!(!record(&home, 1).iter().any(|(_, kind, _)| kind == "comment"), "nothing noted: {:?}", record(&home, 1));
    // A note: on the record, and the work keeps waiting for review.
    let noted = tool(&handle, "pm", "comment", json!({ "id": 1, "text": "Diff read; screenshots next.", "review": "note" })).await;
    assert!(!noted.is_error && noted.text.contains("still waits for review"), "{noted:?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    assert_eq!(sent_back(&home, 1), 0);
    // A member's comment is a comment, whatever it says.
    let by_member = tool(&handle, "frontend", "comment", json!({ "id": 1, "text": "Ready when you are." })).await;
    assert!(!by_member.is_error, "{by_member:?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);

    // Changes: back to frontend, told why, picked up again on the same branch.
    let branch = handle.snapshot("acme").await.tickets.iter().find(|t| t.id == 1).and_then(|t| t.branch.clone()).expect("a branch");
    let back = tool(&handle, "pm", "comment", json!({ "id": 1, "text": "The header overlaps the logo at 760px.", "review": "changes" })).await;
    assert!(!back.is_error && back.text.contains("Sent T-1 back to frontend"), "{back:?}");
    let seen = until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let returned = seen.iter().find_map(|e| match e {
        Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Todo => Some(ticket.clone()),
        _ => None,
    });
    let returned = returned.expect("back to todo");
    assert_eq!((returned.owner.as_deref(), returned.branch.as_deref()), (Some("frontend"), Some(branch.as_str())));
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("sent T-1 · Hero back to frontend"))), "the person reads it: {seen:#?}");
    let prompt = ticket_prompt(&bin, "The PM reviewed").await;
    assert!(prompt.contains("The PM reviewed your hand-in and sent it back: The header overlaps the logo at 760px."), "{prompt}");
    let history = record(&home, 1);
    assert!(history.iter().any(|(by, kind, text)| (by.as_str(), kind.as_str(), text.as_str()) == ("pm", "sent back", "The header overlaps the logo at 760px.")), "{history:?}");
    // Not in review any more: a later change request has nothing to send back, and says so.
    let late = tool(&handle, "pm", "comment", json!({ "id": 1, "text": "also the footer", "review": "changes" })).await;
    assert!(!late.is_error && late.text.contains("nothing was sent back"), "{late:?}");
    assert_eq!(sent_back(&home, 1), 1);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_branch_only_behind_main_is_accepted_and_merged_and_a_real_conflict_goes_back() {
    // Behind main, merging cleanly: accepted and merged with a merge commit, no rebase asked.
    let (handle, mut events, repo, home, _bin) = handed_in(false).await;
    commit_on_main(&repo, "OTHER.md", "main moved on\n");
    let ok = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Diff read: the hero renders." })).await;
    assert!(!ok.is_error && ok.text.contains("Accepted T-1"), "{ok:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert_eq!(git(&repo, &["show", "main:HERO.md"]), "made by the squad");
    assert_eq!(git(&repo, &["show", "main:OTHER.md"]), "main moved on");
    assert_eq!(sent_back(&home, 1), 0);
    handle.shutdown().await;

    // A real conflict: not accepted, back to its member with the files, before anything merges.
    let (handle, _events, repo, home, bin) = handed_in(false).await;
    commit_on_main(&repo, "HERO.md", "a different hero\n");
    let main_before = git(&repo, &["rev-parse", "main"]);
    let refused = tool(&handle, "pm", "accept_work", json!({ "tickets": [1], "checked": "Diff read." })).await;
    assert!(!refused.is_error && refused.text.contains("Nothing accepted") && refused.text.contains("T-1 conflicts with main in HERO.md: sent back to frontend"), "{refused:?}");
    let prompt = ticket_prompt(&bin, "conflicts with main").await;
    assert!(prompt.contains("conflicts with main in HERO.md") && prompt.contains("no rebase needed"), "{prompt}");
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before, "nothing merged");
    assert!(!record(&home, 1).iter().any(|(_, kind, _)| kind.starts_with("accepted")), "{:?}", record(&home, 1));
    handle.shutdown().await;
}

#[tokio::test]
async fn change_requests_left_as_plain_comments_go_back_to_their_members_once_at_start() {
    let (handle, _events, _repo, home, bin) = handed_in(true).await;
    handle.shutdown().await;
    // What the PM's `comment` once left: a change request on work in review, as a plain note.
    let journal = Journal::for_squad(&home, "acme").unwrap();
    journal.ticket_event(1, "pm", "comment", "Fix the header, then hand in again.").unwrap();
    journal.ticket_event(1, "pm", "comment", "Still there on the same commit.").unwrap();
    drop(journal);

    let (handle, _events) = runtime::start(config(&bin, &home, true));
    let prompt = ticket_prompt(&bin, "sent it back").await;
    assert!(prompt.contains("sent it back: Fix the header, then hand in again.\n\nStill there on the same commit."), "{prompt}");
    assert!(prompt.contains("merge main into your branch — no rebase needed"), "{prompt}");
    assert_eq!(sent_back(&home, 1), 1);
    let history = record(&home, 1);
    assert!(history.iter().any(|(by, kind, _)| by == "pm" && kind == "sent back"), "{history:?}");
    handle.shutdown().await;

    // Once only: the same again later is left as it is.
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let mut again = journal.ticket(1).unwrap().unwrap();
    again.status = TicketStatus::Review;
    journal.save_ticket(&again).unwrap();
    journal.ticket_event(1, "frontend", "handed in", "Fixed.").unwrap();
    journal.ticket_event(1, "pm", "comment", "Another plain comment.").unwrap();
    drop(journal);
    let (handle, _events) = runtime::start(config(&bin, &home, true));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    assert_eq!(sent_back(&home, 1), 1);
    handle.shutdown().await;
}
