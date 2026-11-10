//! Orchestration: the squad at a glance (`views/orchestra.rs`) — a lane per
//! member with the ticket it is on and for how long, the subagents it runs
//! (at work, and those that just handed back), whom it waits on, and who
//! waits on it; where work is stuck, each in plain words; and, across the
//! top, what waits on the person.
//!
//! Derived from the model alone, read-only, so it is always as the squad
//! stands and updates with every event the daemon sends. "Who waits on whom"
//! comes from the board: a ticket's `depends_on`, the member holding each
//! ticket (the one on it now, else its assignee, else its owner), a ticket
//! put down for the person (`waiting`) or handed in for their review, and
//! what a member asks them (a permission, a decision). A wait is followed
//! ticket by ticket until it reaches work under way, the person, a ticket no
//! one holds, or a ticket already on the way (a loop).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use equipe_core::protocol::plan_use::{Report, Who};
use equipe_core::protocol::{Engine, SessionId, Ticket, TicketId, TicketStatus};

use crate::board::{Risk, risk};
use crate::state::{Member, Model, Presence, Section};

/// A subagent at it this long, in seconds, runs unusually long.
pub const HELPER_LONG: u64 = 20 * 60;
/// Work under way that hasn't moved this long, in seconds, while its member
/// isn't at it, has stalled.
pub const STALLED_AFTER: u64 = 2 * 60 * 60;
/// How long a subagent that handed back still shows in its member's lane.
pub const FINISHED_FOR: Duration = Duration::from_secs(15 * 60);
/// At most this many subagents that handed back show in a lane.
pub const FINISHED_SHOWN: usize = 3;

/// What the person owes a member's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Why {
    /// A member asks permission for a step.
    Permission,
    /// A member put a decision to the person.
    Decision,
    /// A ticket put down until the person does their part.
    YourTurn,
    /// Work handed in for the person's review.
    Review,
}

/// One ticket on a wait's way, and who holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub ticket: TicketId,
    /// The member on it, else the one it is for (a member's name, or a role);
    /// `None`: nobody.
    pub member: Option<String>,
}

/// Where a wait ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The last ticket is under way (or being accepted): it moves.
    Moving,
    /// The last ticket is someone's, not started yet.
    NotStarted,
    /// The last ticket is no one's.
    Nobody,
    /// It waits on the person.
    Person(Why),
    /// Back to a ticket already on the way: a loop no one can finish.
    Cycle,
}

/// A member's ticket that can't go on, and what it waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    /// The member's own ticket that waits; `None`: the member itself (a
    /// permission or a decision asked outside any ticket).
    pub ticket: Option<TicketId>,
    /// The tickets in the way, the one holding it up directly first. Empty
    /// when it waits on the person itself.
    pub hops: Vec<Hop>,
    pub end: End,
}

impl Wait {
    /// The member it waits on directly, if another's ticket holds it up.
    pub fn on_member(&self) -> Option<&str> {
        self.hops.first()?.member.as_deref()
    }

    /// It waits on the person itself, nobody between.
    pub fn on_person(&self) -> Option<Why> {
        match self.end {
            End::Person(why) if self.hops.is_empty() => Some(why),
            _ => None,
        }
    }

    /// The one it waits on waits too: on someone further, on the person, or
    /// round in a loop.
    pub fn is_chain(&self) -> bool {
        !self.hops.is_empty() && (self.hops.len() >= 2 || matches!(self.end, End::Person(_) | End::Cycle))
    }
}

/// What a member is doing, the most pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Doing {
    /// Its work waits on the person.
    NeedsYou,
    Working,
    /// Its work waits on another's.
    Waiting,
    Idle,
}

/// The ticket a member is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub id: TicketId,
    pub title: String,
    pub status: TicketStatus,
    /// How long it has been where it is, in seconds, when known.
    pub for_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperState {
    Running,
    /// At it, but unheard from this long (seconds), as the daemon watches it.
    Quiet(u64),
    Finished { failed: bool },
}

/// A subagent of a member's: at work, or handed back a moment ago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    /// The conversation it works for (the member's own, or one of its lanes').
    pub session: SessionId,
    pub id: String,
    pub what: String,
    /// How long it has been at it — or took, once it handed back.
    pub secs: u64,
    pub state: HelperState,
}

impl Helper {
    pub fn running(&self) -> bool {
        !matches!(self.state, HelperState::Finished { .. })
    }

    /// Still at it past [`HELPER_LONG`].
    pub fn long(&self) -> bool {
        self.running() && self.secs >= HELPER_LONG
    }
}

/// One member's lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub session: SessionId,
    pub name: String,
    pub role: String,
    pub engine: Option<Engine>,
    pub model: Option<String>,
    pub doing: Doing,
    pub current: Option<Current>,
    /// Its subagents: those at work (oldest first), then those that handed
    /// back lately (latest first).
    pub helpers: Vec<Helper>,
    pub waits: Vec<Wait>,
    /// Who waits on it directly: each member and its ticket that waits.
    pub holds: Vec<(String, TicketId)>,
    /// Its other open tickets, past the one it is on.
    pub queued: usize,
}

