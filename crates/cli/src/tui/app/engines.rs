//! Both engines' plans and members moving between them, in the terminal: the
//! plans as last read (the footer, `/usage`), the squad's choice (`/engines
//! auto|suggest|off`, `/engines setup` for the recommended setup again), and
//! the card over the composer — `y` makes the moves, `n` leaves things as they
//! are; for moves equipe made on its own, `u` undoes them and `n` closes it.
//! Only the person's key answers a card.

use crossterm::event::KeyCode;
use equipe_core::protocol::engines::{Move, Proposal, Switching, Why, Window};
use equipe_core::protocol::{Command, Engine, EnginePlan, Event, PlanUsage, Snapshot};
use equipe_core::t;

use super::{App, Pane};

/// What the screen knows of both plans and the squad's choice.
#[derive(Debug, Clone, Default)]
pub struct Engines {
    /// Each engine's plan as last read: Claude first.
    pub plans: Vec<EnginePlan>,
    pub mode: Switching,
    /// Cards waiting, oldest first.
    pub proposals: Vec<Proposal>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

impl Engines {
    pub fn from(snapshot: &Snapshot) -> Self {
        let mut plans = snapshot.plans.clone();
        // An older daemon says only Claude's.
        if plans.is_empty()
            && let Some(usage) = snapshot.plan
        {
            plans.push(EnginePlan { engine: Engine::Claude, usage, plan_type: None, at: now() });
        }
        let mut engines = Engines { plans, mode: snapshot.engine_switching, proposals: snapshot.engine_proposals.clone() };
        engines.plans.sort_by_key(|p| p.engine != Engine::Claude);
        engines
    }

    /// An engine's plan, just read.
    pub fn read(&mut self, engine: Engine, usage: PlanUsage) {
        self.plans.retain(|p| p.engine != engine);
        self.plans.push(EnginePlan { engine, usage, plan_type: None, at: now() });
        self.plans.sort_by_key(|p| p.engine != Engine::Claude);
    }

    pub fn plan(&self, engine: Engine) -> Option<&EnginePlan> {
        self.plans.iter().find(|p| p.engine == engine)
    }
}

pub fn mode_word(mode: Switching) -> &'static str {
    match mode {
        Switching::Off => t!("engines.mode_off"),
        Switching::Suggest => t!("engines.mode_suggest"),
        Switching::Auto => t!("engines.mode_auto"),
    }
}

pub fn mode_about(mode: Switching) -> &'static str {
    match mode {
        Switching::Off => t!("engines.mode_off_about"),
        Switching::Suggest => t!("engines.mode_suggest_about"),
        Switching::Auto => t!("engines.mode_auto_about"),
    }
}

/// "35분", "2시간", "3일": how long until `at`.
pub fn span(at: i64) -> String {
    let secs = (at - now()).max(60) as u64;
    match secs {
        ..3_600 => t!("engines.span_minutes", count = secs / 60),
        3_600..172_800 => t!("engines.span_hours", count = (secs + 1_800) / 3_600),
        _ => t!("engines.span_days", count = (secs + 43_200) / 86_400),
    }
}

fn window_word(window: Option<Window>) -> &'static str {
    match window {
        Some(Window::Weekly) => t!("engines.window_week"),
        _ => t!("engines.window_5h"),
    }
}

/// What a card says, in one line.
pub fn headline(p: &Proposal) -> String {
    let names = p.moves.iter().map(|m| m.member.as_str()).collect::<Vec<_>>().join("·");
    let engine = p.engine.map(engine_name).unwrap_or_default();
    let to = p.to().map(engine_name).unwrap_or(engine);
    let (window, used, span) = (window_word(p.window), p.used_percent, span(p.resets_at));
    match (p.why, p.applied) {
        (Why::Setup, _) => t!("engines.setup_title").to_owned(),
        (Why::StepDown, false) => t!("engines.step_down_ask", names = names),
        (Why::StepDown, true) => t!("engines.step_down", names = names),
        (Why::Return, _) => t!("engines.return", engine = engine, window = window, used = used, names = names),
        (Why::Exhausted, false) => t!("engines.hand_over_ask", engine = engine, span = span, names = names, to = to),
        (Why::Exhausted, true) => t!("engines.hand_over", engine = engine, span = span, names = names, to = to),
        (_, false) => t!("engines.ask", engine = engine, window = window, used = used, span = span, names = names, to = to),
        (_, true) => t!("engines.made", engine = engine, window = window, used = used, names = names, to = to),
    }
}

/// One move as a row: "backend  Claude opus → Codex 기본 모델 · medium".
pub fn move_row(m: &Move) -> (String, String) {
    let model = |m: Option<&str>| m.map_or_else(|| t!("engines.default_model").to_owned(), str::to_owned);
    let after = match &m.effort {
        Some(effort) => format!("{} {} · {effort}", engine_name(m.to), model(m.model.as_deref())),
        None => format!("{} {}", engine_name(m.to), model(m.model.as_deref())),
    };
    let done = if m.done { format!("  ✓ {}", t!("engines.done")) } else { String::new() };
    (m.member.clone(), format!("{} {} → {after}{done}", engine_name(m.from), model(m.was_model.as_deref())))
}

