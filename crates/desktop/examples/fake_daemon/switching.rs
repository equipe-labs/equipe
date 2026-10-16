//! Both engines' plans and the cards for moving members between them.
//!
//! `FAKE_USAGE`: what each plan says —
//! - `healthy`: Claude 5h 34% · week 22%, Codex 5h 12% · week 9%;
//! - `claude86`: Claude's five hours at 86% (resets in 2 hours), Codex roomy —
//!   with a card proposing backend and frontend move to Codex;
//! - `claude_out`: Claude's week used up (resets in 3 days) — a card proposing
//!   the work be handed over;
//! - `both_low`: Claude 5h 91%, Codex week 88% — a card proposing lighter settings.
//!
//! `FAKE_SWITCHING=auto` makes it the card that tells what equipe did, with
//! 되돌리기 (backend moved already, frontend at its next ticket); `off`, no
//! card. `FAKE_SETUP=1` shows the recommended setup card instead.
//! `FAKE_ENGINES_MIXED=1` puts backend on Codex (gpt-6.1-sol).

use equipe_core::protocol::engines::{Move, Proposal, Switching, Why, Window};
use equipe_core::protocol::{Command, Engine, EnginePlan, Event, PlanUsage, PlanWindow, SessionInfo};
use tokio::sync::broadcast;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty() && v != "0")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

const SQUAD: &str = "demo";
const CARD: u64 = 8001;
const SETUP: u64 = 8002;

fn plan(engine: Engine, five: u8, five_left: i64, week: u8, week_left: i64, read_ago: i64) -> EnginePlan {
    let now = now();
    EnginePlan {
        engine,
        usage: PlanUsage {
            five_hour: Some(PlanWindow { used_percent: five, resets_at: now + five_left }),
            seven_day: Some(PlanWindow { used_percent: week, resets_at: now + week_left }),
        },
        plan_type: None,
        at: now - read_ago,
    }
}

const H: i64 = 3600;
const D: i64 = 24 * H;

/// Each engine's plan, as `FAKE_USAGE` says.
pub fn plans() -> Vec<EnginePlan> {
    match env("FAKE_USAGE").as_deref() {
        None => vec![],
        Some("claude86") => vec![plan(Engine::Claude, 86, 2 * H, 41, 3 * D, 60), plan(Engine::Codex, 12, 4 * H, 18, 5 * D, 300)],
        Some("claude_out") => vec![plan(Engine::Claude, 64, 3 * H, 100, 3 * D, 120), plan(Engine::Codex, 9, 4 * H, 14, 5 * D, 240)],
        Some("both_low") => vec![plan(Engine::Claude, 91, 3 * H, 58, 3 * D, 60), plan(Engine::Codex, 47, 2 * H, 88, 2 * D, 600)],
        Some(_) => vec![plan(Engine::Claude, 34, 3 * H, 22, 3 * D, 60), plan(Engine::Codex, 12, 4 * H, 9, 5 * D, 2400)],
    }
}

/// Claude's plan, as an older screen reads it.
pub fn claude() -> Option<PlanUsage> {
    plans().into_iter().find(|p| p.engine == Engine::Claude).map(|p| p.usage)
}

pub fn switching() -> Switching {
    env("FAKE_SWITCHING").and_then(|v| Switching::parse(&v)).unwrap_or_default()
}

fn mv(name: &str, from: Engine, to: Engine, was: &str) -> Move {
    Move { member: name.into(), role: name.into(), from, to, model: None, effort: None, was_model: Some(was.into()), done: false }
}