impl Lane {
    pub fn helpers_running(&self) -> usize {
        self.helpers.iter().filter(|h| h.running()).count()
    }

    /// Nothing to show past its name: drawn as one short row.
    pub fn is_quiet(&self) -> bool {
        self.doing == Doing::Idle && self.current.is_none() && self.helpers.is_empty() && self.holds.is_empty()
    }
}

/// Where work is stuck, and why — each said in plain words on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stuck {
    /// The tickets wait on each other round in a loop: the loop, its first
    /// ticket again at its end.
    Cycle { hops: Vec<Hop> },
    /// A member's last turn on its ticket failed.
    Failed { member: String, ticket: TicketId },
    /// A member waits on another who waits too.
    Chain { member: String, wait: Wait },
    /// A member's ticket waits on one no one holds.
    Nobody { member: String, ticket: TicketId, on: TicketId },
    /// Nothing heard from a member at work on a ticket this long (seconds).
    Quiet { member: String, ticket: TicketId, secs: u64 },
    /// A ticket under way hasn't moved this long (seconds), and its member isn't at it.
    Stalled { member: String, ticket: TicketId, secs: u64 },
    /// Past the time its member gave, by this much (seconds).
    Late { member: String, ticket: TicketId, secs: u64 },
    /// A subagent unheard from this long (seconds).
    HelperQuiet { member: String, what: String, secs: u64 },
    /// A subagent at it this long (seconds).
    HelperLong { member: String, what: String, secs: u64 },
}

impl Stuck {
    /// The member whose lane it is about, to point at.
    pub fn member(&self) -> Option<&str> {
        match self {
            Stuck::Cycle { hops } => hops.first().and_then(|h| h.member.as_deref()),
            Stuck::Failed { member, .. }
            | Stuck::Chain { member, .. }
            | Stuck::Nobody { member, .. }
            | Stuck::Quiet { member, .. }
            | Stuck::Stalled { member, .. }
            | Stuck::Late { member, .. }
            | Stuck::HelperQuiet { member, .. }
            | Stuck::HelperLong { member, .. } => Some(member),
        }
    }

    /// The ticket it is about, to open.
    pub fn ticket(&self) -> Option<TicketId> {
        match self {
            Stuck::Cycle { hops } => hops.first().map(|h| h.ticket),
            Stuck::Chain { wait, .. } => wait.ticket,
            Stuck::Failed { ticket, .. } | Stuck::Nobody { ticket, .. } | Stuck::Quiet { ticket, .. } | Stuck::Stalled { ticket, .. } | Stuck::Late { ticket, .. } => Some(*ticket),
            Stuck::HelperQuiet { .. } | Stuck::HelperLong { .. } => None,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Stuck::Cycle { .. } => 0,
            Stuck::Failed { .. } => 1,
            Stuck::Chain { .. } => 2,
            Stuck::Nobody { .. } => 3,
            Stuck::Quiet { .. } => 4,
            Stuck::Stalled { .. } => 5,
            Stuck::Late { .. } => 6,
            Stuck::HelperQuiet { .. } => 7,
            Stuck::HelperLong { .. } => 8,
        }
    }
}

/// What waits on the person, counted by kind — the breakdown of the
/// sidebar's Needs you, adding up to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForYou {
    pub permissions: usize,
    pub decisions: usize,
    pub turns: usize,
    pub reviews: usize,
    /// Proposals, and accepted work not on main yet.
    pub other: usize,
}

impl ForYou {
    pub fn total(&self) -> usize {
        self.permissions + self.decisions + self.turns + self.reviews + self.other
    }

    /// Each kind with how many, in Needs you's order, the empty ones left out.
    pub fn parts(&self) -> Vec<(Section, usize)> {
        [(Section::Permission, self.permissions), (Section::Decide, self.decisions), (Section::Waiting, self.turns), (Section::Review, self.reviews), (Section::Proposed, self.other)].into_iter().filter(|(_, n)| *n > 0).collect()
    }
}

/// The whole screen's worth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overview {
    pub for_you: ForYou,
    pub lanes: Vec<Lane>,
    pub stuck: Vec<Stuck>,
}

pub fn for_you(model: &Model) -> ForYou {
    let mut out = ForYou::default();
    for (section, _) in model.needs_you() {
        match section {
            Section::Permission => out.permissions += 1,
            Section::Decide => out.decisions += 1,
            Section::Waiting | Section::Stalled => out.turns += 1,
            Section::Review => out.reviews += 1,
            Section::Proposed | Section::NotOnMain => out.other += 1,
        }
    }
    out
}

