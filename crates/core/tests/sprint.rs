//! The sprint loop end to end, as the PM and the person drive it
//! (docs/SPRINTS.md): a squad with a product live is asked to plan; the
//! PM's sprint goes to the person on one card; one tap starts its items as
//! tickets with their hypotheses; a ticket goes live and the review judges
//! its hypothesis through the product's data (a made-up source here) by the
//! rule fixed before it shipped; the PM's retro closes the sprint and the
//! next one is proposed; the person says not now. A stand-in `claude` plays
//! the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use equipe_core::protocol::autonomy::{Kind, Level};
use equipe_core::protocol::decide::Answer;
use equipe_core::protocol::deploys::Deploy;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::sprint::{self as wire, ExperimentStatus, ItemStatus, Market, Phase, Reading, SprintView, Verdict};
use equipe_core::protocol::{Access, Command, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::sprint::{self, data};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-sprint-{name}-{}-{n}", std::process::id()));
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
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
    seen
}

async fn tool(handle: &Handle, squad: &str, member: &str, name: &str, arguments: serde_json::Value) -> ToolResult {
    handle.tool(ToolCall { run: None, squad: squad.into(), member: member.into(), name: name.into(), arguments }).await
}

async fn view(handle: &Handle, squad: &str) -> SprintView {
    serde_json::from_value(handle.sprint(wire::GET, json!({ "squad": squad })).await.unwrap()).unwrap()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn said(seen: &[Event], words: &str) -> Option<String> {
    seen.iter().find_map(|e| match e {
        Event::UserMessage { text, .. } if text.contains(words) => Some(text.clone()),
        _ => None,
    })
}

fn asked(seen: &[Event]) -> equipe_core::protocol::decide::QuestionSet {
    seen.iter().rev().find_map(|e| if let Event::QuestionsAsked { questions, .. } = e { Some(questions.clone()) } else { None }).expect("a card")
}

/// A squad with its PM started and its product's numbers read from `fake`.
async fn squad(name: &str, fake: data::Fake) -> (Handle, UnboundedReceiver<Event>, PathBuf, PathBuf) {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, name, &project).unwrap();
    data::register(name, Arc::new(fake));
    let (handle, mut events) = runtime::start(config(&claude, &home));
    let pm = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some(name.into()) };
    handle.send(Command::Start { session: 1, spec: pm });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
    let dir = equipe_core::journal::squad_dir(&home, name);
    (handle, events, dir, project)
}

fn plan(items: serde_json::Value) -> serde_json::Value {
    json!({
        "north_star": { "name": "people who sign up each week", "metric": "event:signup" },
        "goal": "More people finish signing up",
        "done_when": "The shorter form is live and measured",
        "market_guess": "consumers",
        "items": items,
    })
}

