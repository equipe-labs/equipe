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
/// A step that took this long is written to the log once it ends, with how
/// long each of its parts took: no step should hold every screen this long.
pub const SLOW_LOGGED: Duration = Duration::from_secs(2);
/// How many of the latest slow steps are kept to be looked at.
const SLOW_KEPT: usize = 32;
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
    /// The part of the step under way, and since when.
    part: Instant,
    /// The parts of the step under way that are over, and how long each took.
    parts: Vec<(Cow<'static, str>, Duration)>,
    /// A step at least this long is kept in `slow` (and logged past [`SLOW_LOGGED`]).
    slow_after: Duration,
    /// The latest slow steps, oldest first.
    slow: std::collections::VecDeque<SlowStep>,
}

/// A step of the loop that took long, and how long each of its parts took.
#[derive(Debug, Clone, PartialEq)]
pub struct SlowStep {
    pub doing: String,
    pub took: Duration,
    /// Its parts, as the loop named them, in order; the first is the step's own start.
    pub parts: Vec<(String, Duration)>,
}

impl SlowStep {
    /// The parts that took longest, for a log line: `idle boards 12.3s, …`.
    pub fn slowest_parts(&self, most: usize) -> String {
        let mut parts: Vec<&(String, Duration)> = self.parts.iter().filter(|(_, d)| *d >= Duration::from_millis(50)).collect();
        parts.sort_by_key(|p| std::cmp::Reverse(p.1));
        parts.iter().take(most).map(|(part, took)| format!("{part} {:.1}s", took.as_secs_f64())).collect::<Vec<_>>().join(", ")
    }
}

tokio::task_local! {
    /// The pulse of the loop this task is: what runs on it marks its parts.
    static ON_LOOP: Arc<Pulse>;
}

/// Run `work` as the loop `pulse` tells of: [`mark`] inside it names the
/// part under way.
pub async fn scoped<F: std::future::Future>(pulse: Arc<Pulse>, work: F) -> F::Output {
    ON_LOOP.scope(pulse, work).await
}

