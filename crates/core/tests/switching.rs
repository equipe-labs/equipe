//! Moving members between engines, driven the way screens and engines drive
//! it: each plan's usage in as it is read, the card out as an event, the
//! person's answer in as their command — and the member closed and started
//! again on the other engine at its ticket boundary — or, moved now, at the
//! end of its turn mid-ticket — never mid-turn, its first turn opening with
//! equipe's hand-over packet. A stand-in `claude` and a Codex login
//! written for the test play the engines; nothing reaches a network.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_core::protocol::engines::{Proposal, Switching, Why};
use equipe_core::protocol::rpc::ToolResult;
use equipe_core::protocol::{Access, Command, Engine, Event, PlanUsage, PlanWindow, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-engines-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in `claude`, signed in, that writes down each turn's prompt (one
/// file per turn) and then runs `turn`.
fn claude(turn: &str) -> (PathBuf, PathBuf) {
    let bin = scratch("bin").join("claude");
    let prompts = scratch("prompts");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n\
if IFS= read -r line; then n=$(ls '{dir}' | wc -l | tr -d ' '); printf '%s' \"$line\" > '{dir}/turn-'$(printf %03d $n)-$$; fi\n{turn}\n",
        dir = prompts.display()
    );
    write_script(&bin, &script);
    (bin, prompts)
}

/// A turn that answers at once.
const ANSWERS: &str = r#"cat <<EOF
{"type":"system","subtype":"init","model":"claude-test","session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#;

/// A turn that goes on for a long while.
const WORKS_LONG: &str = "exec sleep 30";

/// Every prompt written so far.
fn prompts(dir: &Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_default()).collect()
}

async fn prompt_with(dir: &Path, wanted: &str) -> String {
    for _ in 0..200 {
        if let Some(p) = prompts(dir).into_iter().find(|p| p.contains(wanted)) {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no prompt with {wanted:?}: {:#?}", prompts(dir));
}

/// A home with the squad `acme`, its project, and a Codex login made up for the test.
fn home() -> (PathBuf, PathBuf) {
    let project = scratch("site").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
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
    (home, project)
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
        lang: equipe_core::i18n::Lang::Ko,
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

/// Everything that comes within `wait`.
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
    SessionSpec { engine, model, access: Access::ReadOnly, cwd: cwd.to_owned(), role: Some(role.into()), squad: Some("acme".into()) }
}

async fn add(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, spec: SessionSpec) {
    handle.send(Command::Start { session: id, spec });
    until(events, |e| matches!(e, Event::SessionStarted { session, .. } if *session == id)).await;
}

async fn create_ticket(handle: &Handle, owner: &str) -> ToolResult {
    let call = equipe_core::protocol::rpc::ToolCall {
        run: None,
        squad: "acme".into(),
        member: "pm".into(),
        name: "create_ticket".into(),
        arguments: json!({ "title": "Review the release", "body": "Check the changelog against main.", "owner": owner }),
    };
    handle.tool(call).await
}

/// A plan's usage, its windows starting over at the same moments each time it is read.
fn usage(five: u8, week: u8) -> PlanUsage {
    static START: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let now = *START.get_or_init(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);
    PlanUsage {
        five_hour: Some(PlanWindow { used_percent: five, resets_at: now + 3 * 3600 }),
        seven_day: Some(PlanWindow { used_percent: week, resets_at: now + 4 * 24 * 3600 }),
    }
}

fn proposed(events: &[Event]) -> Proposal {
    events.iter().find_map(|e| if let Event::EngineProposed { proposal, .. } = e { Some(proposal.clone()) } else { None }).expect("a card")
}

/// A PM on Claude and a qa on Codex.
async fn pm_and_codex_qa(claude: &Path) -> (Handle, UnboundedReceiver<Event>) {
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(claude, &home));
    add(&handle, &mut events, 1, member(&project, "pm", None)).await;
    add(&handle, &mut events, 2, member(&project, "qa", Some(Engine::Codex))).await;
    (handle, events)
}

#[tokio::test]
async fn in_auto_an_idle_member_moves_at_once_and_its_first_turn_opens_with_the_brief() {
    let (bin, dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Auto });
    until(&mut events, |e| matches!(e, Event::EngineSwitchingChanged { mode: Switching::Auto, .. })).await;
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(96, 20));
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    let card = proposed(&seen);
    assert_eq!((card.why, card.engine, card.applied, card.used_percent), (Why::Drain, Some(Engine::Codex), true, 96));
    assert_eq!(card.moves.iter().map(|m| (m.member.as_str(), m.from, m.to)).collect::<Vec<_>>(), [("qa", Engine::Codex, Engine::Claude)]);
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "closed as /close does");
    // The card says it's done; the member's conversation says what happened.
    let done = until(&mut events, |e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.moves.iter().all(|m| m.done))).await;
    assert!(!done.is_empty());
    let snapshot = handle.snapshot("acme").await;
    let qa = snapshot.sessions.iter().find(|s| s.member.as_ref().is_some_and(|m| m.name == "qa")).expect("qa is back");
    assert_eq!(qa.engine, Engine::Claude);
    assert_eq!(snapshot.plans.iter().map(|p| p.engine).collect::<Vec<_>>(), [Engine::Claude, Engine::Codex]);
    assert!(snapshot.plans.iter().all(|p| p.at > 0), "each read with its time");
    assert_eq!(snapshot.engine_switching, Switching::Auto);
    // Its next ticket: the first turn on Claude opens with equipe's brief.
    create_ticket(&handle, "qa").await;
    let prompt = prompt_with(&dir, "[equipe] You are qa").await;
    assert!(prompt.contains("on Claude what you were doing on Codex") && prompt.contains("Review the release"), "{prompt}");
}