#[tokio::test]
async fn a_sprint_is_planned_started_measured_reviewed_and_the_next_proposed() {
    // Before a change goes live: 100 sign-ups a week; after: 150.
    let live = now() - 8 * sprint::DAY;
    let first = (live / sprint::DAY) as i64;
    let fake = data::Fake::new(move |_, from, _| Ok(if from < first { Reading { value: 100.0, sample: 100.0 } } else { Reading { value: 150.0, sample: 150.0 } }));
    let (handle, mut events, dir, project) = squad("orbit", fake).await;

    // Analytics connected: the loop runs, with no sprint yet.
    let v = view(&handle, "orbit").await;
    assert!(v.on && v.measurable && v.current.is_none(), "{v:?}");
    assert_eq!((v.weekday, v.hour, v.length_days, v.budget_tokens), (5, 17, 7, sprint::BUDGET));

    // Asked for one: the PM is handed a planning, then proposes.
    let ended = handle.sprint(wire::END, json!({ "squad": "orbit" })).await.unwrap();
    assert_eq!(ended["started"], true, "{ended}");
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { text, .. } if text.contains("[Sprint planning"))).await;
    let planning = said(&seen, "[Sprint planning").unwrap();
    assert!(planning.contains("Go-to-market: not said yet") && !planning.contains("No product analytics"), "{planning}");

    let refused = tool(&handle, "orbit", "pm", sprint::PLAN, json!({ "goal": "g", "done_when": "d", "items": [] })).await;
    assert!(refused.is_error && refused.text.contains("the `north_star`") && refused.text.contains("at least one"), "{}", refused.text);
    let proposed = tool(&handle, "orbit", "pm", sprint::PLAN, plan(json!([
        {
            "kind": "iterate", "title": "Shorter signup form", "change": "Ask only for an email first.", "why": "Most people stop at the second field",
            "expected": "+20% sign-ups", "impact": "high", "size": "S", "risk": "low",
            "hypothesis": { "belief": "Fewer fields get more people to sign up", "metric": "event:signup", "rule": { "min_change_pct": 20, "window_days": 7, "min_sample": 50 }, "events": ["signup"] },
        },
        { "kind": "marketing", "title": "Launch post for a founders community", "change": "Draft the launch post.", "why": "People asked where to find it", "expected": "+50 visits", "impact": "medium", "size": "M", "risk": "low", "outside": true },
    ]))).await;
    assert!(!proposed.is_error, "{}", proposed.text);
    assert!(proposed.text.contains("Proposed S-1") && proposed.text.contains("P-1 Shorter signup form (recommended)"), "{}", proposed.text);
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let card = asked(&seen);
    assert_eq!(card.questions.len(), 3, "start?, items, and whether it is sold");
    assert!(card.questions[0].question.contains("Start sprint S-1?"), "{}", card.questions[0].question);
    assert!(card.questions[2].options[0].recommended, "the PM's guess");
    let v = view(&handle, "orbit").await;
    assert_eq!(v.proposed.as_ref().map(|p| p.phase), Some(Phase::Proposed));
    assert_eq!(v.items.len(), 2);

    // One tap: the recommended items start as tickets, the hypothesis registered on the first.
    handle.send(Command::AnswerQuestions { squad: "orbit".into(), id: card.id, answers: vec![Answer::pick(0), Answer::skipped(), Answer::pick(0)], about: None });
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.title.starts_with("S-1 P-2"))).await;
    let ticket = seen.iter().find_map(|e| match e {
        Event::TicketChanged { ticket, .. } if ticket.title.starts_with("S-1 P-1") => Some(ticket.clone()),
        _ => None,
    });
    let ticket = ticket.expect("P-1's ticket");
    assert!(ticket.body.contains("Hypothesis H-1") && ticket.body.contains("`signup`"), "{}", ticket.body);
    let kept = sprint::load(&dir);
    let s1 = kept.current().unwrap();
    assert_eq!((s1.id, s1.phase, s1.tickets.len()), (1, Phase::Active, 2));
    assert!(s1.ends_at.unwrap() > now(), "reviewed at its slot");
    assert_eq!(kept.market, Some(Market::Consumers));
    assert_eq!(kept.experiments[0].status, ExperimentStatus::Waiting);
    assert_eq!(kept.experiments[0].ticket, Some(ticket.id));
    assert!(kept.items.iter().all(|i| i.status == ItemStatus::Started));

    // The data analyst may refine it until it is live; the PM too.
    let refined = tool(&handle, "orbit", "pm", sprint::DEFINE, json!({ "ticket": ticket.id, "belief": "Fewer fields get more people to sign up", "metric": "event:signup", "rule": { "min_change_pct": 25, "window_days": 7, "min_sample": 50 } })).await;
    assert!(!refined.is_error && refined.text.contains("Refined H-1"), "{}", refined.text);

    // It went live eight days ago; the person ends the sprint now: its review judges it.
    let journal = equipe_core::journal::Journal::open(&dir.join("squad.db")).unwrap();
    let deploy = Deploy { id: 1, project: "site".into(), path: project.clone(), source: Default::default(), environment: None, url: None, from: None, to: "abc123".into(), at: live, last_at: live, items: vec![], internal: 0, tickets: vec![ticket.id], told: true };
    journal.set_deploys(&[deploy]).unwrap();
    let ended = handle.sprint(wire::END, json!({ "squad": "orbit" })).await.unwrap();
    assert_eq!(ended["started"], true, "{ended}");
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { text, .. } if text.contains("[Sprint review · S-1"))).await;
    let review = said(&seen, "[Sprint review · S-1").unwrap();
    for must in ["Goal: More people finish signing up", "Tickets: 0 of 2 done", "H-1 \"Fewer fields get more people to sign up\"", "worked — moved +50%", "Go-to-market: sold to people (B2C)", "propose adding the marketing role"] {
        assert!(review.contains(must), "{must}\n---\n{review}");
    }
    let kept = sprint::load(&dir);
    let x = &kept.experiments[0];
    assert_eq!((x.status, x.result.as_ref().map(|r| r.verdict)), (ExperimentStatus::Judged, Some(Verdict::Worked)));
    assert_eq!(x.rule.min_change_pct, 25.0, "judged by the rule it had when it went live");
    assert_eq!(kept.current().map(|s| s.phase), Some(Phase::Review));

    // Too late to change the rule now.
    let locked = tool(&handle, "orbit", "pm", sprint::DEFINE, json!({ "ticket": ticket.id, "belief": "b", "metric": "event:signup", "rule": { "min_change_pct": 1, "window_days": 7, "min_sample": 1 } })).await;
    assert!(locked.is_error, "{}", locked.text);

    // The PM writes the retro and proposes the next sprint in one call.
    let mut next = plan(json!([{ "kind": "iterate", "title": "Social sign-in", "change": "Add one-tap sign-in.", "why": "H-1 worked: fewer steps win", "evidence": ["H-1"], "expected": "+15% sign-ups", "impact": "high", "size": "M", "risk": "medium" }]));
    next["retro"] = json!({ "shipped": ["A shorter signup form"], "learned": ["Fewer fields brought half again as many sign-ups"], "wrong": [], "next": ["Make signing up one tap"] });
    let planned = tool(&handle, "orbit", "pm", sprint::PLAN, next).await;
    assert!(!planned.is_error && planned.text.contains("S-1 is closed with your retro") && planned.text.contains("Proposed S-2"), "{}", planned.text);
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let card = asked(&seen);
    assert_eq!(card.questions.len(), 2, "the person already said it is sold");

    // Not now: nothing starts; the items wait as later.
    handle.send(Command::DismissQuestions { squad: "orbit".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::QuestionsClosed { id, .. } if *id == card.id)).await;
    let v = view(&handle, "orbit").await;
    let last = v.last.as_ref().expect("S-1 closed");
    assert_eq!(last.retro.as_ref().unwrap().learned, ["Fewer fields brought half again as many sign-ups"]);
    assert!(v.current.is_none() && v.proposed.is_none(), "{v:?}");
    assert!(v.experiments.iter().any(|x| x.id == 1 && x.result.is_some()));
    let kept = sprint::load(&dir);
    assert_eq!(kept.sprints.iter().find(|s| s.id == 2).map(|s| (s.phase, s.tickets.len())), Some((Phase::Declined, 2)), "S-1's open tickets carried over");
    assert!(kept.items.iter().filter(|i| i.sprint == 2).all(|i| i.status == ItemStatus::Later));
    data::unregister("orbit");
}

