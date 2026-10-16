//! A change to the team the PM proposed (`propose_team_change`), in the
//! terminal: a prompt over the composer of the PM's conversation — `y`
//! approves (the daemon then does what `/add` or `/close` does), `n` says no —
//! while nothing is typed. `/add` stays as it was.

use crossterm::event::KeyCode;
use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::protocol::{Command, Event};
use equipe_core::t;

use super::{App, Pane};

/// What a change does, in a few words: "mobile 한 명 더 추가", "qa-2 내보내기".
pub fn describe(change: &TeamChange) -> String {
    match (change.change, change.count) {
        (Change::Add, 1) => t!("app.team_add_one", role = change.role),
        (Change::Add, n) => t!("app.team_add_many", role = change.role, n = n),
        (Change::Remove, _) => t!("app.team_remove", name = change.member.as_deref().unwrap_or(&change.role)),
    }
}

impl App {
    /// The change the conversation on screen shows: the oldest — in the PM's.
    pub fn shown_team_change(&self) -> Option<&TeamChange> {
        if !self.viewed()?.is_pm() {
            return None;
        }
        self.team_changes.first()
    }

    /// A change proposed, or closed (decided anywhere, or moot).
    pub(super) fn on_team_change(&mut self, event: Event) {
        match event {
            Event::TeamChangeProposed { change, .. } => {
                if !self.team_changes.iter().any(|c| c.id == change.id) {
                    self.flash = Some(t!("app.team_proposed", name = change.by, what = describe(&change)));
                    self.team_changes.push(change);
                }
            }
            Event::TeamChangeClosed { id, .. } => self.team_changes.retain(|c| c.id != id),
            _ => {}
        }
    }

    /// `y` or `n` while the change shows and nothing is typed: the person's
    /// answer, as their command. `None`: not its key.
    pub(super) fn on_team_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        if self.pane != Pane::Conversation || ctrl || alt || !self.composer.is_empty() {
            return None;
        }
        let id = self.shown_team_change()?.id;
        let squad = self.squad.clone();
        let command = match code {
            KeyCode::Char('y') => Command::ApproveTeamChange { squad, id },
            KeyCode::Char('n') => Command::DeclineTeamChange { squad, id },
            _ => return None,
        };
        // Out of sight at once, so a second key can't answer it twice.
        self.team_changes.retain(|c| c.id != id);
        Some(vec![command])
    }
}
