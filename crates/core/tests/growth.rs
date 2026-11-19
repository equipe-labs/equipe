//! The growth loop end to end, as the PM and the person drive it
//! (docs/GROWTH.md): a run's numbers kept, the PM's card put to the person,
//! accepted — a ticket on the board serving a request of its own — then the
//! ticket goes live and the card's follow-ups are scheduled 7 and 14 days on.
//! A stand-in `claude` plays the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::growth::{self, Store};
use equipe_core::protocol::decide::Answer;
use equipe_core::protocol::deploys::Deploy;
use equipe_core::protocol::growth::{Evidence, Status};
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-growth-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude` that answers "ok" to every turn.
fn claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\n\
cat <<EOF\n{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}\n\
{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}\nEOF\n";
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
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
    if tokio::time::timeout(Duration::from_secs(15), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A run's numbers, as the weekly pass keeps them.
fn evidence() -> Evidence {
    Evidence {
        id: "E1".into(),
        metric: "funnel:question_sent>answer_received".into(),
        label: "question_sent → answer_received conversion within 1 day of 216 people".into(),
        value: 41.2,
        previous: Some(45.0),
        unit: "%".into(),
        from: "2026-11-09".into(),
        to: "2026-11-15".into(),
        query: "SELECT count() AS entered …".into(),
        source: "posthog m1".into(),
        better: Default::default(),
        measure: None,
    }
}

#[tokio::test]
async fn an_accepted_card_becomes_a_ticket_and_its_follow_ups_are_scheduled_once_live() {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let dir = equipe_core::journal::squad_dir(&home, "acme");
    growth::save(&dir, &Store { evidence: vec![evidence()], last_run: Some(now()), ..Store::default() }).unwrap();

    let (handle, mut events) = runtime::start(config(&claude, &home));
    let pm = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: 1, spec: pm });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;

    // Without numbers no card is made.
    let bare = json!({ "cards": [{ "title": "Stream answers", "change": "Stream the first sentence.", "evidence": [], "expected": "+5 points", "size": "M", "risk": "low" }] });
    let refused = tool(&handle, "pm", growth::PROPOSE, bare).await;
    assert!(refused.is_error && refused.text.contains("without numbers"), "{}", refused.text);

    let card = json!({ "cards": [{
        "title": "Stream the first sentence of each answer",
        "change": "Show the answer's first sentence within a second instead of waiting for all of it.",
        "evidence": ["E1"],
        "expected": "+5 points of conversion",
        "size": "M",
        "risk": "low",
    }] });
    let proposed = tool(&handle, "pm", growth::PROPOSE, card).await;
    assert!(!proposed.is_error, "{}", proposed.text);
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let Some(Event::QuestionsAsked { questions: set, .. }) = seen.iter().rev().find(|e| matches!(e, Event::QuestionsAsked { .. })) else { unreachable!() };
    assert_eq!(set.questions.len(), 1);
    assert!(set.questions[0].question.contains("41.2%"), "its numbers are on the card: {}", set.questions[0].question);
    assert_eq!(set.questions[0].options.len(), 3, "accept, later, no");

    // Accepted: a ticket on the board, serving a request of its own.
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers: vec![Answer { picked: vec![0], ..Answer::default() }], about: None });
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.title.starts_with("Growth G-1"))).await;
    let ticket = seen.iter().find_map(|e| match e {
        Event::TicketChanged { ticket, .. } if ticket.title.starts_with("Growth G-1") => Some(ticket.clone()),
        _ => None,
    });
    let ticket = ticket.unwrap();
    assert!(ticket.body.contains("41.2%"), "the evidence goes with the ticket: {}", ticket.body);
    let kept = growth::load(&dir);
    let card = &kept.cards[0];
    assert_eq!(card.status, Status::Accepted);
    assert_eq!(card.ticket, Some(ticket.id));
    assert!(card.request.is_some(), "it serves a request");

    // Live: a deploy brought it. The next look schedules its follow-ups.
    let live = now() - 60;
    let journal = equipe_core::journal::Journal::open(&dir.join("squad.db")).unwrap();
    let deploy = Deploy { id: 1, project: "site".into(), path: project.clone(), source: Default::default(), environment: None, url: None, from: None, to: "abc123".into(), at: live, last_at: live, items: vec![], internal: 0, tickets: vec![ticket.id], told: true };
    journal.set_deploys(&[deploy]).unwrap();
    let started = handle.growth(equipe_core::protocol::growth::RUN, json!({ "squad": "acme" })).await.unwrap();
    assert_eq!(started["started"], true, "{started}");
    let kept = growth::load(&dir);
    let card = &kept.cards[0];
    assert_eq!(card.status, Status::Live);
    assert_eq!(card.live_at, Some(live));
    let due: Vec<(u32, u64)> = card.follow_ups.iter().map(|f| (f.after_days, f.due_at)).collect();
    assert_eq!(due, [(7, live + 7 * growth::DAY), (14, live + 14 * growth::DAY)], "measured again 7 and 14 days on");

    // Screens read it all back.
    let listed = handle.growth(equipe_core::protocol::growth::LIST, json!({ "squad": "acme" })).await.unwrap();
    assert_eq!(listed["cards"][0]["status"], "live");
}
