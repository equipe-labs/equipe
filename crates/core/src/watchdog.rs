//! The core loop's pulse, and a watchdog on it.
//!
//! Everything the daemon does for screens goes through one loop
//! ([`crate::runtime`]): when a step of it never ends, every screen waits —
//! a hello, `equipe daemon status`. The loop says what it is doing as it
//! starts each step ([`Pulse::begin`]); a thread of its own, outside the
//! async runtime so it runs even when every worker is stuck, notices a step
//! that takes too long and writes it to the daemon's log, and the gateway
//! answers `busy` from the pulse when the loop doesn't answer.
//!
//! The daemon also asks itself what a screen asks, now and then
//! ([`crate::runtime::Handle::heal_when_hung`]). When nothing answers for
//! [`HUNG_AFTER`] and the loop is on no step — the runtime itself has stopped,
//! not the loop at work — the watchdog says so in the log and ends the
//! daemon, for the login service to start it again ([`hung`]).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use equipe_protocol::rpc::Stall;

/// A step longer than this is written to the log.
pub const STALL_LOGGED: Duration = Duration::from_secs(15);
/// While it lasts, said again this often.
const STALL_REPEATED: Duration = Duration::from_secs(60);
/// How often the watchdog looks.
const LOOK_EVERY: Duration = Duration::from_secs(2);
/// A daemon that hasn't answered its own ask for this long, with its loop on
/// no step, is hung.
pub const HUNG_AFTER: Duration = Duration::from_secs(90);
/// How often the daemon asks itself.
pub const PROBE_EVERY: Duration = Duration::from_secs(10);
/// How a hung daemon exits: not 0, so the login service (`KeepAlive`,
/// `SuccessfulExit` false) starts it again. `EX_TEMPFAIL`.
pub const EXIT_HUNG: i32 = 75;
/// How long the hang dump (`sample`) may take before the daemon leaves anyway.
const DUMP_WAIT: Duration = Duration::from_secs(20);

/// Whether the daemon is hung: nothing has answered its own ask for `after`,
/// counted from the last answer or from the end of the loop's last step,
/// whichever came later, and the loop is on no step now. A step under way,
/// however long, is the loop at work, and the stall line's to tell; once it
/// ends, the ask has `after` again to be answered.
pub fn hung(since_answer: Duration, since_step_end: Duration, in_step: bool, after: Duration) -> bool {
    !in_step && since_answer.min(since_step_end) >= after
}

type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

/// What the core loop is doing, and since when.
pub struct Pulse {
    // Locked only to read or write these, never across anything else.
    state: Mutex<State>,
    clock: Clock,
}

struct State {
    doing: Cow<'static, str>,
    /// The step under way; `None` between steps (waiting for work).
    since: Option<(Instant, SystemTime)>,
    /// The step under way was written to the log, last at this time.
    logged: Option<Instant>,
    /// When the last step ended.
    ended: Instant,
    /// When the loop last answered the daemon's own ask.
    answered: Instant,
    /// Watched for a hang (the daemon's, not a test's): after how long, and
    /// where the dump goes.
    heal: Option<(Duration, Option<PathBuf>)>,
}

/// What a look of the watchdog finds.
#[derive(Debug, PartialEq)]
enum Seen {
    Nothing,
    Stalled(Stall),
    /// Hung: the line for the log.
    Hung(String),
}

impl Pulse {
    pub fn new() -> Arc<Self> {
        Self::with_clock(Box::new(Instant::now))
    }

