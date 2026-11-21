//! When this machine next needs the daemon running (`Snapshot::next_wake_at`).
//!
//! A machine that sleeps or is paused while nobody uses it (a hosted team's
//! machine above all) runs nothing, so work the daemon starts on its own on a
//! clock — a routine's run, a sprint's review or its small items starting, a
//! hypothesis to judge, a decide-and-tell window closing, a paused run going
//! on, the PM woken over an empty board — never starts unless something wakes
//! the machine first. The snapshot names the earliest such time, so whatever
//! hosts the machine can wake it then.
//!
//! Only future work that starts by itself counts: not work already running
//! (members working keep the machine up on their own), not what waits on the
//! person (a sprint proposal on "ask", a routine turned off). Every time is
//! worked out from what the daemon keeps on disk (the squad's journal and
//! sprint store) or holds for a run paused now, each time a snapshot is taken,
//! so it follows every change to a schedule at once and survives a restart.

use crate::autonomy;
use crate::protocol::autonomy::{Kind, Level};
use crate::protocol::routines::Routine;
use crate::protocol::sprint::{ExperimentStatus, Phase};
use crate::sprint::{self as rules, Store};

/// How much earlier than the work the machine is asked to be up: a waker that
/// looks every few minutes may come that late, and the daemon takes a moment
/// to start.
pub const MARGIN: u64 = 5 * 60;

/// When the machine should be up for the earliest of `dues` still ahead of
/// `now`, [`MARGIN`] early (or the work's own time, when that is nearer than
/// the margin: never a time already past). `None`: nothing ahead.
pub fn wake_at(dues: impl IntoIterator<Item = u64>, now: u64) -> Option<u64> {
    let due = dues.into_iter().filter(|&at| at > now).min()?;
    let early = due - MARGIN.min(due);
    Some(if early > now { early } else { due })
}

/// The routines' next runs: those on.
pub fn routines(routines: &[Routine]) -> impl Iterator<Item = u64> + '_ {
    routines.iter().filter(|r| r.on).filter_map(|r| r.next_run)
}

/// When the PM is woken over an empty board (`heartbeat::idle_due`): on,
/// nothing open, quiet for `hours`, and not woken in a day.
pub fn idle(hours: u32, open: usize, last: Option<u64>, woken_at: Option<u64>) -> Option<u64> {
    if hours == 0 || open > 0 {
        return None;
    }
    let quiet = last.map_or(0, |at| at + u64::from(hours) * 60 * 60);
    let again = woken_at.map_or(0, |at| at + crate::heartbeat::IDLE_WAKE_EVERY);
    Some(quiet.max(again))
}

/// What the squad's sprint loop does on its own next (`Runtime::sprint_look`):
/// the sprint under way reviewed at its end, a review the PM let pass closed,
/// a planning ask made again, a proposal's small items started (unless
/// sprints are on "ask"), and its hypotheses checked a day after going live
/// and judged when their window ends. `on`: the loop runs for the squad.
pub fn sprint(store: &Store, on: bool, sprint_level: Level) -> Vec<u64> {
    let mut dues = Vec::new();
    if on {
        match store.current().map(|s| (s.phase, s.ends_at)) {
            Some((Phase::Active, Some(end))) => dues.push(end),
            Some((Phase::Review, Some(end))) => dues.push(end + rules::PATIENCE),
            None if store.proposed().is_none() => dues.extend(store.planning_asked.map(|at| at + rules::PLAN_AGAIN)),
            _ => {}
        }
        // Started from the proposal, or the sprint under way (`sprint_auto_start`).
        let startable = store.proposed().is_some() || store.current().is_some_and(|s| s.phase == Phase::Active);
        if sprint_level != Level::Ask && startable {
            dues.extend(store.auto_at);
        }
    }
    for x in store.experiments.iter().filter(|x| x.status == ExperimentStatus::Live) {
        if let (false, Some(live)) = (x.checked, x.live_at) {
            let check = live + rules::DAY;
            if x.due_at.is_some_and(|d| d > check) {
                dues.push(check);
            }
        }
        dues.extend(x.due_at);
    }
    dues
}

/// Decide-and-tell sets taken on the member's recommendation when their
/// window ends (`Runtime::autonomy_due`) — not those of a kind on "ask".
pub fn autonomy(store: &autonomy::Store) -> impl Iterator<Item = u64> + '_ {
    store.due.iter().filter(|d| store.policy.level(d.kind) != Level::Ask).map(|d| d.at)
}