/// The squad as it stands: `model.unix` is now for the board, `now` for subagents.
pub fn overview(model: &Model, now: Instant) -> Overview {
    let members: Vec<&Member> = model.ordered().into_iter().filter(|m| m.presence != Presence::Closed).collect();
    let mut lanes: Vec<Lane> = members.iter().map(|m| lane(model, m, now)).collect();
    // Who waits on whom, read backwards: each lane's members waiting on it.
    let waiting: Vec<(String, TicketId, String)> =
        lanes.iter().flat_map(|l| l.waits.iter().filter_map(move |w| Some((l.name.clone(), w.ticket?, w.on_member()?.to_owned())))).filter(|(me, _, on)| me != on).collect();
    for lane in &mut lanes {
        lane.holds = waiting.iter().filter(|(_, _, on)| *on == lane.name).map(|(me, t, _)| (me.clone(), *t)).collect();
    }
    // The PM first, then the most pressing; the order members joined between equals.
    lanes.sort_by_key(|l| (l.role != equipe_core::roles::PM, l.doing));
    let stuck = stuck(model, &lanes);
    Overview { for_you: for_you(model), lanes, stuck }
}

fn lane(model: &Model, m: &Member, now: Instant) -> Lane {
    let held = held_by(model, &m.name);
    let current = m.ticket.and_then(|id| model.tickets.iter().find(|t| t.id == id)).filter(|t| !t.status.is_closed()).or_else(|| held.first().copied());
    let mut waits: Vec<Wait> = Vec::new();
    // Asked outside any ticket: the member itself waits on the person.
    let asks_permission = model.approvals.open_items().any(|a| a.member == m.name && a.ticket.is_none_or(|t| !held.iter().any(|h| h.id == t)));
    let asks_decision = model.asked.open().any(|s| s.member == m.name && s.ticket.is_none_or(|t| !held.iter().any(|h| h.id == t)));
    if asks_permission {
        waits.push(Wait { ticket: None, hops: Vec::new(), end: End::Person(Why::Permission) });
    }
    if asks_decision {
        waits.push(Wait { ticket: None, hops: Vec::new(), end: End::Person(Why::Decision) });
    }
    waits.extend(held.iter().filter_map(|t| wait_of(model, t, &m.name)));
    let working = m.presence == Presence::Working || model.lanes_working(m.session) > 0;
    // A turn held on a permission or a decision is the person's to unblock, at work or not.
    let blocked_mid_turn = waits.iter().any(|w| matches!(w.on_person(), Some(Why::Permission | Why::Decision)));
    let doing = match () {
        _ if blocked_mid_turn || (!working && waits.iter().any(|w| w.on_person().is_some())) => Doing::NeedsYou,
        _ if working => Doing::Working,
        _ if !waits.is_empty() => Doing::Waiting,
        _ => Doing::Idle,
    };
    Lane {
        session: m.session,
        name: m.name.clone(),
        role: m.role.clone(),
        engine: m.engine,
        model: m.model.clone(),
        doing,
        current: current.map(|t| Current { id: t.id, title: t.title.clone(), status: t.status, for_secs: model.in_state_for(t) }),
        helpers: helpers(model, m.session, now),
        waits,
        holds: Vec::new(),
        queued: held.iter().filter(|t| Some(t.id) != current.map(|c| c.id)).count(),
    }
}

/// A member's subagents: at work, then those that handed back lately.
pub fn helpers(model: &Model, member: SessionId, now: Instant) -> Vec<Helper> {
    let secs = |since: Instant| now.saturating_duration_since(since).as_secs();
    let mut out: Vec<Helper> = model
        .subagents_of(member)
        .into_iter()
        .map(|(session, sub)| Helper {
            session,
            id: sub.id.clone(),
            what: sub.what.clone(),
            secs: secs(sub.since),
            state: match sub.stuck {
                Some(quiet) => HelperState::Quiet(secs(quiet)),
                None => HelperState::Running,
            },
        })
        .collect();
    let mut finished: Vec<(Instant, Helper)> = model
        .subagents_all(member)
        .into_iter()
        .filter_map(|(session, sub)| {
            let end = sub.ended.as_ref()?;
            let at = sub.since + end.took;
            (now.saturating_duration_since(at) < FINISHED_FOR).then(|| (at, Helper { session, id: sub.id.clone(), what: sub.what.clone(), secs: end.took.as_secs(), state: HelperState::Finished { failed: end.failed } }))
        })
        .collect();
    finished.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    out.extend(finished.into_iter().take(FINISHED_SHOWN).map(|(_, h)| h));
    out
}

/// Who holds a ticket: the member on it now, else its assignee, else its
/// owner — a member's name, or a role, named after the member of that role
/// when there is one. `None`: nobody.
pub fn holder(model: &Model, t: &Ticket) -> Option<String> {
    if let Some(m) = model.member_on(t.id) {
        return Some(m.name.clone());
    }
    let name = t.assignee.as_deref().or(t.owner.as_deref()).map(str::trim).filter(|n| !n.is_empty())?;
    let open = || model.members.iter().filter(|m| m.presence != Presence::Closed);
    if let Some(m) = open().find(|m| m.name == name) {
        return Some(m.name.clone());
    }
    Some(open().find(|m| m.role == name).map_or_else(|| name.to_owned(), |m| m.name.clone()))
}

