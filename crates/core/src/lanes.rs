//! Several tickets at once under one member: its ticket runs.
//!
//! A member is one identity — its role, memory, settings and conversation
//! with the person — however many tickets it works. Each ready ticket it
//! takes runs in a session of its own beside that conversation (a *run*,
//! [`equipe_protocol::RunOf`]): its own worktree, engine conversation, turn,
//! steering, stop, questions and estimate, under the ticket's run generation
//! ([`equipe_protocol::TicketExecution`]). Work that queues is spread over the
//! members the squad has; it never calls for more members. A member is added
//! for a role nobody holds, or when the person asks.
//!
//! How many runs go at once is bounded by what the plans and the disk can
//! take, never by how many members there are — [`member_limit`] and
//! [`engine_limit`], the *lane policy*:
//!
//! | Plan used (tightest window) | Runs per member | Runs per engine, squad-wide |
//! |-----------------------------|-----------------|-----------------------------|
//! | unknown                     | 3               | 8                           |
//! | under 25%                   | 4               | 10                          |
//! | 25% to under 70%            | 3               | 8                           |
//! | 70% to under 90%            | 2               | 4                           |
//! | 90% and over                | 1               | 2                           |
//!
//! That is the squad's *auto* parallel work, and on auto the engine's room
//! is further held down by *pacing to budget* ([`crate::pacing`]) when the
//! week's burn would use the plan up before it starts over — never below one
//! run, with the PM's first run ([`Member::pm`]) and urgent tickets always
//! admitted. On *max*
//! ([`equipe_protocol::lanes::Parallel`]) the plan's usage is left out: the
//! person's caps hold instead — 6 runs per member and 16 per engine unless
//! they said otherwise — and the plan is used faster. Only an engine whose
//! plan is used up still runs as on auto.
//!
//! Either way, with the disk low ([`DiskLevel::Low`]) every member runs one
//! ticket at a time; from [`DiskLevel::Critical`] new code work waits
//! altogether (the hard safety), while tasks and actions still start. An
//! engine out of its plan takes no new ticket at all (the engine hold).
//!
//! Which member of a role takes a ticket is fixed, and the same every time:
//! the ready tickets in the board's order, each to the member of its role
//! with the fewest tickets in hand — runs at work and tickets waiting on the
//! person both count — ties going to whoever joined first (never the order
//! of names: `frontend-10` would come before `frontend-2`). A ticket owned by
//! a member by name goes to that member only.

use std::collections::HashMap;

use equipe_protocol::disk::DiskLevel;
use equipe_protocol::lanes::Parallel;
use equipe_protocol::{Engine, PlanUsage, Ticket, TicketId, TicketStatus};

use crate::board;

/// Runs per member when nothing says otherwise.
pub const LANES_PER_MEMBER: usize = 3;
/// Runs per engine across the squad when nothing says otherwise: one plan
/// serves every member on it.
pub const LANES_PER_ENGINE: usize = 8;

/// The tightest of a plan's windows, in percent used; `None` when unknown.
pub fn plan_used(usage: Option<&PlanUsage>) -> Option<u8> {
    let usage = usage?;
    usage.five_hour.iter().chain(usage.seven_day.iter()).map(|w| w.used_percent).max()
}

/// A plan used up: nothing new runs there whatever the squad chose, and
/// max runs no more than auto would.
fn used_up(plan_used: Option<u8>) -> bool {
    plan_used.is_some_and(|p| p >= 100)
}

/// How many runs one member has at once, by its engine's plan, the disk and
/// the squad's parallel work.
pub fn member_limit(plan_used: Option<u8>, disk: DiskLevel, parallel: &Parallel) -> usize {
    if disk >= DiskLevel::Low {
        return 1;
    }
    if parallel.is_max() && !used_up(plan_used) {
        return parallel.clamped().per_member as usize;
    }
    match plan_used {
        None => LANES_PER_MEMBER,
        Some(p) if p >= 90 => 1,
        Some(p) if p >= 70 => 2,
        Some(p) if p < 25 => LANES_PER_MEMBER + 1,
        Some(_) => LANES_PER_MEMBER,
    }
}

