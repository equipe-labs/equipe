//! Design sessions and taste signals, driven as a screen and the PM drive
//! them: a session started from a screen (its kickoff to the designer, every
//! screen told), the PM's proposal on a card — once per ticket, its answer
//! carried out — and the person's reactions kept in the project's style
//! profile. A stand-in `claude` plays the engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::decide::Answer;
use equipe_core::protocol::design::{QuickReaction, TasteSignal};
use equipe_core::protocol::design_session::SessionFrom;
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
    let dir = std::env::temp_dir().join(format!("equipe-dsession-{name}-{}-{n}", std::process::id()));
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

/// A home where the squad `acme` works on the project `site`, which has a design system.
fn squad_home() -> (PathBuf, PathBuf) {
    let project = scratch("site").join("site");
    std::fs::create_dir_all(project.join("design")).unwrap();
    std::fs::write(project.join("design/tokens.json"), "{}").unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    (home, project)
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

#[tokio::test]
async fn a_session_started_from_a_screen_kicks_the_designer_off_and_tells_every_screen() {
    let claude = claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;

    // No designer yet: said, nothing started.
    handle.send(Command::StartDesignSession { squad: "acme".into(), project: Some("site".into()), page: "Pricing".into(), from: SessionFrom::DesignScreen });
    let seen = until(&mut events, |e| matches!(e, Event::Error { .. })).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::DesignSessionStarted { .. })));

    add(&handle, &mut events, 2, member(&project, "designer")).await;
    handle.send(Command::StartDesignSession { squad: "acme".into(), project: Some("site".into()), page: "  Pricing \n page ".into(), from: SessionFrom::Ticket { ticket: 4 } });
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
    let started = seen.iter().find_map(|e| if let Event::DesignSessionStarted { designer, page, project, from, .. } = e { Some((*designer, page.clone(), project.clone(), from.clone())) } else { None });
    assert_eq!(started, Some((2, "Pricing page".to_owned(), Some("site".to_owned()), SessionFrom::Ticket { ticket: 4 })));
    let text = seen.iter().find_map(|e| if let Event::UserMessage { session: 2, text, .. } = e { Some(text.clone()) } else { None }).unwrap();
    let (page, words) = equipe_core::design::session::split_kickoff(&text).expect("a kickoff");
    assert_eq!(page, "Pricing page");
    assert!(text.contains("started from: ticket T-4") && text.contains("`design-session` skill") && text.contains("style profile: thin"), "{text}");
    assert!(!words.is_empty(), "the person's words after the block");
    handle.shutdown().await;
}

fn proposal(set: &[Event]) -> Option<equipe_core::protocol::decide::QuestionSet> {
    set.iter().find_map(|e| if let Event::QuestionsAsked { questions, .. } = e { Some(questions.clone()) } else { None })
}

