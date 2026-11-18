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

/// A stand-in `claude` that answers "ok" and writes down each turn's message,
/// one line a turn, in the file it returns.
fn recording_claude() -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("rec");
    let log = dir.join("turns.log");
    let bin = dir.join("claude");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
IFS= read -r line; printf '%s\\n' \"$line\" >> '{}'\n\
cat <<EOF\n{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}\n\
{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}\nEOF\n",
        log.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

fn turns(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_owned).collect()
}

/// Say `text` to the designer (2) and wait for its turn to end.
async fn say_to_designer(handle: &Handle, events: &mut UnboundedReceiver<Event>, text: &str) {
    handle.send(Command::Submit { session: 2, text: text.into(), about: None });
    until(events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
}

#[tokio::test]
async fn a_sessions_turns_are_quick_prototype_rounds_until_it_ends() {
    let (claude, log) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;
    handle.send(Command::StartDesignSession { squad: "acme".into(), project: Some("site".into()), page: "Pricing".into(), from: SessionFrom::DesignScreen });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let kickoff = turns(&log).pop().unwrap();
    assert!(kickoff.contains("prototype mode") && kickoff.contains("21st.dev, Mobbin") && kickoff.contains("show_references"), "{kickoff}");

    say_to_designer(&handle, &mut events, "airier, please").await;
    let round = turns(&log).pop().unwrap();
    for rule in ["design session on", "prototype round", "no axe", "no design QA", "no new ticket per feedback item", "CHANGELOG.md"] {
        assert!(round.contains(rule), "{rule}: {round}");
    }

    handle.send(Command::EndDesignSession { squad: "acme".into(), designer: 2 });
    say_to_designer(&handle, &mut events, "thanks").await;
    let after = turns(&log).pop().unwrap();
    assert!(!after.contains("prototype round"), "ended: ticket work again — {after}");
    handle.shutdown().await;
}

#[tokio::test]
async fn use_this_opens_the_implementation_ticket_with_the_prototype_as_spec_then_updates_it() {
    let (claude, log) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;
    handle.send(Command::StartDesignSession { squad: "acme".into(), project: Some("site".into()), page: "Pricing".into(), from: SessionFrom::DesignScreen });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;

    let pick = |variant: &str| Command::UseDesignVariant {
        squad: "acme".into(),
        designer: 2,
        project: Some("site".into()),
        page: "Pricing".into(),
        variant: Some(variant.into()),
        url: Some(format!("http://localhost:5173/proto/{}", variant.to_lowercase())),
    };
    handle.send(pick("b"));
    let seen = until(&mut events, |e| matches!(e, Event::DesignVariantUsed { .. })).await;
    let Some(Event::DesignVariantUsed { ticket, variant, updated: false, .. }) = seen.last().cloned() else { panic!("{seen:#?}") };
    assert_eq!(variant.as_deref(), Some("B"));
    let made = seen.iter().rev().find_map(|e| if let Event::TicketChanged { ticket: t, .. } = e { (t.id == ticket).then(|| t.clone()) } else { None }).expect("the ticket");
    assert!(made.title.contains("Pricing") && made.title.contains('B'), "{}", made.title);
    assert!(made.body.contains("- Variant: B") && made.body.contains("- Prototype: http://localhost:5173/proto/b") && made.body.contains("full design checks run on this ticket"), "{}", made.body);

    // Picked again before it started: the same ticket, updated.
    handle.send(pick("A"));
    let seen = until(&mut events, |e| matches!(e, Event::DesignVariantUsed { .. })).await;
    assert!(matches!(seen.last(), Some(Event::DesignVariantUsed { ticket: t, updated: true, .. }) if *t == ticket), "{seen:#?}");
    let edited = seen.iter().rev().find_map(|e| if let Event::TicketChanged { ticket: t, .. } = e { (t.id == ticket).then(|| t.clone()) } else { None }).expect("updated");
    assert!(edited.body.contains("- Variant: A"), "{}", edited.body);

    // The designer hears of it with its next message, and its rounds are over.
    say_to_designer(&handle, &mut events, "go").await;
    let told = turns(&log).pop().unwrap();
    assert!(told.contains("variant A of the prototype") && told.contains(&format!("full checks run on T-{ticket}")), "{told}");
    assert!(!told.contains("prototype round"), "{told}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_withdraws_its_own_card_when_the_persons_words_settle_it() {
    let (claude, log) = recording_claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;
    let asked = tool(&handle, "designer", "ask_decision", json!({ "questions": [{ "question": "Which left menu?", "options": [{ "label": "Rail" }, { "label": "Drawer" }] }] })).await;
    assert!(!asked.is_error, "{asked:?}");
    let seen = until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let set = proposal(&seen).unwrap();

    // The person's message carries the open card, and says to withdraw rather than steer them.
    say_to_designer(&handle, &mut events, "show me references first").await;
    let said = turns(&log).pop().unwrap();
    assert!(said.contains(&format!("#{}", set.id)) && said.contains("Which left menu?") && said.contains("withdraw_decision"), "{said}");

    let done = tool(&handle, "designer", "withdraw_decision", json!({ "id": set.id, "why": "references first" })).await;
    assert!(!done.is_error && done.text.contains("Withdrawn"), "{done:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::QuestionsClosed { id, answers: None, .. } if *id == set.id)), "{seen:#?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 2, text, .. } if text.contains("withdrew its card") && text.contains("Which left menu?"))), "{seen:#?}");
    let again = tool(&handle, "designer", "withdraw_decision", json!({})).await;
    assert!(again.is_error, "nothing left: {again:?}");
    // Someone else's card is not its to take back.
    tool(&handle, "pm", "ask_decision", json!({ "questions": [{ "question": "Ship Friday?", "options": [{ "label": "Yes" }, { "label": "No" }] }] })).await;
    until(&mut events, |e| matches!(e, Event::QuestionsAsked { .. })).await;
    let theirs = tool(&handle, "designer", "withdraw_decision", json!({})).await;
    assert!(theirs.is_error, "{theirs:?}");
    handle.shutdown().await;
}

#[tokio::test]
async fn a_member_revises_its_own_ticket_that_hasnt_started_and_says_so() {
    let claude = claude();
    let (home, project) = squad_home();
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm")).await;
    add(&handle, &mut events, 2, member(&project, "designer")).await;
    // The PM's own ticket first, and the designer's waiting on it: not started.
    tool(&handle, "pm", "create_ticket", json!({ "title": "Billing page", "body": "-", "owner": "pm", "kind": "task" })).await;
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket: t, .. } if t.title == "Billing page")).await;
    let Some(Event::TicketChanged { ticket: other, .. }) = seen.last().cloned() else { panic!() };
    tool(&handle, "pm", "create_ticket", json!({ "title": "Left menu options", "body": "Three options on a card.", "owner": "designer", "kind": "task", "depends_on": [other.id] })).await;
    let seen = until(&mut events, |e| matches!(e, Event::TicketChanged { ticket: t, .. } if t.title == "Left menu options")).await;
    let Some(Event::TicketChanged { ticket, .. }) = seen.last().cloned() else { panic!() };
    let revised = tool(&handle, "designer", "revise_ticket", json!({ "id": ticket.id, "body": "References first, then a live prototype.", "why": "the person asked for references first" })).await;
    assert!(!revised.is_error && revised.text.contains("body"), "{revised:?}");
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 2, .. })).await;
    assert!(seen.iter().any(|e| matches!(e, Event::TicketChanged { ticket: t, .. } if t.id == ticket.id && t.body == "References first, then a live prototype.")), "{seen:#?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Notice { session: 2, text, .. } if text.contains(&format!("updated T-{}", ticket.id)))), "{seen:#?}");
    // Not its own: refused.
    let refused = tool(&handle, "designer", "revise_ticket", json!({ "id": other.id, "status": "cancelled" })).await;
    assert!(refused.is_error && refused.text.contains("isn't yours"), "{refused:?}");
    // Its own, dropped.
    let dropped = tool(&handle, "designer", "revise_ticket", json!({ "id": ticket.id, "status": "cancelled" })).await;
    assert!(!dropped.is_error, "{dropped:?}");
    handle.shutdown().await;
}

/// A stand-in `claude` that logs each turn's first line; a turn that says
/// "show it" runs `equipe design session show` on `page` as variant A, then
/// waits a moment for anything said to it mid-turn (logged too) before it ends.
fn showing_claude(page: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("show");
    let log = dir.join("turns.log");
    let bin = dir.join("claude");
    let url = format!("file://{}", page.display());
    let script = format!(
        "#!/bin/bash\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
IFS= read -r line; printf '%s\\n' \"$line\" >> '{log}'\n\
echo '{{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-test\",\"session_id\":\"sess-1\"}}'\n\
case \"$line\" in *'show it'*)\n\
echo '{{\"type\":\"assistant\",\"parent_tool_use_id\":null,\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{{\"command\":\"equipe design session show a.html --variant A\"}}}}]}}}}'\n\
printf '%s\\n' '{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"Variant A sent.\\ndesign-session: variant A {url}\"}}]}}}}'\n\
IFS= read -r -t 4 more && printf 'MID %s\\n' \"$more\" >> '{log}';;\n\
esac\n\
echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":1}}}}'\n",
        log = log.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

