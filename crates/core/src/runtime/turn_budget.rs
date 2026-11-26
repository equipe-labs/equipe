//! A ticket's budget held mid-turn (`crate::spend::mid_turn`). The budget used
//! to be read only between turns, so one long turn could run far past it: a
//! logo turn at o1path (2026-11-26) used 19.4M tokens against an 8M budget
//! before anything stopped it. The engine now says the turn's running total
//! as its requests come (`Activity::Spent`); past the ticket's budget the
//! member is told once to finish with what it has (two options for the
//! person to pick, if that is what is left), and well past it the turn is
//! wound down where it is — its conversation and worktree kept — and the
//! ticket goes to the PM as any ticket over budget does at its turn's end.

use super::*;
use crate::spend::{self, MidTurn};

/// One running turn against its ticket's budget.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TurnBudget {
    /// Where the ticket stops (`spend::limit`); `None`: no budget.
    limit: Option<u64>,
    /// What the ticket's earlier turns used.
    before: u64,
    told: bool,
    stopped: bool,
}

impl Runtime {
    /// A turn's running total, from its engine: held against its ticket's budget.
    pub(super) fn turn_spent(&mut self, id: SessionId, tokens: u64) {
        let Some((squad, ticket, name)) = self.sessions.get(&id).and_then(|s| Some((s.squad.clone()?, s.work.as_ref()?.ticket, s.name().unwrap_or("member").to_owned()))) else {
            return;
        };
        if !self.turn_budgets.contains_key(&id) {
            let budget = self.ticket_budget_now(&squad, ticket);
            self.turn_budgets.insert(id, budget);
        }
        let Some(budget) = self.turn_budgets.get_mut(&id) else { return };
        let Some(limit) = budget.limit else { return };
        match spend::mid_turn(budget.before, tokens, limit) {
            MidTurn::Under => {}
            MidTurn::WrapUp if !budget.told => {
                budget.told = true;
                let spent = budget.before + tokens;
                crate::daemon_log!("equipe: {squad}: {name}'s turn on T-{ticket} passed its budget mid-turn ({} of {}): told to finish", crate::usage::amount(spent), crate::usage::amount(limit));
                let prompt = spend::wrap_up_prompt(spent, limit);
                match self.sessions.get(&id).and_then(|s| s.steer.clone()) {
                    Some(steer) => {
                        let _ = steer.send(prompt);
                    }
                    None => self.leave_memo(id, prompt),
                }
            }
            MidTurn::Stop if !budget.stopped => {
                budget.stopped = true;
                budget.told = true;
                crate::daemon_log!(
                    "equipe: {squad}: {name}'s turn on T-{ticket} went on far past its budget ({} of {}): wound down where it is",
                    crate::usage::amount(budget.before + tokens),
                    crate::usage::amount(limit)
                );
                // Ended as far as it went, its conversation kept; the turn's end hands the ticket to the PM (`guards`).
                if let Some(wind) = self.sessions.get_mut(&id).and_then(|s| s.wind_down.take()) {
                    let _ = wind.send(());
                }
            }
            _ => {}
        }
    }

    /// `ticket`'s budget as its turn starts: where it stops, and what its turns used before.
    fn ticket_budget_now(&mut self, squad: &str, ticket: TicketId) -> TurnBudget {
        let Some(journal) = self.journal(squad) else { return TurnBudget::default() };
        let Some(found) = journal.ticket(ticket).ok().flatten() else { return TurnBudget::default() };
        let settings = spend::for_ticket(journal.spend_settings(), &found.title, &found.body);
        let record = journal.ticket_entries(ticket).unwrap_or_default();
        let limit = spend::limit(&settings, &record);
        let before = journal.ticket_spend(&[ticket]).ok().and_then(|m| m.get(&ticket).copied()).map_or(0, |(_, tokens)| tokens);
        TurnBudget { limit, before, ..TurnBudget::default() }
    }
}