#[tokio::test]
async fn the_pm_proposes_a_session_once_per_ticket_and_the_persons_choice_is_carried_out() {
    let claude = claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;

    // Quiet hours all day: nothing is proposed, and nothing is used up.
    let quiet = equipe_core::protocol::importance::NotifySettings { quiet_on: true, quiet_from: 0, quiet_to: 23 * 60 + 59, utc_offset_secs: Some(0), ..Default::default() };
    handle.send(Command::SetNotify { squad: "acme".into(), settings: quiet.clone() });
    until(&mut events, |e| matches!(e, Event::NotifyChanged { .. })).await;
    let hushed = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Pricing page", "ticket": 7 })).await;
    let at_night = equipe_core::escalation::is_quiet(&quiet, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    if at_night {
        assert!(hushed.is_error && hushed.text.contains("quiet hours"), "{hushed:?}");
    }
    handle.send(Command::SetNotify { squad: "acme".into(), settings: equipe_core::protocol::importance::NotifySettings { quiet_on: false, ..quiet } });
    until(&mut events, |e| matches!(e, Event::NotifyChanged { .. })).await;

    // Only the PM proposes one.
    let not_pm = tool(&handle, "designer", "propose_design_session", json!({ "screen": "Pricing" })).await;
    assert!(not_pm.is_error, "{not_pm:?}");

    let said = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Pricing page", "ticket": 7, "project": "site", "why": "A new screen." })).await;
    assert!(!said.is_error && said.text.contains("Proposed"), "{said:?}");
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let set = proposal(&seen).unwrap();
    assert_eq!((set.member.as_str(), set.session, set.questions.len()), ("pm", 1, 1));
    let card = &set.questions[0];
    assert!(card.question.ends_with("A new screen."), "{}", card.question);
    // The profile is thin: start, tune first, not now, go ahead.
    assert_eq!(card.options.len(), 4);

    // Never twice for the same ticket — even before the answer.
    let again = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Pricing page", "ticket": 7 })).await;
    assert!(again.is_error && again.text.contains("already on the person's card"), "{again:?}");

    // The person starts it: the designer has the kickoff, every screen opens it.
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers: vec![Answer::pick(0)], about: None });
    let seen = until(&mut events, |e| matches!(e, Event::UserMessage { session: 2, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::DesignSessionStarted { designer: 2, from: SessionFrom::Pm { ticket: Some(7) }, .. })), "{seen:#?}");
    let after = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Pricing page", "ticket": 7 })).await;
    assert!(after.is_error && after.text.contains("already started"), "{after:?}");

    // Another screen, put away unanswered: "Not now", for good.
    let other = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Onboarding", "project": "site" })).await;
    assert!(!other.is_error, "{other:?}");
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let set = proposal(&seen).unwrap();
    handle.send(Command::DismissQuestions { squad: "acme".into(), id: set.id });
    until(&mut events, |e| matches!(e, Event::QuestionsClosed { .. })).await;
    let held = tool(&handle, "pm", "propose_design_session", json!({ "screen": "  onboarding ", "project": "site" })).await;
    assert!(held.is_error && held.text.contains("Not now"), "{held:?}");
    let kept = equipe_core::design::session::read(&equipe_core::journal::squad_dir(&home, "acme"));
    assert_eq!(kept.len(), 2, "both kept, through restarts");

    // "Tune my taste first": every screen opens the tuner.
    tool(&handle, "pm", "propose_design_session", json!({ "screen": "Settings", "project": "site" })).await;
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let set = proposal(&seen).unwrap();
    handle.send(Command::AnswerQuestions { squad: "acme".into(), id: set.id, answers: vec![Answer::pick(1)], about: None });
    let seen = until(&mut events, |e| matches!(e, Event::OpenTasteTuner { .. })).await;
    assert!(matches!(seen.last(), Some(Event::OpenTasteTuner { project: Some(p), .. }) if p == "site"));

    // Asked for by the person: started at once, no card.
    let asked = tool(&handle, "pm", "propose_design_session", json!({ "screen": "Home", "project": "site", "asked": true })).await;
    assert!(!asked.is_error && asked.text.contains("Started"), "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::DesignSessionStarted { .. })).await;
    assert!(proposal(&seen).is_none(), "no card");
    handle.shutdown().await;
}

#[tokio::test]
async fn reactions_in_a_session_are_kept_in_the_projects_style_profile() {
    let claude = claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    let signals = vec![
        TasteSignal::React { picture: None, label: "Round 2 hero".into(), reaction: QuickReaction::MoreLikeThis, reasons: vec![] },
        TasteSignal::Rejected { what: "Round 1".into(), picture: None, reasons: vec![equipe_core::protocol::design::PickReason::TooDense] },
    ];
    handle.send(Command::TasteSignals { squad: "acme".into(), project: Some("site".into()), signals });
    let seen = until(&mut events, |e| matches!(e, Event::Design { taste, .. } if taste.iter().any(|t| t.profile.is_some()))).await;
    let Some(Event::Design { taste, .. }) = seen.last() else { unreachable!() };
    let site = taste.iter().find(|t| t.project.as_deref() == Some("site")).unwrap();
    let profile = site.profile.as_ref().unwrap();
    assert_eq!(profile.liked, ["more like Round 2 hero"]);
    assert_eq!(profile.disliked, ["Round 1 (too dense)"]);
    assert!(profile.leans.iter().any(|l| l.lean < 0), "\"too dense\" leans airy: {profile:?}");
    let file = std::fs::read_to_string(project.join("design/taste.md")).unwrap();
    assert!(file.contains("## Style profile") && file.contains("- Liked: more like Round 2 hero"), "{file}");
    assert!(equipe_core::taste::tuner::signals_file(&project.join("design")).is_file());
    handle.shutdown().await;
}
