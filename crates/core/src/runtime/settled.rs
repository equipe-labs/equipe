//! Questions put to the person about a ticket that is settled — done,
//! cancelled, merged — are withdrawn: a decision card for work already
//! finished only asks again what was decided. Its member's conversation
//! hears it in one line — and so do its secret, sign-in and `ask_person`
//! cards (`runtime::withdraw`). Cards kept from before (a daemon that didn't do
//! this) are swept when the squad's questions are first read.

use equipe_protocol::decide::QuestionSet;

use super::*;

impl Runtime {
    /// Ticket `ticket` of `squad` closed: the questions asked about it are
    /// withdrawn — the design review's and the retro's, which aren't about
    /// one ticket's work, aside.
    pub(super) fn withdraw_settled_questions(&mut self, squad: &str, ticket: TicketId) {
        // Its secret, sign-in and `ask_person` cards go with it too.
        self.withdraw_ticket_cards(squad, ticket, super::withdraw::TicketTurn::Closed);
        let settled = |s: &QuestionSet| s.ticket == Some(ticket) && !s.review && s.retro.is_none();
        let gone: Vec<(SessionId, String, usize)> =
            self.question_sets(squad).iter().filter(|s| settled(s)).map(|s| (s.session, s.member.clone(), s.questions.len())).collect();
        if gone.is_empty() {
            return;
        }
        self.drop_questions(squad, settled);
        for (session, member, count) in gone {
            // Its run may be over with the ticket: the member's own conversation then.
            let to = Some(session).filter(|s| self.sessions.contains_key(s)).or_else(|| self.member_id(squad, &member));
            if let Some(session) = to {
                let text = say!(self, "news.questions_withdrawn", id = ticket, count = count);
                self.publish(Event::Notice { session, text, undo: None });
            }
        }
        crate::daemon_log!("equipe: {squad}: questions about T-{ticket} withdrawn: it is settled");
    }

    /// `kept`, as a squad's questions are first read, without those about a
    /// ticket already settled.
    pub(super) fn without_settled(&mut self, squad: &str, kept: Vec<QuestionSet>) -> Vec<QuestionSet> {
        let Some(journal) = self.journals.get(squad) else { return kept };
        let closed = |t: TicketId| journal.ticket(t).ok().flatten().is_some_and(|t| t.status.is_closed());
        let before = kept.len();
        let kept: Vec<QuestionSet> = kept.into_iter().filter(|s| s.review || s.retro.is_some() || !s.ticket.is_some_and(closed)).collect();
        if kept.len() < before {
            crate::daemon_log!("equipe: {squad}: {} question card(s) about settled tickets withdrawn", before - kept.len());
        }
        kept
    }
}
