//! Handed-in work put to the person to confirm (`propose_accept`, or a screen
//! reading their own words): kept while it waits — in memory only, as what
//! the person said is about the board as it stands — and shown on every
//! screen. Confirmed by the person's own command, it runs exactly what the
//! board's buttons send: [`Command::Accept`], then [`Command::Merge`] when the
//! screen's way says so, or [`Command::Reopen`] with the note. Nothing here
//! accepts, merges or sends back any other way, and nobody's words confirm.

use equipe_protocol::accept::{AcceptProposal, Confirmed, Land, Verdict};

use super::*;
use crate::accept::{self as rules, Declined, Outcome};

impl Runtime {
    /// `squad`'s proposals waiting for the person, oldest first.
    pub(super) fn accepts(&mut self, squad: &str) -> &mut Vec<AcceptProposal> {
        self.accepts.entry(squad.to_owned()).or_default()
    }

    /// What the person said no to a moment ago, still fresh.
    fn declined_lately(&mut self, squad: &str) -> Vec<Declined> {
        let now = now_secs();
        let list = self.accepts_declined.entry(squad.to_owned()).or_default();
        list.retain(|d| now.saturating_sub(d.at) < rules::DECLINED_FOR);
        list.clone()
    }

    fn put_up(&mut self, squad: &str, proposal: AcceptProposal) {
        self.accepts(squad).push(proposal.clone());
        self.publish(Event::AcceptProposed { squad: squad.to_owned(), proposal });
    }

    /// `propose_accept`: checked against the board and what already waits,
    /// kept, and shown; the PM told the person confirms on a card.
    pub(super) fn propose_accept(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM puts handed-in work to the person to confirm, and only the person accepts it. Don't take \
their words as an acceptance, and don't walk them through buttons: say in one line that a confirmation card puts it to \
them, and note what they said on the ticket with `comment`, for the PM."
                .into());
        }
        let tickets = self.tickets(squad);
        let open = self.accepts.get(squad).cloned().unwrap_or_default();
        let declined = self.declined_lately(squad);
        match rules::check(args, &tickets, &open, &declined, now_secs())? {
            Outcome::Already { tickets, dropped } => Ok(rules::already_reply(&tickets, &dropped)),
            Outcome::Propose(checked) => {
                let reply = rules::proposed_reply(&checked, &tickets);
                let proposal = checked.into_proposal(equipe_protocol::new_session_id(), member, id, now_secs());
                self.put_up(squad, proposal);
                Ok(reply)
            }
        }
    }

    /// A screen read the person's own words as plainly accepting: the card
    /// goes up in that conversation, by the same rules; what doesn't fit is
    /// left to the member there to answer.
    pub(super) fn propose_accept_from_words(&mut self, squad: &str, session: SessionId, verdict: Verdict, named: &[TicketId], note: &str) {
        if !self.sessions.get(&session).is_some_and(|s| s.squad.as_deref() == Some(squad)) {
            return;
        }
        let tickets = self.tickets(squad);
        let open = self.accepts.get(squad).cloned().unwrap_or_default();
        let declined = self.declined_lately(squad);
        if let Ok(Outcome::Propose(checked)) = rules::check_tickets(verdict, named, note, "", &tickets, &open, &declined, now_secs()) {
            let proposal = checked.into_proposal(equipe_protocol::new_session_id(), equipe_protocol::accept::FROM_WORDS, session, now_secs());
            self.put_up(squad, proposal);
        }
    }

    /// Take proposal `id` out of what waits, if it still does.
    fn take_accept(&mut self, squad: &str, id: u64) -> Option<AcceptProposal> {
        let list = self.accepts(squad);
        let at = list.iter().position(|p| p.id == id)?;
        Some(list.remove(at))
    }

    /// The person's yes, for the tickets they kept on the card: each gets what
    /// the board's buttons send, the card closes everywhere. Decided already,
    /// elsewhere: nothing more.
    pub(super) async fn confirm_accept(&mut self, squad: &str, id: u64, picked: Vec<Confirmed>) {
        let Some(proposal) = self.take_accept(squad, id) else { return };
        let mut acted = Vec::new();
        for pick in picked.iter().filter(|p| proposal.tickets.contains(&p.ticket)) {
            let ticket = pick.ticket;
            let Some(t) = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()) else { continue };
            if rules::unfit(&t, proposal.verdict).is_some() {
                continue;
            }
            let squad = squad.to_owned();
            match proposal.verdict {
                Verdict::Changes => {
                    // What R on the board sends.
                    Box::pin(self.handle(Command::Reopen { squad, ticket, note: proposal.note.clone() })).await;
                }
                Verdict::Accept | Verdict::Merge => {
                    // What A on the board sends: accept, then land it the screen's way.
                    if t.status == TicketStatus::Review {
                        Box::pin(self.handle(Command::Accept { squad: squad.clone(), ticket })).await;
                    }
                    if let Some(land) = pick.merge.filter(|_| !t.kind.in_home() && t.branch.is_some()) {
                        Box::pin(self.handle(Command::Merge { squad, ticket, push: land == Land::Push })).await;
                    }
                }
            }
            acted.push(ticket);
        }
        self.publish(Event::AcceptClosed { squad: squad.to_owned(), id, confirmed: true, tickets: acted.clone() });
        let left: Vec<TicketId> = proposal.tickets.iter().copied().filter(|t| !acted.contains(t)).collect();
        if !left.is_empty() {
            let ids = left.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
            self.brief_pm(squad, format!("The person left {ids} out when they confirmed the card: as it was, it waits for them."), None);
        }
    }

    /// The person's no: nothing changes; the PM is told, and the same card
    /// isn't put to them again for a while.
    pub(super) fn decline_accept(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.take_accept(squad, id) else { return };
        self.publish(Event::AcceptClosed { squad: squad.to_owned(), id, confirmed: false, tickets: Vec::new() });
        self.accepts_declined.entry(squad.to_owned()).or_default().push(Declined { verdict: proposal.verdict, tickets: proposal.tickets.clone(), at: now_secs() });
        let ids = proposal.tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        let (what_en, key) = match proposal.verdict {
            Verdict::Accept => (format!("accepting {ids}"), "news.accept_declined"),
            Verdict::Merge => (format!("accepting and merging {ids}"), "news.merge_declined"),
            Verdict::Changes => (format!("sending {ids} back"), "news.changes_declined"),
        };
        let line = format!("The person said no to the card ({what_en}): it stays as it is. Don't put it to them again unless they ask.");
        let shown = self.said(key, &[("tickets", ids)]);
        self.brief_pm(squad, line, Some(shown));
    }

    /// `ticket` changed: a proposal it was on that has nothing left to
    /// confirm (decided on the board, sent back, dropped) no longer waits.
    /// Nobody is told — there is nothing to decide.
    pub(super) fn settle_accepts(&mut self, squad: &str, ticket: TicketId) {
        if !self.accepts.get(squad).is_some_and(|l| l.iter().any(|p| p.tickets.contains(&ticket))) {
            return;
        }
        let tickets = self.tickets(squad);
        let (kept, gone): (Vec<AcceptProposal>, Vec<AcceptProposal>) = std::mem::take(self.accepts(squad)).into_iter().partition(|p| rules::still_open(p, &tickets));
        *self.accepts(squad) = kept;
        for p in gone {
            self.publish(Event::AcceptClosed { squad: squad.to_owned(), id: p.id, confirmed: false, tickets: Vec::new() });
        }
    }
}