/// The open tickets a member holds, the one it is on first, then by where
/// they stand (under way, waiting, to do) and the board's order. Proposals
/// and ideas aren't anyone's work yet.
fn held_by<'a>(model: &'a Model, name: &str) -> Vec<&'a Ticket> {
    let mut out: Vec<&Ticket> = model
        .tickets
        .iter()
        .filter(|t| matches!(t.status, TicketStatus::InProgress | TicketStatus::Waiting | TicketStatus::Todo | TicketStatus::Review))
        .filter(|t| holder(model, t).as_deref() == Some(name))
        .collect();
    let on = model.members.iter().find(|m| m.name == name).and_then(|m| m.ticket);
    let stage = |s: TicketStatus| match s {
        TicketStatus::InProgress => 0,
        TicketStatus::Waiting => 1,
        TicketStatus::Review => 2,
        _ => 3,
    };
    out.sort_by_key(|t| (Some(t.id) != on, stage(t.status), t.order_key(), t.id));
    out
}

/// What the person owes a ticket now, if anything.
pub fn person_owes(model: &Model, t: &Ticket) -> Option<Why> {
    if model.approvals.open_items().any(|a| a.ticket == Some(t.id)) {
        return Some(Why::Permission);
    }
    if model.asked.open().any(|s| s.ticket == Some(t.id)) {
        return Some(Why::Decision);
    }
    if model.is_acted(t) {
        return None;
    }
    match model.section_of(t) {
        Some(Section::Waiting | Section::Stalled) => Some(Why::YourTurn),
        Some(Section::Review) => Some(Why::Review),
        _ => None,
    }
}

/// The first ticket `t` depends on that isn't finished. One not on the board
/// any more is taken as finished: a screen gets the open tickets and the
/// latest closed ones, so one it doesn't have closed long ago.
fn blocker<'a>(model: &'a Model, t: &Ticket) -> Option<&'a Ticket> {
    t.depends_on.iter().filter_map(|d| model.tickets.iter().find(|o| o.id == *d)).find(|o| !o.status.is_closed())
}

/// Whether a member's ticket waits, and on what — followed to its end.
pub fn wait_of(model: &Model, t: &Ticket, me: &str) -> Option<Wait> {
    if let Some(why) = person_owes(model, t) {
        // Handed in: the member's part is done, the work is the person's.
        return Some(Wait { ticket: Some(t.id), hops: Vec::new(), end: End::Person(why) });
    }
    if t.status == TicketStatus::Review {
        // Being accepted — by the PM, or past a designer's look first: it moves.
        return None;
    }
    let mut hops: Vec<Hop> = Vec::new();
    let mut seen: HashSet<TicketId> = HashSet::from([t.id]);
    let mut at = t;
    let mut end = None;
    while let Some(dep) = blocker(model, at) {
        hops.push(Hop { ticket: dep.id, member: holder(model, dep) });
        if !seen.insert(dep.id) {
            end = Some(End::Cycle);
            break;
        }
        if let Some(why) = person_owes(model, dep) {
            end = Some(End::Person(why));
            break;
        }
        at = dep;
    }
    let last = hops.last()?;
    let end = end.unwrap_or(match at.status {
        TicketStatus::InProgress | TicketStatus::Review => End::Moving,
        _ if last.member.is_none() => End::Nobody,
        _ => End::NotStarted,
    });
    // A member's own queue — its next ticket after one of its own — isn't a
    // wait on anyone; what holds its own up is told on that one. Only a loop
    // of its own is said.
    let own = hops.iter().all(|h| h.member.as_deref() == Some(me));
    if own && end != End::Cycle {
        return None;
    }
    // Its own tickets on the way first are its queue too: the wait is on the first that isn't (a loop keeps its whole way).
    if end != End::Cycle {
        let lead = hops.iter().take_while(|h| h.member.as_deref() == Some(me)).count();
        hops.drain(..lead);
    }
    Some(Wait { ticket: Some(t.id), hops, end })
}

