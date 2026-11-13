//! The person's own window: a hand-off of a member's browser for the person
//! to sign in (or pass a human check) in a normal Chrome window.
//!
//! A member's browser is driven by automation (`assets/browser-host.cjs`
//! launches Chrome under Playwright, with a debugging port). Sites behind
//! Cloudflare and the like see that, and keep failing a "verify you are
//! human" check for the person too — rightly, since the browser in front of
//! them *is* automated. So for a hand-off the automation steps aside:
//!
//! 1. the profile's automated browser is closed (the sign-ins it holds are
//!    saved as on any exit) and left closed;
//! 2. once the profile's folder is free, the same folder is opened in a plain
//!    Chrome — no automation switch, no debugging port, no attachment, the
//!    browser's own user agent — at the page asked for;
//! 3. when the person presses Done (or closes that window and a short grace
//!    passes), the plain window is closed, the folder is free again, and the
//!    automated browser reopens on it — the sign-in's cookies are there — and
//!    goes back to where its member was.
//!
//! This module is the part with no I/O of its own: the state of each
//! hand-off ([`Handoffs`]), moved by what happens (the person's Done, the
//! browser host's answer, time passing) into what to do next ([`Action`]).
//! What the machine and Chrome say come through [`Machine`], so every edge
//! case runs in tests with a fake one. The daemon (`runtime.rs`) carries the
//! actions out against the browser host.
//!
//! Nothing here hides automation from a site or answers a check: the only
//! thing a check ever sees in the window is a person, in a genuine browser.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the browser host has to say it closed a profile's browser.
pub const SUSPEND_WAIT: Duration = Duration::from_secs(10);
/// How long the profile's folder may stay locked by the automated browser
/// after the host says it closed it, before the hand-off goes to the panel.
pub const UNLOCK_WAIT: Duration = Duration::from_secs(20);
/// A window the person closed without pressing Done counts as Done after this.
pub const GRACE: Duration = Duration::from_secs(4);
/// How long the plain window has to quit when asked, before it is made to.
pub const CLOSE_WAIT: Duration = Duration::from_secs(10);
/// How long the automated browser has, once reopened, to show its member's page.
pub const RESUME_WAIT: Duration = Duration::from_secs(20);

// ── the setting ──

/// Where a hand-off for a sign-in goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The browser panel of the app, with the automated browser in it (how
    /// it was before the person's own window).
    Panel,
    /// A normal Chrome window on the daemon's Mac.
    Window,
}

impl Mode {
    pub fn parse(word: &str) -> Option<Mode> {
        match word.trim().to_ascii_lowercase().as_str() {
            "panel" => Some(Mode::Panel),
            "window" => Some(Mode::Window),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Mode::Panel => "panel",
            Mode::Window => "window",
        }
    }
}

fn mode_file(home: &Path) -> PathBuf {
    home.join("browser").join("handoff")
}

/// The setting, as `equipe browser handoff panel|window` keeps it. Not set:
/// `default`.
pub fn mode(home: &Path, default: Mode) -> Mode {
    std::fs::read_to_string(mode_file(home)).ok().and_then(|t| Mode::parse(&t)).unwrap_or(default)
}

pub fn set_mode(home: &Path, mode: Mode) -> std::io::Result<()> {
    std::fs::create_dir_all(home.join("browser"))?;
    std::fs::write(mode_file(home), format!("{}\n", mode.word()))
}

/// What the setting is when the person never set it: the window, on a Mac.
pub fn default_mode() -> Mode {
    if cfg!(target_os = "macos") { Mode::Window } else { Mode::Panel }
}

// ── Chrome, plain ──

