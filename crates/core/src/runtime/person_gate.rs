//! What reaches the person as a card (`crate::person_gate` has the rules):
//! where the PM accepts the team's work, handed-in work it would put to them
//! that is small — nothing about it weighty, by what it is, its title, or the
//! PM's own `why` — is the PM's to decide, no card; it goes into the person's
//! daily digest ("Decided for you", under Your requests). What is weighty
//! goes up with why, each ticket named by title and what changes for people.

use equipe_protocol::requests::Decided;

use super::*;
use crate::person_gate::{self as rules, Why};

/// What `propose_accept` does with work, by the gate.
pub(super) enum Gate {
    /// Small: no card; the PM is told so, in this reply.
    Pm(String),
    /// A card, with this line on why it needs the person (empty: no why to give).
    Card(String),
}

/// The `why` the PM gave, if any.
fn why_arg(args: &Value) -> Result<Option<Why>, String> {
    let Some(code) = board::arg_str(args, "why").map(str::trim).filter(|w| !w.is_empty()) else { return Ok(None) };
    let why = match code {
        "data" => Why::Data,
        "security" => Why::Security,
        "money" => Why::Money,
        "legal" => Why::Legal,
        "visible" => Why::Visible,
        "irreversible" => Why::Irreversible,
        "outward" => Why::Outward,
        other => return Err(format!("`why` is data, security, money, legal, visible, irreversible or outward, not {other}.")),
    };
    Ok(Some(why))
}

impl Runtime {
    /// Whether `tickets` go to the person on a card, and why.
    pub(super) fn gate_card(&mut self, squad: &str, member: &str, tickets: &[TicketId], args: &Value) -> Result<Gate, String> {
        let said = why_arg(args)?;
        let board: Vec<Ticket> = tickets.iter().filter_map(|t| self.find_ticket(squad, *t)).collect();
        let weighed: Vec<(&Ticket, Option<Why>)> = board.iter().map(|t| (t, rules::weigh(t, &[]).or(said))).collect();
        // The PM never accepts its own work: that always goes to the person.
        let own = board.iter().any(|t| t.assignee.as_deref() == Some(member) || (t.assignee.is_none() && t.owner.as_deref() == Some(member)));
        let pm_decides = !own && self.journals.get(squad).is_some_and(Journal::pm_accepts);
        if pm_decides && weighed.iter().all(|(_, why)| why.is_none()) {
            let list = board.iter().map(|t| format!("T-{} {}", t.id, rules::named(t))).collect::<Vec<_>>().join("; ");
            return Ok(Gate::Pm(format!(
                "No card: none of this needs the person — {list}. It is small and low-risk, so it is yours to decide: accept it \
with accept_work (or send it back with comment, review: changes). It goes into their daily digest under Your requests. Put to \
them only what touches production data, permissions or security, money, legal matters, a big visible change, or what \
can't be taken back — then give `why`."
            )));
        }
        let weighty: Vec<String> = weighed
            .iter()
            .filter_map(|(t, why)| why.map(|w| format!("“{}” ({})", t.title, crate::i18n::core(self.config.lang).get(w.key()))))
            .collect();
        Ok(Gate::Card(if weighty.is_empty() { String::new() } else { say!(self, "gate.card_why", list = weighty.join(", ")) }))
    }

    /// What the PM decided for the person in the last day, without a card:
    /// the work it accepted or sent back, the latest first.
    pub(super) fn decided_for_person(&self, squad: &str) -> Vec<Decided> {
        let Some(journal) = self.journals.get(squad) else { return Vec::new() };
        let since = now_secs().saturating_sub(crate::requests::LISTED_DONE);
        journal
            .requests()
            .decided_by_pm(since)
            .into_iter()
            .filter_map(|(ticket, at, accepted, landed_by)| {
                let t = journal.ticket(ticket).ok().flatten()?;
                Some(Decided { ticket, title: t.title.clone(), what: rules::what_changes(&t), accepted, at, landed_by })
            })
            .collect()
    }

    /// Each minute: the screens hear the digest when it changed.
    pub(super) fn tell_decided(&mut self, squad: &str) {
        let decided = self.decided_for_person(squad);
        let key: Vec<(TicketId, u64)> = decided.iter().map(|d| (d.ticket, d.at)).collect();
        // Nothing told yet and nothing to tell: no news either.
        if self.decided_told.get(squad).map_or(key.is_empty(), |told| *told == key) {
            return;
        }
        self.decided_told.insert(squad.to_owned(), key);
        self.publish(Event::DecidedChanged { squad: squad.to_owned(), decided });
    }
}
