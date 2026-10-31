//! A routine the PM proposed (`propose_routine`), in the terminal: a prompt
//! over the composer of the PM's conversation — `y` turns it on (saved as the
//! person's own), `n` says no — while nothing is typed. And `/wake`: how long
//! an empty board waits before the PM is woken to propose what's next.

use crossterm::event::KeyCode;
use equipe_core::protocol::routines::{IDLE_WAKE_HOURS, RoutineProposal, Schedule};
use equipe_core::protocol::{Command, Event};
use equipe_core::t;

use super::{App, Pane};

fn day_word(weekday: u8) -> &'static str {
    match weekday {
        1 => t!("view.routine_day_1"),
        2 => t!("view.routine_day_2"),
        3 => t!("view.routine_day_3"),
        4 => t!("view.routine_day_4"),
        5 => t!("view.routine_day_5"),
        6 => t!("view.routine_day_6"),
        _ => t!("view.routine_day_7"),
    }
}

/// When it runs, in words: "매일 09:00", "매주 월요일 09:00".
pub fn schedule_words(when: Schedule) -> String {
    let (hour, minute) = when.hour_minute();
    let time = format!("{hour:02}:{minute:02}");
    match when {
        Schedule::Daily { .. } => t!("view.routine_daily", time = time),
        Schedule::Weekly { weekday, .. } => t!("view.routine_weekly", day = day_word(weekday), time = time),
    }
}

impl App {
    /// The proposal the conversation on screen shows: the oldest — in the PM's.
    pub fn shown_routine_proposal(&self) -> Option<&RoutineProposal> {
        if !self.viewed()?.is_pm() {
            return None;
        }
        self.routine_proposals.first()
    }

    /// A routine proposed, or closed (decided anywhere); the idle wake changed.
    pub(super) fn on_routine_event(&mut self, event: Event) {
        match event {
            Event::RoutineProposed { proposal, .. } => {
                if !self.routine_proposals.iter().any(|p| p.id == proposal.id) {
                    self.flash = Some(t!("app.routine_proposed", name = proposal.by, title = proposal.routine.title));
                    self.routine_proposals.push(proposal);
                }
            }
            Event::RoutineProposalClosed { id, .. } => self.routine_proposals.retain(|p| p.id != id),
            Event::IdleWakeChanged { hours, .. } => self.idle_wake_hours = hours,
            _ => {}
        }
    }

    /// `y` or `n` while the proposal shows and nothing is typed: the person's
    /// answer, as their command. `None`: not its key.
    pub(super) fn on_routine_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        if self.pane != Pane::Conversation || ctrl || alt || !self.composer.is_empty() {
            return None;
        }
        let id = self.shown_routine_proposal()?.id;
        let squad = self.squad.clone();
        let command = match code {
            KeyCode::Char('y') => Command::ApproveRoutine { squad, id },
            KeyCode::Char('n') => Command::DeclineRoutine { squad, id },
            _ => return None,
        };
        // Out of sight at once, so a second key can't answer it twice.
        self.routine_proposals.retain(|p| p.id != id);
        Some(vec![command])
    }

    /// `/wake [on|off]`: whether the PM is woken after the board has lain
    /// empty a day. Bare, it says how it is now.
    pub(super) fn wake_command(&mut self, rest: &str) -> Vec<Command> {
        let hours = match rest.trim() {
            "" => {
                self.flash = Some(match self.idle_wake_hours {
                    0 => t!("app.wake_now_off").into(),
                    hours => t!("app.wake_now_on", hours = hours),
                });
                return Vec::new();
            }
            "on" => IDLE_WAKE_HOURS,
            "off" => 0,
            _ => {
                self.flash = Some("/wake on · /wake off".into());
                return Vec::new();
            }
        };
        self.idle_wake_hours = hours;
        self.flash = Some(if hours == 0 { t!("app.wake_off").into() } else { t!("app.wake_on", hours = hours) });
        vec![Command::SetIdleWake { squad: self.squad.clone(), hours }]
    }
}
