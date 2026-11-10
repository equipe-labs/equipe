//! Both engines' plans and the cards for moving members between them.
//!
//! `FAKE_USAGE`: what each plan says (`weekly`: a weekly window only) —
//! - `healthy`: Claude 5h 34% · week 22%, Codex 5h 12% · week 9%;
//! - `claude86`: Claude's five hours at 86% (resets in 2 hours), Codex roomy —
//!   with a card proposing backend and frontend move to Codex;
//! - `claude_week`: Claude's week at 86%, running out at this pace in 1 day
//!   6 hours, 2 days before it resets — a card proposing backend and frontend
//!   (64% of the week's use, both mid-ticket) move, with "Move now";
//! - `claude_out`: Claude's week used up (resets in 3 days) — a card proposing
//!   the work be handed over;
//! - `both_low`: Claude 5h 91%, Codex week 88% — a card proposing lighter settings;
//! - `week61`: Claude 5h 48% · week 61% (refills in 2 days 10 hours), Codex week 23% — and
//!   T-42 burning it fast (`fake_daemon/plan_use.rs`).
//!
//! `FAKE_SWITCHING=auto` makes it the card that tells what equipe did, with
//! Undo (backend moved already, frontend at its next ticket); `off`, no
//! card. `FAKE_SETUP=1` shows the recommended setup card instead.
//! `FAKE_ENGINES_MIXED=1` puts backend on Codex (gpt-6.1-sol).
//! `FAKE_MODELS=1` has the PM set to `opus` while an older Claude Code runs
//! it as Opus 4.8, backend on Codex, Codex's list of models with their
//! efforts, and the designer's new members started on Fable at `max`.
//! `FAKE_CODEX_MODELS`: Codex's list on its own — `1`, listed at once;
//! `slow`, two seconds after it is asked; `silent`, never (the model
//! picker's "didn't say"). A move to the other engine from the picker
//! (`MoveEngine`) shows as the person's own, waiting for the member's next
//! ticket — or, moved now, at the end of its turn — with its undo; "Move
//! now" on a card whose moves are made marks them so.

use equipe_core::protocol::engines::{Forecast, Move, Proposal, Switching, Why, Window};
use equipe_core::protocol::{Command, Engine, EnginePlan, Event, ModelChoice, PlanUsage, PlanWindow, RoleModel, SessionInfo};
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
        Some("claude_week") => vec![plan(Engine::Claude, 41, 3 * H, 86, 3 * D + 6 * H, 60), plan(Engine::Codex, 12, 4 * H, 18, 5 * D, 300)],
        Some("claude_out") => vec![plan(Engine::Claude, 64, 3 * H, 100, 3 * D, 120), plan(Engine::Codex, 9, 4 * H, 14, 5 * D, 240)],
        Some("week61") => vec![plan(Engine::Claude, 48, 3 * H, 61, 2 * D + 10 * H, 60), plan(Engine::Codex, 12, 4 * H, 23, 5 * D, 300)],
        Some("both_low") => vec![plan(Engine::Claude, 91, 3 * H, 58, 3 * D, 60), plan(Engine::Codex, 47, 2 * H, 88, 2 * D, 600)],
        // Plans with a weekly window only, Claude's read half an hour ago.
        Some("weekly") => [plan(Engine::Claude, 0, 0, 15, 3 * D, 31 * 60), plan(Engine::Codex, 0, 0, 4, 5 * D, 60)]
            .into_iter()
            .map(|mut p| {
                p.usage.five_hour = None;
                p
            })
            .collect(),
        Some(_) => vec![plan(Engine::Claude, 34, 3 * H, 22, 3 * D, 60), plan(Engine::Codex, 12, 4 * H, 9, 5 * D, 2400)],
    }
}

/// Claude's plan, as an older screen reads it.
pub fn claude() -> Option<PlanUsage> {
    plans().into_iter().find(|p| p.engine == Engine::Claude).map(|p| p.usage)
}

/// Unless named, a squad that chose to be asked: its cards wait for an answer.
pub fn switching() -> Switching {
    env("FAKE_SWITCHING").and_then(|v| Switching::parse(&v)).unwrap_or(Switching::Suggest)
}

