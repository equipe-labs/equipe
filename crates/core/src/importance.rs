//! How much each thing that waits on the person matters, so the ones that
//! hold the most up are handled first.
//!
//! Many things pile up for the person: requests for permission, questions,
//! tickets put down for them, sign-ins, runs to bring back, proposals, work
//! accepted but not in main. By kind and by time, the minor ones get handled
//! first and the ones that block a lot are missed. This scores each by what
//! it **holds up**, in a rule short enough to explain:
//!
//! ```text
//! score = BLOCKED_EACH × tickets blocked (up to BLOCKED_CAP)
//!       + the highest priority among what it blocks and its own ticket
//!       + DEPLOY_HOLD, when deploy or release work waits on it
//!       + IDLE_MEMBER × members parked because of it (up to IDLE_CAP)
//! ```
//!
//! *Blocked* means open work tickets (to do, in progress, waiting, in review)
//! that wait on it through `depends_on`, directly or through other tickets.
//! The score never includes age: age only breaks a tie (the older first), so
//! the order is stable while the person works through it.
//!
//! Everything here is pure: tickets and waiting things in, [`Impact`]s out,
//! so every rule is tested on fixtures. The daemon uses it for the Status
//! list and for what the PM is told; the desktop, for Needs you's order.

use std::collections::{HashMap, HashSet, VecDeque};

use equipe_protocol::importance::{Impact, WaitKind, Waiting};
use equipe_protocol::{ExecutionState, Priority, Ticket, TicketId, TicketStatus};

/// Points for each ticket blocked.
pub const BLOCKED_EACH: u32 = 10;
/// Tickets blocked that count: past this, more doesn't make it matter more.
pub const BLOCKED_CAP: u32 = 10;
/// Points for the highest priority among what it blocks (and its own ticket).
pub const PRIORITY_URGENT: u32 = 60;
pub const PRIORITY_HIGH: u32 = 30;
/// A normal ticket blocked counts a little; one's own normal ticket counts nothing.
pub const PRIORITY_NORMAL_BLOCKED: u32 = 5;
/// Points when deploy or release work waits on it, or the PM said to hold deploys for it.
pub const DEPLOY_HOLD: u32 = 50;
/// Points for each member parked or idle because of it.
pub const IDLE_MEMBER: u32 = 12;
/// Members that count.
pub const IDLE_CAP: u32 = 3;

/// One thing that waits on the person, as the scoring needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub kind: WaitKind,
    /// The tickets it is about (a permission's ticket, a waiting ticket, the
    /// tickets of a card to confirm): what it blocks is what waits on them.
    pub tickets: Vec<TicketId>,
    /// Tickets it holds up directly, besides what waits on `tickets` (the
    /// tickets a change to the team would put hands on).
    pub holding: Vec<TicketId>,
    /// Since when it has waited (unix seconds); 0: not known, which is last among equals.
    pub since: u64,
    /// Members parked or idle because of it: a run paused on a request, a lane blocked.
    pub idle_members: u32,
    /// It holds a merge or deploy by its nature: accepted work not in main.
    pub holds_deploy: bool,
}

impl Waiter {
    pub fn new(kind: WaitKind) -> Self {
        Waiter { kind, tickets: Vec::new(), holding: Vec::new(), since: 0, idle_members: 0, holds_deploy: false }
    }
}

/// Whether a ticket is work a person wants done: held up, it costs them.
/// Ideas and proposals are not yet work.
fn is_work(t: &Ticket) -> bool {
    matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress | TicketStatus::Waiting | TicketStatus::Review)
}

/// The words that make a ticket deploy work: its title says it ships something.
const DEPLOY_WORDS: [&str; 6] = ["deploy", "release", "rollout", "publish", "launch", "ship"];

fn deploy_word(text: &str) -> bool {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| DEPLOY_WORDS.iter().any(|d| w.starts_with(d)))
}

/// Whether a ticket is deploy or release work.
pub fn is_deploy_work(t: &Ticket) -> bool {
    deploy_word(&t.title)
}