/// On the core loop, the step under way goes on to part `doing` (see
/// [`Pulse::step`]): a slow step's log line, and a stall's, then say which
/// part held it. Elsewhere, nothing.
pub fn mark(doing: &'static str) {
    let _ = ON_LOOP.try_with(|pulse| pulse.step(doing));
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
        let state = State {
            doing: Cow::Borrowed("starting"),
            since: None,
            logged: None,
            ended: now,
            answered: now,
            heal: None,
            part: now,
            parts: Vec::new(),
            slow_after: SLOW_LOGGED,
            slow: std::collections::VecDeque::new(),
        };
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
        state.part = now;
        state.parts.clear();
    }

    /// The step under way goes on to a part of it: said instead, its start
    /// kept, and how long the part before took kept with its name.
    pub fn step(&self, doing: &'static str) {
        let now = self.now();
        let mut state = self.state();
        if state.since.is_some() {
            let took = now.saturating_duration_since(state.part);
            let before = std::mem::replace(&mut state.doing, Cow::Borrowed(doing));
            state.parts.push((before, took));
            state.part = now;
        } else {
            state.doing = Cow::Borrowed(doing);
        }
    }

    /// The step is over. One the log heard about says it is over too; a
    /// slow one is kept, and logged with its slowest parts.
    pub fn end(&self) {
        let now = self.now();
        let mut state = self.state();
        if let Some((at, _)) = state.since {
            let took = now.saturating_duration_since(at);
            if state.logged.is_some() {
                crate::daemon_log!("equipe: watchdog: the daemon answers again — {} took {}s", state.doing, took.as_secs());
            }
            if took >= state.slow_after {
                let last = now.saturating_duration_since(state.part);
                let mut parts: Vec<(String, Duration)> = state.parts.iter().map(|(p, d)| (p.to_string(), *d)).collect();
                parts.push((state.doing.to_string(), last));
                let first = parts.first().map_or_else(|| state.doing.to_string(), |(p, _)| p.clone());
                let slow = SlowStep { doing: first, took, parts };
                if took >= SLOW_LOGGED {
                    crate::daemon_log!("equipe: slow: {} took {:.1}s ({})", slow.doing, took.as_secs_f64(), slow.slowest_parts(4));
                }
                if state.slow.len() >= SLOW_KEPT {
                    state.slow.pop_front();
                }
                state.slow.push_back(slow);
            }
        }
        state.since = None;
        state.logged = None;
        state.ended = now;
        state.parts.clear();
    }

    /// Keep each step at least `after` long to be looked at ([`Self::slow_steps`]),
    /// not only those past [`SLOW_LOGGED`] — for a measurement.
    pub fn keep_steps_from(&self, after: Duration) {
        self.state().slow_after = after;
    }

    /// The latest steps that took long, oldest first, taken.
    pub fn slow_steps(&self) -> Vec<SlowStep> {
        self.state().slow.drain(..).collect()
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
        // What held it: the part under way, and the parts before it that took long.
        let mut doing = state.doing.to_string();
        let mut before: Vec<&(Cow<'static, str>, Duration)> = state.parts.iter().filter(|(_, d)| *d >= Duration::from_secs(1)).collect();
        before.sort_by_key(|p| std::cmp::Reverse(p.1));
        if !before.is_empty() {
            let said: Vec<String> = before.iter().take(3).map(|(p, d)| format!("{p} {}s", d.as_secs())).collect();
            doing = format!("{doing} (after {})", said.join(", "));
        }
        (secs >= after).then(|| Stall {
            doing,
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
    let Some(tm) = equipe_providers::platform::local_time(unix) else { return format!("{unix}") };
    format!("{:02}:{:02}:{:02}", tm.hour, tm.minute, tm.second)
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
    fn a_slow_step_is_kept_with_how_long_each_of_its_parts_took() {
        let secs = Arc::new(AtomicU64::new(0));
        let to = |n: u64| secs.store(n, Ordering::SeqCst);
        let pulse = Pulse::with_clock(moved(&secs));
        pulse.begin("minute tick");
        pulse.step("minute tick: idle boards");
        to(12);
        pulse.step("minute tick: engines");
        // Under way: the stall names the part it is on, and the long one before it.
        let stall = pulse.stall(Duration::ZERO).unwrap();
        assert_eq!(stall.doing, "minute tick: engines (after minute tick: idle boards 12s)");
        to(16);
        pulse.end();
        let slow = pulse.slow_steps();
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].doing, "minute tick");
        assert_eq!(slow[0].took, Duration::from_secs(16));
        let parts: Vec<(&str, u64)> = slow[0].parts.iter().map(|(p, d)| (p.as_str(), d.as_secs())).collect();
        assert_eq!(parts, [("minute tick", 0), ("minute tick: idle boards", 12), ("minute tick: engines", 4)]);
        assert_eq!(slow[0].slowest_parts(2), "minute tick: idle boards 12.0s, minute tick: engines 4.0s");
        assert!(pulse.slow_steps().is_empty(), "taken");
        // A quick step isn't kept, unless asked to keep shorter ones.
        pulse.begin("busy");
        pulse.end();
        assert!(pulse.slow_steps().is_empty());
        pulse.keep_steps_from(Duration::ZERO);
        pulse.begin("busy");
        pulse.end();
        assert_eq!(pulse.slow_steps().len(), 1);
    }

    #[tokio::test]
    async fn a_mark_names_the_part_only_on_the_loop() {
        let pulse = Pulse::new();
        pulse.begin("a turn's end");
        mark("turn end: dispatch");
        assert_eq!(pulse.stall(Duration::ZERO).unwrap().doing, "a turn's end", "not on the loop: nothing");
        scoped(pulse.clone(), async { mark("turn end: dispatch") }).await;
        assert_eq!(pulse.stall(Duration::ZERO).unwrap().doing, "turn end: dispatch");
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
