//! Handed-in work the person said in words to accept, merge or send back, in
//! the terminal: a prompt over the composer of the conversation it was said
//! in — `y` confirms (the daemon then sends what A on the board sends, the
//! project's way of landing it as asked here), `n` says no — while nothing is
//! typed. `/accept` stays as it was.

use crossterm::event::KeyCode;
use equipe_core::protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};
use equipe_core::protocol::{Command, Event};
use equipe_core::t;

use super::{App, MergeWay, Pane, needs_way};

/// The tickets, as the prompt names them: "T-42, T-71".
pub fn ids(proposal: &AcceptProposal) -> String {
    proposal.tickets.iter().map(|id| format!("T-{id}")).collect::<Vec<_>>().join(", ")
}

/// What it does, in a few words: "T-42, T-71 수락".
pub fn describe(proposal: &AcceptProposal) -> String {
    let tickets = ids(proposal);
    match proposal.verdict {
        Verdict::Accept => t!("app.accept_what", tickets = tickets),
        Verdict::Merge => t!("app.accept_what_merge", tickets = tickets),
        Verdict::Changes => t!("app.accept_what_changes", tickets = tickets),
    }
}

impl App {
    /// The proposal the conversation on screen shows: the oldest said in it.
    pub fn shown_accept(&self) -> Option<&AcceptProposal> {
        let here = self.selected()?.id;
        self.accepts.iter().find(|p| p.session == here)
    }

    /// A proposal put up, or closed (decided anywhere, or moot).
    pub(super) fn on_accept(&mut self, event: Event) {
        match event {
            Event::AcceptProposed { proposal, .. } => {
                if !self.accepts.iter().any(|p| p.id == proposal.id) {
                    self.flash = Some(t!("app.accept_proposed", what = describe(&proposal)));
                    self.accepts.push(proposal);
                }
            }
            Event::AcceptClosed { id, .. } => self.accepts.retain(|p| p.id != id),
            _ => {}
        }
    }

    /// How a confirmed ticket lands: merged the project's way (to merge: at
    /// least merged), as A on the board would.
    fn landing(&self, proposal: &AcceptProposal, ticket: u32) -> Confirmed {
        let Some(t) = self.tickets.iter().find(|t| t.id == ticket) else { return Confirmed::only(ticket) };
        if proposal.verdict == Verdict::Changes || !(needs_way(t) || t.not_on_main()) {
            return Confirmed::only(ticket);
        }
        let way = self.merge_ways.get(&self.project_label(t)).copied();
        let merge = match (way, proposal.verdict) {
            (Some(MergeWay::Push), _) => Some(Land::Push),
            (Some(MergeWay::Merge), _) | (_, Verdict::Merge) => Some(Land::Merge),
            _ => None,
        };
        Confirmed { ticket, merge }
    }

    /// `y` or `n` while the proposal shows and nothing is typed: the person's
    /// answer, as their command. `None`: not its key.
    pub(super) fn on_accept_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        if self.pane != Pane::Conversation || ctrl || alt || !self.composer.is_empty() {
            return None;
        }
        let proposal = self.shown_accept()?.clone();
        let squad = self.squad.clone();
        let command = match code {
            KeyCode::Char('y') => {
                let tickets = proposal.tickets.iter().map(|t| self.landing(&proposal, *t)).collect();
                self.flash = Some(t!("app.accept_confirmed", what = describe(&proposal)));
                Command::ConfirmAccept { squad, id: proposal.id, tickets }
            }
            KeyCode::Char('n') => Command::DeclineAccept { squad, id: proposal.id },
            _ => return None,
        };
        // Out of sight at once, so a second key can't answer it twice.
        self.accepts.retain(|p| p.id != proposal.id);
        Some(vec![command])
    }
}