/// How many runs one engine has at once across the squad.
pub fn engine_limit(plan_used: Option<u8>, disk: DiskLevel, parallel: &Parallel) -> usize {
    let limit = match plan_used {
        _ if parallel.is_max() && !used_up(plan_used) => parallel.clamped().per_engine as usize,
        None => LANES_PER_ENGINE,
        Some(p) if p >= 90 => 2,
        Some(p) if p >= 70 => 4,
        Some(p) if p < 25 => LANES_PER_ENGINE + 2,
        Some(_) => LANES_PER_ENGINE,
    };
    if disk >= DiskLevel::Low { limit.min(LANES_PER_ENGINE / 2) } else { limit }
}

/// A member as the lane policy sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member<'a> {
    pub name: &'a str,
    pub role: &'a str,
    /// When it joined: the tie-break.
    pub joined: u64,
    pub engine: Engine,
    /// Its runs at work now.
    pub runs: usize,
    /// Its tickets waiting on the person: no run, but in its hands.
    pub waiting: usize,
    /// How many runs it may have at once ([`member_limit`]).
    pub limit: usize,
    /// Whether it takes new work at all: not while its engine is held, it
    /// is moving engines, or it only finishes what it holds.
    pub takes_new: bool,
    /// The squad's PM: with no run at work it always gets a slot, whatever
    /// pacing holds the engine's room down ([`crate::pacing`]).
    pub pm: bool,
}