/// Where Chrome is, for the window: `$EQUIPE_PLAIN_CHROME`, a path written in
/// `<home>/browser/plain-chrome` (a Chrome installed somewhere unusual), else
/// where Chrome installs itself on a Mac. `None`: not found — the hand-off
/// goes to the panel, and says why.
pub fn chrome_path(home: &Path) -> Option<PathBuf> {
    let given = std::env::var_os("EQUIPE_PLAIN_CHROME").map(PathBuf::from).or_else(|| {
        let text = std::fs::read_to_string(home.join("browser").join("plain-chrome")).ok()?;
        let line = text.lines().next()?.trim();
        (!line.is_empty()).then(|| PathBuf::from(line))
    });
    if let Some(path) = given {
        return path.is_file().then_some(path);
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let usual = "Google Chrome.app/Contents/MacOS/Google Chrome";
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(user) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(user.join("Applications"));
    }
    roots.into_iter().map(|r| r.join(usual)).find(|p| p.is_file())
}

/// The arguments of the plain window: the profile's folder, and the page.
/// Nothing that makes the browser automated or hides that it is — a normal
/// launch. `--hide-crash-restore-bubble`: the folder's last exit is marked as
/// a crash to keep session sign-ins (`assets/keep-sessions.js`); no "Restore
/// pages?" bubble for that.
pub fn chrome_args(dir: &Path, url: &str) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", dir.display()),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        "--hide-crash-restore-bubble".to_owned(),
    ];
    if !url.is_empty() {
        args.push(url.to_owned());
    }
    args
}

/// Whether a Chrome has `dir` in use: its `SingletonLock` names a process
/// that is still there. A lock left by a Chrome that is gone doesn't count.
pub fn profile_locked(dir: &Path) -> bool {
    let lock = dir.join("SingletonLock");
    let Ok(target) = std::fs::read_link(&lock) else { return false };
    // `host-pid`
    let target = target.to_string_lossy().into_owned();
    let Some(pid) = target.rsplit('-').next().and_then(|p| p.parse::<u32>().ok()) else { return false };
    process_alive(pid)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    if pid <= 1 {
        return false;
    }
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

/// What the Mac and Chrome say and do, for the machine's sake — a fake in tests.
pub trait Machine: Send {
    /// A Chrome to open the window with, if there is one.
    fn chrome(&self) -> Option<PathBuf>;
    /// Open `url` in a plain Chrome on `dir`: a new window, or — one is open
    /// for `profile` already — a new tab of it.
    fn open(&mut self, profile: &str, dir: &Path, url: &str) -> std::io::Result<()>;
    /// Whether the window opened for `profile` is still there.
    fn window_open(&mut self, profile: &str) -> bool;
    /// Ask the window to close — or, `force`, make it.
    fn close(&mut self, profile: &str, force: bool);
    /// Whether a Chrome has `dir` in use.
    fn locked(&self, dir: &Path) -> bool;
    /// Quit every Chrome that has `dir` in use (a daemon that came back after
    /// one that died mid-hand-off). How many.
    fn stop_strays(&self, dir: &Path) -> usize;
}

/// This Mac's Chrome.
pub struct System {
    home: PathBuf,
    windows: BTreeMap<String, std::process::Child>,
}

impl System {
    pub fn new(home: &Path) -> Self {
        Self { home: home.to_owned(), windows: BTreeMap::new() }
    }
}

impl Machine for System {
    fn chrome(&self) -> Option<PathBuf> {
        chrome_path(&self.home)
    }

    fn open(&mut self, profile: &str, dir: &Path, url: &str) -> std::io::Result<()> {
        let chrome = self.chrome().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "Chrome is not installed"))?;
        std::fs::create_dir_all(dir)?;
        let mut command = std::process::Command::new(chrome);
        command.args(chrome_args(dir, url)).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // One window per profile: another page goes to it as a tab (that
        // Chrome hands it to the one running and quits).
        if self.window_open(profile) {
            command.spawn()?;
            return Ok(());
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        self.windows.insert(profile.to_owned(), command.spawn()?);
        Ok(())
    }

    fn window_open(&mut self, profile: &str) -> bool {
        self.windows.get_mut(profile).is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn close(&mut self, profile: &str, force: bool) {
        let Some(child) = self.windows.get_mut(profile) else { return };
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        let signal = if force { crate::group::Signal::Kill } else { crate::group::Signal::Terminate };
        crate::group::end(child.id(), signal);
    }

    fn locked(&self, dir: &Path) -> bool {
        profile_locked(dir)
    }

    fn stop_strays(&self, dir: &Path) -> usize {
        stop_chromes_on(dir)
    }
}

/// The main processes of the Chrome(s) running on `dir`, from `ps -axo pid=,command=`
/// (their helpers, `--type=…`, go with them).
fn stray_pids(ps: &str, dir: &Path) -> Vec<u32> {
    let flag = format!("--user-data-dir={}", dir.display());
    ps.lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(' ')?;
            let rest = &command[command.find(&flag)? + flag.len()..];
            // The folder, whole: `…/chrome` is not `…/chromes/jh`.
            if !(rest.is_empty() || rest.starts_with(' ')) || command.contains(" --type=") {
                return None;
            }
            pid.parse().ok()
        })
        .collect()
}