/// The sprint's own level, for [`sprint`].
pub fn sprint_level(store: &autonomy::Store) -> Level {
    store.policy.level(Kind::Sprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::routines::Schedule;
    use crate::protocol::sprint::Sprint;

    const NOW: u64 = 1_795_300_000;
    const HOUR: u64 = 3600;

    fn routine(on: bool, next: Option<u64>) -> Routine {
        Routine { id: 1, title: "Weekly report".into(), prompt: "Sum up the week".into(), when: Schedule::Daily { hour: 9, minute: 0 }, on, last_run: None, next_run: next }
    }

    fn sprint_in(phase: Phase, ends_at: Option<u64>) -> Sprint {
        Sprint {
            id: 1,
            goal: "More signups".into(),
            done_when: "Signup is one step".into(),
            phase,
            proposed_at: NOW - HOUR,
            started_at: None,
            ends_at,
            closed_at: None,
            items: vec![],
            tickets: vec![],
            experiments: vec![],
            budget_tokens: 0,
            spent_tokens: 0,
            set: 0,
            retro: None,
            facts: None,
        }
    }

    #[test]
    fn the_wake_is_the_earliest_work_ahead_a_margin_early_and_never_past() {
        assert_eq!(wake_for_one(NOW + 30 * 60), Some(NOW + 30 * 60 - MARGIN));
        assert_eq!(wake_at([NOW + 2 * HOUR, NOW + HOUR, NOW - HOUR], NOW), Some(NOW + HOUR - MARGIN), "the earliest; past work is the running daemon's");
        assert_eq!(wake_for_one(NOW + 120), Some(NOW + 120), "nearer than the margin: its own time, not one already past");
        assert_eq!(wake_at([NOW, NOW - 1], NOW), None);
        assert_eq!(wake_at(std::iter::empty(), NOW), None);
    }

    fn wake_for_one(at: u64) -> Option<u64> {
        wake_at([at], NOW)
    }

    #[test]
    fn only_routines_turned_on_count() {
        let all = [routine(true, Some(NOW + HOUR)), routine(false, Some(NOW + 60)), routine(true, None)];
        assert_eq!(routines(&all).collect::<Vec<_>>(), [NOW + HOUR]);
    }

    #[test]
    fn the_idle_wake_counts_only_while_on_and_the_board_is_empty() {
        assert_eq!(idle(24, 0, Some(NOW), None), Some(NOW + 24 * HOUR));
        assert_eq!(idle(4, 0, Some(NOW), Some(NOW - HOUR)), Some(NOW - HOUR + 24 * HOUR), "not woken twice in a day");
        assert_eq!(idle(0, 0, Some(NOW), None), None, "off");
        assert_eq!(idle(24, 2, Some(NOW), None), None, "work open: nothing to wake the PM for");
    }

    #[test]
    fn a_sprint_wakes_for_its_review_its_auto_start_and_its_hypotheses() {
        let mut store = Store { sprints: vec![sprint_in(Phase::Active, Some(NOW + 3 * HOUR))], auto_at: Some(NOW + 30 * 60), ..Default::default() };
        assert_eq!(sprint(&store, true, Level::Tell), [NOW + 3 * HOUR, NOW + 30 * 60]);
        assert_eq!(sprint(&store, true, Level::Ask), [NOW + 3 * HOUR], "on ask, its items wait for the person's tap");
        assert!(sprint(&store, false, Level::Tell).is_empty(), "the loop off: nothing of it runs");
        store.sprints = vec![sprint_in(Phase::Declined, None)];
        assert!(sprint(&store, true, Level::Tell).is_empty(), "the person said not now: nothing left to start");
        store.sprints = vec![sprint_in(Phase::Review, Some(NOW))];
        store.auto_at = None;
        assert_eq!(sprint(&store, true, Level::Tell), [NOW + rules::PATIENCE]);
        store.sprints.clear();
        store.planning_asked = Some(NOW);
        assert_eq!(sprint(&store, true, Level::Tell), [NOW + rules::PLAN_AGAIN]);
    }

    #[test]
    fn decide_and_tell_windows_count_unless_their_kind_is_on_ask() {
        let mut store = autonomy::Store { due: vec![autonomy::Due { set: 1, kind: Kind::Sprint, at: NOW + HOUR }, autonomy::Due { set: 2, kind: Kind::Design, at: NOW + 2 * HOUR }], ..Default::default() };
        store.policy.sprint = Level::Ask;
        store.policy.design = Level::Tell;
        assert_eq!(autonomy(&store).collect::<Vec<_>>(), [NOW + 2 * HOUR]);
        assert_eq!(sprint_level(&store), Level::Ask);
        store.policy.sprint = Level::Tell;
        assert_eq!(autonomy(&store).collect::<Vec<_>>(), [NOW + HOUR, NOW + 2 * HOUR]);
    }
}