#[tokio::test]
async fn moved_by_the_person_mid_ticket_it_waits_for_its_ticket_unless_now_and_then_carries_it_on_the_other_engine() {
    let (bin, dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    create_ticket(&handle, "qa").await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), .. })).await;
    // Its turn on Codex ends (there is no Codex here), the ticket still in hand.
    for _ in 0..100 {
        let snapshot = handle.snapshot("acme").await;
        let run = snapshot.sessions.iter().find(|s| s.run.is_some_and(|r| r.of == 2));
        if run.is_some_and(|qa| qa.state == equipe_core::protocol::SessionState::Ready && qa.work.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Gently: it keeps its ticket where it is.
    handle.send(Command::MoveEngine { squad: "acme".into(), members: vec!["qa".into()], engine: Engine::Claude, model: None, effort: None, now: false });
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.why == Why::Person)).await;
    let card = proposed(&seen);
    assert!(!card.moves[0].now);
    let later = within(&mut events, Duration::from_millis(1200)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "on its ticket: not before it ends");
    // "Move now": at the end of its turn, its ticket handed over and carried on on Claude.
    handle.send(Command::ApplyEngineProposal { squad: "acme".into(), id: card.id, now: true });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })));
    assert!(seen.iter().any(|e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.moves[0].now)), "the card says it moves now");
    let prompt = prompt_with(&dir, "[equipe] You are qa").await;
    for wanted in ["on Claude what you were doing on Codex", "the person moved you", "## The ticket: T-1 Review the release", "Check the changelog against main.", "## Where things stand", "## Before you change anything"] {
        assert!(prompt.contains(wanted), "{wanted} in: {prompt}");
    }
    let snapshot = handle.snapshot("acme").await;
    let qa = snapshot.sessions.iter().find(|s| s.run.is_some() && s.member.as_ref().is_some_and(|m| m.name == "qa")).expect("qa is back on it");
    assert_eq!((qa.engine, qa.work.as_ref().map(|w| w.ticket)), (Engine::Claude, Some(1)), "the same ticket, carried on");
}