/// Quit the Chrome(s) running on `dir`. How many.
fn stop_chromes_on(dir: &Path) -> usize {
    let Ok(out) = std::process::Command::new("ps").args(["-axo", "pid=,command="]).output() else { return 0 };
    stray_pids(&String::from_utf8_lossy(&out.stdout), dir).into_iter().filter(|pid| crate::group::end(*pid, crate::group::Signal::Terminate)).count()
}

// ── what to do next ──

/// Why a hand-off goes to the browser panel instead of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelWhy {
    /// No Chrome at the usual path (or the one named for it).
    NoChrome,
    /// The window wouldn't open.
    NoWindow,
    /// The automated browser held the profile too long: it didn't let go.
    Locked,
    /// The profile is between a window and its automated browser just now.
    Busy,
}

/// One step to carry out, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Close the profile's automated browser, and keep it closed.
    Suspend { profile: String },
    /// Open the plain window on the profile's folder at `url`.
    Open { profile: String, dir: PathBuf, url: String },
    /// Another page into the open window, as a tab.
    OpenMore { profile: String, dir: PathBuf, url: String },
    /// Close the plain window; `force` when asking wasn't enough.
    CloseWindow { profile: String, force: bool },
    /// Open the automated browser on the profile again, and take each lane
    /// (`(lane, url)`) back to where it was.
    Resume { profile: String, lanes: Vec<(String, String)> },
    /// Give these lanes back to their members: the person is done.
    HandBack { lanes: Vec<String> },
    /// This lane's hand-off goes to the browser panel, as it did before, at `url`.
    Panel { lane: String, url: String, why: PanelWhy },
    /// What a screen shows for the lane: the window is open (a placeholder
    /// instead of the picture) or it isn't.
    Window { lane: String, open: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Asked the host to close the profile's browser.
    Suspending,
    /// Closed; waiting for the folder to come free.
    Unlocking,
    /// The person's window is up. `gone`: since when it isn't.
    Open { gone: Option<Instant> },
    /// Done; waiting for the window to quit and the folder to come free.
    Closing { forced: bool },
    /// The automated browser is coming back.
    Resuming,
}

#[derive(Debug)]
struct Entry {
    dir: PathBuf,
    phase: Phase,
    since: Instant,
    /// The lanes handed over, each with the page to go back to.
    lanes: BTreeMap<String, String>,
    /// The lanes whose person hasn't pressed Done.
    holders: BTreeSet<String>,
    /// Done before the window ever opened.
    cancelled: bool,
    /// The window couldn't be had: at the end, the lanes go to the panel for
    /// this reason, instead of being handed back.
    to_panel: Option<PanelWhy>,
}

impl Entry {
    fn enter(&mut self, phase: Phase, now: Instant) {
        self.phase = phase;
        self.since = now;
    }
}

/// A hand-off in progress, as kept across a daemon restart.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub profile: String,
    pub dir: PathBuf,
    pub lanes: Vec<String>,
}

/// The hand-offs under way, a profile each — the main profile is `""`.
#[derive(Debug, Default)]
pub struct Handoffs {
    entries: BTreeMap<String, Entry>,
}