#[tokio::test]
async fn a_page_the_designer_shows_is_served_for_the_persons_screen_and_nobody_watching_is_said_at_once() {
    use equipe_core::screens::Place;
    let (home, project) = squad_home();
    let page = project.join("design/sessions/pricing/a.html");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "<h1>A</h1>").unwrap();
    let (claude, log) = showing_claude(&page);
    let (handle, mut events) = runtime::start(config(&claude, &home));
    add(&handle, &mut events, 2, member(&project, "designer")).await;

    // Nobody follows the squad: the page is served all the same, and the designer hears at once that nobody saw it.
    say_to_designer(&handle, &mut events, "show it").await;
    let said = turns(&log);
    assert!(said.iter().any(|l| l.starts_with("MID ") && l.contains("reached nobody")), "{said:?}");
    let served = equipe_core::prototype::Server::now().and_then(|s| s.address_of(&page)).expect("served for the person's screen");
    assert!(served.starts_with("http://127.0.0.1:") && served.ends_with("/sessions/pricing/a.html"), "{served}");
    assert!(!said[0].contains("another machine"), "nothing said of where the person is before anyone followed: {}", said[0]);

    // The person's app follows from another Mac: the designer's next turn opens with it, once, and the show is quiet.
    let _seat = handle.screens().sit("acme", Place::Elsewhere, true);
    std::fs::remove_file(&log).unwrap();
    say_to_designer(&handle, &mut events, "show it").await;
    let said = turns(&log);
    assert!(said[0].contains("from another machine") && said[0].contains("file://"), "{}", said[0]);
    assert!(!said.iter().any(|l| l.starts_with("MID ")), "a screen shows it: nothing to say mid-turn — {said:?}");
    say_to_designer(&handle, &mut events, "thanks").await;
    assert!(!turns(&log).last().unwrap().contains("another machine"), "told once");
    handle.shutdown().await;
}