#[tokio::test]
async fn a_member_at_work_is_never_moved_mid_turn_and_a_draining_plan_moves_it_at_its_turns_end_with_its_ticket() {
    let (bin, _dir) = claude(WORKS_LONG);
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, member(&project, "pm", None)).await;
    add(&handle, &mut events, 2, member(&project, "qa", Some(Engine::Claude))).await;
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Auto });
    create_ticket(&handle, "qa").await;
    let started = until(&mut events, |e| matches!(e, Event::WorkStarted { of: Some(2), .. })).await;
    let run = started.iter().find_map(|e| if let Event::WorkStarted { session, of: Some(2), .. } = e { Some(*session) } else { None }).unwrap();
    handle.usage_read(Engine::Codex, usage(5, 5));
    handle.usage_read(Engine::Claude, usage(96, 30));
    // Both signed in, the team on Claude: the setup is proposed too — this is the other card.
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.why == Why::Drain)).await;
    let card = seen.iter().rev().find_map(|e| if let Event::EngineProposed { proposal, .. } = e { Some(proposal.clone()) } else { None }).unwrap();
    assert_eq!((card.why, card.applied), (Why::Drain, true));
    assert!(card.moves.iter().any(|m| m.member == "qa" && m.to == Engine::Codex && !m.done));
    assert!(card.moves.iter().any(|m| m.member == "qa" && m.now), "a drain moves at the end of the turn");
    // Mid-turn: nothing happens to it.
    let later = within(&mut events, Duration::from_millis(1500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "{later:#?}");
    // Its turn stopped, at a clean point on its ticket: it moves now, its
    // ticket handed over — here Codex can't start it, so it carries on on
    // Claude, its ticket and the packet with it.
    // The person stops its run: its ticket's turn, not the member.
    handle.send(Command::Interrupt { session: run });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), .. } if m.name == "qa")).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "{seen:#?}");
    assert!(seen.iter().any(|e| matches!(e, Event::TicketChanged { ticket, .. } if ticket.id == 1 && ticket.note.as_deref().is_some_and(|n| n.starts_with("Handed over from Claude to Codex")))));
    assert!(handle.snapshot("acme").await.engine_proposals.iter().any(|p| p.id == card.id), "the card stays until the person closes it");
}

#[tokio::test]
async fn a_squad_that_never_chose_moves_off_an_exhausted_engine_on_its_own_and_says_so() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    assert_eq!(handle.snapshot("acme").await.engine_switching, Switching::Auto, "never chosen: auto");
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(100, 20));
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    let card = proposed(&seen);
    assert_eq!((card.why, card.engine, card.applied), (Why::Exhausted, Some(Engine::Codex), true), "made, not proposed");
    assert_eq!(card.moves.iter().map(|m| (m.member.as_str(), m.from, m.to)).collect::<Vec<_>>(), [("qa", Engine::Codex, Engine::Claude)]);
    // Told, and undoable: the card stays, saying it was made.
    assert!(handle.snapshot("acme").await.engine_proposals.iter().any(|p| p.id == card.id && p.applied));
}

/// A stand-in `claude` that tells the plan's usage only when equipe reads it
/// on its own (the tiny turn of `claude_code::read_plan`); a member's turn
/// answers without it.
fn claude_telling_its_plan_when_read() -> (PathBuf, PathBuf) {
    let resets = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 4 * 24 * 3600;
    let plan = format!(r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","unifiedWindows":{{"seven_day":{{"utilization":0.12,"resetsAt":{resets}}}}}}}}}"#);
    claude(&format!("if [ \"$2\" = \"Reply with the single word: ok\" ]; then echo '{plan}'; fi\n{ANSWERS}"))
}

#[tokio::test]
async fn an_engine_out_with_the_other_never_read_reads_it_and_then_moves() {
    let (bin, _dir) = claude_telling_its_plan_when_read();
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    // No Claude turn has told its plan: equipe reads it, and the move follows.
    handle.usage_read(Engine::Codex, usage(100, 20));
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    let read = seen.iter().find_map(|e| if let Event::PlanUsage { engine: Engine::Claude, usage } = e { Some(*usage) } else { None }).expect("Claude's plan read");
    assert_eq!(read.seven_day.map(|w| w.used_percent), Some(12));
    assert!(proposed(&seen).applied);
}

#[tokio::test]
async fn in_suggest_only_the_persons_click_moves_and_a_no_is_not_asked_again() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Suggest });
    until(&mut events, |e| matches!(e, Event::EngineSwitchingChanged { mode: Switching::Suggest, .. })).await;
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(96, 20));
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { .. })).await;
    let card = proposed(&seen);
    assert!(!card.applied, "chosen: a card to answer");
    let later = within(&mut events, Duration::from_millis(1000)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { .. })), "nothing moves without the person");
    assert_eq!(handle.snapshot("acme").await.engine_proposals, vec![card.clone()]);
    // No: the card closes, and the same readings don't bring it back.
    handle.send(Command::DismissEngineProposal { squad: "acme".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::EngineProposalClosed { applied: false, .. })).await;
    handle.usage_read(Engine::Codex, usage(97, 20));
    let later = within(&mut events, Duration::from_millis(800)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::EngineProposed { .. })), "{later:#?}");
    // Exhausted, with its reset hours away: asked again — and the person's yes moves it.
    handle.usage_read(Engine::Codex, usage(100, 20));
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { .. })).await;
    let card = proposed(&seen);
    assert_eq!((card.why, card.applied), (Why::Exhausted, false));
    handle.send(Command::ApplyEngineProposal { squad: "acme".into(), id: card.id, now: false });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    assert!(seen.iter().any(|e| matches!(e, Event::EngineProposalClosed { applied: true, .. })));
}