impl Handoffs {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn of_lane(&self, lane: &str) -> Option<&str> {
        self.entries.iter().find(|(_, e)| e.lanes.contains_key(lane)).map(|(p, _)| p.as_str())
    }

    /// The person still has this lane's browser in a window.
    pub fn holds(&self, lane: &str) -> bool {
        self.entries.values().any(|e| e.holders.contains(lane))
    }

    /// This lane is part of a hand-off still going (including the automated
    /// browser coming back): whatever its pages say is not for the person to
    /// be asked about again meanwhile.
    pub fn busy_lane(&self, lane: &str) -> bool {
        self.of_lane(lane).is_some()
    }

    /// The profile's folder belongs to the person's window: nothing automated
    /// may open on it. (Not once its browser is on its way back.)
    pub fn paused(&self, profile: &str) -> bool {
        self.entries.get(profile).is_some_and(|e| e.phase != Phase::Resuming)
    }

    /// The profiles whose automated browser is closed for a window.
    pub fn paused_profiles(&self) -> Vec<String> {
        self.entries.iter().filter(|(_, e)| e.phase != Phase::Resuming).map(|(p, _)| p.clone()).collect()
    }

    /// Every lane taken part in a hand-off of `profile`.
    pub fn lanes_of(&self, profile: &str) -> Vec<String> {
        self.entries.get(profile).map(|e| e.lanes.keys().cloned().collect()).unwrap_or_default()
    }

    /// Bring each open window forward: Chrome handed no page opens its
    /// profile's window in front again.
    pub fn raise(&self) -> Vec<Action> {
        self.entries
            .iter()
            .filter(|(_, e)| matches!(e.phase, Phase::Open { .. }))
            .map(|(p, e)| Action::OpenMore { profile: p.clone(), dir: e.dir.clone(), url: String::new() })
            .collect()
    }

    /// The hand-offs that have a Chrome on their folder, to keep on disk.
    pub fn saved(&self) -> Vec<Saved> {
        self.entries
            .iter()
            .filter(|(_, e)| !matches!(e.phase, Phase::Suspending | Phase::Resuming))
            .map(|(p, e)| Saved { profile: p.clone(), dir: e.dir.clone(), lanes: e.lanes.keys().cloned().collect() })
            .collect()
    }

    /// A member hands `lane`'s browser, in `profile` (folder `dir`), to the
    /// person for `url`. `chrome`: there is a Chrome to open it in.
    pub fn begin(&mut self, now: Instant, profile: &str, dir: &Path, lane: &str, url: &str, chrome: bool) -> Vec<Action> {
        if !chrome {
            return vec![Action::Panel { lane: lane.to_owned(), url: url.to_owned(), why: PanelWhy::NoChrome }];
        }
        let window = |open| Action::Window { lane: lane.to_owned(), open };
        let Some(e) = self.entries.get_mut(profile) else {
            let e = Entry {
                dir: dir.to_owned(),
                phase: Phase::Suspending,
                since: now,
                lanes: BTreeMap::from([(lane.to_owned(), url.to_owned())]),
                holders: BTreeSet::from([lane.to_owned()]),
                cancelled: false,
                to_panel: None,
            };
            self.entries.insert(profile.to_owned(), e);
            return vec![window(true), Action::Suspend { profile: profile.to_owned() }];
        };
        if matches!(e.phase, Phase::Closing { .. } | Phase::Resuming) {
            return vec![Action::Panel { lane: lane.to_owned(), url: url.to_owned(), why: PanelWhy::Busy }];
        }
        // Another member of the same profile, or the same member again: the
        // one window serves them all.
        e.lanes.insert(lane.to_owned(), url.to_owned());
        e.holders.insert(lane.to_owned());
        e.cancelled = false;
        let mut out = vec![window(true)];
        if matches!(e.phase, Phase::Open { .. }) && !url.is_empty() {
            out.push(Action::OpenMore { profile: profile.to_owned(), dir: e.dir.clone(), url: url.to_owned() });
        }
        out
    }