/// The squad's parallel work: `FAKE_PARALLEL=max` (max's default caps) or
/// `max:<per member>:<per engine>`; else auto.
pub fn parallel() -> equipe_core::protocol::lanes::Parallel {
    use equipe_core::protocol::lanes::Parallel;
    let Some(v) = env("FAKE_PARALLEL") else { return Parallel::default() };
    let mut parts = v.split(':');
    if parts.next() != Some("max") {
        return Parallel::default();
    }
    let defaults = Parallel::default();
    let per_member = parts.next().and_then(|n| n.parse().ok()).unwrap_or(defaults.per_member);
    let per_engine = parts.next().and_then(|n| n.parse().ok()).unwrap_or(defaults.per_engine);
    Parallel::max(per_member, per_engine)
}

fn mv(name: &str, from: Engine, to: Engine, was: &str) -> Move {
    Move { member: name.into(), role: name.into(), from, to, model: None, effort: None, was_model: Some(was.into()), done: false, now: false }
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
    let card = |why, engine, window, used, resets_at, moves| Proposal { id: CARD, why, engine: Some(engine), window: Some(window), used_percent: used, resets_at, moves, applied: auto, proposed_at: now as u64, forecast: None };
    match env("FAKE_USAGE").as_deref() {
        Some("claude86") => {
            let mut moves = vec![mv("backend", Engine::Claude, Engine::Codex, "claude-opus-5-5"), mv("frontend", Engine::Claude, Engine::Codex, "claude-sonnet-5-5")];
            moves[0].done = auto;
            vec![card(Why::Shift, Engine::Claude, Window::FiveHour, 86, now + 2 * H, moves)]
        }
        Some("claude_week") => {
            let moves = vec![mv("backend", Engine::Claude, Engine::Codex, "claude-opus-5-5"), mv("frontend", Engine::Claude, Engine::Codex, "claude-sonnet-5-5")];
            let resets_at = now + 3 * D + 6 * H;
            let forecast = Some(Forecast { runs_out_at: Some(now + D + 6 * H), resets_at, load_bp: Some(6_400) });
            vec![Proposal { forecast, ..card(Why::Shift, Engine::Claude, Window::Weekly, 86, resets_at, moves) }]
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
        forecast: None,
    }
}

/// `FAKE_ENGINES_MIXED`: backend runs on Codex.
pub fn mix(sessions: &mut [SessionInfo]) {
    let models = env("FAKE_MODELS").is_some();
    if models {
        for s in sessions.iter_mut().filter(|s| s.member.as_ref().is_some_and(|m| m.name == "pm")) {
            s.model = Some("claude-opus-4-8".into());
            s.asked_model = Some("opus".into());
        }
    }
    if env("FAKE_ENGINES_MIXED").is_none() && !models {
        return;
    }
    for s in sessions.iter_mut().filter(|s| s.member.as_ref().is_some_and(|m| m.name == "backend")) {
        s.engine = Engine::Codex;
        s.model = Some("gpt-6.1-sol".into());
    }
}

/// What new members of a role start with (`FAKE_MODELS`).
pub fn role_models() -> Vec<RoleModel> {
    if env("FAKE_MODELS").is_none() {
        return Vec::new();
    }
    vec![RoleModel { role: "designer".into(), engine: Engine::Claude, model: Some("fable".into()), effort: Some("max".into()) }]
}

/// Codex's models as its own list gives them: their efforts, what Codex says
/// of each (in its own words, English), its default first.
fn codex_models() -> Vec<ModelChoice> {
    let efforts = |e: &[&str]| e.iter().map(|e| (*e).to_owned()).collect();
    let about = |s: &str| Some(s.to_owned());
    vec![
        ModelChoice { id: "gpt-6.1-sol".into(), name: Some("GPT-6.1 Sol".into()), efforts: efforts(&["low", "medium", "high", "xhigh"]), default_effort: Some("medium".into()), description: about("Frontier agentic coding model."), default: true },
        ModelChoice { id: "gpt-6-astra".into(), name: Some("GPT-6 Astra".into()), efforts: efforts(&["low", "medium", "high"]), default_effort: Some("high".into()), description: about("Deep reasoning for the hardest problems."), default: false },
        ModelChoice { id: "gpt-6-luna".into(), name: Some("GPT-6 Luna".into()), efforts: efforts(&["minimal", "low", "medium"]), default_effort: Some("low".into()), description: about("Fast and light, for small edits."), default: false },
    ]
}

/// How Codex answers for its list: at once, a while later, or not at all.
fn codex_answer() -> Option<std::time::Duration> {
    match env("FAKE_CODEX_MODELS").as_deref() {
        Some("silent") => None,
        Some("slow") => Some(std::time::Duration::from_secs(2)),
        Some(_) => Some(std::time::Duration::ZERO),
        None if env("FAKE_MODELS").is_some() || env("FAKE_ENGINES_MIXED").is_some() => Some(std::time::Duration::ZERO),
        None => None,
    }
}

/// The person's own move from the picker, waiting for each member's next
/// ticket — or, `now`, for the end of its turn.
fn person_move(members: &[String], engine: Engine, model: Option<String>, now: bool) -> Proposal {
    let from = if engine == Engine::Codex { Engine::Claude } else { Engine::Codex };
    let moves = members.iter().map(|m| Move { member: m.clone(), role: m.clone(), from, to: engine, model: model.clone(), effort: None, was_model: None, done: false, now }).collect();
    Proposal { id: CARD + 2, why: Why::Person, engine: None, window: None, used_percent: 0, resets_at: 0, moves, applied: true, proposed_at: self::now() as u64, forecast: None }
}

/// The person's answers, as the daemon would take them. Whether it was one.
pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let squad = || SQUAD.to_owned();
    match command {
        Command::SetEngineSwitching { mode, .. } => {
            let _ = events.send(Event::EngineSwitchingChanged { squad: squad(), mode: *mode });
        }
        Command::SetDesignReviewer { reviewer, .. } => {
            let _ = events.send(Event::DesignReviewerChanged { squad: squad(), reviewer: *reviewer });
        }
        Command::SetParallel { parallel, .. } => {
            let _ = events.send(Event::ParallelChanged { squad: squad(), parallel: parallel.clamped() });
        }
        Command::ApplyEngineProposal { id, now, .. } => {
            // On moves made already, "Move now" marks those waiting; the card stays.
            if let Some(mut made) = proposals().into_iter().find(|p| p.id == *id && p.applied).filter(|_| *now) {
                for m in made.moves.iter_mut().filter(|m| !m.done) {
                    m.now = true;
                }
                let _ = events.send(Event::EngineProposed { squad: squad(), proposal: made });
                return true;
            }
            let _ = events.send(Event::EngineProposalClosed { squad: squad(), id: *id, applied: true });
            let text = if *now {
                "Moving backend·frontend. backend switches to Codex when this turn ends, mid-ticket: its ticket is handed over — same branch, with what's done and what's next — and carried on there."
            } else {
                "Moving backend·frontend. backend keeps Claude for the ticket in progress and switches to Codex from its next ticket, its work handed over."
            };
            let _ = events.send(Event::Notice { undo: None, session: 1, text: text.into() });
        }
        Command::DismissEngineProposal { id, .. } | Command::UndoEngineProposal { id, .. } => {
            let _ = events.send(Event::EngineProposalClosed { squad: squad(), id: *id, applied: false });
        }
        Command::ShowEngineSetup { .. } => {
            let _ = events.send(Event::EngineProposed { squad: squad(), proposal: setup() });
        }
        Command::Models { engine: Engine::Codex } => {
            if let Some(wait) = codex_answer() {
                let events = events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = events.send(Event::Models { engine: Engine::Codex, models: codex_models() });
                });
            }
        }
        Command::MoveEngine { members, engine, model, now, .. } => {
            let _ = events.send(Event::EngineProposed { squad: squad(), proposal: person_move(members, *engine, model.clone(), *now) });
        }
        Command::SetRoleModel { default, .. } => {
            let mut defaults: Vec<RoleModel> = role_models().into_iter().filter(|d| (d.role.as_str(), d.engine) != (default.role.as_str(), default.engine)).collect();
            defaults.extend((default.model.is_some() || default.effort.is_some()).then(|| default.clone()));
            let _ = events.send(Event::RoleModelsChanged { squad: squad(), defaults });
        }
        _ => return false,
    }
    true
}
