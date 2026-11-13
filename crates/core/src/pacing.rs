//! A reserve for the person, on *auto*: the squad runs at full speed — as
//! many runs at once as the lane policy allows ([`crate::lanes`]) — for as
//! long as an engine's weekly window has more than [`RESERVE`] points left.
//! Development is never slowed to stretch a plan that has room.
//!
//! Only when the window drops under the reserve before it starts over does
//! the engine hold back what is left for the person: new runs on it are
//! admitted for the PM (so the squad can still answer) and for urgent and
//! high tickets only; everything else waits. The hold lasts until the
//! window resets — or, when the other engine is out of its plan and comes
//! back before that, until it is back, as the squad has somewhere to work
//! again then. Nothing running is ever stopped mid-turn, and a lighter
//! model or less thinking is never the answer (quality stays the person's
//! call).
//!
//! The rule, without a clock or a daemon (the runtime applies it):
//!
//! - **Enter** — on a fresh reading with under [`RESERVE`] points left and
//!   more than [`NEAR_RESET`] to go until the hold would end.
//! - **Leave** — the window started over, [`NEAR_RESET`] or less is left
//!   until the hold would end (nothing is left to save), or the room is
//!   back at [`RESERVE`] plus [`LEAVE_MARGIN`] points, so a reading that
//!   wobbles around the line doesn't flap.
//! - **A reading older than [`STALE_AFTER`]** is not acted on: what is, stays.
//! - **Max** — the person's override: nothing here applies.
//!
//! Every change is told once, plainly (`engines.pace_*` in the locales) and
//! counted in the squad's journal ([`kind`]).

use equipe_protocol::lanes::Parallel;
use equipe_protocol::{Priority, Ticket, TicketId, TicketStatus};

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;

/// Points of the weekly window that stay for the PM and urgent work: while
/// more than this is left, nothing is held back.
pub const RESERVE: f64 = 15.0;
/// …and the room that must be back, above [`RESERVE`], to let go of the hold.
pub const LEAVE_MARGIN: f64 = 1.0;
/// A reading older than this is not acted on.
pub const STALE_AFTER: i64 = 30 * MINUTE;
/// Under this many seconds until the hold would end, it has nothing left to save.
pub const NEAR_RESET: i64 = HOUR;

/// The codes the squad's journal counts reserve changes under (with the
/// turn endings, for the usage report).
pub mod kind {
    /// The engine fell under its reserve: only the PM and urgent work start.
    pub const ON: &str = "reserve_on";
    /// …and went back to full speed.
    pub const OFF: &str = "reserve_off";
}

/// An engine's week, as the reserve needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The weekly window, in percent used.
    pub used: f64,
    /// When it starts over (unix seconds).
    pub resets_at: i64,
    /// When the other engine is back, if it is out of its plan now.
    pub other_back: Option<i64>,
    /// How old the reading is, in seconds.
    pub age: i64,
}

impl Budget {
    /// Points of the window left.
    pub fn left(&self) -> f64 {
        100. - self.used.clamp(0., 100.)
    }
}

/// What the reserve keeps between calls, per squad and engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Only the PM and urgent work start on the engine.
    pub held: bool,
    /// When it last changed.
    pub since: i64,
}

/// Until when the reserve is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// The window's own reset.
    Reset(i64),
    /// The other engine's return, before the reset.
    OtherBack(i64),
}

impl Until {
    pub fn at(self) -> i64 {
        match self {
            Until::Reset(at) | Until::OtherBack(at) => at,
        }
    }
}

/// What changed, to be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Under the reserve: only the PM and urgent work start, until `until`.
    Held { until: Until },
    /// Full speed again.
    Lifted { until: Until },
}

impl Change {
    pub fn kind(&self) -> &'static str {
        match self {
            Change::Held { .. } => kind::ON,
            Change::Lifted { .. } => kind::OFF,
        }
    }
}

/// Until when the reserve would be held at `now`: the other engine's return
/// when it is out and back before the reset, else the reset. `None`: the
/// window has started over.
pub fn horizon(b: &Budget, now: i64) -> Option<Until> {
    if b.resets_at <= now {
        return None;
    }
    Some(match b.other_back.filter(|t| *t > now && *t < b.resets_at) {
        Some(t) => Until::OtherBack(t),
        None => Until::Reset(b.resets_at),
    })
}

/// The next state of an engine's reserve at `now`. `None` change: nothing to tell.
pub fn step(prev: State, b: &Budget, parallel: &Parallel, now: i64) -> (State, Option<Change>) {
    // The person's override.
    if parallel.is_max() {
        return (State::default(), None);
    }
    let kept = (prev, None);
    let lift = |until| (State { held: false, since: now }, prev.held.then_some(Change::Lifted { until }));
    let Some(until) = horizon(b, now) else {
        // The week started over: nothing to save.
        return lift(Until::Reset(b.resets_at));
    };
    // Not read lately: what is, stays.
    if b.age > STALE_AFTER {
        return kept;
    }
    // Close to the end: the little left is for using.
    if until.at() - now <= NEAR_RESET {
        return lift(until);
    }
    let left = b.left();
    match prev.held {
        false if left < RESERVE => (State { held: true, since: now }, Some(Change::Held { until })),
        true if left >= RESERVE + LEAVE_MARGIN => lift(until),
        _ => kept,
    }
}

/// Whether a run counts against an engine's admission slots. Paused runs —
/// waiting out a limit or a retry — burn nothing and keep their lease; while
/// the reserve is held they don't hold a slot a ticket could start in.
pub fn holds_slot(parked: bool, held: bool) -> bool {
    !(parked && held)
}

/// Whether a ticket of `priority` may start on an engine whose reserve is held: urgent and
/// high priority only (the PM's own work is admitted by who it is, see
/// [`crate::lanes::Member::pm`]).
pub fn admitted(priority: Priority) -> bool {
    matches!(priority, Priority::Urgent | Priority::High)
}

/// The tickets to start first while a reserve is held, so the little room
/// goes to what matters most: urgent, then high, each in the board's order.
pub fn urgent_first(tickets: &[Ticket]) -> Vec<TicketId> {
    let mut ready: Vec<&Ticket> = tickets.iter().filter(|t| t.status == TicketStatus::Todo && admitted(t.priority)).collect();
    ready.sort_by_key(|t| (t.priority, t.order_key(), t.id));
    ready.into_iter().map(|t| t.id).collect()
}

#[cfg(test)]
mod tests;