    fn with_clock(clock: Clock) -> Arc<Self> {
        let now = clock();
        let state = State { doing: Cow::Borrowed("starting"), since: None, logged: None, ended: now, answered: now, heal: None };
        Arc::new(Self { state: Mutex::new(state), clock })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn now(&self) -> Instant {
        (self.clock)()
    }

    /// The loop starts a step.
    pub fn begin(&self, doing: impl Into<Cow<'static, str>>) {
        let now = self.now();
        let mut state = self.state();
        state.doing = doing.into();
        state.since = Some((now, SystemTime::now()));
        state.logged = None;
    }

    /// The step under way goes on to a part of it: said instead, its start kept.
    pub fn step(&self, doing: &'static str) {
        self.state().doing = Cow::Borrowed(doing);
    }

    /// The step is over. One the log heard about says it is over too.
    pub fn end(&self) {
        let now = self.now();
        let mut state = self.state();
        if state.logged.is_some()
            && let Some((at, _)) = state.since
        {
            crate::daemon_log!("equipe: watchdog: the daemon answers again — {} took {}s", state.doing, now.saturating_duration_since(at).as_secs());
        }
        state.since = None;
        state.logged = None;
        state.ended = now;
    }

    /// The loop answered the daemon's own ask.
    pub fn answered(&self) {
        let now = self.now();
        self.state().answered = now;
    }

    /// Watch for a hang from now: one of [`HUNG_AFTER`], say, ends the
    /// process, a dump of its threads left at `dump` first.
    pub fn heal_after(&self, after: Duration, dump: Option<PathBuf>) {
        let now = self.now();
        let mut state = self.state();
        state.answered = now;
        state.heal = Some((after, dump));
    }

    /// The loop is done: its silence from now on is no hang.
    pub fn stop_healing(&self) {
        self.state().heal = None;
    }

    /// The step under way, when it has gone on for `after` or longer.
    pub fn stall(&self, after: Duration) -> Option<Stall> {
        let now = self.now();
        let state = self.state();
        let (at, wall) = state.since?;
        let secs = now.saturating_duration_since(at);
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
        let now = self.now();
        let mut state = self.state();
        if state.logged.is_some_and(|at| now.saturating_duration_since(at) < STALL_REPEATED) {
            return None;
        }
        state.logged = Some(now);
        Some(stall)
    }

    /// The line for the log when the daemon is hung now ([`hung`]).
    fn hung_now(&self) -> Option<String> {
        let now = self.now();
        let state = self.state();
        let (after, dump) = state.heal.as_ref()?;
        let (since_answer, since_end) = (now.saturating_duration_since(state.answered), now.saturating_duration_since(state.ended));
        if !hung(since_answer, since_end, state.since.is_some(), *after) {
            return None;
        }
        let quiet = since_answer.min(since_end);
        let since = SystemTime::now().checked_sub(quiet).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let dump = dump.as_ref().map_or_else(String::new, |d| format!("; what each thread was on is in {}", d.display()));
        Some(format!(
            "equipe: watchdog: the daemon hasn't answered itself for {}s (since {}), and its loop is on no step (it was last on {}): it is hung, so it leaves for the login service to start it again{dump}",
            quiet.as_secs(),
            clock(since),
            state.doing,
        ))
    }

    /// One look of the watchdog.
    fn look(&self) -> Seen {
        if let Some(line) = self.hung_now() {
            return Seen::Hung(line);
        }
        self.due().map_or(Seen::Nothing, Seen::Stalled)
    }

    fn dump(&self) -> Option<PathBuf> {
        self.state().heal.as_ref().and_then(|(_, dump)| dump.clone())
    }
}

/// Watch `pulse` from a thread of its own, until it goes. A hang ends the
/// process ([`leave_hung`]).
pub fn watch(pulse: &Arc<Pulse>) {
    watch_with(pulse, LOOK_EVERY, leave_hung);
}

/// [`watch`], looking every `every`, and on a hang calling `hung` (with the
/// log line and where the dump goes) and watching no more.
fn watch_with(pulse: &Arc<Pulse>, every: Duration, hung: impl FnOnce(&str, Option<PathBuf>) + Send + 'static) {
    let pulse: Weak<Pulse> = Arc::downgrade(pulse);
    let _ = std::thread::Builder::new().name("equipe-watchdog".into()).spawn(move || {
        loop {
            std::thread::sleep(every);
            let Some(pulse) = pulse.upgrade() else { return };
            match pulse.look() {
                Seen::Nothing => {}
                Seen::Stalled(stall) => crate::daemon_log!(
                    "equipe: watchdog: the daemon hasn't answered for {}s (since {}): it is on {} — screens wait meanwhile; `equipe daemon restart` if it stays",
                    stall.secs,
                    clock(stall.since),
                    stall.doing
                ),
                Seen::Hung(line) => {
                    hung(&line, pulse.dump());
                    return;
                }
            }
        }
    });
}

/// Say why in the log, leave a dump of the threads where it is cheap
/// (macOS's `sample`, a couple of seconds), and exit for the login service to
/// start the daemon again.
fn leave_hung(line: &str, dump: Option<PathBuf>) {
    crate::daemon_log!("{line}");
    if let Some(dump) = dump {
        sample_into(&dump);
    }
    std::process::exit(EXIT_HUNG);
}

/// Each thread's stack, sampled for a second, into `file`; given up after
/// [`DUMP_WAIT`].
fn sample_into(file: &std::path::Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let started = std::process::Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .args(["1", "-mayDie", "-file"])
        .arg(file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = started else { return };
    let until = Instant::now() + DUMP_WAIT;
    while Instant::now() < until {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
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
    use std::sync::atomic::{AtomicU64, Ordering};

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

    #[test]
    fn hung_is_silence_with_the_loop_on_no_step() {
        let s = Duration::from_secs;
        let after = HUNG_AFTER;
        assert!(hung(s(91), s(500), false, after), "no answer since, no step: hung");
        assert!(hung(s(90), s(90), false, after), "from the limit on");
        assert!(!hung(s(89), s(500), false, after), "answered within it");
        assert!(!hung(s(500), s(500), true, after), "a long step the pulse knows of is the loop at work");
        assert!(!hung(s(500), s(30), false, after), "a step just ended: the ask has its time again");
        assert!(!hung(s(30), s(500), false, after));
    }

    /// A clock the test moves: the real one, `secs` on.
    fn moved(secs: &Arc<AtomicU64>) -> Clock {
        let (base, secs) = (Instant::now(), secs.clone());
        Box::new(move || base + Duration::from_secs(secs.load(Ordering::SeqCst)))
    }

    #[test]
    fn a_hung_daemon_leaves_and_a_long_step_or_an_unwatched_one_does_not() {
        let secs = Arc::new(AtomicU64::new(0));
        let to = |n: u64| secs.store(n, Ordering::SeqCst);
        let pulse = Pulse::with_clock(moved(&secs));
        let (told, heard) = std::sync::mpsc::channel();
        watch_with(&pulse, Duration::from_millis(2), move |line, dump| {
            let _ = told.send((line.to_owned(), dump));
        });
        let quiet = Duration::from_millis(100);
        to(10_000);
        assert!(heard.recv_timeout(quiet).is_err(), "not watched for a hang (a test's runtime): never");
        pulse.heal_after(HUNG_AFTER, Some(PathBuf::from("/x/daemon-hang.txt")));
        to(10_089);
        assert!(heard.recv_timeout(quiet).is_err(), "89s: not yet");
        pulse.answered();
        to(10_170);
        assert!(heard.recv_timeout(quiet).is_err(), "81s since it answered");
        pulse.begin("command merge");
        to(10_400);
        assert!(heard.recv_timeout(quiet).is_err(), "a step of 230s is the loop at work");
        pulse.end();
        to(10_489);
        assert!(heard.recv_timeout(quiet).is_err(), "89s since the step ended");
        to(10_491);
        let (line, dump) = heard.recv_timeout(Duration::from_secs(5)).expect("a hung daemon leaves");
        assert!(line.contains("hasn't answered itself for 91s") && line.contains("last on command merge") && line.contains("/x/daemon-hang.txt"), "{line}");
        assert_eq!(dump, Some(PathBuf::from("/x/daemon-hang.txt")));
        to(20_000);
        assert!(heard.recv_timeout(quiet).is_err(), "once: the process is gone");
    }

    #[test]
    fn a_loop_that_is_done_is_no_hang() {
        let secs = Arc::new(AtomicU64::new(0));
        let pulse = Pulse::with_clock(moved(&secs));
        pulse.heal_after(HUNG_AFTER, None);
        secs.store(91, Ordering::SeqCst);
        assert!(matches!(pulse.look(), Seen::Hung(_)));
        pulse.stop_healing();
        assert_eq!(pulse.look(), Seen::Nothing);
    }
}