impl App {
    /// The card the conversation on screen shows: the oldest, in any member's conversation.
    pub fn shown_engine_proposal(&self) -> Option<&Proposal> {
        self.engines.proposals.first()
    }

    /// A card proposed or updated, closed, or the squad's choice changed.
    pub(super) fn on_engines_event(&mut self, event: Event) {
        match event {
            Event::EngineProposed { proposal, .. } => match self.engines.proposals.iter_mut().find(|p| p.id == proposal.id) {
                Some(shown) => *shown = proposal,
                None => {
                    self.flash = Some(headline(&proposal));
                    self.engines.proposals.push(proposal);
                }
            },
            Event::EngineProposalClosed { id, .. } => self.engines.proposals.retain(|p| p.id != id),
            Event::EngineSwitchingChanged { mode, .. } => self.engines.mode = mode,
            _ => {}
        }
    }

    /// `y`, `n` or `u` while a card shows and nothing is typed: the person's
    /// answer, as their command. `None`: not its key.
    pub(super) fn on_engines_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        if self.pane != Pane::Conversation || ctrl || alt || !self.composer.is_empty() {
            return None;
        }
        let card = self.shown_engine_proposal()?;
        let (id, applied) = (card.id, card.applied);
        let squad = self.squad.clone();
        let command = match code {
            KeyCode::Char('y') if !applied => Command::ApplyEngineProposal { squad, id },
            KeyCode::Char('u') if applied => Command::UndoEngineProposal { squad, id },
            KeyCode::Char('n') => Command::DismissEngineProposal { squad, id },
            _ => return None,
        };
        // Out of sight at once, so a second key can't answer it twice.
        self.engines.proposals.retain(|p| p.id != id);
        Some(vec![command])
    }

    /// `/engines [auto|suggest|off|setup]`.
    pub(super) fn engines_command(&mut self, rest: &str) -> Vec<Command> {
        let squad = self.squad.clone();
        match rest.trim() {
            "" => {
                self.flash = Some(t!("engines.now", mode = mode_word(self.engines.mode), about = mode_about(self.engines.mode)));
                Vec::new()
            }
            "setup" => vec![Command::ShowEngineSetup { squad }],
            word => match Switching::parse(word) {
                Some(mode) => {
                    self.engines.mode = mode;
                    self.flash = Some(t!("engines.set", mode = mode_word(mode), about = mode_about(mode)));
                    vec![Command::SetEngineSwitching { squad, mode }]
                }
                None => {
                    self.flash = Some(t!("engines.usage").into());
                    Vec::new()
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::PlanWindow;

    fn card(applied: bool) -> Proposal {
        Proposal {
            id: 7,
            why: Why::Shift,
            engine: Some(Engine::Claude),
            window: Some(Window::FiveHour),
            used_percent: 86,
            resets_at: now() + 2 * 3600,
            moves: vec![
                Move { member: "backend".into(), role: "backend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("opus".into()), done: false },
                Move { member: "frontend".into(), role: "frontend".into(), from: Engine::Claude, to: Engine::Codex, model: None, effort: None, was_model: Some("sonnet".into()), done: true },
            ],
            applied,
            proposed_at: 0,
        }
    }

    #[test]
    fn a_card_says_the_engine_the_limit_the_reset_and_who_moves() {
        assert_eq!(headline(&card(false)), "Claude 5-hour limit at 86% · resets in 2 hours — move backend·frontend to Codex from their next ticket?");
        assert_eq!(headline(&card(true)), "backend·frontend: Claude 5-hour limit at 86% → Codex from the next ticket");
        assert_eq!(move_row(&card(true).moves[0]), ("backend".to_owned(), "Claude opus → Codex default model".to_owned()));
        assert!(move_row(&card(true).moves[1]).1.ends_with("✓ done"));
        let setup = Proposal { why: Why::Setup, engine: None, window: None, ..card(false) };
        assert!(headline(&setup).contains("recommended setup"));
    }

    #[test]
    fn an_older_daemons_plan_is_claudes_and_a_reading_replaces_its_engines() {
        let snapshot = Snapshot { squad: "acme".into(), plan: Some(PlanUsage { five_hour: Some(PlanWindow { used_percent: 40, resets_at: 9 }), seven_day: None }), ..Default::default() };
        let mut engines = Engines::from(&snapshot);
        assert_eq!(engines.plans.iter().map(|p| p.engine).collect::<Vec<_>>(), [Engine::Claude]);
        assert_eq!(engines.mode, Switching::Suggest);
        engines.read(Engine::Codex, PlanUsage::default());
        engines.read(Engine::Claude, PlanUsage::default());
        assert_eq!(engines.plans.iter().map(|p| p.engine).collect::<Vec<_>>(), [Engine::Claude, Engine::Codex], "Claude first");
        assert_eq!(engines.plan(Engine::Claude).unwrap().usage, PlanUsage::default());
    }
}