/// The card on show, if any.
pub fn proposals() -> Vec<Proposal> {
    if env("FAKE_SETUP").is_some() {
        return vec![setup()];
    }
    let mode = switching();
    if mode == Switching::Off {
        return vec![];
    }
    let auto = mode == Switching::Auto;
    let now = now();
    let card = |why, engine, window, used, resets_at, moves| Proposal { id: CARD, why, engine: Some(engine), window: Some(window), used_percent: used, resets_at, moves, applied: auto, proposed_at: now as u64 };
    match env("FAKE_USAGE").as_deref() {
        Some("claude86") => {
            let mut moves = vec![mv("backend", Engine::Claude, Engine::Codex, "claude-opus-5-5"), mv("frontend", Engine::Claude, Engine::Codex, "claude-sonnet-5-5")];
            moves[0].done = auto;
            vec![card(Why::Shift, Engine::Claude, Window::FiveHour, 86, now + 2 * H, moves)]
        }
        Some("claude_out") => {
            let moves = vec![mv("backend", Engine::Claude, Engine::Codex, "claude-opus-5-5"), mv("frontend", Engine::Claude, Engine::Codex, "claude-sonnet-5-5"), mv("designer", Engine::Claude, Engine::Codex, "claude-opus-5-5")];
            vec![card(Why::Exhausted, Engine::Claude, Window::Weekly, 100, now + 3 * D, moves)]
        }
        Some("both_low") => {
            let step = |name: &str, was: &str, model: Option<&str>, effort: Option<&str>| Move {
                model: model.map(str::to_owned),
                effort: effort.map(str::to_owned),
                ..mv(name, Engine::Claude, Engine::Claude, was)
            };
            let moves = vec![step("backend", "claude-opus-5-5", Some("sonnet"), None), step("designer", "claude-opus-5-5", Some("sonnet"), None), step("frontend", "claude-sonnet-5-5", Some("sonnet"), Some("low"))];
            vec![card(Why::StepDown, Engine::Claude, Window::FiveHour, 91, now + 3 * H, moves)]
        }
        _ => vec![],
    }
}

/// The recommended setup for the fake squad: designer on Opus, the reviewer on Codex.
fn setup() -> Proposal {
    let seat = |name: &str, from: Engine, to: Engine, was: &str, model: &str| Move {
        model: Some(model.into()),
        effort: Some("medium".into()),
        ..mv(name, from, to, was)
    };
    Proposal {
        id: SETUP,
        why: Why::Setup,
        engine: None,
        window: None,
        used_percent: 0,
        resets_at: 0,
        moves: vec![
            seat("backend", Engine::Claude, Engine::Claude, "claude-sonnet-5-5", "opus"),
            seat("designer", Engine::Claude, Engine::Claude, "claude-sonnet-5-5", "opus"),
            seat("qa", Engine::Claude, Engine::Codex, "claude-sonnet-5-5", "gpt-6.1-sol"),
            seat("data", Engine::Claude, Engine::Codex, "claude-sonnet-5-5", "gpt-6.1-sol"),
        ],
        applied: false,
        proposed_at: now() as u64,
    }
}

/// `FAKE_ENGINES_MIXED`: backend runs on Codex.
pub fn mix(sessions: &mut [SessionInfo]) {
    if env("FAKE_ENGINES_MIXED").is_none() {
        return;
    }
    for s in sessions.iter_mut().filter(|s| s.member.as_ref().is_some_and(|m| m.name == "backend")) {
        s.engine = Engine::Codex;
        s.model = Some("gpt-6.1-sol".into());
    }
}

/// The person's answers, as the daemon would take them. Whether it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let squad = || SQUAD.to_owned();
    match command {
        Command::SetEngineSwitching { mode, .. } => {
            let _ = events.send(Event::EngineSwitchingChanged { squad: squad(), mode: *mode });
        }
        Command::ApplyEngineProposal { id, .. } => {
            let _ = events.send(Event::EngineProposalClosed { squad: squad(), id: *id, applied: true });
            let _ = events.send(Event::Notice { session: 1, text: "옮기기로 했어요 — backend·frontend는 각자 다음 티켓부터 바뀝니다".into() });
        }
        Command::DismissEngineProposal { id, .. } | Command::UndoEngineProposal { id, .. } => {
            let _ = events.send(Event::EngineProposalClosed { squad: squad(), id: *id, applied: false });
        }
        Command::ShowEngineSetup { .. } => {
            let _ = events.send(Event::EngineProposed { squad: squad(), proposal: setup() });
        }
        _ => return false,
    }
    true
}