/// Whether the PM's note on a ticket says to hold deploys for it ("Hold
/// every deploy until the migration is approved").
pub fn pm_holds_deploys(t: &Ticket) -> bool {
    t.note.as_deref().is_some_and(|n| {
        let n = n.to_lowercase();
        n.contains("hold") && deploy_word(&n)
    })
}

fn priority_points(priority: Priority, own: bool) -> u32 {
    match priority {
        Priority::Urgent => PRIORITY_URGENT,
        Priority::High => PRIORITY_HIGH,
        Priority::Normal if !own => PRIORITY_NORMAL_BLOCKED,
        Priority::Normal | Priority::Low => 0,
    }
}

/// The board as a graph of what waits on what, built once for many scores.
pub struct Graph<'a> {
    by_id: HashMap<TicketId, &'a Ticket>,
    /// Ticket → the tickets that list it in `depends_on`.
    dependents: HashMap<TicketId, Vec<TicketId>>,
}

impl<'a> Graph<'a> {
    pub fn new(tickets: &'a [Ticket]) -> Self {
        let mut dependents: HashMap<TicketId, Vec<TicketId>> = HashMap::new();
        for t in tickets {
            for dep in &t.depends_on {
                dependents.entry(*dep).or_default().push(t.id);
            }
        }
        Graph { by_id: tickets.iter().map(|t| (t.id, t)).collect(), dependents }
    }