/// Which member starts which ready ticket now, in the order they start:
/// every ready ticket a member has room for, in one pass. `room` is how many
/// more runs each engine may start across the squad ([`engine_limit`] less
/// those at work); an engine missing from it has none. A ticket is ready as
/// the board says ([`board::assignments`]): `todo`, owned, knowing where it
/// works, everything it depends on finished, in the board's order.
pub fn assign(tickets: &[Ticket], members: &[Member<'_>], single_project: bool, first: &[TicketId], room: &HashMap<Engine, usize>) -> Vec<(TicketId, String)> {
    let finished = |id: &TicketId| tickets.iter().any(|t| t.id == *id && t.status.is_closed());
    let mut held: Vec<usize> = members.iter().map(|m| m.runs + m.waiting).collect();
    let mut runs: Vec<usize> = members.iter().map(|m| m.runs).collect();
    let mut room = room.clone();
    let mut out = Vec::new();
    for ticket in board::ready_in_order(tickets, first) {
        let Some(owner) = ticket.owner.as_deref() else { continue };
        if !board::knows_where(ticket, single_project) || !ticket.depends_on.iter().all(finished) {
            continue;
        }
        // A member that has left hands its tickets to its role.
        let owner = if members.iter().any(|m| m.role == owner || m.name == owner) { owner } else { board::role_of(owner) };
        let by_role = members.iter().any(|m| m.role == owner);
        let free = |at: &usize| {
            let m = &members[*at];
            // The PM's first run is never paced away.
            let room = room.get(&m.engine).copied().unwrap_or(0) > 0 || (m.pm && runs[*at] == 0);
            m.takes_new && runs[*at] < m.limit && room
        };
        let pick = (0..members.len())
            .filter(|at| if by_role { members[*at].role == owner } else { members[*at].name == owner })
            .filter(free)
            .min_by_key(|at| (held[*at], members[*at].joined));
        if let Some(at) = pick {
            held[at] += 1;
            runs[at] += 1;
            if let Some(left) = room.get_mut(&members[at].engine) {
                *left = left.saturating_sub(1);
            }
            out.push((ticket.id, members[at].name.to_owned()));
        }
    }
    out
}

/// Whether a member tool only reads: a run that no longer holds its ticket
/// may still call it. Any other — a ticket changed, the person asked, an
/// estimate, a hand-in, a fact remembered — it may not.
pub fn reads_only(tool: &str) -> bool {
    [
        board::BOARD,
        board::WORKDIR,
        board::SYSTEM_TOOLS,
        board::DESIGN_TOOLS,
        crate::briefs::DOCS,
        crate::signals::SIGNALS,
        crate::controls::SETTINGS,
        crate::metrics::tool::METRICS,
        crate::plan_use::TOOL,
    ]
    .contains(&tool)
}

/// A member's tickets waiting on the person, for [`Member::waiting`].
pub fn waiting_on_person(tickets: &[Ticket], name: &str) -> usize {
    tickets.iter().filter(|t| t.status == TicketStatus::Waiting && t.assignee.as_deref() == Some(name)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::disk::DiskLevel;
    use equipe_protocol::{PlanWindow, TicketKind};

    fn ticket(id: TicketId, owner: &str) -> Ticket {
        Ticket {
            id,
            title: format!("T{id}"),
            body: String::new(),
            status: TicketStatus::Todo,
            kind: TicketKind::Task,
            owner: Some(owner.into()),
            assignee: None,
            project: None,
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
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        }
    }

    fn member<'a>(name: &'a str, role: &'a str, joined: u64, runs: usize) -> Member<'a> {
        Member { name, role, joined, engine: Engine::Claude, runs, waiting: 0, limit: 10, takes_new: true, pm: false }
    }

    fn room(n: usize) -> HashMap<Engine, usize> {
        HashMap::from([(Engine::Claude, n), (Engine::Codex, n)])
    }

    #[test]
    fn ten_ready_tickets_for_one_member_start_together_up_to_its_limit() {
        let tickets: Vec<Ticket> = (1..=10).map(|id| ticket(id, "frontend")).collect();
        let all = assign(&tickets, &[member("frontend", "frontend", 0, 0)], true, &[], &room(100));
        assert_eq!(all.len(), 10);
        assert!(all.iter().all(|(_, name)| name == "frontend"));
        assert_eq!(all.iter().map(|(t, _)| *t).collect::<Vec<_>>(), (1..=10).collect::<Vec<_>>(), "in the board's order");
        let limited = assign(&tickets, &[Member { limit: 3, ..member("frontend", "frontend", 0, 1) }], true, &[], &room(100));
        assert_eq!(limited, vec![(1, "frontend".into()), (2, "frontend".into())], "one at work already: two more, never past three");
    }

    #[test]
    fn a_role_spreads_to_whoever_holds_least_ties_to_whoever_joined_first() {
        let tickets: Vec<Ticket> = (1..=10).map(|id| ticket(id, "backend")).collect();
        // Names that sort the other way from when they joined.
        let team = [member("backend", "backend", 1, 0), member("backend-10", "backend", 3, 0), member("backend-2", "backend", 2, 0)];
        let got = assign(&tickets, &team, true, &[], &room(100));
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["backend", "backend-2", "backend-10", "backend", "backend-2", "backend-10", "backend", "backend-2", "backend-10", "backend"]);
        let count = |n: &str| names.iter().filter(|x| **x == n).count();
        assert_eq!((count("backend"), count("backend-2"), count("backend-10")), (4, 3, 3));
        // The same input, the same answer.
        assert_eq!(assign(&tickets, &team, true, &[], &room(100)), got);
        // Waiting on the person counts: whoever already holds more gets less.
        let mut busy = team.clone();
        busy[0].waiting = 2;
        let got = assign(&tickets[..3], &busy, true, &[], &room(100));
        assert_eq!(got.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["backend-2", "backend-10", "backend-2"]);
    }

    #[test]
    fn a_ticket_owned_by_name_goes_to_that_member_only_and_waits_for_it() {
        let mut tickets: Vec<Ticket> = (1..=5).map(|id| ticket(id, "frontend-2")).collect();
        tickets.push(ticket(6, "frontend"));
        let team = [member("frontend", "frontend", 1, 0), member("frontend-2", "frontend", 2, 0)];
        let got = assign(&tickets, &team, true, &[], &room(100));
        assert!(got.iter().filter(|(t, _)| *t <= 5).all(|(_, n)| n == "frontend-2"), "{got:?}");
        assert_eq!(got.iter().find(|(t, _)| *t == 6).map(|(_, n)| n.as_str()), Some("frontend"));
        // Held: its tickets wait for it; the role's go to the other member.
        let held = [member("frontend", "frontend", 1, 0), Member { takes_new: false, ..member("frontend-2", "frontend", 2, 0) }];
        let got = assign(&tickets, &held, true, &[], &room(100));
        assert_eq!(got, vec![(6, "frontend".into())]);
    }

    #[test]
    fn an_engine_has_room_for_so_many_runs_across_the_squad() {
        let tickets: Vec<Ticket> = (1..=6).map(|id| ticket(id, if id % 2 == 0 { "backend" } else { "frontend" })).collect();
        let team = [member("frontend", "frontend", 1, 0), member("backend", "backend", 2, 0)];
        let got = assign(&tickets, &team, true, &[], &room(3));
        assert_eq!(got.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(assign(&tickets, &team, true, &[], &HashMap::new()).is_empty(), "no room, nothing starts");
    }

    #[test]
    fn the_lane_policy_follows_the_plan_and_the_disk() {
        let used = |p: u8| PlanUsage { five_hour: Some(PlanWindow { used_percent: p, resets_at: 0 }), seven_day: Some(PlanWindow { used_percent: 10, resets_at: 0 }) };
        assert_eq!(plan_used(None), None);
        assert_eq!(plan_used(Some(&used(55))), Some(55), "the tightest window");
        let auto = Parallel::default();
        assert_eq!(member_limit(None, DiskLevel::Ok, &auto), 3);
        assert_eq!(member_limit(Some(10), DiskLevel::Ok, &auto), 4);
        assert_eq!(member_limit(Some(50), DiskLevel::Ok, &auto), 3);
        assert_eq!(member_limit(Some(75), DiskLevel::Ok, &auto), 2);
        assert_eq!(member_limit(Some(95), DiskLevel::Ok, &auto), 1);
        assert_eq!(member_limit(Some(10), DiskLevel::Low, &auto), 1, "a low disk: one at a time");
        assert_eq!(engine_limit(None, DiskLevel::Ok, &auto), 8);
        assert_eq!(engine_limit(Some(10), DiskLevel::Ok, &auto), 10);
        assert_eq!(engine_limit(Some(75), DiskLevel::Ok, &auto), 4);
        assert_eq!(engine_limit(Some(95), DiskLevel::Ok, &auto), 2);
        assert_eq!(engine_limit(Some(10), DiskLevel::Low, &auto), 4);
    }

    #[test]
    fn max_holds_the_persons_caps_whatever_the_plans_usage() {
        let max = Parallel::max(6, 16);
        for used in [None, Some(0), Some(10), Some(50), Some(75), Some(95), Some(99)] {
            assert_eq!(member_limit(used, DiskLevel::Ok, &max), 6, "{used:?}");
            assert_eq!(engine_limit(used, DiskLevel::Ok, &max), 16, "{used:?}");
        }
        let set = Parallel::max(9, 25);
        assert_eq!((member_limit(Some(95), DiskLevel::Ok, &set), engine_limit(Some(95), DiskLevel::Ok, &set)), (9, 25));
        // Set past the bounds by hand: held within them.
        let wild = Parallel { per_member: 99, per_engine: 0, ..Parallel::max(1, 1) };
        assert_eq!((member_limit(None, DiskLevel::Ok, &wild), engine_limit(None, DiskLevel::Ok, &wild)), (12, 1));
        // The caps kept while on auto change nothing there.
        let kept = Parallel { mode: equipe_protocol::lanes::ParallelMode::Auto, ..set };
        assert_eq!((member_limit(Some(95), DiskLevel::Ok, &kept), engine_limit(Some(95), DiskLevel::Ok, &kept)), (1, 2));
    }

    #[test]
    fn max_still_gives_way_to_a_used_up_plan_and_a_short_disk() {
        let max = Parallel::max(6, 16);
        // A plan used up: no more than auto would run there.
        assert_eq!(member_limit(Some(100), DiskLevel::Ok, &max), member_limit(Some(100), DiskLevel::Ok, &Parallel::default()));
        assert_eq!((member_limit(Some(100), DiskLevel::Ok, &max), engine_limit(Some(100), DiskLevel::Ok, &max)), (1, 2));
        // A low disk: one at a time per member, and the engine's room halved as on auto.
        assert_eq!(member_limit(Some(10), DiskLevel::Low, &max), 1);
        assert_eq!(engine_limit(Some(10), DiskLevel::Low, &max), LANES_PER_ENGINE / 2);
        assert_eq!(member_limit(None, DiskLevel::Critical, &max), 1);
        assert_eq!(engine_limit(None, DiskLevel::Critical, &max), LANES_PER_ENGINE / 2);
        // A small cap stays small on a low disk.
        assert_eq!(engine_limit(None, DiskLevel::Low, &Parallel::max(6, 2)), 2);
    }

    #[test]
    fn paced_room_goes_to_urgent_tickets_first() {
        use equipe_protocol::Priority;
        let mut tickets: Vec<Ticket> = (1..=6).map(|id| ticket(id, "frontend")).collect();
        tickets[3].priority = Priority::High;
        tickets[4].priority = Priority::Urgent;
        let team = [member("frontend", "frontend", 1, 0)];
        let urgent = crate::pacing::urgent_first(&tickets);
        assert_eq!(urgent, [5, 4], "urgent, then high");
        let got = assign(&tickets, &team, true, &urgent, &room(2));
        assert_eq!(got.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [5, 4], "the two places go to the most urgent");
        let plain = assign(&tickets, &team, true, &[], &room(2));
        assert_eq!(plain.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [1, 2], "unpaced: the board's order");
    }

    #[test]
    fn the_pm_always_gets_a_slot_however_far_pacing_holds_the_room_down() {
        let tickets: Vec<Ticket> = (1..=3).map(|id| ticket(id, "pm")).collect();
        let pm = |runs| Member { pm: true, ..member("pm", "pm", 0, runs) };
        let none = HashMap::from([(Engine::Claude, 0)]);
        assert_eq!(assign(&tickets, &[pm(0)], true, &[], &none), vec![(1, "pm".into())], "no room at all: its first run still starts");
        assert!(assign(&tickets, &[pm(1)], true, &[], &none).is_empty(), "one at work: it waits like any other");
        let other = [Member { pm: false, ..member("pm", "pm", 0, 0) }];
        assert!(assign(&tickets, &other, true, &[], &none).is_empty(), "a member that isn't the PM has no such slot");
    }

    #[test]
    fn on_max_a_held_engine_still_starts_nothing() {
        let tickets: Vec<Ticket> = (1..=10).map(|id| ticket(id, "frontend")).collect();
        let max = Parallel::max(6, 16);
        let room = HashMap::from([(Engine::Claude, engine_limit(Some(95), DiskLevel::Ok, &max))]);
        let free = Member { limit: member_limit(Some(95), DiskLevel::Ok, &max), ..member("frontend", "frontend", 0, 0) };
        assert_eq!(assign(&tickets, std::slice::from_ref(&free), true, &[], &room).len(), 6, "a plan at 95%: six at once on max");
        // Out of its plan, the engine is held: its members take nothing new.
        let held = Member { takes_new: false, ..free };
        assert!(assign(&tickets, &[held], true, &[], &room).is_empty());
    }
}
