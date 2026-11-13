//! Pace to budget: on *auto*, an engine's weekly plan is spent no faster
//! than it can last, by taking fewer new runs at once — never by a lighter
//! model or less thinking (quality stays the person's call), and never by
//! stopping a run mid-turn: only runs that have not started wait.
//!
//! The rule, without a clock or a daemon (the runtime applies it):
//!
//! - **Sustainable burn** — what is left of the week, less a margin of
//!   [`MARGIN`] points that should still be there when it starts over,
//!   spread over the time until then.
//! - **The other engine** — when it is out of its plan until `T`, before this
//!   week starts over, the squad needs this plan to carry it at least to
//!   `T`. Lasting until the reset comes first; only when even one run could
//!   not last that long is the horizon `T` instead, with [`FLOOR_RESERVE`]
//!   points kept, so the squad is never left with nothing before the other
//!   engine is back.
//! - **Measured burn** over the runs at work now (points a second) is
//!   compared with it. Above: runs at once drop in proportion — to
//!   `runs × sustainable / burn`, never under one run (the PM and the most
//!   urgent ticket are always admitted: see [`urgent_first`] and
//!   [`crate::lanes::Member::pm`]). Under [`RAISE_BELOW`] of it with the cap
//!   binding: they come back up, by half again at most per step, and the
//!   pacing ends when the engine's own limit is no tighter than the cap.
//! - **Hysteresis and dwell** — the cap moves only after [`MIN_DWELL`]
//!   since it last moved, and only to raise when the burn is well under the
//!   rate, so it doesn't flap on a noisy reading.
//! - **Max** — the person's override: nothing here applies.
//!
//! Every change is told once, plainly (`engines.pace_*` in the locales) and
//! counted in the squad's journal ([`kind`]).

use equipe_protocol::lanes::Parallel;
use equipe_protocol::{Ticket, TicketId, TicketStatus};

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;

/// Points of the week that should be left when it starts over.
pub const MARGIN: f64 = 10.0;
/// Points kept when the plan only has to reach the other engine's return.
pub const FLOOR_RESERVE: f64 = 2.0;
/// The cap moves at most this often.
pub const MIN_DWELL: i64 = 25 * MINUTE;
/// Raise only when the burn is under this share of the sustainable rate.
pub const RAISE_BELOW: f64 = 0.8;
/// …and aim at this share of it, to leave room.
pub const RAISE_TO: f64 = 0.9;
/// A reading older than this is not acted on.
pub const STALE_AFTER: i64 = 30 * MINUTE;
/// Never fewer runs at once on an engine, as admitted by the cap.
pub const LEAST: usize = 1;
/// Under this many seconds to the reset, pacing has nothing left to save.
const NEAR_RESET: i64 = HOUR;

/// The codes the squad's journal counts pacing changes under (with the
/// turn endings, for the usage report).
pub mod kind {
    /// The cap on an engine's runs went down.
    pub const DOWN: &str = "paced_down";
    /// …went up, still paced.
    pub const UP: &str = "paced_up";
    /// …was lifted: full speed again.
    pub const OFF: &str = "paced_off";
}

/// An engine's week, as pacing needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The weekly window, in percent used.
    pub used: f64,
    /// When it starts over (unix seconds).
    pub resets_at: i64,
    /// Measured burn of the week, in points a second; `None`: not known.
    pub burn: Option<f64>,
    /// Runs at work now, that burn being theirs.
    pub runs: usize,
    /// When the other engine is back, if it is out of its plan now.
    pub other_back: Option<i64>,
    /// How old the reading is, in seconds.
    pub age: i64,
}

/// What pacing keeps between calls, per squad and engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// The runs at once allowed on the engine; `None`: not paced.
    pub cap: Option<usize>,
    /// When the cap last changed.
    pub since: i64,
}

/// Until when the paced plan is meant to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// Its own reset.
    Reset(i64),
    /// The other engine's return: the plan can't last to its own reset.
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
    /// Fewer runs at once, `to` of the `of` the lane policy would allow.
    Lowered { to: usize, of: usize, until: Until },
    Raised { to: usize, of: usize, until: Until },
    /// No longer paced.
    Lifted { until: Until },
}

impl Change {
    pub fn kind(&self) -> &'static str {
        match self {
            Change::Lowered { .. } => kind::DOWN,
            Change::Raised { .. } => kind::UP,
            Change::Lifted { .. } => kind::OFF,
        }
    }
}

