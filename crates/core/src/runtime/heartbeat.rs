//! The squad's heartbeat (`crate::heartbeat`): routines the PM proposes,
//! kept while they wait for the person — in memory only, like a change to
//! the team — and turned on only by the person's click, as their own
//! [`Command::SaveRoutine`] would; and the PM woken once over a board left
//! empty, at most once a day, unless the person turned that off.

use equipe_protocol::routines::RoutineProposal;

use super::*;
use crate::heartbeat as rules;

impl Runtime {
    /// `squad`'s proposed routines waiting for the person, oldest first.
    pub(super) fn routine_proposals(&mut self, squad: &str) -> &mut Vec<RoutineProposal> {
        self.routine_proposals.entry(squad.to_owned()).or_default()
    }

    /// `propose_routine`: checked, kept, and shown; the PM told the person
    /// decides on a card.
    pub(super) fn propose_routine(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM proposes routines. Tell the PM what should come regularly, and it proposes it to the person.".into());
        }
        let kept = self.routines(squad);
        let waiting = self.routine_proposals(squad).clone();
        let (routine, reason) = rules::check(args, &kept, &waiting, MAX_ROUTINES)?;
        let title = routine.title.clone();
        let proposal = RoutineProposal { id: equipe_protocol::new_session_id(), routine, reason, by: member.to_owned(), session: id, proposed_at: now_secs() };
        self.routine_proposals(squad).push(proposal.clone());
        self.publish(Event::RoutineProposed { squad: squad.to_owned(), proposal });
        Ok(format!(
            "\"{title}\" is on a card in front of the person: one click turns it on. You are told what they decide; \
don't ask them in words meanwhile."
        ))
    }

    fn take_routine_proposal(&mut self, squad: &str, id: u64) -> Option<RoutineProposal> {
        let waiting = self.routine_proposals(squad);
        let at = waiting.iter().position(|p| p.id == id)?;
        Some(waiting.remove(at))
    }

    /// The person's yes: saved as their own `SaveRoutine` saves it, the card
    /// closed everywhere, and the PM told. Decided already: nothing more.
    pub(super) fn approve_routine(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.take_routine_proposal(squad, id) else { return };
        let title = proposal.routine.title.clone();
        let before = self.routines(squad).len();
        self.save_routine(squad, proposal.routine);
        let saved = self.routines(squad).len() > before;
        self.publish(Event::RoutineProposalClosed { squad: squad.to_owned(), id, approved: saved });
        if saved {
            let line = format!("The person turned on your routine \"{title}\": it comes to you on its schedule as a [Routine · {title}] message.");
            let shown = say!(self, "news.routine_on", title = title);
            self.brief_pm(squad, line, Some(shown));
        }
    }

    /// The person's no: nothing is saved; the PM is told, not to propose it again.
    pub(super) fn decline_routine(&mut self, squad: &str, id: u64) {
        let Some(proposal) = self.take_routine_proposal(squad, id) else { return };
        let title = proposal.routine.title;
        self.publish(Event::RoutineProposalClosed { squad: squad.to_owned(), id, approved: false });
        let line = format!("The person declined your routine \"{title}\". Don't propose it again unless they ask.");
        let shown = say!(self, "news.routine_declined", title = title);
        self.brief_pm(squad, line, Some(shown));
    }

    /// The person's word, in a conversation of `session`'s squad: the board isn't idle.
    pub(super) fn note_person_active(&mut self, session: SessionId) {
        let Some(squad) = self.sessions.get(&session).and_then(|s| s.squad.clone()) else { return };
        if let Some(Err(err)) = self.journal(&squad).map(|j| j.set_active_at(now_secs())) {
            crate::daemon_log!("equipe: {squad}: when the person was last here wasn't kept: {err}");
        }
    }

    /// The person sets how long an empty board waits before the PM is woken
    /// (0: never); a board already quiet that long wakes it now.
    pub(super) async fn set_idle_wake(&mut self, squad: &str, hours: u32) {
        let Some(journal) = self.journal(squad) else { return };
        if let Err(err) = journal.set_idle_wake_hours(hours) {
            crate::daemon_log!("equipe: {squad}: the idle wake wasn't kept: {err}");
        }
        self.wake_if_idle(squad).await;
        self.publish(Event::IdleWakeChanged { squad: squad.to_owned(), hours });
    }

    /// Each minute: every squad whose board lies empty and quiet wakes its PM.
    pub(super) async fn wake_idle_pms(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.wake_if_idle(&squad).await;
        }
    }

    /// The PM woken once over an empty, quiet board — as a routine is handed
    /// to it, after its turn if one runs. `true`: it was.
    async fn wake_if_idle(&mut self, squad: &str) -> bool {
        let Some((hours, active_at, woken_at)) = self.journal(squad).map(|j| (j.idle_wake_hours(), j.active_at(), j.idle_woken_at())) else {
            return false;
        };
        if hours == 0 {
            return false;
        }
        let tickets = self.tickets(squad);
        let now = now_secs();
        if !rules::idle_due(hours, rules::open_tickets(&tickets), rules::last_activity(&tickets, active_at), woken_at, now) {
            return false;
        }
        let Some(pm) = self.pm_of(squad) else { return false };
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_idle_woken_at(now)) {
            crate::daemon_log!("equipe: {squad}: the idle wake's time wasn't kept: {err}");
            return false;
        }
        crate::daemon_log!("equipe: {squad}: the board lay empty for {hours}h; the PM is woken");
        let text = rules::idle_message(hours, self.routines(squad).len());
        let about = self.hq(squad).map(|hq| board::project_name(&hq));
        self.said_about = Some(about);
        match self.sessions.get(&pm) {
            Some(s) if s.is_working() => self.leave_memo(pm, text),
            _ => self.submit(pm, text).await,
        }
        self.said_about = None;
        true
    }
}
