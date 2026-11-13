//! Pace to budget, as the daemon makes it so ([`crate::pacing`]): on auto,
//! every minute (as the plans' use is read) each engine's weekly burn is held
//! against what its plan can sustain, and the squad's new runs on it are
//! capped to match. [`Runtime::dispatch_to`] asks [`Runtime::pace_of`] when it
//! sizes the room; nothing running is stopped. A change is told once — the
//! PM's news line and every screen — and counted in the squad's journal for
//! the usage report.

use super::*;
use crate::pacing::{self, Budget, Change, State, Until};
use equipe_protocol::lanes::{Pacing, Parallel};

#[derive(Default)]
pub(super) struct Pacers {
    by: HashMap<(String, Engine), State>,
}

fn named(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

fn other_of(engine: Engine) -> Engine {
    if engine == Engine::Claude { Engine::Codex } else { Engine::Claude }
}

impl Runtime {
    /// Where `engine`'s pacing stands for `squad`: the cap on its runs, if any.
    pub(super) fn pace_of(&self, squad: &str, engine: Engine) -> State {
        self.pacers.by.get(&(squad.to_owned(), engine)).copied().unwrap_or_default()
    }

    /// The engines paced for `squad`, for the screens.
    pub(super) fn pacing_now(&self, squad: &str) -> Vec<Pacing> {
        [Engine::Claude, Engine::Codex]
            .into_iter()
            .filter_map(|engine| {
                let state = self.pace_of(squad, engine);
                let cap = state.cap?;
                let (_, until) = self.pace_budget(squad, engine)?;
                Some(Pacing { engine, cap: cap as u32, of: self.pace_base(engine) as u32, until: until.at(), other_engine: matches!(until, Until::OtherBack(_)), since: state.since })
            })
            .collect()
    }

    /// The runs at once the lane policy allows `engine` without pacing.
    pub(super) fn pace_base(&self, engine: Engine) -> usize {
        let used = crate::lanes::plan_used(self.plans.iter().find(|p| p.engine == engine).map(|p| &p.usage));
        crate::lanes::engine_limit(used, self.disk.status.level, &Parallel::default())
    }

    /// Runs at work on `engine` for `squad` that hold an admission slot.
    /// While the engine is paced, runs paused for a limit or a retry don't:
    /// they burn nothing. A member's conversation with no ticket is no run at
    /// all, so it never holds one.
    pub(super) fn pace_runs(&self, squad: &str, engine: Engine, paced: bool) -> usize {
        self.sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad) && s.engine == engine && s.run_of.is_some())
            .filter(|s| pacing::holds_slot(Runtime::is_parked(s), paced))
            .count()
    }

    /// The week's budget of `engine` for `squad`, and until when it should last.
    fn pace_budget(&self, squad: &str, engine: Engine) -> Option<(Budget, Until)> {
        let (week, at, burn) = self.week_burn(squad, engine)?;
        let now = now_secs() as i64;
        let paced = self.pace_of(squad, engine).cap.is_some();
        let runs = self.pace_runs(squad, engine, paced);
        let budget = Budget { used: f64::from(week.used_percent), resets_at: week.resets_at, burn, runs, other_back: self.engine_out(squad, other_of(engine)), age: now - at };
        let per_run = burn.filter(|_| runs > 0).map(|b| b / runs as f64);
        let (_, until) = pacing::sustainable(&budget, now, per_run)?;
        Some((budget, until))
    }

    /// Look at every squad's engines again.
    pub(super) fn pace_pass(&mut self) {
        for squad in self.squads_with_members() {
            self.pace_squad(&squad);
        }
    }

    fn pace_squad(&mut self, squad: &str) {
        let parallel = self.journals.get(squad).map(Journal::parallel).unwrap_or_default();
        let now = now_secs() as i64;
        let mut changed = false;
        for engine in [Engine::Claude, Engine::Codex] {
            let prev = self.pace_of(squad, engine);
            let Some((budget, _)) = self.pace_budget(squad, engine) else {
                // No week read, or it just started over: nothing to pace.
                if prev.cap.is_some() {
                    self.pacers.by.remove(&(squad.to_owned(), engine));
                    changed = true;
                }
                continue;
            };
            let (next, change) = pacing::step(prev, self.pace_base(engine), &budget, &parallel, now);
            if next == prev {
                continue;
            }
            changed = true;
            if next.cap.is_some() {
                self.pacers.by.insert((squad.to_owned(), engine), next);
            } else {
                self.pacers.by.remove(&(squad.to_owned(), engine));
            }
            match change {
                Some(change) => self.tell_pacing(squad, engine, change),
                // The person chose full speed.
                None if parallel.is_max() && prev.cap.is_some() => {
                    self.log_engines(squad, format!("pacing {} ended: the person runs at full speed", named(engine)));
                    self.count_pacing(squad, engine, pacing::kind::OFF);
                    let text = say!(self, "engines.pace_override", engine = named(engine));
                    self.brief_pm(squad, format!("equipe: {text}"), Some(text));
                }
                None => {}
            }
        }
        if changed {
            let pacing = self.pacing_now(squad);
            self.publish(Event::PacingChanged { squad: squad.to_owned(), pacing });
            // Room that opened is used at the next dispatch: every turn's end makes one.
        }
    }

    fn count_pacing(&mut self, squad: &str, engine: Engine, kind: &str) {
        if let Some(journal) = self.journal(squad) {
            let _ = journal.count_turn_ending(kind, None, named(engine));
        }
    }

    /// Say a change once: the PM's news and every screen's, and the journal.
    fn tell_pacing(&mut self, squad: &str, engine: Engine, change: Change) {
        let now = now_secs() as i64;
        let until = match change {
            Change::Lowered { until, .. } | Change::Raised { until, .. } | Change::Lifted { until } => until,
        };
        let (name, span, other) = (named(engine), self.span(until.at() - now), named(other_of(engine)));
        let text = match change {
            Change::Lowered { to, of, until: Until::Reset(_) } => say!(self, "engines.pace_lower", engine = name, cap = to, base = of, span = span),
            Change::Lowered { to, of, until: Until::OtherBack(_) } => say!(self, "engines.pace_lower_other", engine = name, cap = to, base = of, span = span, other = other),
            Change::Raised { to, of, .. } => say!(self, "engines.pace_raise", engine = name, cap = to, base = of, span = span),
            Change::Lifted { .. } => say!(self, "engines.pace_off", engine = name, span = span),
        };
        self.log_engines(squad, format!("pacing {name}: {text}"));
        self.count_pacing(squad, engine, change.kind());
        let line = format!("equipe: {text}. If the person asks why work starts more slowly, say so plainly; urgent tickets and your own work still start at once. Don't lower models or effort to get around it.");
        self.brief_pm(squad, line, Some(text));
    }
}
