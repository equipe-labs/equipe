//! A cheap look, before a member starts a ticket, at whether the ticket may
//! already be obsolete — no model call, and conservative: a flag only holds the
//! ticket for the PM (one line on its brief), which confirms by touching the
//! ticket (`update_ticket`) or cancels it. Work that starts on a ticket nobody
//! needs any more is among the costliest waste there is: a whole lane, often
//! millions of tokens, thrown away at cancellation.
//!
//! Two things are flagged, for a ticket that has not started (`todo`, no
//! branch yet):
//!
//! - **a blocker that was cancelled**: a ticket it depends on is `cancelled`.
//!   The board counts a cancelled ticket as finished, so the dependent would
//!   start as if its blocker had been done — but what it waited for never was;
//! - **a twin**: another open ticket (`todo`, in progress, waiting or in
//!   review) of the same project, kind and owner has the same title once
//!   normalized, and a lower number (the first one carries on, the later one is
//!   held). Short or generic titles never count.

use equipe_protocol::{Ticket, TicketId, TicketStatus};

/// A title needs at least this many words, and characters, to count as a twin:
/// "Fix bug" repeats honestly.
const TWIN_WORDS: usize = 3;
const TWIN_CHARS: usize = 16;

/// What made a ticket worth holding for the PM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// A ticket it depends on was cancelled.
    CancelledBlocker(TicketId),
    /// Another open ticket carries the same title.
    Twin(TicketId),
}

/// A title as compared: lowercase words of letters and digits, one space apart.
pub fn normalize(title: &str) -> String {
    title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect::<Vec<_>>().join(" ")
}

/// Whether `ticket` has not started: waiting for a member, no work on a branch yet.
fn not_started(ticket: &Ticket) -> bool {
    ticket.status == TicketStatus::Todo && ticket.branch.is_none() && ticket.assignee.is_none()
}

/// Why `ticket`, about to start, should be held for the PM — if so.
pub fn flag(ticket: &Ticket, all: &[Ticket]) -> Option<Flag> {
    if !not_started(ticket) {
        return None;
    }
    if let Some(blocker) = ticket.depends_on.iter().find(|id| all.iter().any(|t| t.id == **id && t.status == TicketStatus::Cancelled)) {
        return Some(Flag::CancelledBlocker(*blocker));
    }
    let title = normalize(&ticket.title);
    if title.split(' ').count() < TWIN_WORDS || title.chars().count() < TWIN_CHARS {
        return None;
    }
    let open = |t: &Ticket| matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress | TicketStatus::Waiting | TicketStatus::Review);
    all.iter()
        .filter(|t| t.id < ticket.id && open(t) && t.project == ticket.project && t.kind == ticket.kind && t.owner == ticket.owner)
        .find(|t| normalize(&t.title) == title)
        .map(|t| Flag::Twin(t.id))
}

/// Whether the PM has dealt with a ticket held for it: the record has a line
/// of the PM's (`update_ticket` writes `updated`) after the last hold.
pub fn confirmed(history: &[(String, String, String)]) -> bool {
    match history.iter().rposition(|(_, kind, _)| kind == HELD) {
        Some(held) => history[held + 1..].iter().any(|(_, kind, _)| kind == "updated"),
        None => false,
    }
}

/// Whether a ticket was held already (its record says so): not told again.
pub fn held_before(history: &[(String, String, String)]) -> bool {
    history.iter().any(|(_, kind, _)| kind == HELD)
}

/// The ticket record's line for a hold.
pub const HELD: &str = "held for the PM";

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::TicketKind;

    fn ticket(id: TicketId, title: &str) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status: TicketStatus::Todo,
            kind: TicketKind::Code,
            owner: Some("backend".into()),
            assignee: None,
            project: Some("/p/api".into()),
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: None,
            model: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            images: Default::default(),
        }
    }

    #[test]
    fn a_cancelled_blocker_holds_the_ticket_that_waited_on_it() {
        let mut blocker = ticket(1, "Merge conflicts in the login branch");
        blocker.status = TicketStatus::Cancelled;
        let mut next = ticket(2, "Integrate the login branch after the conflict fix");
        next.depends_on = vec![1];
        assert_eq!(flag(&next, &[blocker.clone(), next.clone()]), Some(Flag::CancelledBlocker(1)));
        // A finished blocker is the ordinary case.
        blocker.status = TicketStatus::Done;
        assert_eq!(flag(&next, &[blocker, next.clone()]), None);
    }

    #[test]
    fn the_later_of_two_tickets_with_the_same_title_is_held_not_the_first() {
        let first = ticket(3, "Rate limit the login endpoint");
        let twin = ticket(7, "  Rate-limit the LOGIN endpoint! ");
        let all = [first.clone(), twin.clone()];
        assert_eq!(flag(&twin, &all), Some(Flag::Twin(3)));
        assert_eq!(flag(&first, &all), None);
    }

    #[test]
    fn nothing_is_flagged_that_only_looks_alike() {
        let first = ticket(3, "Rate limit the login endpoint");
        let mut other_project = ticket(7, "Rate limit the login endpoint");
        other_project.project = Some("/p/web".into());
        let mut other_kind = ticket(8, "Rate limit the login endpoint");
        other_kind.kind = TicketKind::Task;
        let mut other_owner = ticket(9, "Rate limit the login endpoint");
        other_owner.owner = Some("frontend".into());
        let mut closed = ticket(2, "Rate limit the login endpoint");
        closed.status = TicketStatus::Done;
        let later = ticket(10, "Rate limit the login endpoint");
        let all = [first.clone(), other_project.clone(), other_kind.clone(), other_owner.clone(), closed, later.clone()];
        for t in [&other_project, &other_kind, &other_owner] {
            assert_eq!(flag(t, &all), None, "{}", t.id);
        }
        // Only the earlier open ticket counts: the done twin (2) doesn't, the first (3) does.
        assert_eq!(flag(&later, &all), Some(Flag::Twin(3)));
        // A different wording is a different ticket.
        let near = ticket(11, "Rate limit the signup endpoint");
        assert_eq!(flag(&near, &[first.clone(), near.clone()]), None);
        // Short and generic titles repeat honestly.
        let (a, b) = (ticket(20, "Fix bug"), ticket(21, "Fix bug"));
        assert_eq!(flag(&b, &[a, b.clone()]), None);
        // A ticket already under way is never held.
        let mut started = ticket(12, "Rate limit the login endpoint");
        started.branch = Some("equipe/x-t12".into());
        assert_eq!(flag(&started, &[first.clone(), started.clone()]), None);
        // An idea or a proposal is not open work.
        let mut idea = ticket(1, "Rate limit the login endpoint");
        idea.status = TicketStatus::Idea;
        let dup = ticket(13, "Rate limit the login endpoint");
        assert_eq!(flag(&dup, &[idea, dup.clone()]), None);
    }

    #[test]
    fn the_pm_touching_a_held_ticket_confirms_it() {
        let line = |kind: &str| ("equipe".to_owned(), kind.to_owned(), String::new());
        assert!(!held_before(&[line("created")]));
        assert!(held_before(&[line("created"), line(HELD)]));
        assert!(!confirmed(&[line("created"), line(HELD)]));
        assert!(confirmed(&[line(HELD), line("updated")]));
        // An update before the hold doesn't count.
        assert!(!confirmed(&[line("updated"), line(HELD)]));
    }
}
