//! Deciding in the person's own words, driven the way a screen and an engine
//! drive it: the person's `say` in, what waits on them out as events — low
//! risk done after its undo window, the rest a card, or (all by words) read
//! back and confirmed by their next own message — and every decision in the
//! squad's log. A member's words, quoted or pasted words, and an older
//! screen's `submit` never decide.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

use equipe_core::protocol::accept::{FROM_WORDS, Verdict};
use equipe_core::protocol::by_words::{Kind, Outcome, Reply, Said, Stage, Tier, Via};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Decision, Event, Mode, SessionSpec, TicketStatus};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-words-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let draft = path.with_extension("draft");
    std::fs::write(&draft, script).unwrap();
    assert!(Process::new("cp").arg(&draft).arg(path).status().unwrap().success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude`, signed in: a ticket's turn writes `HERO.md`; every
/// turn — the PM's too — answers "yes, merge it": a member's yes, which
/// must never confirm anything.
fn claude() -> PathBuf {
    let bin = scratch("bin").join("claude");
    let answer = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"yes, merge it"}}}
{"type":"result","subtype":"success","is_error":false,"result":"yes, merge it","usage":{"input_tokens":3,"output_tokens":1}}
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
        codex_binary: "/nonexistent/codex".into(),
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
    if tokio::time::timeout(Duration::from_secs(30), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

/// What came in the next `ms`.
async fn within(rx: &mut UnboundedReceiver<Event>, ms: u64) -> Vec<Event> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(ms), async {
        while let Some(event) = rx.recv().await {
            seen.push(event);
        }
    })
    .await;
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

async fn status(handle: &Handle, id: u32) -> TicketStatus {
    handle.snapshot("acme").await.tickets.iter().find(|t| t.id == id).map(|t| t.status).expect("on the board")
}

/// The person's own message, typed into the PM's conversation.
fn say(text: &str, all: bool) -> Command {
    Command::Say { squad: "acme".into(), session: 1, text: text.into(), via: Via::Typed, all, later: false, pasted: false, about: None, device: None }
}

/// A PM and a frontend: T-1 handed in, in review — code, or (`task`) research.
async fn handed_in(task: bool) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf) {
    let repo = repo();
    let home = scratch("home");
    squads::create(&home, "acme", &repo).unwrap();
    let (handle, mut events) = runtime::start(config(&claude(), &home));
    add(&handle, &mut events, 1, member(&repo, "pm")).await;
    add(&handle, &mut events, 2, member(&repo, "frontend")).await;
    let kind = if task { "task" } else { "code" };
    tool(&handle, "pm", "create_ticket", json!({ "title": "Hero", "body": "-", "owner": "frontend", "kind": kind })).await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { ticket: 1, .. })).await;
    let handed = tool(&handle, "frontend", "submit_for_review", json!({ "summary": "Hero done." })).await;
    assert!(!handed.is_error, "{handed:?}");
    until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Review)).await;
    (handle, events, repo, home)
}

async fn log(handle: &Handle, events: &mut UnboundedReceiver<Event>) -> Vec<equipe_core::protocol::by_words::Entry> {
    handle.send(Command::WordsLog { squad: "acme".into() });
    let seen = until(events, |e| matches!(e, Event::WordsLog { .. })).await;
    let Some(Event::WordsLog { entries, .. }) = seen.last() else { unreachable!() };
    entries.clone()
}

fn live(events: &[Event]) -> equipe_core::protocol::by_words::Live {
    events.iter().rev().find_map(|e| if let Event::WordsLive { live, .. } = e { Some(live.clone()) } else { None }).expect("a words request")
}

#[tokio::test]
async fn low_risk_words_act_after_their_undo_window_and_undo_takes_it_back() {
    let (handle, mut events, _repo, _home) = handed_in(true).await;
    // "Accept T-1" on research: shown with its undo, nothing done yet, the words on to the PM.
    handle.send(say("accept T-1", false));
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    let shown = live(&seen);
    assert_eq!((shown.stage, shown.tier, shown.kind, shown.read_back.as_str()), (Stage::Undo, Tier::Low, Kind::AcceptTask, "Accept T-1"));
    assert_eq!(status(&handle, 1).await, TicketStatus::Review, "not before its window closes");
    // Undone by the person's own words: nothing happens, and "undo" goes no further.
    handle.send(say("undo", false));
    let seen = until(&mut events, |e| matches!(e, Event::WordsClosed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::WordsClosed { outcome: Outcome::Undone, .. })), "{seen:#?}");
    let after = within(&mut events, 6_000).await;
    assert!(!after.iter().any(|e| matches!(e, Event::UserMessage { text, .. } if text == "undo")), "{after:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    // Said again and left: done when the window closes, the way the board's accept does it.
    handle.send(say("T-1 수락", false));
    let seen = until(&mut events, |e| matches!(e, Event::WordsClosed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::WordsClosed { outcome: Outcome::Done, .. })), "{seen:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Done);
    let kept = log(&handle, &mut events).await;
    assert_eq!(kept.iter().map(|e| e.outcome).collect::<Vec<_>>(), [Outcome::Done, Outcome::Undone], "newest first");
    assert_eq!((kept[0].said.as_str(), kept[0].via, kept[0].tickets.as_slice(), kept[0].all_by_words), ("T-1 수락", Via::Typed, &[1][..], false));
    assert_eq!(kept[1].answers[0].read, Reply::Undo);
    assert_eq!(kept[1].flags, [equipe_core::protocol::by_words::Flag::Undone]);
    handle.shutdown().await;
}

#[tokio::test]
async fn with_words_mode_off_high_risk_keeps_its_card() {
    let (handle, mut events, _repo, _home) = handed_in(false).await;
    // "Merge T-1" on code, all by words off: the card, as ever — nothing merged, nothing read back.
    handle.send(say("merge T-1", false));
    let seen = until(&mut events, |e| matches!(e, Event::AcceptProposed { .. })).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::WordsLive { .. })), "{seen:#?}");
    let Some(Event::AcceptProposed { proposal, .. }) = seen.last() else { unreachable!() };
    assert_eq!((proposal.verdict, proposal.by.as_str(), proposal.tickets.as_slice()), (Verdict::Merge, FROM_WORDS, &[1][..]));
    // A yes after it confirms nothing: there is nothing read back.
    handle.send(say("yes", false));
    let after = within(&mut events, 1_500).await;
    assert!(after.iter().any(|e| matches!(e, Event::UserMessage { text, .. } | Event::SteerQueued { text, .. } | Event::MessageQueued { text, .. } if text == "yes")), "on to the PM: {after:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    let kept = log(&handle, &mut events).await;
    assert_eq!(kept.iter().map(|e| (e.outcome, e.kind)).collect::<Vec<_>>(), [(Outcome::NeedsCard, Kind::Merge)]);
    handle.shutdown().await;
}

#[tokio::test]
async fn all_by_words_reads_back_first_and_only_the_persons_next_clear_yes_runs_it() {
    let (handle, mut events, repo, _home) = handed_in(false).await;
    handle.send(say("merge T-1", true));
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { session: 1, .. })).await;
    let shown = live(&seen);
    assert_eq!((shown.stage, shown.tier, shown.read_back.as_str()), (Stage::ReadBack, Tier::High, "Merge T-1 into main, on this Mac"));
    assert!(shown.items[0].starts_with("T-1 “Hero” — equipe/"), "{:?}", shown.items);
    // The read-back, in the conversation, before anything runs.
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 1, text } if text.starts_with("Merge T-1 into main, on this Mac\n") && text.ends_with("Say yes to go ahead, or no to leave it on its card."))), "{seen:#?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::AcceptProposed { .. })), "no card: words decide it");
    // The PM answers "yes, merge it" — a member's yes: nothing.
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 1, .. })).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    // An older screen's `submit`, a pasted yes and a quoted yes: none of them the person's answer.
    handle.send(Command::Submit { session: 1, text: "yes".into(), about: None });
    handle.send(Command::Say { squad: "acme".into(), session: 1, text: "yes".into(), via: Via::Typed, all: true, later: false, pasted: true, about: None, device: None });
    let after = within(&mut events, 1_000).await;
    assert!(!after.iter().any(|e| matches!(e, Event::WordsClosed { .. })), "{after:#?}");
    handle.send(say("> yes", true));
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, text } if text.starts_with("I didn't take that"))).await;
    assert_eq!(live(&seen).asked_again, 1);
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    // Their own clear yes: done — accepted, and merged into main.
    handle.send(say("ㄱㄱ", true));
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.merged == Some(true))).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::UserMessage { text, .. } if text == "ㄱㄱ")), "an answer goes no further: {seen:#?}");
    assert_eq!(git(&repo, &["show", "main:HERO.md"]), "made by the squad");
    let kept = log(&handle, &mut events).await;
    let entry = &kept[0];
    assert_eq!((entry.outcome, entry.kind, entry.all_by_words, entry.said.as_str(), entry.read_back.as_str()), (Outcome::Done, Kind::Merge, true, "merge T-1", "Merge T-1 into main, on this Mac"));
    assert_eq!(entry.answers.iter().map(|a| (a.words.as_str(), a.read)).collect::<Vec<_>>(), [("> yes", Reply::Unclear), ("ㄱㄱ", Reply::Yes)]);
    assert!(entry.read_back_at.is_some() && entry.decided_at >= entry.read_back_at && entry.asked_again == 1, "{entry:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_no_holds_it_on_its_card_and_an_outward_yes_waits_out_its_cancel_window() {
    let (handle, mut events, _repo, _home) = handed_in(false).await;
    // A no: left as it was.
    handle.send(say("merge T-1", true));
    until(&mut events, |e| matches!(e, Event::WordsLive { .. })).await;
    handle.send(say("아니 하지 마", true));
    let seen = until(&mut events, |e| matches!(e, Event::WordsClosed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::WordsClosed { outcome: Outcome::Held, .. })), "{seen:#?}");
    assert_eq!(status(&handle, 1).await, TicketStatus::Review);
    // Pushing is outward: read back, then a yes counts down — and a cancel in it stops it.
    handle.send(say("merge T-1 and push", true));
    let seen = until(&mut events, |e| matches!(e, Event::WordsLive { .. })).await;
    assert_eq!((live(&seen).tier, live(&seen).read_back.as_str()), (Tier::Outward, "Merge T-1 into main and push main to origin"));
    handle.send(say("ok", true));
    let seen = until(&mut events, |e| matches!(e, Event::WordsLive { live, .. } if live.stage == Stage::Countdown)).await;
    assert!(live(&seen).until > 0);
    let counted = until(&mut events, |e| matches!(e, Event::Notice { text, .. } if text.contains("in 10 s — say cancel"))).await;
    assert!(!counted.is_empty());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(status(&handle, 1).await, TicketStatus::Review, "not before the window closes");
    handle.send(say("cancel", true));
    let seen = until(&mut events, |e| matches!(e, Event::WordsClosed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::WordsClosed { outcome: Outcome::Cancelled, .. })), "{seen:#?}");
    let after = within(&mut events, 9_000).await;
    assert!(!after.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.status == TicketStatus::Done)), "{after:#?}");
    let kept = log(&handle, &mut events).await;
    assert_eq!(kept.iter().map(|e| e.outcome).collect::<Vec<_>>(), [Outcome::Cancelled, Outcome::Held]);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_widening_setting_by_words_is_read_back_counted_down_and_a_phones_voice_decide_is_logged() {
    let (handle, mut events, _repo, _home) = handed_in(true).await;
    let handle2 = handle.clone();
    let asking = tokio::spawn(async move { tool(&handle2, "pm", "change_setting", json!({ "setting": "mode", "member": "frontend", "value": "full", "why": "go faster" })).await });
    until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    // Off: "승인" (Korean for "approve") is only words for the PM; the request waits on its card.
    handle.send(say("승인", false));
    let after = within(&mut events, 1_000).await;
    assert!(!after.iter().any(|e| matches!(e, Event::WordsLive { .. } | Event::ModeChanged { .. })), "{after:#?}");
    // All by words: read back as outward — widening what members may do.
    handle.send(say("승인", true));
    let seen = until(&mut events, |e| matches!(e, Event::WordsLive { .. })).await;
    let shown = live(&seen);
    assert_eq!((shown.stage, shown.tier, shown.kind), (Stage::ReadBack, Tier::Outward, Kind::WidenSetting));
    assert!(shown.read_back.starts_with("Change a setting as pm asked: ") && shown.read_back.contains("frontend"), "{}", shown.read_back);
    handle.send(say("응", true));
    until(&mut events, |e| matches!(e, Event::WordsLive { live, .. } if live.stage == Stage::Countdown)).await;
    let seen = until(&mut events, |e| matches!(e, Event::ModeChanged { mode: Some(Mode::Full), .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::ApprovalResolved { decision: Decision::Allow, .. })), "{seen:#?}");
    assert!(asking.await.unwrap().text.starts_with("The person said yes"));
    // A phone's call mode decides on its own: logged as said, by voice.
    handle.send(Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::Ask) });
    let handle3 = handle.clone();
    let again = tokio::spawn(async move { tool(&handle3, "pm", "change_setting", json!({ "setting": "mode", "member": "frontend", "value": "auto" })).await });
    let seen = until(&mut events, |e| matches!(e, Event::ApprovalRequested { .. })).await;
    let Some(Event::ApprovalRequested { approval, .. }) = seen.last() else { unreachable!() };
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow, words: None, said: Some(Said { via: Via::Voice, utterance: "응 해".into() }) });
    let _ = again.await;
    let kept = log(&handle, &mut events).await;
    assert_eq!((kept[0].via, kept[0].said.as_str(), kept[0].kind, kept[0].outcome), (Via::Voice, "응 해", Kind::WidenSetting, Outcome::Done));
    assert_eq!((kept[1].via, kept[1].kind, kept[1].outcome, kept[1].answers[0].words.as_str()), (Via::Typed, Kind::WidenSetting, Outcome::Done, "응"), "ours, not logged twice");
    assert_eq!(kept.len(), 2);
    handle.shutdown().await;
}
