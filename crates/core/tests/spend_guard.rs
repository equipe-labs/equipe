//! A ticket whose turns cost more than the squad allows one stops getting
//! turns and goes to the PM, with what it cost — and goes on, with another
//! budget, once the PM puts it back. Driven the way engines and screens drive
//! it: a stand-in engine plays the members, each turn reporting many tokens,
//! and nothing reaches a network.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Engine, Event, SessionSpec, Ticket, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::{spend, squads};
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-spend-guard-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`, signed in: every turn answers in a line, without a
/// board call, and says it used 600,001 tokens. Every prompt is kept.
fn engine() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("bin");
    let bin = dir.join("claude");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "auth" ]; then echo '{{"loggedIn":true,"authMethod":"claude.ai"}}'; exit 0; fi
if [ "$2" = --input-format ] && IFS= read -r line; then set -- -p "$(printf '%s' "$line" | python3 -c 'import json,sys; print(json.load(sys.stdin)["message"]["content"], end="")')" "$@"; fi
printf '%s\n<<end>>\n' "$*" >> '{log}'
echo '{{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}}'
echo '{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"ok"}}}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{{"input_tokens":600000,"output_tokens":1}}}}'
"#,
        log = dir.join("prompts.log").display(),
    );
    let draft = dir.join("claude.draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn prompts(bin: &Path) -> Vec<String> {
    let log = String::from_utf8_lossy(&std::fs::read(bin.with_file_name("prompts.log")).unwrap_or_default()).into_owned();
    log.split("<<end>>").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

fn said(bin: &Path, wanted: &str) -> usize {
    prompts(bin).iter().filter(|p| p.contains(wanted)).count()
}

async fn prompt_with(bin: &Path, wanted: &str) -> String {
    for _ in 0..400 {
        if let Some(p) = prompts(bin).into_iter().rev().find(|p| p.contains(wanted)) {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no prompt said {wanted:?}: {:#?}", prompts(bin));
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A home with the squad `acme` on a fresh repository, each ticket allowed a million tokens.
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
    let journal = Journal::for_squad(&home, "acme").unwrap();
    journal.set_spend_settings(&spend::Settings { ticket_budget: 1_000_000, repeat_turns: 0 }).unwrap();
    (home, repo)
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
        equipe_binary: Some("/opt/equipe".into()),
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

async fn until(rx: &mut UnboundedReceiver<Event>, done: impl Fn(&Event) -> bool) {
    let wait = async {
        while let Some(event) = rx.recv().await {
            if done(&event) {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.expect("timed out");
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, role: &str, cwd: &Path) {
    let spec = SessionSpec { engine: Some(Engine::Claude), model: None, access: Access::EditFiles, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

async fn ticket(handle: &Handle, id: u32) -> Ticket {
    handle.snapshot("acme").await.tickets.into_iter().find(|t| t.id == id).expect("on the board")
}

async fn becomes(handle: &Handle, id: u32, status: TicketStatus) -> Ticket {
    for _ in 0..400 {
        let t = ticket(handle, id).await;
        if t.status == status {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("T-{id} never became {status:?}: {:#?}", ticket(handle, id).await);
}

/// The first turn (600k tokens) is under the million: the member is reminded
/// to finish. The second takes the ticket past it: no third turn — the ticket
/// is a proposal again, its record saying what it cost, and the PM is woken
/// with its card. Put back to todo, it starts again with another million.
#[tokio::test]
async fn a_ticket_past_its_token_budget_stops_getting_turns_and_reaches_the_pm() {
    let bin = engine();
    let (home, repo) = squad();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, "pm", &repo).await;
    add(&handle, &mut events, 2, "frontend", &repo).await;
    let made = tool(&handle, "pm", "create_ticket", json!({ "title": "Hero section", "body": "Build it.", "owner": "frontend" })).await;
    assert!(!made.is_error, "{made:?}");

    prompt_with(&bin, "You're on T-1").await;
    prompt_with(&bin, "Your turn on T-1 ended without ending the work").await;
    let stopped = becomes(&handle, 1, TicketStatus::Proposed).await;
    assert!(stopped.note.as_deref().is_some_and(|n| n.contains("cost too much")), "{:?}", stopped.note);

    // The PM is woken with the card: what it cost and its three ways on.
    let card = prompt_with(&bin, "was stopped before another turn").await;
    for words in ["T-1", "Hero section", "1.2M tokens", "2 turns", "status todo", "cancel"] {
        assert!(card.contains(words), "{words}: {card}");
    }
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let record = journal.ticket_entries(1).unwrap();
    let stop = record.iter().find(|e| e.kind == spend::STOP).expect("the stop is on its record");
    assert_eq!(spend::stopped_at(&stop.text), Some(1_200_002));

    // No turn after the stop, however long one waits.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(said(&bin, "You're on T-1"), 1);
    assert_eq!(said(&bin, "Your turn on T-1 ended without ending the work"), 1);
    assert_eq!(ticket(&handle, 1).await.status, TicketStatus::Proposed);

    // The PM puts it back: it starts again, with another million to spend.
    let back = tool(&handle, "pm", "update_ticket", json!({ "id": 1, "status": "todo" })).await;
    assert!(!back.is_error, "{back:?}");
    for _ in 0..400 {
        if said(&bin, "You're on T-1") >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(said(&bin, "You're on T-1"), 2, "it went on: {:#?}", prompts(&bin));
    handle.shutdown().await;
}
