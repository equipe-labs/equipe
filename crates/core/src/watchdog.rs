//! The core loop's pulse, and a watchdog on it.
//!
//! Everything the daemon does for screens goes through one loop
//! ([`crate::runtime`]): when a step of it never ends, every screen waits —
//! a hello, `equipe daemon status`. The loop says what it is doing as it
//! starts each step ([`Pulse::begin`]); a thread of its own, outside the
//! async runtime so it runs even when every worker is stuck, notices a step
//! that takes too long and writes it to the daemon's log, and the gateway
//! answers `busy` from the pulse when the loop doesn't answer.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use equipe_protocol::rpc::Stall;

/// A step longer than this is written to the log.
pub const STALL_LOGGED: Duration = Duration::from_secs(15);
/// While it lasts, said again this often.
const STALL_REPEATED: Duration = Duration::from_secs(60);
/// How often the watchdog looks.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// What the core loop is doing, and since when.
pub struct Pulse {
    // Locked only to read or write these, never across anything else.
    state: Mutex<State>,
}

struct State {
    doing: Cow<'static, str>,
    /// The step under way; `None` between steps (waiting for work).
    since: Option<(Instant, SystemTime)>,
    /// The step under way was written to the log, last at this time.
    logged: Option<Instant>,
}

impl Pulse {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State { doing: Cow::Borrowed("starting"), since: None, logged: None }) })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The loop starts a step.
    pub fn begin(&self, doing: impl Into<Cow<'static, str>>) {
        let mut state = self.state();
        state.doing = doing.into();
        state.since = Some((Instant::now(), SystemTime::now()));
        state.logged = None;
    }

    /// The step under way goes on to a part of it: said instead, its start kept.
    pub fn step(&self, doing: &'static str) {
        self.state().doing = Cow::Borrowed(doing);
    }

    /// The step is over. One the log heard about says it is over too.
    pub fn end(&self) {
        let mut state = self.state();
        if state.logged.is_some()
            && let Some((at, _)) = state.since
        {
            crate::daemon_log!("equipe: watchdog: the daemon answers again — {} took {}s", state.doing, at.elapsed().as_secs());
        }
        state.since = None;
        state.logged = None;
    }

    /// The step under way, when it has gone on for `after` or longer.
    pub fn stall(&self, after: Duration) -> Option<Stall> {
        let state = self.state();
        let (at, wall) = state.since?;
        let secs = at.elapsed();
        (secs >= after).then(|| Stall {
            doing: state.doing.to_string(),
            since: wall.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            secs: secs.as_secs(),
        })
    }

    /// The step under way, to be written to the log now: once it has gone on
    /// past [`STALL_LOGGED`], and again every [`STALL_REPEATED`] while it lasts.
    fn due(&self) -> Option<Stall> {
        let stall = self.stall(STALL_LOGGED)?;
        let mut state = self.state();
        if state.logged.is_some_and(|at| at.elapsed() < STALL_REPEATED) {
            return None;
        }
        state.logged = Some(Instant::now());
        Some(stall)
    }
}

/// Watch `pulse` from a thread of its own, until it goes.
pub fn watch(pulse: &Arc<Pulse>) {
    let pulse: Weak<Pulse> = Arc::downgrade(pulse);
    let _ = std::thread::Builder::new().name("equipe-watchdog".into()).spawn(move || {
        loop {
            std::thread::sleep(LOOK_EVERY);
            let Some(pulse) = pulse.upgrade() else { return };
            if let Some(stall) = pulse.due() {
                crate::daemon_log!(
                    "equipe: watchdog: the daemon hasn't answered for {}s (since {}): it is on {} — screens wait meanwhile; `equipe daemon restart` if it stays",
                    stall.secs,
                    clock(stall.since),
                    stall.doing
                );
            }
        }
    });
}

/// A time of day as the machine's clock reads it: `13:21:05`.
pub fn clock(unix: u64) -> String {
    // SAFETY: localtime_r only writes the tm it is given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = unix as libc::time_t;
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return format!("{unix}");
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_under_way_is_told_once_it_lasts_and_not_between_steps() {
        let pulse = Pulse::new();
        assert_eq!(pulse.stall(Duration::ZERO), None, "waiting for work is no stall");
        pulse.begin("minute tick");
        pulse.step("minute tick: settle merged");
        let stall = pulse.stall(Duration::ZERO).unwrap();
        assert_eq!(stall.doing, "minute tick: settle merged");
        assert!(stall.since > 0);
        assert_eq!(pulse.stall(Duration::from_secs(3600)), None, "not long enough yet");
        assert_eq!(pulse.due(), None, "the log hears only of a long one");
        pulse.end();
        assert_eq!(pulse.stall(Duration::ZERO), None);
        pulse.begin(format!("command {}", "merge"));
        assert_eq!(pulse.stall(Duration::ZERO).unwrap().doing, "command merge");
    }
}