#[tokio::test]
async fn signed_in_to_both_with_the_team_on_one_the_setup_is_proposed_once_and_again_on_asking() {
    let (bin, _dir) = claude(ANSWERS);
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, member(&project, "pm", Some(Engine::Claude))).await;
    add(&handle, &mut events, 2, member(&project, "qa", Some(Engine::Claude))).await;
    handle.usage_read(Engine::Claude, usage(20, 20));
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { .. })).await;
    let card = proposed(&seen);
    assert_eq!((card.why, card.engine), (Why::Setup, None));
    let qa = card.moves.iter().find(|m| m.member == "qa").expect("qa reviews on Codex");
    assert_eq!((qa.from, qa.to, qa.model.as_deref(), qa.effort.as_deref()), (Engine::Claude, Engine::Codex, Some("gpt-6.1-sol"), Some("medium")));
    handle.send(Command::DismissEngineProposal { squad: "acme".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::EngineProposalClosed { .. })).await;
    // Once: not again on its own.
    handle.usage_read(Engine::Claude, usage(21, 20));
    assert!(!within(&mut events, Duration::from_millis(800)).await.iter().any(|e| matches!(e, Event::EngineProposed { .. })));
    // Asked for from settings: shown again.
    handle.send(Command::ShowEngineSetup { squad: "acme".into() });
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { .. })).await;
    assert_eq!(proposed(&seen).why, Why::Setup);
}

#[tokio::test]
async fn a_ticket_an_engine_out_of_its_plan_keeps_waiting_is_told_once_never_silent() {
    let (bin, _dir) = claude(ANSWERS);
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, member(&project, "pm", None)).await;
    add(&handle, &mut events, 2, member(&project, "qa", Some(Engine::Claude))).await;
    // Claude's five hours are out and start over within the hour: waited for, nothing new starts on it.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let out = PlanUsage { five_hour: Some(PlanWindow { used_percent: 100, resets_at: now + 40 * 60 }), seven_day: Some(PlanWindow { used_percent: 40, resets_at: now + 3 * 24 * 3600 }) };
    handle.usage_read(Engine::Codex, usage(5, 5));
    handle.usage_read(Engine::Claude, out);
    let _ = within(&mut events, Duration::from_millis(300)).await;
    create_ticket(&handle, "qa").await;
    let told = |e: &Event| format!("{e:?}").contains("T-1 대기 중이에요");
    let seen = until(&mut events, told).await;
    assert!(!seen.iter().any(|e| matches!(e, Event::WorkStarted { of: Some(2), .. })), "held, not started");
    // Once per hold, however often the board is looked at again.
    create_ticket(&handle, "qa").await;
    let later = within(&mut events, Duration::from_millis(1200)).await;
    assert!(!later.iter().any(|e| told(e) || format!("{e:?}").contains("T-2 대기 중")), "{later:#?}");
}

#[tokio::test]
async fn the_conversations_the_person_opened_with_the_pm_move_with_it_to_its_new_session() {
    let (bin, _dir) = claude(ANSWERS);
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, member(&project, "pm", None)).await;
    handle.send(Command::NewConversation { squad: "acme".into(), session: 77, project: None });
    until(&mut events, |e| matches!(e, Event::SessionStarted { session: 77, .. })).await;
    // Moved by the person; there is no Codex here, so it starts again on Claude — a new session either way.
    handle.send(Command::MoveEngine { squad: "acme".into(), members: vec!["pm".into()], engine: Engine::Codex, model: None, effort: None, now: true });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), .. } if m.name == "pm")).await;
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 1 })), "the old session closed");
    assert!(!seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 77 })), "the conversation stayed: {seen:#?}");
    let snapshot = handle.snapshot("acme").await;
    let pm = snapshot.sessions.iter().find(|s| s.lane.is_none() && s.member.as_ref().is_some_and(|m| m.name == "pm")).expect("the pm").session;
    let talk = snapshot.sessions.iter().find(|s| s.session == 77).expect("the conversation is still listed");
    assert_eq!(talk.lane.as_ref().map(|l| (l.of, l.kept)), Some((pm, true)), "it belongs to the new session");
}

