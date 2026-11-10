//! Routines (`protocol::routines`): when each runs next, in the daemon
//! machine's local time, and which are due. The runtime hands a due one to
//! the PM as the person's message and works out its next run.

use crate::protocol::routines::{Routine, Schedule};

/// A run missed by more than this (the Mac was off, asleep) is let go: the
/// routine waits for its next time rather than running hours late.
pub const LATE: u64 = 6 * 60 * 60;

/// A moment as a calendar reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i32,
    pub month: u32,
    /// May run past the month's end: the clock carries it over.
    pub day: i32,
    /// 1 Monday … 7 Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Unix seconds to a calendar and back.
pub trait Clock {
    fn parts(&self, unix: u64) -> Parts;
    fn unix(&self, parts: Parts) -> u64;
}

/// This machine's local time (and its summer time).
pub struct Local;
/// UTC: the same everywhere, for tests.
pub struct Utc;

fn calendar_parts(tm: equipe_providers::platform::CalendarTime) -> Parts {
    Parts {
        year: tm.year,
        month: tm.month,
        day: tm.day as i32,
        weekday: if tm.weekday == 0 { 7 } else { tm.weekday as u8 },
        hour: tm.hour as u8,
        minute: tm.minute as u8,
    }
}

impl Clock for Local {
    fn parts(&self, unix: u64) -> Parts {
        calendar_parts(equipe_providers::platform::local_time(unix).expect("valid Unix time"))
    }

    fn unix(&self, parts: Parts) -> u64 {
        equipe_providers::platform::local_unix(parts.year, parts.month, parts.day, parts.hour, parts.minute).unwrap_or(0)
    }
}

impl Clock for Utc {
    fn parts(&self, unix: u64) -> Parts {
        calendar_parts(equipe_providers::platform::utc_time(unix).expect("valid Unix time"))
    }

    fn unix(&self, parts: Parts) -> u64 {
        equipe_providers::platform::utc_unix(parts.year, parts.month, parts.day, parts.hour, parts.minute).unwrap_or(0)
    }
}

/// The first time after `now` that `when` names.
pub fn next_run(when: Schedule, now: u64, clock: &impl Clock) -> u64 {
    let today = clock.parts(now);
    let (hour, minute) = when.hour_minute();
    let (ahead, step) = match when {
        Schedule::Daily { .. } => (0, 1),
        Schedule::Weekly { weekday, .. } => ((i32::from(weekday) - i32::from(today.weekday)).rem_euclid(7), 7),
    };
    let at = |days: i32| clock.unix(Parts { day: today.day + days, hour, minute, ..today });
    let first = at(ahead);
    if first > now { first } else { at(ahead + step) }
}

/// What to do with a routine at `now`: run it, let a missed run go, or wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Run,
    /// Missed by more than [`LATE`]: only its next run is worked out again.
    Skip,
    Wait,
}

pub fn due(routine: &Routine, now: u64) -> Due {
    match routine.next_run {
        Some(at) if routine.on && at <= now => {
            if now - at > LATE {
                Due::Skip
            } else {
                Due::Run
            }
        }
        _ => Due::Wait,
    }
}

/// The message the PM is handed for a routine: its title, then what it asks.
pub fn message(routine: &Routine) -> String {
    format!("[Routine · {}]\n\n{}", routine.title.trim(), routine.prompt.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 2026-10-19 08:00 UTC.
    const MON_8: u64 = 1_792_396_800;

    fn routine(when: Schedule, next: Option<u64>) -> Routine {
        Routine { id: 1, title: "Weekly report".into(), prompt: "Sum up the week".into(), when, on: true, last_run: None, next_run: next }
    }

    #[test]
    fn a_routine_runs_next_at_its_time_today_this_week_or_the_next() {
        assert_eq!(Utc.parts(MON_8), Parts { year: 2026, month: 10, day: 19, weekday: 1, hour: 8, minute: 0 });
        let hour = 3600;
        // Daily at 9: today; at 8 (now): tomorrow, not this very minute.
        assert_eq!(next_run(Schedule::Daily { hour: 9, minute: 0 }, MON_8, &Utc), MON_8 + hour);
        assert_eq!(next_run(Schedule::Daily { hour: 8, minute: 0 }, MON_8, &Utc), MON_8 + 24 * hour);
        // Weekly: Monday 9 is today; Monday 7 is next week's; Friday 17:30 this week's.
        assert_eq!(next_run(Schedule::Weekly { weekday: 1, hour: 9, minute: 0 }, MON_8, &Utc), MON_8 + hour);
        assert_eq!(next_run(Schedule::Weekly { weekday: 1, hour: 7, minute: 0 }, MON_8, &Utc), MON_8 + 7 * 24 * hour - hour);
        assert_eq!(next_run(Schedule::Weekly { weekday: 5, hour: 17, minute: 30 }, MON_8, &Utc), MON_8 + 4 * 24 * hour + 9 * hour + 1800);
        // Sunday from a Monday: six days on. Across a month's end: the clock carries.
        assert_eq!(Utc.parts(next_run(Schedule::Weekly { weekday: 7, hour: 8, minute: 0 }, MON_8, &Utc)).day, 25);
        let oct_31 = Utc.unix(Parts { year: 2026, month: 10, day: 31, weekday: 6, hour: 22, minute: 0 });
        let next = Utc.parts(next_run(Schedule::Daily { hour: 9, minute: 0 }, oct_31, &Utc));
        assert_eq!((next.month, next.day, next.hour), (11, 1, 9));
    }

    #[test]
    fn a_routine_runs_when_due_and_lets_a_long_missed_run_go() {
        let when = Schedule::Daily { hour: 9, minute: 0 };
        assert_eq!(due(&routine(when, Some(MON_8 + 60)), MON_8), Due::Wait);
        assert_eq!(due(&routine(when, Some(MON_8)), MON_8 + 120), Due::Run, "a minute's tick late");
        assert_eq!(due(&routine(when, Some(MON_8)), MON_8 + LATE + 60), Due::Skip, "the Mac was off all morning");
        let off = Routine { on: false, ..routine(when, Some(MON_8)) };
        assert_eq!(due(&off, MON_8 + 60), Due::Wait);
        assert_eq!(due(&routine(when, None), MON_8), Due::Wait);
        assert_eq!(message(&routine(when, None)), "[Routine · Weekly report]\n\nSum up the week");
    }
}