#[tokio::test]
async fn small_low_risk_items_start_on_their_own_only_when_the_person_lets_them() {
    let fake = data::Fake::new(|_, _, _| Ok(Reading { value: 10.0, sample: 10.0 }));
    let (handle, mut events, dir, _) = squad("comet", fake).await;
    handle.send(Command::SetAutonomy { squad: "comet".into(), kind: Kind::Sprint, level: Level::JustDo });
    until(&mut events, |e| matches!(e, Event::AutonomyChanged { .. })).await;
    let planned = tool(&handle, "comet", "pm", sprint::PLAN, plan(json!([
        { "kind": "fix", "title": "Fix the broken footer link", "change": "Point it at the pricing page.", "why": "Users wrote in about it", "expected": "fewer support mails", "impact": "low", "size": "S", "risk": "low" },
        { "kind": "new", "title": "Team accounts", "change": "Let a team share one account.", "why": "Three customers asked", "expected": "+10% paid", "impact": "high", "size": "L", "risk": "medium" },
        { "kind": "marketing", "title": "Newsletter announcement", "change": "Draft it.", "why": "New feature", "expected": "+5% visits", "impact": "low", "size": "S", "risk": "low" },
    ]))).await;
    assert!(!planned.is_error, "{}", planned.text);
    assert!(planned.text.contains("Started on their own") && planned.text.contains("Fix the broken footer link"), "{}", planned.text);
    let kept = sprint::load(&dir);
    let started: Vec<&str> = kept.items.iter().filter(|i| i.status == ItemStatus::Started).map(|i| i.title.as_str()).collect();
    assert_eq!(started, ["Fix the broken footer link"], "never go-to-market, never large or risky");
    assert!(kept.items.iter().find(|i| i.status == ItemStatus::Started).unwrap().auto_started);
    assert_eq!(kept.current().map(|s| s.phase), Some(Phase::Active), "under way with what started");
    // The card still asks about the rest, the started one said so.
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let card = asked(&seen);
    assert!(card.questions[1].options.iter().any(|o| o.description.contains("Already started on its own")), "{:?}", card.questions[1].options);
    data::unregister("comet");
}

#[tokio::test]
async fn the_loop_waits_for_a_live_product_and_obeys_the_persons_switch() {
    let claude = claude();
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "nova", &project).unwrap();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    let pm = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.clone(), role: Some("pm".into()), squad: Some("nova".into()) };
    handle.send(Command::Start { session: 1, spec: pm });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 1, .. })).await;
    let v = view(&handle, "nova").await;
    assert!(!v.on && v.why_off.as_deref().unwrap_or_default().contains("nothing is live"), "{v:?}");
    assert!(!v.measurable);
    let ended = handle.sprint(wire::END, json!({ "squad": "nova" })).await.unwrap();
    assert_eq!(ended["started"], false);
    // The person turns it on before anything is live, a fortnight long, reviewed Mondays at 10.
    let set: SprintView = serde_json::from_value(handle.sprint(wire::SET, json!({ "squad": "nova", "on": true, "length_days": 14, "weekday": 1, "hour": 10 })).await.unwrap()).unwrap();
    assert!(set.on && set.length_days == 14 && (set.weekday, set.hour) == (1, 10), "{set:?}");
    assert!(handle.sprint(wire::SET, json!({ "squad": "nova", "length_days": 9 })).await.is_err());
    // …and off again.
    let off: SprintView = serde_json::from_value(handle.sprint(wire::SET, json!({ "squad": "nova", "on": false })).await.unwrap()).unwrap();
    assert!(!off.on && off.why_off.unwrap().contains("turned the sprint loop off"));
    assert!(handle.sprint("sprint.nope", json!({ "squad": "nova" })).await.is_err());
    assert!(handle.sprint(wire::GET, json!({ "squad": "nobody" })).await.is_err());
}