    /// The host closed `profile`'s automated browser.
    pub fn suspended(&mut self, now: Instant, profile: &str) {
        if let Some(e) = self.entries.get_mut(profile)
            && e.phase == Phase::Suspending
        {
            e.enter(Phase::Unlocking, now);
        }
    }

    /// The person pressed Done for `lane`. The window closes when no one is
    /// left in it.
    pub fn done(&mut self, now: Instant, lane: &str) -> Vec<Action> {
        let Some(profile) = self.of_lane(lane).map(str::to_owned) else { return Vec::new() };
        let Some(e) = self.entries.get_mut(&profile) else { return Vec::new() };
        e.holders.remove(lane);
        if !e.holders.is_empty() {
            return Vec::new();
        }
        match e.phase {
            // Before the window came up: it never does.
            Phase::Suspending | Phase::Unlocking => {
                e.cancelled = true;
                Vec::new()
            }
            Phase::Open { .. } => {
                e.enter(Phase::Closing { forced: false }, now);
                vec![Action::CloseWindow { profile, force: false }]
            }
            Phase::Closing { .. } | Phase::Resuming => Vec::new(),
        }
    }

    /// The window wouldn't open: the automated browser comes back, and the
    /// lanes go to the panel.
    pub fn failed(&mut self, now: Instant, profile: &str) {
        let Some(e) = self.entries.get_mut(profile) else { return };
        e.to_panel = Some(PanelWhy::NoWindow);
        e.holders.clear();
        e.enter(Phase::Closing { forced: false }, now);
    }

    /// The automated browser is back, each lane at its page (or it isn't and
    /// time ran out): hand the lanes back — or, a window that couldn't be had,
    /// give them to the panel.
    pub fn resumed(&mut self, profile: &str) -> Vec<Action> {
        let Some(e) = self.entries.remove(profile) else { return Vec::new() };
        if let Some(why) = e.to_panel {
            return e.lanes.into_iter().map(|(lane, url)| Action::Panel { lane, url, why }).collect();
        }
        if e.lanes.is_empty() {
            return Vec::new();
        }
        vec![Action::HandBack { lanes: e.lanes.into_keys().collect() }]
    }

    /// The lanes with a window up (or on its way, or just closed and the
    /// automated browser not back yet): a screen shows a placeholder for each.
    pub fn window_lanes(&self) -> Vec<String> {
        self.entries.values().filter(|e| e.phase != Phase::Resuming).flat_map(|e| e.lanes.keys().cloned()).collect()
    }

    /// The profiles whose automated browser is coming back and haven't been
    /// handed back yet, and for how long.
    pub fn resuming(&self, now: Instant) -> Vec<(String, Duration)> {
        self.entries.iter().filter(|(_, e)| e.phase == Phase::Resuming).map(|(p, e)| (p.clone(), now.saturating_duration_since(e.since))).collect()
    }