/// The sustainable burn in points a second, and until when it holds.
/// `per_run` is the burn of one run, when known: it says whether even one
/// could last until the reset.
pub fn sustainable(b: &Budget, now: i64, per_run: Option<f64>) -> Option<(f64, Until)> {
    let left = 100. - b.used.clamp(0., 100.);
    let to_reset = b.resets_at - now;
    if to_reset <= 0 {
        return None;
    }
    let rate = |room: f64, secs: i64| room.max(0.) / secs.max(1) as f64;
    let own = rate(left - MARGIN, to_reset);
    match b.other_back.filter(|t| *t > now && *t < b.resets_at) {
        Some(t) if per_run.is_some_and(|p| own < p) => Some((rate(left - FLOOR_RESERVE, t - now).max(own), Until::OtherBack(t))),
        _ => Some((own, Until::Reset(b.resets_at))),
    }
}

/// The next state of an engine's pacing at `now`: `base` is the runs at once
/// the lane policy allows it without pacing. `None` change: nothing to tell.
pub fn step(prev: State, base: usize, b: &Budget, parallel: &Parallel, now: i64) -> (State, Option<Change>) {
    // The person's override.
    if parallel.is_max() {
        return (State::default(), None);
    }
    let held = |cap| (State { cap, since: prev.since }, None);
    let Some((_, until)) = sustainable(b, now, None) else {
        // The week started over: nothing to save.
        return if prev.cap.is_some() { (State { cap: None, since: now }, Some(Change::Lifted { until: Until::Reset(b.resets_at) })) } else { held(None) };
    };
    // Not read lately, or no burn to go by: what is, stays.
    let Some(burn) = b.burn.filter(|_| b.age <= STALE_AFTER) else { return held(prev.cap) };
    if b.resets_at - now < NEAR_RESET {
        return if prev.cap.is_some() { (State { cap: None, since: now }, Some(Change::Lifted { until })) } else { held(None) };
    }
    let reference = b.runs.max(1);
    let per_run = (b.runs > 0).then(|| burn / b.runs as f64);
    let (rate, until) = sustainable(b, now, per_run).unwrap_or((0., until));
    let settled = now - prev.since >= MIN_DWELL;
    match prev.cap {
        None if burn > rate => {
            let to = target(reference, rate, burn).max(LEAST);
            if to >= base {
                return held(None);
            }
            (State { cap: Some(to), since: now }, Some(Change::Lowered { to, of: base, until }))
        }
        None => held(None),
        Some(cap) if burn > rate && settled => {
            let to = target(reference, rate, burn).max(LEAST).min(cap);
            if to < cap { (State { cap: Some(to), since: now }, Some(Change::Lowered { to, of: base, until })) } else { held(Some(cap)) }
        }
        // Only a binding cap, and a burn well under the rate, gives room back.
        Some(cap) if burn < rate * RAISE_BELOW && settled && b.runs >= cap.min(base) => {
            let want = target(reference, rate * RAISE_TO, burn.max(f64::MIN_POSITIVE));
            let to = want.min(cap + cap.div_ceil(2)).max(cap);
            if to >= base {
                (State { cap: None, since: now }, Some(Change::Lifted { until }))
            } else if to > cap {
                (State { cap: Some(to), since: now }, Some(Change::Raised { to, of: base, until }))
            } else {
                held(Some(cap))
            }
        }
        Some(cap) => held(Some(cap)),
    }
}

/// Runs in proportion: what `runs` become when the burn is brought from
/// `burn` down (or up) to `rate`.
fn target(runs: usize, rate: f64, burn: f64) -> usize {
    let n = runs as f64 * rate / burn;
    if n.is_finite() { n.floor().clamp(0., 1_000.) as usize } else { 1_000 }
}

/// The cap in force: the lane policy's room, held down by pacing.
pub fn capped(base: usize, state: State) -> usize {
    state.cap.map_or(base, |cap| cap.min(base).max(LEAST.min(base)))
}

/// Whether a run counts against an engine's admission slots. Paused runs —
/// waiting out a limit or a retry — burn nothing and keep their lease; while
/// the engine is paced they don't hold a slot a ticket could start in.
pub fn holds_slot(parked: bool, paced: bool) -> bool {
    !(parked && paced)
}

/// The tickets to admit first while paced, so the little room goes to what
/// matters most: urgent, then high, each in the board's order.
pub fn urgent_first(tickets: &[Ticket]) -> Vec<TicketId> {
    use equipe_protocol::Priority;
    let mut ready: Vec<&Ticket> = tickets.iter().filter(|t| t.status == TicketStatus::Todo && matches!(t.priority, Priority::Urgent | Priority::High)).collect();
    ready.sort_by_key(|t| (t.priority, t.order_key(), t.id));
    ready.into_iter().map(|t| t.id).collect()
}

#[cfg(test)]
mod tests;