fn stuck(model: &Model, lanes: &[Lane]) -> Vec<Stuck> {
    let mut out: Vec<Stuck> = Vec::new();
    let mut loops: HashSet<Vec<TicketId>> = HashSet::new();
    for lane in lanes {
        let mut chained: HashSet<TicketId> = HashSet::new();
        for w in &lane.waits {
            let Some(ticket) = w.ticket else { continue };
            match w.end {
                End::Cycle => {
                    // The loop itself, from the ticket that came round again; each loop once.
                    let again = w.hops.last().map(|h| h.ticket);
                    let mut way: Vec<Hop> = std::iter::once(Hop { ticket, member: Some(lane.name.clone()) }).chain(w.hops.iter().cloned()).collect();
                    if let Some(start) = way.iter().position(|h| Some(h.ticket) == again) {
                        way.drain(..start);
                    }
                    let mut key: Vec<TicketId> = way.iter().map(|h| h.ticket).collect();
                    key.sort_unstable();
                    key.dedup();
                    if loops.insert(key) {
                        out.push(Stuck::Cycle { hops: way });
                    }
                }
                End::Nobody => {
                    if let Some(on) = w.hops.last() {
                        out.push(Stuck::Nobody { member: lane.name.clone(), ticket, on: on.ticket });
                    }
                }
                _ if w.is_chain() && w.on_member() != Some(lane.name.as_str()) => {
                    // Once per ticket held up directly: two tickets waiting on the same one are one chain.
                    if chained.insert(w.hops[0].ticket) {
                        out.push(Stuck::Chain { member: lane.name.clone(), wait: w.clone() });
                    }
                }
                _ => {}
            }
        }
        if let Some(current) = &lane.current
            && let Some(t) = model.tickets.iter().find(|t| t.id == current.id)
        {
            match risk(model, t) {
                Some(Risk::Failed) => out.push(Stuck::Failed { member: lane.name.clone(), ticket: t.id }),
                Some(Risk::Stuck(secs)) => out.push(Stuck::Quiet { member: lane.name.clone(), ticket: t.id, secs }),
                Some(Risk::Late(secs)) => out.push(Stuck::Late { member: lane.name.clone(), ticket: t.id, secs }),
                None => {
                    let idle = lane.doing != Doing::Working && t.status == TicketStatus::InProgress && person_owes(model, t).is_none() && blocker(model, t).is_none();
                    if let Some(secs) = model.in_state_for(t).filter(|s| idle && *s >= STALLED_AFTER) {
                        out.push(Stuck::Stalled { member: lane.name.clone(), ticket: t.id, secs });
                    }
                }
            }
        }
        for h in &lane.helpers {
            match h.state {
                HelperState::Quiet(secs) => out.push(Stuck::HelperQuiet { member: lane.name.clone(), what: h.what.clone(), secs }),
                _ if h.long() => out.push(Stuck::HelperLong { member: lane.name.clone(), what: h.what.clone(), secs: h.secs }),
                _ => {}
            }
        }
    }
    out.sort_by_key(Stuck::rank);
    out
}

