//! The reserve for the person, as the daemon makes it so ([`crate::pacing`]):
//! on auto, every minute (as the plans' use is read) each engine's weekly
//! window is held against its reserve. While more than the reserve is left
//! nothing is held back; under it, [`Runtime::dispatch_to`] (which asks
//! [`Runtime::pace_of`]) starts only the PM's work and urgent or high tickets
//! on that engine. Nothing running is stopped. A change is told once — the
//! PM's news line and every screen — and counted in the squad's journal for
//! the usage report.

use super::*;
use crate::pacing::{self, Budget, Change, State, Until};
use equipe_protocol::lanes::Pacing;

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
    /// Where `engine`'s reserve stands for `squad`: whether it is held.
    pub(super) fn pace_of(&self, squad: &str, engine: Engine) -> State {
        self.pacers.by.get(&(squad.to_owned(), engine)).copied().unwrap_or_default()
    }

    /// The engines whose reserve is held for `squad`, for the screens.
    pub(super) fn pacing_now(&self, squad: &str) -> Vec<Pacing> {
        [Engine::Claude, Engine::Codex]
            .into_iter()
            .filter_map(|engine| {
                let state = self.pace_of(squad, engine);
                if !state.held {
                    return None;
                }
                let budget = self.pace_budget(squad, engine)?;
                let until = pacing::horizon(&budget, now_secs() as i64)?;
                Some(Pacing {
                    engine,
                    left: budget.left().floor() as u32,
                    reserve: pacing::RESERVE as u32,
                    until: until.at(),
                    other_engine: matches!(until, Until::OtherBack(_)),
                    since: state.since,
                })
            })
            .collect()
    }

    /// The weekly window of `engine` for `squad`, as the reserve reads it.
    fn pace_budget(&self, squad: &str, engine: Engine) -> Option<Budget> {
        let (week, at) = self.week_window(engine)?;
        Some(Budget { used: f64::from(week.used_percent), resets_at: week.resets_at, other_back: self.engine_out(squad, other_of(engine)), age: now_secs() as i64 - at })
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
            let Some(budget) = self.pace_budget(squad, engine) else {
                // No week read, or it just started over: nothing to hold.
                if prev.held {
                    self.pacers.by.remove(&(squad.to_owned(), engine));
                    changed = true;
                }
                continue;
            };
            let (next, change) = pacing::step(prev, &budget, &parallel, now);
            if next == prev {
                continue;
            }
            changed = true;
            if next.held {
                self.pacers.by.insert((squad.to_owned(), engine), next);
            } else {
                self.pacers.by.remove(&(squad.to_owned(), engine));
            }
            match change {
                Some(change) => self.tell_pacing(squad, engine, change),
                // The person chose full speed.
                None if parallel.is_max() && prev.held => {
                    self.log_engines(squad, format!("reserve on {} ended: the person runs at full speed", named(engine)));
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
            Change::Held { until } | Change::Lifted { until } => until,
        };
        let (name, span, other) = (named(engine), self.span(until.at() - now), named(other_of(engine)));
        let reserve = pacing::RESERVE as u32;
        let text = match change {
            Change::Held { until: Until::Reset(_) } => say!(self, "engines.pace_hold", engine = name, reserve = reserve, span = span),
            Change::Held { until: Until::OtherBack(_) } => say!(self, "engines.pace_hold_other", engine = name, reserve = reserve, span = span, other = other),
            Change::Lifted { .. } => say!(self, "engines.pace_off", engine = name),
        };
        self.log_engines(squad, format!("reserve on {name}: {text}"));
        self.count_pacing(squad, engine, change.kind());
        let line = match change {
            Change::Held { .. } => format!("equipe: {text}. If the person asks why other work isn't starting, say so plainly; the PM's own work and urgent or high tickets still start at once. Don't lower models or effort to get around it."),
            Change::Lifted { .. } => format!("equipe: {text}."),
        };
        self.brief_pm(squad, line, Some(text));
    }
}