    /// Time passing: what each hand-off waits for may have happened.
    pub fn tick(&mut self, now: Instant, machine: &mut dyn Machine) -> Vec<Action> {
        let mut out = Vec::new();
        let profiles: Vec<String> = self.entries.keys().cloned().collect();
        for profile in profiles {
            let Some(e) = self.entries.get_mut(&profile) else { continue };
            let waited = now.saturating_duration_since(e.since);
            match e.phase {
                // The host never answered (it is gone, or stuck): go on as if it had.
                Phase::Suspending if waited >= SUSPEND_WAIT => e.enter(Phase::Unlocking, now),
                Phase::Suspending | Phase::Resuming => {}
                Phase::Unlocking => {
                    if !machine.locked(&e.dir) {
                        if e.cancelled {
                            e.enter(Phase::Closing { forced: false }, now);
                        } else {
                            e.enter(Phase::Open { gone: None }, now);
                            let first = e.lanes.values().next().cloned().unwrap_or_default();
                            out.push(Action::Open { profile: profile.clone(), dir: e.dir.clone(), url: first.clone() });
                            for url in e.lanes.values().filter(|u| **u != first && !u.is_empty()).collect::<BTreeSet<_>>() {
                                out.push(Action::OpenMore { profile: profile.clone(), dir: e.dir.clone(), url: url.clone() });
                            }
                        }
                    } else if waited >= UNLOCK_WAIT {
                        // The automated browser would not let go of the folder.
                        e.to_panel = Some(PanelWhy::Locked);
                        e.holders.clear();
                        e.enter(Phase::Resuming, now);
                        out.push(Action::Resume { profile: profile.clone(), lanes: e.lanes.iter().map(|(l, u)| (l.clone(), u.clone())).collect() });
                        out.extend(e.lanes.keys().map(|lane| Action::Window { lane: lane.clone(), open: false }));
                    }
                }
                Phase::Open { gone } => {
                    if machine.window_open(&profile) {
                        e.phase = Phase::Open { gone: None };
                    } else {
                        // The person closed the window without Done: Done, after a moment
                        // (a window that comes back — Chrome relaunching — isn't one).
                        let since = gone.unwrap_or(now);
                        if now.saturating_duration_since(since) >= GRACE {
                            e.holders.clear();
                            e.enter(Phase::Closing { forced: false }, now);
                            out.push(Action::CloseWindow { profile: profile.clone(), force: false });
                        } else {
                            e.phase = Phase::Open { gone: Some(since) };
                        }
                    }
                }
                Phase::Closing { forced } => {
                    let still = machine.window_open(&profile);
                    if (!still && !machine.locked(&e.dir)) || waited >= CLOSE_WAIT + UNLOCK_WAIT {
                        e.enter(Phase::Resuming, now);
                        out.push(Action::Resume { profile: profile.clone(), lanes: e.lanes.iter().map(|(l, u)| (l.clone(), u.clone())).collect() });
                        out.extend(e.lanes.keys().map(|lane| Action::Window { lane: lane.clone(), open: false }));
                    } else if still && !forced && waited >= CLOSE_WAIT {
                        e.phase = Phase::Closing { forced: true };
                        out.push(Action::CloseWindow { profile: profile.clone(), force: true });
                    }
                }
            }
        }
        out
    }
}

/// Carry out what concerns the machine itself, as the daemon does: open or
/// close the window. A window that wouldn't open is told to `handoffs`, which
/// brings the automated browser back and gives the lanes to the panel.
pub fn perform(machine: &mut dyn Machine, handoffs: &mut Handoffs, now: Instant, action: &Action) {
    match action {
        Action::Open { profile, dir, url } => {
            if machine.open(profile, dir, url).is_err() {
                handoffs.failed(now, profile);
            }
        }
        Action::OpenMore { profile, dir, url } => {
            let _ = machine.open(profile, dir, url);
        }
        Action::CloseWindow { profile, force } => machine.close(profile, *force),
        _ => {}
    }
}

// ── across a daemon restart ──

fn saved_file(home: &Path) -> PathBuf {
    home.join("browser").join("window-handoffs.json")
}

/// Keep what is in a window now, so a daemon that comes back finds it.
pub fn keep(home: &Path, saved: &[Saved]) {
    let file = saved_file(home);
    if saved.is_empty() {
        let _ = std::fs::remove_file(&file);
        return;
    }
    let _ = std::fs::create_dir_all(home.join("browser"));
    let _ = std::fs::write(&file, serde_json::to_string(saved).unwrap_or_default());
}

/// A daemon that started: any hand-off its predecessor left in a window ends —
/// the plain window is closed, so the profile's folder is free for the
/// automated browser, which the next lane that asks opens, signed in as the
/// person left it. The hand-offs found.
pub fn recover(home: &Path, machine: &dyn Machine) -> Vec<Saved> {
    let file = saved_file(home);
    let found: Vec<Saved> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    for saved in &found {
        machine.stop_strays(&saved.dir);
    }
    let _ = std::fs::remove_file(&file);
    found
}

#[cfg(test)]
mod tests;