    /// Every open work ticket that waits on any of `roots`, directly or
    /// through others, once each, `roots` themselves left out; and `held`,
    /// held up as they are, with what waits on them. A cycle ends.
    pub fn blocked(&self, roots: &[TicketId], held: &[TicketId]) -> Vec<&'a Ticket> {
        let mut seen: HashSet<TicketId> = roots.iter().copied().collect();
        let mut queue: VecDeque<TicketId> = roots.iter().copied().collect();
        let mut out = Vec::new();
        for id in held {
            if let Some(t) = self.by_id.get(id).copied()
                && seen.insert(*id)
                && !t.status.is_closed()
            {
                queue.push_back(*id);
                if is_work(t) {
                    out.push(t);
                }
            }
        }
        while let Some(id) = queue.pop_front() {
            for next in self.dependents.get(&id).into_iter().flatten() {
                let Some(t) = self.by_id.get(next).copied() else { continue };
                // A finished or dropped ticket no longer waits; what hangs from it hangs from something else.
                if !seen.insert(*next) || t.status.is_closed() {
                    continue;
                }
                queue.push_back(*next);
                if is_work(t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// What `waiter` holds up.
    pub fn impact(&self, waiter: &Waiter) -> Impact {
        let blocked = self.blocked(&waiter.tickets, &waiter.holding);
        let own: Vec<&Ticket> = waiter.tickets.iter().filter_map(|id| self.by_id.get(id).copied()).filter(|t| !t.status.is_closed()).collect();
        let blocks = blocked.len() as u32;
        let holds_deploy = waiter.holds_deploy || blocked.iter().any(|t| is_deploy_work(t)) || own.iter().any(|t| pm_holds_deploys(t));
        let priority = blocked.iter().chain(own.iter()).map(|t| t.priority).min();
        let points = blocked
            .iter()
            .map(|t| priority_points(t.priority, false))
            .chain(own.iter().map(|t| priority_points(t.priority, true)))
            .max()
            .unwrap_or(0);
        let score = BLOCKED_EACH * blocks.min(BLOCKED_CAP) + points + if holds_deploy { DEPLOY_HOLD } else { 0 } + IDLE_MEMBER * waiter.idle_members.min(IDLE_CAP);
        let mut impact = Impact { score, blocks, priority, holds_deploy, idle_members: waiter.idle_members, reason: String::new() };
        impact.reason = reason(&impact);
        impact
    }
}

/// The facts of an impact in one short English line, for the PM.
pub fn reason(impact: &Impact) -> String {
    let mut parts = Vec::new();
    if impact.blocks > 0 {
        let urgent = impact.priority.filter(|p| matches!(p, Priority::Urgent | Priority::High)).map(|p| format!(", {}", p.label())).unwrap_or_default();
        parts.push(format!("blocks {} ticket{}{urgent}", impact.blocks, if impact.blocks == 1 { "" } else { "s" }));
    } else if let Some(p) = impact.priority.filter(|p| matches!(p, Priority::Urgent | Priority::High)) {
        parts.push(format!("is {} itself", p.label()));
    }
    if impact.holds_deploy {
        parts.push("holds deploys".to_owned());
    }
    if impact.idle_members > 0 {
        parts.push(format!("{} member{} idle", impact.idle_members, if impact.idle_members == 1 { "" } else { "s" }));
    }
    if parts.is_empty() { "blocks nothing".to_owned() } else { parts.join(", ") }
}

/// The order of the more important first: higher score, then the longer
/// waiting (an unknown time last), then as given.
pub fn order(waiters: &[Waiter], impacts: &[Impact]) -> Vec<usize> {
    let mut at: Vec<usize> = (0..waiters.len()).collect();
    at.sort_by_key(|&i| (std::cmp::Reverse(impacts[i].score), if waiters[i].since == 0 { u64::MAX } else { waiters[i].since }, i));
    at
}

/// Every waiter's impact, and the order of importance as indices into `waiters`.
pub fn rank(tickets: &[Ticket], waiters: &[Waiter]) -> (Vec<Impact>, Vec<usize>) {
    let graph = Graph::new(tickets);
    let impacts: Vec<Impact> = waiters.iter().map(|w| graph.impact(w)).collect();
    let by = order(waiters, &impacts);
    (impacts, by)
}

// ── the daemon's list ──

/// A thing waiting on the person that is not a ticket on its own: the
/// daemon's runtime gathers these (requests, questions, cards, secrets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub kind: WaitKind,
    /// What is asked, in a line.
    pub label: String,
    /// The tickets it is about.
    pub tickets: Vec<TicketId>,
    /// The tickets it holds up directly ([`Waiter::holding`]).
    pub holding: Vec<TicketId>,
    pub since: u64,
    pub idle_members: u32,
}

/// The kind a ticket waits for the person as, if it does: the same rule as
/// the Status screen's count, and the unmerged accepted work besides.
fn ticket_kind(t: &Ticket, pm_accepts: bool) -> Option<WaitKind> {
    match t.status {
        TicketStatus::Waiting => Some(WaitKind::Ticket),
        TicketStatus::InProgress if t.execution.as_ref().is_some_and(|e| e.state == ExecutionState::Stalled) => Some(WaitKind::Stalled),
        TicketStatus::Review if !pm_accepts && t.ready_for_review() => Some(WaitKind::Review),
        // Where the PM sorts the team's proposals, only its own are the person's: not told here.
        TicketStatus::Proposed if !pm_accepts => Some(WaitKind::Proposal),
        TicketStatus::Done if t.waits_to_merge() => Some(WaitKind::NotOnMain),
        _ => None,
    }
}

/// Everything waiting on the person, the most blocking first: `tickets` that
/// wait on them, and `asks` the runtime holds. All of it, scored.
pub fn waiting_list(tickets: &[Ticket], asks: &[Ask], pm_accepts: bool) -> Vec<Waiting> {
    let mut waiters = Vec::new();
    let mut shown: Vec<(Option<TicketId>, String)> = Vec::new();
    for t in tickets {
        let Some(kind) = ticket_kind(t, pm_accepts) else { continue };
        waiters.push(Waiter {
            kind,
            tickets: vec![t.id],
            holding: Vec::new(),
            since: t.updated_at.unwrap_or(0),
            // A run that stopped leaves its member with nothing to do on it.
            idle_members: u32::from(kind == WaitKind::Stalled),
            holds_deploy: kind == WaitKind::NotOnMain,
        });
        shown.push((Some(t.id), t.title.clone()));
    }
    for a in asks {
        waiters.push(Waiter { kind: a.kind, tickets: a.tickets.clone(), holding: a.holding.clone(), since: a.since, idle_members: a.idle_members, holds_deploy: false });
        shown.push((a.tickets.first().copied(), a.label.clone()));
    }
    let (impacts, by) = rank(tickets, &waiters);
    by.into_iter()
        .map(|i| {
            let (ticket, title) = shown[i].clone();
            let status = ticket.and_then(|id| tickets.iter().find(|t| t.id == id)).map(|t| t.status);
            Waiting { kind: waiters[i].kind, ticket, status, title, since: Some(waiters[i].since).filter(|s| *s > 0), impact: impacts[i].clone() }
        })
        .collect()
}

/// The PM's note of what waits on the person, for it to remind them of the
/// right things: the top `n`, each with what it holds up. `None` when none of
/// them holds anything up (then the order is no news).
pub fn pm_note(list: &[Waiting], n: usize, now: u64) -> Option<String> {
    if !list.first().is_some_and(|w| w.impact.is_blocking()) {
        return None;
    }
    let lines: Vec<String> = list
        .iter()
        .take(n)
        .map(|w| {
            let what = w.ticket.map_or_else(|| w.title.clone(), |id| format!("T-{id} {}", w.title));
            let cut: String = what.chars().take(90).collect();
            let waited = w.since.map(|s| format!(", waiting {}", ago(now.saturating_sub(s)))).unwrap_or_default();
            format!("- {cut} — {}{waited}", w.impact.reason)
        })
        .collect();
    Some(format!(
        "Waiting on the person, most blocking first. When you remind them of what waits, lead with these, and say what each holds up:\n{}",
        lines.join("\n")
    ))
}

fn ago(secs: u64) -> String {
    match secs {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3_600 => format!("{}h", s / 3_600),
        s => format!("{}m", s / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: TicketId, title: &str, status: TicketStatus, priority: Priority, depends_on: &[TicketId]) -> Ticket {
        Ticket {
            id,
            title: title.into(),
            body: String::new(),
            status,
            kind: Default::default(),
            owner: None,
            assignee: None,
            project: None,
            depends_on: depends_on.to_vec(),
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "you".into(),
            note: None,
            asks: Vec::new(),
            remaining: Vec::new(),
            images: Vec::new(),
            priority,
            rank: 0,
            updated_at: None,
            design_qa: None,
            plan: None,
            model: None,
        }
    }

    fn todo(id: TicketId, deps: &[TicketId]) -> Ticket {
        ticket(id, &format!("work {id}"), TicketStatus::Todo, Priority::Normal, deps)
    }

    fn on(ids: &[TicketId]) -> Waiter {
        Waiter { tickets: ids.to_vec(), ..Waiter::new(WaitKind::Ticket) }
    }

    #[test]
    fn what_waits_on_it_counts_through_other_tickets_once_each() {
        // 2 and 3 wait on 1; 4 on 2 and on 3 (once); 5 on 4; 6 doesn't.
        let tickets = vec![ticket(1, "sign in", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1]), todo(3, &[1]), todo(4, &[2, 3]), todo(5, &[4]), todo(6, &[])];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!(impact.blocks, 4, "{impact:?}");
        assert_eq!(impact.score, 4 * BLOCKED_EACH + PRIORITY_NORMAL_BLOCKED);
        assert!(impact.is_blocking());
    }

    #[test]
    fn finished_dropped_and_not_yet_work_do_not_count_and_a_cycle_ends() {
        let tickets = vec![
            ticket(1, "sign in", TicketStatus::Waiting, Priority::Normal, &[3]),
            ticket(2, "done", TicketStatus::Done, Priority::Urgent, &[1]),
            ticket(3, "loops back", TicketStatus::Todo, Priority::Normal, &[1]),
            ticket(4, "idea", TicketStatus::Idea, Priority::Normal, &[1]),
            ticket(5, "proposed", TicketStatus::Proposed, Priority::Normal, &[1]),
            ticket(6, "dropped", TicketStatus::Cancelled, Priority::Normal, &[1]),
        ];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!(impact.blocks, 1, "only 3: {impact:?}");
    }

    #[test]
    fn the_highest_priority_among_what_it_blocks_and_its_own_counts_once() {
        let tickets = vec![
            ticket(1, "own", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "a", TicketStatus::Todo, Priority::Low, &[1]),
            ticket(3, "b", TicketStatus::Todo, Priority::High, &[1]),
        ];
        let impact = Graph::new(&tickets).impact(&on(&[1]));
        assert_eq!((impact.priority, impact.blocks), (Some(Priority::High), 2));
        assert_eq!(impact.score, 2 * BLOCKED_EACH + PRIORITY_HIGH);
        assert!(impact.is_pressing());
        let urgent_itself = vec![ticket(1, "own", TicketStatus::Waiting, Priority::Urgent, &[])];
        let impact = Graph::new(&urgent_itself).impact(&on(&[1]));
        assert_eq!((impact.blocks, impact.score, impact.is_blocking()), (0, PRIORITY_URGENT, true), "an urgent ticket waiting is itself pressing");
    }

    #[test]
    fn deploy_work_behind_it_or_a_pm_hold_note_holds_deploys() {
        let tickets = vec![
            ticket(1, "approve the production migration", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "Deploy the API to production", TicketStatus::Todo, Priority::Normal, &[1]),
            ticket(3, "approve a design", TicketStatus::Waiting, Priority::Normal, &[]),
            todo(4, &[3]),
        ];
        let graph = Graph::new(&tickets);
        let migration = graph.impact(&on(&[1]));
        assert!(migration.holds_deploy);
        assert_eq!(migration.score, BLOCKED_EACH + PRIORITY_NORMAL_BLOCKED + DEPLOY_HOLD);
        assert!(!graph.impact(&on(&[3])).holds_deploy);
        let mut held = tickets.clone();
        held[2].note = Some("Hold every release until this is picked.".into());
        assert!(Graph::new(&held).impact(&on(&[3])).holds_deploy, "the PM's hold note");
        let unmerged = Waiter { holds_deploy: true, ..Waiter::new(WaitKind::NotOnMain) };
        assert!(graph.impact(&unmerged).holds_deploy);
        assert!(!is_deploy_work(&todo(9, &[])) && deploy_word("Ship it") && deploy_word("v2 rollout") && !deploy_word("relation"));
    }

    #[test]
    fn a_parked_member_counts_and_a_request_that_blocks_nothing_is_least() {
        let tickets = vec![ticket(1, "pick", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1])];
        let graph = Graph::new(&tickets);
        let parked = Waiter { idle_members: 2, ..Waiter::new(WaitKind::Permission) };
        assert_eq!(graph.impact(&parked).score, 2 * IDLE_MEMBER);
        assert!(!graph.impact(&parked).is_blocking(), "a parked member is not blocked work");
        assert_eq!(graph.impact(&Waiter { idle_members: 9, ..parked.clone() }).score, IDLE_CAP * IDLE_MEMBER, "capped");
        assert_eq!(graph.impact(&Waiter::new(WaitKind::Decision)).score, 0);
        assert!(graph.impact(&on(&[1])).score > graph.impact(&Waiter { idle_members: 1, ..Waiter::new(WaitKind::Permission) }).score, "one blocked ticket beats one idle member");
    }

    #[test]
    fn the_squads_example_orders_the_sign_in_and_the_migration_before_the_design_picks() {
        let mut tickets = vec![
            ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(2, "Approve the production DB migration", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(3, "Pick the hero image", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(4, "Pick the font", TicketStatus::Waiting, Priority::Normal, &[]),
            ticket(5, "Deploy the servers", TicketStatus::Todo, Priority::Normal, &[2]),
        ];
        tickets.extend((10..15).map(|id| todo(id, &[1])));
        tickets.extend((20..23).map(|id| todo(id, &[2])));
        tickets.push(todo(30, &[3]));
        tickets.push(todo(31, &[4]));
        let waiters = vec![on(&[3]), on(&[1]), Waiter::new(WaitKind::Permission), on(&[2]), on(&[4])];
        let (impacts, by) = rank(&tickets, &waiters);
        assert_eq!(by, vec![3, 1, 0, 4, 2], "migration (holds deploys), sign-in, the picks by age/as given, the approval last: {impacts:?}");
        assert_eq!(impacts[3].blocks, 4);
        assert_eq!(impacts[1].blocks, 5);
    }

    #[test]
    fn a_change_to_the_team_holds_the_tickets_it_would_put_hands_on_and_what_waits_on_them() {
        let tickets = vec![todo(1, &[]), todo(2, &[1]), todo(3, &[]), ticket(4, "idea", TicketStatus::Idea, Priority::Normal, &[])];
        let team = Waiter { holding: vec![1, 3, 4, 99], ..Waiter::new(WaitKind::Decision) };
        let impact = Graph::new(&tickets).impact(&team);
        assert_eq!(impact.blocks, 3, "1, 3 and what waits on 1: not an idea, not a ticket that isn't there");
    }

    #[test]
    fn a_tie_goes_to_the_one_waiting_longest_and_an_unknown_time_goes_last() {
        let tickets = vec![todo(1, &[])];
        let w = |since| Waiter { since, ..Waiter::new(WaitKind::Decision) };
        let waiters = vec![w(500), w(0), w(100), w(300)];
        let (_, by) = rank(&tickets, &waiters);
        assert_eq!(by, vec![2, 3, 0, 1]);
    }

    #[test]
    fn the_daemons_list_scores_tickets_and_asks_together_and_says_why() {
        let mut tickets = vec![
            ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]),
            todo(2, &[1]),
            todo(3, &[1]),
            ticket(4, "handed in", TicketStatus::Review, Priority::Normal, &[]),
            ticket(5, "stopped run", TicketStatus::InProgress, Priority::Normal, &[]),
            ticket(6, "accepted, unmerged", TicketStatus::Done, Priority::Normal, &[]),
        ];
        tickets[1].updated_at = Some(1_000);
        tickets[3].updated_at = Some(500);
        tickets[3].branch = Some("b".into());
        tickets[4].execution = Some(Box::new(equipe_protocol::TicketExecution {
            state: ExecutionState::Stalled,
            session: 1,
            operation: String::new(),
            generation: 1,
            heartbeat_at: 0,
            lease_expires_at: 0,
            reason: None,
            pause: None,
            head: None,
            dirty: None,
        }));
        tickets[5].branch = Some("b".into());
        tickets[5].merged = Some(false);
        let asks = vec![Ask { kind: WaitKind::Permission, label: "run rm".into(), tickets: vec![], holding: vec![], since: 10, idle_members: 1 }];
        let list = waiting_list(&tickets, &asks, false);
        let kinds: Vec<WaitKind> = list.iter().map(|w| w.kind).collect();
        assert_eq!(list.iter().map(|w| w.kind).collect::<Vec<_>>(), vec![WaitKind::NotOnMain, WaitKind::Ticket, WaitKind::Permission, WaitKind::Stalled, WaitKind::Review], "{list:#?}");
        assert_eq!(list[1].ticket, Some(1));
        assert_eq!(list[1].impact.reason, "blocks 2 tickets");
        assert!(kinds.contains(&WaitKind::Review) && kinds.contains(&WaitKind::Stalled) && kinds.contains(&WaitKind::NotOnMain) && kinds.contains(&WaitKind::Permission));
        let unmerged = list.iter().find(|w| w.kind == WaitKind::NotOnMain).unwrap();
        assert!(unmerged.impact.holds_deploy && unmerged.impact.reason.contains("holds deploys"));
        // Where the PM accepts, handed-in work is not the person's.
        assert!(!waiting_list(&tickets, &[], true).iter().any(|w| w.kind == WaitKind::Review));
    }

    #[test]
    fn the_pms_note_names_the_top_few_and_is_silent_when_nothing_is_held_up() {
        let tickets = vec![ticket(1, "AWS admin sign-in", TicketStatus::Waiting, Priority::Normal, &[]), todo(2, &[1]), ticket(3, "Pick a font", TicketStatus::Waiting, Priority::Normal, &[]), todo(4, &[3])];
        let mut list = waiting_list(&tickets, &[], false);
        list[0].since = Some(1_000);
        let note = pm_note(&list, 1, 1_000 + 3 * 86_400).unwrap();
        assert!(note.contains("T-1 AWS admin sign-in — blocks 1 ticket, waiting 3d") && !note.contains("Pick a font"), "{note}");
        let quiet = waiting_list(&[ticket(9, "pick", TicketStatus::Waiting, Priority::Normal, &[])], &[], false);
        assert_eq!(pm_note(&quiet, 3, 0), None);
        assert_eq!(pm_note(&[], 3, 0), None);
    }
}
