//! "Your requests" at the top of Needs you (`views/requests.rs`): each thing
//! the person asked a member for, with one line saying where all of it
//! stands, what of it waits on them (one click each), and, opened, the chain
//! of tickets that came of it. The daemon works out where each stands
//! (`equipe_core::requests`); here is which are listed, the line in the
//! person's language, and the timeline's order — no UI types, so the rules
//! are tested without a window.

use equipe_core::protocol::requests::{Link, Request, Step};
use equipe_core::t;

/// Collapsed, at most this many requests are listed.
pub const SHOWN: usize = 4;

/// The requests listed, those under way first, then those finished lately,
/// each the latest first; and how many more there are than `all` shows.
pub fn listed(requests: &[Request], all: bool) -> (Vec<&Request>, usize) {
    let mut list: Vec<&Request> = requests.iter().collect();
    list.sort_by_key(|r| (r.closed_at.is_some(), std::cmp::Reverse(r.asked_at), std::cmp::Reverse(r.id)));
    if all || list.len() <= SHOWN {
        return (list, 0);
    }
    let more = list.len() - SHOWN;
    list.truncate(SHOWN);
    (list, more)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where `r` stands, in one line in the person's language.
pub fn line(r: &Request) -> String {
    let now = now();
    equipe_core::requests::line(&r.status, equipe_core::i18n::core(crate::i18n::lang()), &|at| equipe_core::requests::clock(at, now))
}

/// Who was asked, and when: "asked designer · 05:15".
pub fn asked(r: &Request) -> String {
    let time = equipe_core::requests::clock(r.asked_at, now());
    match &r.member {
        Some(who) if r.session.is_some() => t!("requests.asked", who = who, time = time),
        _ => t!("requests.asked_board", time = time),
    }
}

/// A step of the timeline, with how deep under its part it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub step: &'a Step,
    pub depth: usize,
}

/// `r`'s tickets as a timeline: each part's ticket, then what came of it
/// under it — a ticket made while working one right after it, a step in.
pub fn timeline(r: &Request) -> Vec<Row<'_>> {
    let steps = &r.status.steps;
    let has = |id| steps.iter().any(|s| s.ticket == id);
    let mut out = Vec::new();
    fn walk<'a>(steps: &'a [Step], parent: Option<u32>, depth: usize, has: &dyn Fn(u32) -> bool, out: &mut Vec<Row<'a>>) {
        for step in steps.iter().filter(|s| s.parent.filter(|p| has(*p)) == parent) {
            if out.iter().any(|r: &Row| r.step.ticket == step.ticket) {
                continue;
            }
            out.push(Row { step, depth });
            walk(steps, Some(step.ticket), depth + 1, has, out);
        }
    }
    walk(steps, None, 0, &has, &mut out);
    // A loop of parents leaves some out: listed after, at the top.
    for step in steps {
        if !out.iter().any(|r| r.step.ticket == step.ticket) {
            out.push(Row { step, depth: 0 });
        }
    }
    out
}

/// How many of `r`'s tickets are open — what stopping it cancels — as the
/// board has them (its own timeline when the board lacks one).
pub fn open_tickets(r: &Request, board: &[equipe_core::protocol::Ticket]) -> usize {
    r.status
        .steps
        .iter()
        .filter(|s| !board.iter().find(|t| t.id == s.ticket).map_or(s.status, |t| t.status).is_closed())
        .count()
}

/// A stop in force, as the list says it: "Stopped: gallery polish · since 11-18".
pub fn stopped(stop: &equipe_core::protocol::requests::Stop) -> String {
    let since = equipe_providers::platform::local_time(stop.since).map(|t| format!("{:02}-{:02}", t.month, t.day)).unwrap_or_default();
    t!("requests.stopped", words = stop.words, since = since)
}

/// Whether a step is equipe's own check, shown quieter.
pub fn is_check(step: &Step) -> bool {
    step.link == Link::Check
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::TicketStatus;
    use equipe_core::protocol::requests::{RequestTicket, Status};

    fn step(ticket: u32, parent: Option<u32>, link: Link) -> Step {
        Step { ticket, title: format!("T{ticket}"), status: TicketStatus::Done, link, parent, who: None, at: None, merged: false }
    }

    fn request(id: u64, asked_at: u64, closed_at: Option<u64>, steps: Vec<Step>) -> Request {
        Request { plan_title: None,
            id,
            title: format!("R{id}"),
            session: Some(2),
            member: Some("designer".into()),
            asked_at,
            tickets: steps.iter().map(|s| RequestTicket { ticket: s.ticket, parent: s.parent, link: s.link }).collect(),
            status: Status { steps, ..Status::default() },
            closed_at,
        }
    }

    #[test]
    fn under_way_first_then_finished_each_the_latest_first_a_few_at_once() {
        let all: Vec<Request> = (1..=6).map(|i| request(i, i * 10, (i % 2 == 0).then_some(100), vec![])).collect();
        let (shown, more) = listed(&all, false);
        assert_eq!(shown.iter().map(|r| r.id).collect::<Vec<_>>(), [5, 3, 1, 6]);
        assert_eq!(more, 2);
        let (shown, more) = listed(&all, true);
        assert_eq!((shown.len(), more), (6, 0));
    }

    #[test]
    fn the_timeline_puts_what_came_of_a_ticket_under_it() {
        let r = request(
            1,
            0,
            None,
            vec![
                step(3565, None, Link::Asked),
                step(3567, None, Link::Asked),
                step(3613, Some(3565), Link::Made),
                step(3639, Some(3613), Link::Check),
                step(3591, Some(3567), Link::Made),
                step(3614, Some(3565), Link::Made),
            ],
        );
        let rows: Vec<(u32, usize)> = timeline(&r).iter().map(|row| (row.step.ticket, row.depth)).collect();
        assert_eq!(rows, [(3565, 0), (3613, 1), (3639, 2), (3614, 1), (3567, 0), (3591, 1)]);
        assert!(is_check(timeline(&r)[2].step));
    }

    #[test]
    fn a_ticket_whose_parent_isnt_in_the_request_starts_a_line_of_its_own() {
        let r = request(1, 0, None, vec![step(9, Some(4), Link::Linked), step(10, Some(9), Link::Made)]);
        let rows: Vec<(u32, usize)> = timeline(&r).iter().map(|row| (row.step.ticket, row.depth)).collect();
        assert_eq!(rows, [(9, 0), (10, 1)]);
    }
}
