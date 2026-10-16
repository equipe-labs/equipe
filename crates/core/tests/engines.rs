//! Moving members between engines, driven the way screens and engines drive
//! it: each plan's usage in as it is read, the card out as an event, the
//! person's answer in as their command — and the member closed and started
//! again on the other engine at its ticket boundary, never mid-turn, its first
//! turn opening with equipe's brief. A stand-in `claude` and a Codex login
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
        persist: false,
        retention_days: 90,
        equipe_binary: None,
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        lang: equipe_core::i18n::Lang::Ko,
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
async fn a_member_at_work_is_never_moved_mid_turn_nor_off_its_ticket() {
    let (bin, _dir) = claude(WORKS_LONG);
    let (home, project) = home();
    let (handle, mut events) = runtime::start(config(&bin, &home));
    add(&handle, &mut events, 1, member(&project, "pm", None)).await;
    add(&handle, &mut events, 2, member(&project, "qa", Some(Engine::Claude))).await;
    handle.send(Command::SetEngineSwitching { squad: "acme".into(), mode: Switching::Auto });
    create_ticket(&handle, "qa").await;
    until(&mut events, |e| matches!(e, Event::WorkStarted { session: 2, .. })).await;
    handle.usage_read(Engine::Codex, usage(5, 5));
    handle.usage_read(Engine::Claude, usage(96, 30));
    // Both signed in, the team on Claude: the setup is proposed too — this is the other card.
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { proposal, .. } if proposal.why == Why::Drain)).await;
    let card = seen.iter().rev().find_map(|e| if let Event::EngineProposed { proposal, .. } = e { Some(proposal.clone()) } else { None }).unwrap();
    assert_eq!((card.why, card.applied), (Why::Drain, true));
    assert!(card.moves.iter().any(|m| m.member == "qa" && m.to == Engine::Codex && !m.done));
    // Mid-turn: nothing happens to it.
    let later = within(&mut events, Duration::from_millis(1500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "{later:#?}");
    // Its turn stopped, it is still on its ticket: it stays on Claude until it puts the ticket down.
    handle.send(Command::Interrupt { session: 2 });
    until(&mut events, |e| matches!(e, Event::TurnEnded { session: 2, .. })).await;
    let later = within(&mut events, Duration::from_millis(1500)).await;
    assert!(!later.iter().any(|e| matches!(e, Event::SessionClosed { session: 2 })), "{later:#?}");
    let snapshot = handle.snapshot("acme").await;
    let qa = snapshot.sessions.iter().find(|s| s.session == 2).expect("still there");
    assert_eq!((qa.engine, qa.work.as_ref().map(|w| w.ticket)), (Engine::Claude, Some(1)));
    assert!(snapshot.engine_proposals.iter().any(|p| p.id == card.id), "the card stays until the person closes it");
    // The person takes it back: nothing is left to do.
    handle.send(Command::UndoEngineProposal { squad: "acme".into(), id: card.id });
    until(&mut events, |e| matches!(e, Event::EngineProposalClosed { id, applied: false, .. } if *id == card.id)).await;
}

#[tokio::test]
async fn in_suggest_only_the_persons_click_moves_and_a_no_is_not_asked_again() {
    let (bin, _dir) = claude(ANSWERS);
    let (handle, mut events) = pm_and_codex_qa(&bin).await;
    handle.usage_read(Engine::Claude, usage(10, 10));
    handle.usage_read(Engine::Codex, usage(96, 20));
    let seen = until(&mut events, |e| matches!(e, Event::EngineProposed { .. })).await;
    let card = proposed(&seen);
    assert!(!card.applied, "suggest is the default: a card to answer");
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
    handle.send(Command::ApplyEngineProposal { squad: "acme".into(), id: card.id });
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