/// Qa on Codex with a ticket in hand whose run waits on the person and has a
/// question open; its engine's turn ended (there is no Codex here). Returns
/// the run's session.
async fn qa_waiting_with_a_question(handle: &Handle, events: &mut UnboundedReceiver<Event>) -> u64 {
    create_ticket(handle, "qa").await;
    let started = until(events, |e| matches!(e, Event::WorkStarted { of: Some(2), .. })).await;
    let run = started.iter().find_map(|e| if let Event::WorkStarted { session, of: Some(2), .. } = e { Some(*session) } else { None }).unwrap();
    for _ in 0..100 {
        let snapshot = handle.snapshot("acme").await;
        if snapshot.sessions.iter().find(|s| s.session == run).is_some_and(|s| s.state == equipe_core::protocol::SessionState::Ready && s.work.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let call = |name: &str, arguments: serde_json::Value| equipe_core::protocol::rpc::ToolCall {
        run: Some(equipe_core::protocol::rpc::RunKey { session: run, ticket: 1, generation: 1 }),
        squad: "acme".into(),
        member: "qa".into(),
        name: name.into(),
        arguments,
    };
    let asked = handle.tool(call("block_on_person", json!({ "needs": ["Say which release this is"] }))).await;
    assert!(!asked.is_error, "{}", asked.text);
    let asked = handle.tool(call("ask_decision", json!({ "questions": [{ "question": "Which release is this?", "options": ["1.0", "2.0"] }] }))).await;
    assert!(!asked.is_error, "{}", asked.text);
    run
}

#[tokio::test]
async fn an_exhausted_engine_moves_a_member_whose_run_waits_on_the_person_and_keeps_both() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    let run = qa_waiting_with_a_question(&handle, &mut events).await;
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(100, 20));
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    assert_eq!(proposed(&seen).why, Why::Exhausted);
    assert!(seen.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "moved although it waits on the person");
    let snapshot = handle.snapshot("acme").await;
    let qa = snapshot.sessions.iter().find(|s| s.run.is_none() && s.lane.is_none() && s.member.as_ref().is_some_and(|m| m.name == "qa")).expect("qa is back");
    assert_eq!(qa.engine, Engine::Claude);
    // Its waiting run stayed with its ticket and went along, now on Claude.
    let kept = snapshot.sessions.iter().find(|s| s.session == run).expect("the waiting run is still there");
    assert_eq!((kept.engine, kept.run.map(|r| r.of), kept.work.as_ref().map(|w| w.ticket)), (Engine::Claude, Some(qa.session), Some(1)));
    // And the question is still open.
    assert_eq!(snapshot.questions.len(), 1, "{:#?}", snapshot.questions);
    assert_eq!(snapshot.questions[0].member, "qa");
}

#[tokio::test]
async fn a_move_that_is_not_exhausted_still_waits_for_a_run_waiting_on_the_person() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    let run = qa_waiting_with_a_question(&handle, &mut events).await;
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(96, 20));
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.why == Why::Drain)).await;
    assert_eq!(proposed(&seen).why, Why::Drain);
    let later = within(&mut events, Duration::from_millis(1500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "a drain waits: {later:#?}");
    let snapshot = handle.snapshot("acme").await;
    assert_eq!(snapshot.sessions.iter().find(|s| s.session == run).map(|s| s.engine), Some(Engine::Codex));
}

#[tokio::test]
async fn a_question_asked_from_a_run_carries_that_runs_ticket_named_or_not() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    qa_waiting_with_a_question(&handle, &mut events).await;
    let named = handle.snapshot("acme").await.questions.pop().expect("asked");
    assert_eq!(named.ticket, Some(1), "{named:#?}");
    // A call that names no run, from a member whose only run is on the ticket.
    let call = equipe_core::protocol::rpc::ToolCall {
        run: None,
        squad: "acme".into(),
        member: "qa".into(),
        name: "ask_decision".into(),
        arguments: json!({ "questions": [{ "question": "Is this the right branch?", "options": ["Yes", "No"] }] }),
    };
    assert!(!handle.tool(call).await.is_error);
    let sets = handle.snapshot("acme").await.questions;
    assert_eq!(sets.len(), 2);
    assert!(sets.iter().all(|s| s.ticket == Some(1)), "{sets:#?}");
}