/// A member's recent load on its engine's plan, from the Usage screen's look
/// at it: its share of the plan's week (`true`), else of the use seen in the
/// look's stretch (`false`), in basis points. `None`: no such look yet, or it
/// used none of it.
pub fn plan_load(report: Option<&Report>, squad: &str, member: &str) -> Option<(u32, bool)> {
    let slice = report?.members.iter().find(|s| matches!(&s.who, Who::Member { squad: sq, member: m } if sq == squad && m == member))?;
    match slice.week_bp {
        Some(bp) if bp > 0 => Some((bp, true)),
        _ => (slice.share_bp > 0).then_some((slice.share_bp, false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convo::{Convo, Sub, SubEnd};
    use crate::state::Verb;
    use equipe_core::protocol::Approval;
    use equipe_core::protocol::decide::QuestionSet;
    use equipe_core::protocol::plan_use::{Mostly, Slice, Span, Tokens};

    fn ticket(id: TicketId, status: &str, who: Option<&str>, deps: &[TicketId]) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("title {id}"), "body": "", "status": status, "owner": who, "assignee": who,
            "project": "/p", "depends_on": deps, "branch": null, "created_by": "pm", "note": null
        }))
        .unwrap()
    }

    fn member(session: SessionId, name: &str, presence: Presence, on: Option<TicketId>) -> Member {
        let role = name.split('-').next().unwrap_or(name).to_owned();
        Member {
            session,
            name: name.into(),
            role,
            presence,
            verb: Verb::Working,
            ticket: on,
            memos: 0,
            context: None,
            model: Some("claude-opus-5-5".into()),
            asked: None,
            engine: Some(Engine::Claude),
            estimate: None,
            target: None,
            last_seen: 0,
            failed: false,
        }
    }

    fn model(members: Vec<Member>, tickets: Vec<Ticket>) -> Model {
        let mut m = Model::new("acme");
        m.unix = 100_000;
        m.members = members;
        m.tickets = tickets;
        m
    }

    fn lane<'a>(o: &'a Overview, name: &str) -> &'a Lane {
        o.lanes.iter().find(|l| l.name == name).unwrap()
    }

    fn hops(w: &Wait) -> Vec<(TicketId, Option<&str>)> {
        w.hops.iter().map(|h| (h.ticket, h.member.as_deref())).collect()
    }

    /// Frontend's ticket depends on backend's API ticket under way: frontend
    /// waits on backend, backend holds frontend up — and nothing is stuck.
    #[test]
    fn a_member_waits_on_anothers_ticket_and_the_other_holds_it_up() {
        let m = model(
            vec![member(1, "pm", Presence::Ready, None), member(2, "frontend", Presence::Ready, Some(45)), member(3, "backend", Presence::Working, Some(12))],
            vec![ticket(45, "todo", Some("frontend"), &[12]), ticket(12, "in_progress", Some("backend"), &[])],
        );
        let o = overview(&m, Instant::now());
        let front = lane(&o, "frontend");
        assert_eq!(front.doing, Doing::Waiting);
        assert_eq!(front.waits.len(), 1);
        assert_eq!(front.waits[0].ticket, Some(45));
        assert_eq!(hops(&front.waits[0]), [(12, Some("backend"))]);
        assert_eq!(front.waits[0].end, End::Moving);
        assert!(!front.waits[0].is_chain());
        let back = lane(&o, "backend");
        assert_eq!(back.doing, Doing::Working);
        assert_eq!(back.holds, [("frontend".to_owned(), 45)]);
        assert!(o.stuck.is_empty(), "{:?}", o.stuck);
        // The PM first, then who works, then who waits.
        assert_eq!(o.lanes.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["pm", "backend", "frontend"]);
        assert!(lane(&o, "pm").is_quiet());
    }

    /// A member's next ticket after one of its own under way is its queue, not a wait.
    #[test]
    fn a_members_own_queue_is_no_wait() {
        let m = model(vec![member(3, "backend", Presence::Working, Some(70))], vec![ticket(70, "in_progress", Some("backend"), &[]), ticket(72, "todo", Some("backend"), &[70])]);
        let o = overview(&m, Instant::now());
        let back = lane(&o, "backend");
        assert!(back.waits.is_empty());
        assert_eq!(back.current.as_ref().map(|c| c.id), Some(70));
        assert_eq!(back.queued, 1);
    }

    /// Its own ticket first, then another's: the wait is on the other's.
    #[test]
    fn a_wait_through_its_own_ticket_is_on_the_next_member() {
        let m = model(vec![member(2, "frontend", Presence::Ready, Some(62))], vec![ticket(62, "todo", Some("frontend"), &[61]), ticket(61, "todo", Some("frontend"), &[60]), ticket(60, "in_progress", Some("designer"), &[])]);
        let o = overview(&m, Instant::now());
        let waits = &lane(&o, "frontend").waits;
        assert_eq!(waits.iter().map(|w| (w.ticket, hops(w))).collect::<Vec<_>>(), [(Some(62), vec![(60, Some("designer"))]), (Some(61), vec![(60, Some("designer"))])]);
        assert_eq!(waits[0].on_member(), Some("designer"));
    }

    /// frontend-2 → backend's T-74 → backend's T-73, which waits on the person:
    /// a chain, told once; backend waits on the person itself.
    #[test]
    fn a_wait_on_someone_who_waits_too_is_a_chain() {
        let mut waiting = ticket(73, "waiting", Some("backend"), &[]);
        waiting.note = Some("Waiting on you:\n- [ ] the DB password".into());
        let m = model(
            vec![member(3, "backend", Presence::Working, Some(70)), member(5, "frontend-2", Presence::Ready, Some(90))],
            vec![ticket(70, "in_progress", Some("backend"), &[]), waiting, ticket(74, "todo", Some("backend"), &[73]), ticket(90, "todo", Some("frontend-2"), &[74]), ticket(91, "todo", Some("frontend-2"), &[74])],
        );
        let o = overview(&m, Instant::now());
        let front = lane(&o, "frontend-2");
        assert_eq!(hops(&front.waits[0]), [(74, Some("backend")), (73, Some("backend"))]);
        assert_eq!(front.waits[0].end, End::Person(Why::YourTurn));
        assert!(front.waits[0].is_chain());
        let back = lane(&o, "backend");
        assert_eq!(back.doing, Doing::Working, "at work on another ticket meanwhile");
        assert!(back.waits.iter().any(|w| w.ticket == Some(73) && w.on_person() == Some(Why::YourTurn)));
        assert!(!back.waits.iter().any(|w| w.ticket == Some(74)), "its own T-74 behind its own T-73 is told by T-73");
        assert_eq!(back.holds, [("frontend-2".to_owned(), 90), ("frontend-2".to_owned(), 91)]);
        let chains: Vec<&Stuck> = o.stuck.iter().filter(|s| matches!(s, Stuck::Chain { .. })).collect();
        assert_eq!(chains.len(), 1, "two tickets held up by the same one: one chain");
        assert_eq!(chains[0].member(), Some("frontend-2"));
    }

    /// Two members whose tickets wait on each other: one loop, said once,
    /// whichever lane found it, and first among what is stuck.
    #[test]
    fn tickets_waiting_on_each_other_are_one_loop() {
        let m = model(
            vec![member(2, "frontend", Presence::Ready, Some(91)), member(3, "backend", Presence::Ready, Some(92)), member(4, "qa", Presence::Ready, Some(93))],
            vec![ticket(91, "todo", Some("frontend"), &[92]), ticket(92, "todo", Some("backend"), &[91]), ticket(93, "todo", Some("qa"), &[91])],
        );
        let o = overview(&m, Instant::now());
        assert_eq!(lane(&o, "frontend").waits[0].end, End::Cycle);
        let loops: Vec<&Stuck> = o.stuck.iter().filter(|s| matches!(s, Stuck::Cycle { .. })).collect();
        assert_eq!(loops.len(), 1, "{:?}", o.stuck);
        let Stuck::Cycle { hops } = loops[0] else { unreachable!() };
        let way: Vec<TicketId> = hops.iter().map(|h| h.ticket).collect();
        assert_eq!(way.first(), way.last(), "the loop comes round: {way:?}");
        assert_eq!(way.len(), 3);
        assert!(matches!(o.stuck[0], Stuck::Cycle { .. }));
        // qa waits on the loop: it ends in it too, but the loop is told once.
        assert_eq!(lane(&o, "qa").waits[0].end, End::Cycle);
    }

    /// A loop that doesn't come back to the ticket it started from is still a loop.
    #[test]
    fn a_loop_further_down_is_found() {
        let m = model(vec![member(2, "frontend", Presence::Ready, Some(1))], vec![ticket(1, "todo", Some("frontend"), &[2]), ticket(2, "todo", Some("backend"), &[3]), ticket(3, "todo", Some("data"), &[2])]);
        let o = overview(&m, Instant::now());
        let w = &lane(&o, "frontend").waits[0];
        assert_eq!(w.end, End::Cycle);
        assert_eq!(hops(w), [(2, Some("backend")), (3, Some("data")), (2, Some("backend"))]);
        let Some(Stuck::Cycle { hops }) = o.stuck.first() else { panic!("{:?}", o.stuck) };
        assert_eq!(hops.iter().map(|h| h.ticket).collect::<Vec<_>>(), [2, 3, 2]);
    }

    #[test]
    fn a_ticket_no_one_holds_is_said() {
        let m = model(vec![member(2, "frontend", Presence::Ready, Some(45))], vec![ticket(45, "todo", Some("frontend"), &[12]), ticket(12, "todo", None, &[])]);
        let o = overview(&m, Instant::now());
        assert_eq!(lane(&o, "frontend").waits[0].end, End::Nobody);
        assert_eq!(o.stuck, [Stuck::Nobody { member: "frontend".into(), ticket: 45, on: 12 }]);
    }

    /// A dependency done, cancelled or gone from the board holds nothing up.
    #[test]
    fn finished_or_forgotten_dependencies_hold_nothing_up() {
        let m = model(vec![member(2, "frontend", Presence::Ready, Some(45))], vec![ticket(45, "todo", Some("frontend"), &[1, 2, 999]), ticket(1, "done", Some("backend"), &[]), ticket(2, "cancelled", Some("backend"), &[])]);
        let o = overview(&m, Instant::now());
        assert!(lane(&o, "frontend").waits.is_empty());
        assert_eq!(lane(&o, "frontend").doing, Doing::Idle);
    }

    /// A ticket owned by a role is held by the member of that role.
    #[test]
    fn a_role_is_held_by_its_member() {
        let m = model(vec![member(2, "frontend", Presence::Ready, Some(45)), member(3, "backend-2", Presence::Working, None)], vec![ticket(45, "todo", Some("frontend"), &[12]), ticket(12, "todo", Some("backend"), &[])]);
        let t = m.tickets.iter().find(|t| t.id == 12).unwrap();
        assert_eq!(holder(&m, t).as_deref(), Some("backend-2"));
        let mut unknown = t.clone();
        unknown.owner = Some("marketing".into());
        unknown.assignee = None;
        assert_eq!(holder(&m, &unknown).as_deref(), Some("marketing"), "a role nobody has keeps its name");
    }

    /// What waits on the person, counted by kind, adds up to Needs you; a
    /// permission asked mid-turn puts the member's lane on the person.
    #[test]
    fn the_persons_strip_adds_up_to_needs_you_and_a_permission_holds_the_lane() {
        let mut waiting = ticket(55, "waiting", Some("frontend"), &[]);
        waiting.note = Some("Waiting on you:\n- [ ] the key".into());
        let mut m = model(
            vec![member(1, "pm", Presence::Ready, None), member(2, "frontend", Presence::Ready, Some(55)), member(3, "backend", Presence::Working, Some(70))],
            vec![waiting, ticket(42, "review", Some("frontend"), &[]), ticket(70, "in_progress", Some("backend"), &[]), ticket(52, "proposed", Some("frontend"), &[])],
        );
        m.approvals.push(Approval { id: 7, session: 3, member: "backend".into(), ticket: Some(70), tool: "Bash".into(), action: "rm -rf build".into(), reason: None, preview: None });
        m.asked.push(serde_json::from_value::<QuestionSet>(serde_json::json!({"id": 9, "session": 1, "member": "pm", "questions": [], "asked_at": 0})).unwrap());
        let o = overview(&m, Instant::now());
        assert_eq!(o.for_you, ForYou { permissions: 1, decisions: 1, turns: 1, reviews: 1, other: 1 });
        assert_eq!(o.for_you.total(), m.needs_you_count());
        assert_eq!(o.for_you.parts().len(), 5);
        assert_eq!(lane(&o, "backend").doing, Doing::NeedsYou, "at work, but held on the permission");
        assert_eq!(lane(&o, "backend").waits[0].on_person(), Some(Why::Permission));
        assert_eq!(lane(&o, "frontend").doing, Doing::NeedsYou);
        assert_eq!(lane(&o, "pm").waits[0], Wait { ticket: None, hops: vec![], end: End::Person(Why::Decision) });
        assert_eq!(o.lanes[0].name, "pm");
        assert!(ForYou::default().parts().is_empty());
    }

    /// Work under way unheard from, failed, late, or not moving while its member rests.
    #[test]
    fn stuck_work_says_why() {
        let mut m = model(
            vec![member(2, "frontend", Presence::Working, Some(47)), member(3, "backend", Presence::Working, Some(70)), member(4, "designer", Presence::Ready, Some(60)), member(5, "qa", Presence::Working, Some(82))],
            vec![ticket(47, "in_progress", Some("frontend"), &[]), ticket(70, "in_progress", Some("backend"), &[]), ticket(60, "in_progress", Some("designer"), &[]), ticket(82, "in_progress", Some("qa"), &[])],
        );
        m.members[0].last_seen = m.unix - 45 * 60;
        m.members[1].failed = true;
        m.since.insert(60, m.unix - 3 * 3600);
        m.since.insert(82, m.unix - 3600);
        m.members[3].last_seen = m.unix;
        m.members[3].estimate = Some(equipe_core::protocol::Estimate { done_at: Some(m.unix - 40 * 60), ..Default::default() });
        let o = overview(&m, Instant::now());
        assert_eq!(
            o.stuck,
            [
                Stuck::Failed { member: "backend".into(), ticket: 70 },
                Stuck::Quiet { member: "frontend".into(), ticket: 47, secs: 45 * 60 },
                Stuck::Stalled { member: "designer".into(), ticket: 60, secs: 3 * 3600 },
                Stuck::Late { member: "qa".into(), ticket: 82, secs: 40 * 60 },
            ]
        );
        // The designer at work again: not stalled, only slow.
        m.members[2].presence = Presence::Working;
        assert!(!overview(&m, Instant::now()).stuck.iter().any(|s| matches!(s, Stuck::Stalled { .. })));
    }

    /// Subagents at work, one at it unusually long and one gone quiet, and
    /// those that handed back lately — not long ago ones.
    #[test]
    fn subagents_show_in_their_members_lane_and_the_long_ones_are_said() {
        let now = Instant::now();
        let mut m = model(vec![member(3, "backend", Presence::Working, Some(70))], vec![ticket(70, "in_progress", Some("backend"), &[])]);
        let mut convo = Convo::new("/p".into(), false);
        convo.subagents.push(Sub::new("a".into(), "Read the logs".into(), String::new(), now - Duration::from_secs(90)));
        convo.subagents.push(Sub::new("b".into(), "Run the whole test suite".into(), String::new(), now - Duration::from_secs(25 * 60)));
        let mut quiet = Sub::new("c".into(), "Compare encodings".into(), String::new(), now - Duration::from_secs(5 * 60));
        quiet.stuck = Some(now - Duration::from_secs(4 * 60));
        convo.subagents.push(quiet);
        let mut done = Sub::new("d".into(), "Grep the errors".into(), String::new(), now - Duration::from_secs(10 * 60));
        done.ended = Some(SubEnd { failed: false, stopped: false, result: None, took: Duration::from_secs(5 * 60) });
        let mut old = Sub::new("e".into(), "Long ago".into(), String::new(), now - Duration::from_secs(3 * 3600));
        old.ended = Some(SubEnd { failed: true, stopped: false, result: None, took: Duration::from_secs(60) });
        convo.finished = vec![old, done];
        m.convos.insert(3, convo);
        let o = overview(&m, now);
        let back = lane(&o, "backend");
        assert_eq!(back.helpers.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a", "d"]);
        assert_eq!(back.helpers_running(), 3);
        assert!(back.helpers[0].long());
        assert_eq!(back.helpers[1].state, HelperState::Quiet(4 * 60));
        assert_eq!(back.helpers[3].state, HelperState::Finished { failed: false });
        assert_eq!(back.helpers[3].secs, 5 * 60, "how long it took");
        assert_eq!(
            o.stuck,
            [Stuck::HelperQuiet { member: "backend".into(), what: "Compare encodings".into(), secs: 4 * 60 }, Stuck::HelperLong { member: "backend".into(), what: "Run the whole test suite".into(), secs: 25 * 60 }]
        );
    }

    #[test]
    fn a_members_plan_load_is_its_share_of_the_week_else_of_the_look() {
        let slice = |member: &str, share_bp: u32, week_bp: Option<u32>| Slice {
            who: Who::Member { squad: "acme".into(), member: member.into() },
            label: None,
            load: 1,
            share_bp,
            week_bp,
            tokens: Tokens::default(),
            calls: 1,
            mostly: Mostly::Writing,
            children: vec![],
        };
        let report = Report {
            engine: Engine::Claude,
            span: Span::Day,
            from: 0,
            to: 1,
            week: None,
            pace: None,
            load: 1,
            week_load: 1,
            tokens: Tokens::default(),
            calls: 1,
            outside: None,
            projects: vec![],
            tickets: vec![],
            members: vec![slice("backend", 4200, Some(610)), slice("frontend", 1800, None), slice("qa", 0, Some(0))],
            beats: vec![],
            spikes: vec![],
            reading: false,
        };
        assert_eq!(plan_load(Some(&report), "acme", "backend"), Some((610, true)));
        assert_eq!(plan_load(Some(&report), "acme", "frontend"), Some((1800, false)));
        assert_eq!(plan_load(Some(&report), "acme", "qa"), None, "none used");
        assert_eq!(plan_load(Some(&report), "other", "backend"), None, "another squad's member");
        assert_eq!(plan_load(None, "acme", "backend"), None);
    }
}