#[tokio::test]
async fn a_run_resumed_on_an_exhausted_engine_pauses_for_it_and_starts_no_turn_there() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    let run = qa_waiting_with_a_question(&handle, &mut events).await;
    // Nothing moves; Codex is out of its limit.
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Off });
    until(&mut events, |e| matches!(e, Event::EngineSwitchingChanged { mode: Switching::Off, .. })).await;
    handle.usage_read(Engine::Codex, usage(100, 20));
    let _ = within(&mut events, Duration::from_millis(500)).await;
    // The person answers: its ticket goes back to it — and a turn on Codex would only fail.
    let set = handle.snapshot("acme").await.questions.pop().expect("asked");
    handle.send(Command::DismissQuestions { squad: "acme".into(), id: set.id });
    let later = within(&mut events, Duration::from_millis(2000)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::TurnEnded { session, .. } if *session == run)), "no turn was started on Codex: {later:#?}");
    let snapshot = handle.snapshot("acme").await;
    let kept = snapshot.sessions.iter().find(|s| s.session == run).expect("the run is still there, paused");
    assert_eq!(kept.engine, Engine::Codex);
    assert!(kept.work.is_some());
}

#[tokio::test]
async fn a_conversation_without_a_ticket_on_an_exhausted_engine_keeps_what_it_was_asked_and_gets_it_where_it_moves() {
    let (bin, dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Off });
    until(&mut events, |e| matches!(e, Event::EngineSwitchingChanged { mode: Switching::Off, .. })).await;
    handle.usage_read(Engine::Codex, usage(100, 20));
    let _ = within(&mut events, Duration::from_millis(500)).await;
    handle.send(Command::Submit { session: 2, text: "Are you there?".into(), about: None });
    let later = within(&mut events, Duration::from_millis(1500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::TurnEnded { session: 2, .. })), "no turn on the exhausted engine: {later:#?}");
    // The person moves it: what it was asked is asked of it on Claude.
    handle.send(Command::MoveEngine { squad: "acme".into(), members: vec!["qa".into()], engine: Engine::Claude, model: None, effort: None, now: true });
    let asked = prompt_with(&dir, "Are you there?").await;
    assert!(asked.contains("Are you there?"), "{asked}");
}

fn started_model(seen: &[Event], name: &str) -> Option<Option<String>> {
    seen.iter().find_map(|e| match e {
        Event::SessionStarted { member: Some(m), model, .. } if m.name == name => Some(model.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn a_person_move_with_no_model_chosen_never_lands_on_the_premium_family_even_with_a_role_default_of_it() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    let default = equipe_core::protocol::RoleModel { role: "qa".into(), engine: Engine::Claude, model: Some("fable".into()), effort: None };
    handle.send(Command::SetRoleModel { squad: "acme".into(), default });
    handle.send(Command::MoveEngine { squad: "acme".into(), members: vec!["qa".into()], engine: Engine::Claude, model: None, effort: None, now: true });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    assert_eq!(started_model(&seen, "qa"), Some(Some("opus".to_owned())), "{seen:#?}");
}

#[tokio::test]
async fn a_person_move_that_names_the_premium_family_keeps_it() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.send(Command::MoveEngine { squad: "acme".into(), members: vec!["qa".into()], engine: Engine::Claude, model: Some("fable".into()), effort: None, now: true });
    let seen = until(&mut events, |e| matches!(e, Event::SessionStarted { member: Some(m), engine: Engine::Claude, .. } if m.name == "qa")).await;
    assert_eq!(started_model(&seen, "qa"), Some(Some("fable".to_owned())), "{seen:#?}");
}

#[tokio::test]
async fn setting_the_premium_family_keeps_it_and_says_what_it_costs() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.send(Command::SetModel { session: 1, model: "fable".into() });
    let seen = until(&mut events, |e| matches!(e, Event::Notice { session: 1, text, .. } if text.contains("Opus"))).await;
    assert!(seen.iter().any(|e| matches!(e, Event::ModelKnown { session: 1, model, .. } if model == "fable")));
}
