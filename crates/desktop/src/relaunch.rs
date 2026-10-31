//! Where an update stands, for the sidebar's pill, settings and the update
//! notices: found and being downloaded ("새 버전 0.49.0 받는 중…"), downloaded
//! and waiting to go in ("다시 시작해 업데이트", as Claude's "Relaunch to
//! update"), or failed ("업데이트 받기 실패 · 다시 시도", the reason in plain
//! words and the whole error in ~/Library/Logs/equipe/updates.log). On a Mac
//! Sparkle does the work and its delegate says each step (src/update.rs); a
//! click installs the waiting update and starts the app again, and quitting
//! installs it anyway.
//!
//! The rules are plain functions here, tested: how a step moves the state
//! ([`next`]), when the app may look for an update on its own
//! ([`may_check_quietly`]: never while Sparkle is in a session or an update is
//! under way or waiting — a second session against one in progress is a known
//! way to Sparkle's "An error occurred while running the updater"), what an
//! error means, and whether the app sits somewhere it can't update itself.

use equipe_core::t;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::watch;

/// Where an update stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Update {
    #[default]
    Idle,
    /// Sparkle found `version`; it downloads it next.
    Found(String),
    Downloading(String),
    /// Downloaded and checked: goes in on a restart.
    Ready(String),
    Failed { version: Option<String>, why: Why },
}

impl Update {
    /// The version downloaded and waiting, if one is.
    pub fn ready(&self) -> Option<&str> {
        match self {
            Update::Ready(v) => Some(v),
            _ => None,
        }
    }

    /// The version being downloaded, if one is.
    pub fn downloading(&self) -> Option<&str> {
        match self {
            Update::Found(v) | Update::Downloading(v) => Some(v),
            _ => None,
        }
    }
}

/// What Sparkle said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Found(String),
    Downloading(String),
    Ready(String),
    /// A check found nothing newer.
    NoUpdate,
    /// The person cancelled (a download, an authorization).
    Cancelled,
    /// An error, by Sparkle's code (SUErrors.h).
    Failed { code: i64 },
    /// The feed names this release, read while an update waits: a restart
    /// lands on it when it is newer than the one downloaded ([`chain`]).
    Newest(String),
}

/// Why an update failed, in words the person can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Run from the disk image or a quarantined download (translocated): it
    /// can't replace itself — it must be in Applications.
    Move,
    /// No permission to write where the app is.
    Permission,
    Download,
    Signature,
    Feed,
    Other(i64),
}

impl Why {
    /// What an error code of Sparkle's means; `None`: not a failure (no update,
    /// or the person cancelled).
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1001 | 4007 | 4008 => return None,
            1003 | 1005 => Why::Move,
            4000 | 4001 | 4012 => Why::Permission,
            2000 | 2001 => Why::Download,
            0..=2 | 3001 | 3002 => Why::Signature,
            1000 | 1002 | 1004 | 3 | 4 => Why::Feed,
            code => Why::Other(code),
        })
    }

    pub fn words(self) -> String {
        match self {
            Why::Move => t!("update.why_move").into(),
            Why::Permission => t!("update.why_permission").into(),
            Why::Download => t!("update.why_download").into(),
            Why::Signature => t!("update.why_signature").into(),
            Why::Feed => t!("update.why_feed").into(),
            Why::Other(code) => t!("update.why_other", code = code),
        }
    }
}

/// Where `now` goes on `step`. A downloaded update stays ready through
/// whatever a later check says — only a newer one, or an install failing,
/// replaces it.
pub fn next(now: &Update, step: Step) -> Update {
    let version = || match now {
        Update::Found(v) | Update::Downloading(v) | Update::Ready(v) => Some(v.clone()),
        Update::Failed { version, .. } => version.clone(),
        Update::Idle => None,
    };
    match (now, step) {
        (_, Step::Ready(v)) => Update::Ready(v),
        // A newer release than the waiting one: the restart lands there, so the pill says it.
        (Update::Ready(v), Step::Newest(newest)) if newer(&newest, v) => Update::Ready(newest),
        (_, Step::Newest(_)) => now.clone(),
        (Update::Ready(_), Step::Failed { code }) if (4000..5000).contains(&code) => match Why::from_code(code) {
            Some(why) => Update::Failed { version: version(), why },
            None => now.clone(),
        },
        (Update::Ready(_), _) => now.clone(),
        (_, Step::Found(v)) => Update::Found(v),
        (_, Step::Downloading(v)) => Update::Downloading(v),
        (_, Step::NoUpdate | Step::Cancelled) => Update::Idle,
        (_, Step::Failed { code }) => match Why::from_code(code) {
            Some(why) => Update::Failed { version: version(), why },
            None => Update::Idle,
        },
    }
}

/// Whether the app may look for an update without being asked: not while
/// Sparkle runs a session, nor while an update is found, downloading or
/// waiting — that one is Sparkle's to finish.
pub fn may_check_quietly(now: &Update, session_in_progress: bool) -> bool {
    !session_in_progress && !matches!(now, Update::Found(_) | Update::Downloading(_) | Update::Ready(_))
}

// ── the newest, in one restart ──
//
// Sparkle commits to an update once it has downloaded and staged it: every
// later check resumes that installer and reads no feed (SPUUpdater.m,
// `installerIsRunning` → `resumeInstallingUpdate`), and it goes in when the
// app quits whatever happens. So with releases out since, a restart would
// land on the staged one and need a second for the newest. Instead: while one
// waits, the app reads the feed itself and the pill names the newest; the
// restart leaves a note (`update-chain`), puts the staged one in, and the app
// starting again on it fetches the newest and puts it in at once — the person
// pressed once and lands on the newest.

/// Whether release `a` is newer than `b` (`0.94.0` > `0.93.10` > `0.93.0`).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim().trim_start_matches('v').split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<u64>>();
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The newest release an appcast names: each item's `sparkle:shortVersionString`,
/// else its `sparkle:version`.
pub fn feed_version(xml: &str) -> Option<String> {
    let mut newest: Option<String> = None;
    for item in xml.split("<item>").skip(1) {
        let tag = |name: &str| {
            let open = format!("<{name}>");
            let start = item.find(&open)? + open.len();
            let end = item[start..].find("</")? + start;
            Some(item[start..end].trim().to_owned()).filter(|v| !v.is_empty())
        };
        if let Some(v) = tag("sparkle:shortVersionString").or_else(|| tag("sparkle:version"))
            && newest.as_deref().is_none_or(|n| newer(&v, n))
        {
            newest = Some(v);
        }
    }
    newest
}

/// The feed the app's Info.plist names (`SUFeedURL`).
pub fn feed_url(plist: &str) -> Option<String> {
    let after = &plist[plist.find("<key>SUFeedURL</key>")? + "<key>SUFeedURL</key>".len()..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")? + start;
    Some(after[start..end].trim().to_owned()).filter(|u| u.starts_with("https://"))
}

/// The note a restart into the newest leaves for the app starting again:
/// the release to land on, and when it was asked (Unix seconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub target: String,
    pub since: u64,
}

/// How long the app starting again keeps at it before giving up.
pub const CHAIN_FOR: u64 = 15 * 60;

/// What the app starting again does with the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStep {
    /// Not there yet: fetch the newest and put it in at once.
    Go,
    /// Landed (or past it): the note goes.
    Done,
    /// Kept at it too long, or a note from the future: it goes, nothing more.
    GiveUp,
}

pub fn chain_step(running: &str, chain: &Chain, now: u64) -> ChainStep {
    if !newer(&chain.target, running) {
        ChainStep::Done
    } else if now < chain.since || now - chain.since > CHAIN_FOR {
        ChainStep::GiveUp
    } else {
        ChainStep::Go
    }
}

impl Chain {
    pub fn parse(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        let target = words.next()?.to_owned();
        let since = words.next()?.parse().ok()?;
        Some(Self { target, since })
    }

    pub fn text(&self) -> String {
        format!("{} {}\n", self.target, self.since)
    }
}

/// Whether a restart should go on to `shown` (the pill's) past `staged` (the
/// one Sparkle downloaded): a note to leave, or none.
pub fn chain_for(shown: &str, staged: &str, now: u64) -> Option<Chain> {
    newer(shown, staged).then(|| Chain { target: shown.to_owned(), since: now })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn chain_path() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("update-chain")
}

/// The version Sparkle downloaded and will put in (the pill may name a newer one).
fn staged() -> &'static std::sync::Mutex<Option<String>> {
    static STAGED: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();
    STAGED.get_or_init(|| std::sync::Mutex::new(None))
}

/// The app starting again on its way to the newest: what Sparkle downloads next goes in at once.
static CHAINING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn chaining() -> bool {
    CHAINING.load(std::sync::atomic::Ordering::SeqCst)
}

/// At launch: a note from a restart into the newest — keep at it, or let it go.
pub fn begin_chain() {
    let Some(chain) = std::fs::read_to_string(chain_path()).ok().and_then(|t| Chain::parse(&t)) else { return };
    match chain_step(env!("CARGO_PKG_VERSION"), &chain, unix_now()) {
        ChainStep::Go => {
            log(&format!("on the way to {}: fetching it to put in at once", chain.target));
            CHAINING.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        step => {
            log(&format!("update chain to {}: {step:?}", chain.target));
            let _ = std::fs::remove_file(chain_path());
        }
    }
}

/// "다시 시작해 업데이트" with a newer release than the one downloaded: leave
/// the note for the app starting again.
pub fn chain_if_newer() {
    let shown = current().ready().map(str::to_owned);
    let staged = staged().lock().ok().and_then(|s| s.clone());
    if let (Some(shown), Some(staged)) = (shown, staged)
        && let Some(chain) = chain_for(&shown, &staged, unix_now())
    {
        log(&format!("restart puts in {staged}, then goes on to {shown}"));
        let _ = std::fs::write(chain_path(), chain.text());
    }
}

/// While an update waits, read the feed (off the main thread, at most every
/// [`LOOK_AGAIN_AFTER`]): a newer release becomes the pill's.
pub fn look_at_feed() {
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    if current().ready().is_none() {
        return;
    }
    {
        let Ok(mut last) = LAST.lock() else { return };
        if last.is_some_and(|at| at.elapsed() < LOOK_AGAIN_AFTER) {
            return;
        }
        *last = Some(std::time::Instant::now());
    }
    let Some(plist) = bundle().and_then(|b| std::fs::read_to_string(b.join("Contents/Info.plist")).ok()) else { return };
    let Some(url) = feed_url(&plist) else { return };
    std::thread::spawn(move || {
        let out = std::process::Command::new("/usr/bin/curl").args(["-fsSL", "--max-time", "20", &url]).output();
        if let Some(newest) = out.ok().filter(|o| o.status.success()).and_then(|o| feed_version(&String::from_utf8_lossy(&o.stdout))) {
            step(Step::Newest(newest));
        }
    });
}

fn channel() -> &'static watch::Sender<Update> {
    static STATE: OnceLock<watch::Sender<Update>> = OnceLock::new();
    STATE.get_or_init(|| watch::channel(Update::Idle).0)
}

/// Follow where the update stands: now, and whenever it changes.
pub fn watch() -> watch::Receiver<Update> {
    channel().subscribe()
}

pub fn current() -> Update {
    channel().borrow().clone()
}

/// What Sparkle said, into the state (and the log).
pub fn step(step: Step) {
    log(&format!("{step:?}"));
    // What Sparkle downloaded: what a restart puts in, whatever the pill names.
    if let Step::Ready(v) = &step
        && let Ok(mut staged) = staged().lock()
    {
        *staged = Some(v.clone());
    }
    channel().send_if_modified(|now| {
        let new = next(now, step);
        let changed = *now != new;
        *now = new;
        changed
    });
}

/// What the sidebar's button says when an update waits.
pub fn label(version: &str) -> String {
    t!("relaunch.label", version = version)
}

/// Where the daemon's own update stands, once the person asked for it
/// (this Mac, or the machine it runs on over SSH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonUpdate {
    Running,
    /// Installed: the daemon starts again on it once its members are idle.
    Done,
    /// The person asked for it now, not once the members are idle.
    Restarting,
    Failed(String),
}

/// The daemon the app talks to, as the update pill needs it.
#[derive(Debug, Clone, Default)]
pub struct Daemon {
    /// Where it runs, as the person knows it: "이 Mac", `tinums-mac-mini`.
    pub place: String,
    /// Its release (`0.86.4`), and whether that is older than this app's.
    pub theirs: String,
    pub older: bool,
    /// On another machine: its update is its own, over SSH.
    pub remote: bool,
    pub update: Option<DaemonUpdate>,
    /// "Restart now" pressed once: the next press restarts it mid-turn.
    pub armed: bool,
}

/// What a press on the update pill does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing: said quietly.
    None,
    Relaunch,
    UpdateDaemon,
    /// The daemon's machine first, then the app restarts into its new version.
    UpdateBoth,
    ArmRestart,
    RestartDaemonNow,
    Reveal,
    CheckNow,
}

/// The one update pill: this app's update and the daemon's, together — the
/// person needn't know they are two. A daemon at work on its update first,
/// then the app's version waiting (with a remote daemon behind it, both in
/// one press), then a daemon behind this app, then the app's download.
pub fn both(app: &Update, daemon: &Daemon, ours: &str) -> Option<(String, Act, bool)> {
    let place = &daemon.place;
    match &daemon.update {
        Some(DaemonUpdate::Running) => return Some((t!("relaunch.daemon_running", place = place), Act::None, false)),
        Some(DaemonUpdate::Restarting) => return Some((t!("relaunch.daemon_restarting", place = place), Act::None, false)),
        Some(DaemonUpdate::Failed(_)) => return Some((t!("relaunch.daemon_failed", place = place), Act::UpdateDaemon, false)),
        _ => {}
    }
    if let Update::Ready(version) = app {
        let behind = daemon.remote && crate::notify::stale_daemon(&daemon.theirs, version).is_some() && daemon.update.is_none();
        return Some(if behind { (t!("relaunch.both", version = version), Act::UpdateBoth, true) } else { (label(version), Act::Relaunch, true) });
    }
    match &daemon.update {
        Some(DaemonUpdate::Done) if daemon.armed => return Some((t!("main.update_daemon_now_sure").into(), Act::RestartDaemonNow, false)),
        Some(DaemonUpdate::Done) => return Some((t!("relaunch.daemon_done", place = place), Act::ArmRestart, false)),
        _ => {}
    }
    if daemon.older {
        return Some((t!("relaunch.daemon", place = place, theirs = daemon.theirs.as_str(), ours = ours), Act::UpdateDaemon, true));
    }
    let words = pill(app)?;
    Some(match app {
        Update::Failed { why: Why::Move | Why::Permission, .. } => (words, Act::Reveal, false),
        Update::Failed { .. } => (words, Act::CheckNow, false),
        _ => (words, Act::None, false),
    })
}

/// What the sidebar's pill says, if anything: being downloaded, waiting, failed.
pub fn pill(now: &Update) -> Option<String> {
    match now {
        Update::Idle => None,
        Update::Found(v) | Update::Downloading(v) => Some(t!("relaunch.downloading", version = v)),
        Update::Ready(v) => Some(label(v)),
        Update::Failed { why: Why::Move | Why::Permission, .. } => Some(t!("relaunch.move").into()),
        Update::Failed { .. } => Some(t!("relaunch.failed").into()),
    }
}

/// Install the waiting update now and start again. False when there is none
/// to install this way (then it goes in when the app quits).
pub fn now() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::update::install_now()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

// ── where the app is ──

/// Somewhere the app can't replace itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misplaced {
    /// Opened straight from the disk image.
    DiskImage,
    /// Opened from a quarantined download: macOS runs it from a random
    /// read-only place (App Translocation).
    Translocated,
    /// Its folder isn't writable by the person.
    ReadOnly,
}

/// Whether the bundle at `bundle` can update itself; `writable`: whether its folder is.
pub fn misplaced_at(bundle: &str, writable: bool) -> Option<Misplaced> {
    if bundle.contains("/AppTranslocation/") {
        Some(Misplaced::Translocated)
    } else if bundle.starts_with("/Volumes/") {
        Some(Misplaced::DiskImage)
    } else if !writable {
        Some(Misplaced::ReadOnly)
    } else {
        None
    }
}

/// The app's bundle, when it runs from one.
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?.to_path_buf();
    bundle.extension().is_some_and(|e| e == "app").then_some(bundle)
}

/// Whether this app, as it runs, can't update itself (asked once).
pub fn misplaced() -> Option<Misplaced> {
    static WHERE: OnceLock<Option<Misplaced>> = OnceLock::new();
    *WHERE.get_or_init(|| {
        let bundle = bundle()?;
        let parent = bundle.parent()?;
        let writable = std::ffi::CString::new(parent.as_os_str().as_encoded_bytes()).is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::W_OK) } == 0);
        misplaced_at(&bundle.to_string_lossy(), writable)
    })
}

/// "Finder에서 보기": the app in its folder — or, translocated (where it
/// runs is a copy), the Applications folder to move it to.
pub fn reveal() {
    let _ = match (misplaced(), bundle()) {
        (Some(Misplaced::Translocated) | None, _) | (_, None) => std::process::Command::new("open").arg("/Applications").spawn(),
        (_, Some(bundle)) => std::process::Command::new("open").arg("-R").arg(bundle).spawn(),
    };
}

// ── the log ──

/// ~/Library/Logs/equipe/updates.log: every step Sparkle took and every error, whole.
pub fn log_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Logs/equipe/updates.log"))
}

pub fn log(line: &str) {
    use std::io::Write as _;
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {} {line}", stamp(secs), env!("CARGO_PKG_VERSION"));
    }
}

/// `secs` since 1970 as `2026-10-16 13:05:09Z`.
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// "로그 보기".
pub fn open_log() {
    if let Some(path) = log_path() {
        if !path.exists() {
            log("(opened from settings)");
        }
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
}

// ── the updater ──

/// The app's updates, for settings: whether there is an updater at all
/// (a packaged app), whether it works on its own, check now, turn it on or off.
pub fn can_update(cx: &gpui_kit::App) -> bool {
    #[cfg(target_os = "macos")]
    {
        cx.try_global::<crate::update::Updates>().is_some_and(|u| u.0.is_some())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        false
    }
}

pub fn automatic(cx: &gpui_kit::App) -> bool {
    #[cfg(target_os = "macos")]
    {
        cx.try_global::<crate::update::Updates>().and_then(|u| u.0.clone()).is_some_and(|u| u.automatic())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        false
    }
}

pub fn set_automatic(on: bool, cx: &gpui_kit::App) {
    #[cfg(target_os = "macos")]
    if let Some(updater) = cx.try_global::<crate::update::Updates>().and_then(|u| u.0.clone()) {
        updater.set_automatic(on);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (on, cx);
}

/// "지금 확인" (and the app menu's "업데이트 확인…"): an update waiting goes
/// in now; otherwise Sparkle looks, in its own window.
pub fn check_now(cx: &gpui_kit::App) {
    #[cfg(target_os = "macos")]
    if let Some(updater) = cx.try_global::<crate::update::Updates>().and_then(|u| u.0.clone()) {
        updater.check();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

/// After a launch, and when the window comes back after a while: look for an
/// update without a window, so a release shows its button in minutes, not hours.
pub const LOOK_AGAIN_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub fn check_quietly(cx: &gpui_kit::App, stale: std::time::Duration) {
    // An update waits: Sparkle won't look again, so the app reads the feed itself.
    look_at_feed();
    #[cfg(target_os = "macos")]
    if let Some(updater) = cx.try_global::<crate::update::Updates>().and_then(|u| u.0.clone()) {
        updater.check_quietly(stale);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (cx, stale);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon(theirs: &str, remote: bool) -> Daemon {
        Daemon { place: "mini".into(), theirs: theirs.into(), older: crate::notify::stale_daemon(theirs, "0.87.0").is_some(), remote, ..Daemon::default() }
    }

    #[test]
    fn one_pill_says_both_updates_and_one_press_does_both() {
        let ready = Update::Ready("0.88.0".into());
        // The app's version waits and the remote daemon is behind it: both, the daemon first.
        assert_eq!(both(&ready, &daemon("0.87.0", true), "0.87.0").map(|p| p.1), Some(Act::UpdateBoth));
        // The daemon on this Mac goes with the app: a restart is enough.
        assert_eq!(both(&ready, &daemon("0.87.0", false), "0.87.0").map(|p| p.1), Some(Act::Relaunch));
        // Nothing waits for the app, the daemon is behind it: the daemon's.
        assert_eq!(both(&Update::Idle, &daemon("0.86.4", true), "0.87.0").map(|p| p.1), Some(Act::UpdateDaemon));
        assert_eq!(both(&Update::Idle, &daemon("0.87.0", true), "0.87.0"), None, "nothing to say");
        // At work on it: said, nothing to press; done: a press arms, the next restarts now.
        let mut d = daemon("0.86.4", true);
        d.update = Some(DaemonUpdate::Running);
        assert_eq!(both(&ready, &d, "0.87.0").map(|p| p.1), Some(Act::None));
        d.update = Some(DaemonUpdate::Done);
        assert_eq!(both(&Update::Idle, &d, "0.87.0").map(|p| p.1), Some(Act::ArmRestart));
        d.armed = true;
        assert_eq!(both(&Update::Idle, &d, "0.87.0").map(|p| p.1), Some(Act::RestartDaemonNow));
        // Done and the app's version waiting: the app's restart, the daemon is already on it.
        assert_eq!(both(&ready, &d, "0.87.0").map(|p| p.1), Some(Act::Relaunch));
        d.update = Some(DaemonUpdate::Failed("no route".into()));
        assert_eq!(both(&Update::Idle, &d, "0.87.0").map(|p| p.1), Some(Act::UpdateDaemon), "failed: a press tries again");
    }

    #[test]
    fn releases_compare_as_numbers() {
        assert!(newer("0.94.0", "0.93.0") && newer("0.93.10", "0.93.9") && newer("1.0.0", "0.99.9") && newer("v0.94.1", "0.94.0"));
        assert!(!newer("0.93.0", "0.93.0") && !newer("0.92.9", "0.93.0") && !newer("0.93", "0.93.0"));
    }

    #[test]
    fn the_feed_names_its_newest_release_and_the_app_where_its_feed_is() {
        let item = |v: &str| format!("<item><title>equipe {v}</title><sparkle:version>{v}</sparkle:version><sparkle:shortVersionString>{v}</sparkle:shortVersionString></item>");
        let xml = format!("<rss><channel>{}{}</channel></rss>", item("0.93.0"), item("0.94.0"));
        assert_eq!(feed_version(&xml).as_deref(), Some("0.94.0"));
        assert_eq!(feed_version("<rss><channel><item><sparkle:version>0.94.0</sparkle:version></item></channel></rss>").as_deref(), Some("0.94.0"), "no short version: the version");
        assert_eq!(feed_version("<rss></rss>"), None);
        let plist = "<dict>\n\t<key>SUFeedURL</key>\n\t<string>https://github.com/equipe-labs/equipe/releases/latest/download/appcast.xml</string>\n</dict>";
        assert_eq!(feed_url(plist).as_deref(), Some("https://github.com/equipe-labs/equipe/releases/latest/download/appcast.xml"));
        assert_eq!(feed_url("<key>SUFeedURL</key><string>http://plain</string>"), None, "only https");
    }

    #[test]
    fn a_newer_release_while_one_waits_becomes_where_the_restart_lands() {
        let ready = Update::Ready("0.93.0".into());
        let newest = next(&ready, Step::Newest("0.94.0".into()));
        assert_eq!(newest.ready(), Some("0.94.0"), "the pill names the newest");
        assert_eq!(next(&ready, Step::Newest("0.93.0".into())), ready, "the same: nothing changes");
        assert_eq!(next(&Update::Idle, Step::Newest("0.94.0".into())), Update::Idle, "nothing waits: Sparkle's own check finds it");
        // The restart: Sparkle's staged one goes in, then on to the newest.
        let chain = chain_for("0.94.0", "0.93.0", 1_000).unwrap();
        assert_eq!(Chain::parse(&chain.text()), Some(chain.clone()));
        assert_eq!(chain_for("0.93.0", "0.93.0", 1_000), None, "the staged one is the newest: no note");
        // The app starting again on 0.93.0: goes on; on 0.94.0: done; too long after: gives up.
        assert_eq!(chain_step("0.93.0", &chain, 1_060), ChainStep::Go);
        assert_eq!(chain_step("0.94.0", &chain, 1_060), ChainStep::Done);
        assert_eq!(chain_step("0.95.0", &chain, 1_060), ChainStep::Done, "past it");
        assert_eq!(chain_step("0.93.0", &chain, 1_000 + CHAIN_FOR + 1), ChainStep::GiveUp, "never in a loop");
        assert_eq!(chain_step("0.93.0", &chain, 10), ChainStep::GiveUp, "a clock gone back");
    }

    #[test]
    fn found_then_downloading_then_ready_and_ready_stays_through_later_checks() {
        let found = next(&Update::Idle, Step::Found("0.49.0".into()));
        assert_eq!(found, Update::Found("0.49.0".into()));
        let downloading = next(&found, Step::Downloading("0.49.0".into()));
        let ready = next(&downloading, Step::Ready("0.49.0".into()));
        assert_eq!(ready.ready(), Some("0.49.0"));
        for later in [Step::NoUpdate, Step::Cancelled, Step::Found("0.49.0".into()), Step::Failed { code: 2001 }, Step::Failed { code: 1001 }] {
            assert_eq!(next(&ready, later.clone()), ready, "{later:?}");
        }
        assert_eq!(next(&ready, Step::Ready("0.50.0".into())).ready(), Some("0.50.0"));
        assert_eq!(next(&ready, Step::Failed { code: 4005 }), Update::Failed { version: Some("0.49.0".into()), why: Why::Other(4005) }, "the install itself failed");
    }

    #[test]
    fn a_failure_says_why_and_no_update_or_a_cancel_is_no_failure() {
        let downloading = Update::Downloading("0.49.0".into());
        assert_eq!(next(&downloading, Step::Failed { code: 2001 }), Update::Failed { version: Some("0.49.0".into()), why: Why::Download });
        assert_eq!(next(&Update::Idle, Step::Failed { code: 1001 }), Update::Idle);
        assert_eq!(next(&Update::Idle, Step::Failed { code: 4007 }), Update::Idle);
        assert_eq!(next(&Update::Idle, Step::Failed { code: 1003 }), Update::Failed { version: None, why: Why::Move });
        assert_eq!(Why::from_code(1005), Some(Why::Move));
        assert_eq!(Why::from_code(4012), Some(Why::Permission));
        assert_eq!(Why::from_code(3001), Some(Why::Signature));
        assert_eq!(Why::from_code(4010), Some(Why::Other(4010)));
        let failed = next(&downloading, Step::Failed { code: 2001 });
        assert_eq!(next(&failed, Step::NoUpdate), Update::Idle, "a later check that works clears it");
        assert_eq!(Why::Other(4010).words(), "업데이트 중 오류가 났어요 (코드 4010) — 로그에 자세한 내용이 있어요");
    }

    #[test]
    fn the_app_never_starts_a_check_against_a_session_or_an_update_under_way() {
        assert!(may_check_quietly(&Update::Idle, false));
        assert!(!may_check_quietly(&Update::Idle, true), "Sparkle is in a session");
        for busy in [Update::Found("1".into()), Update::Downloading("1".into()), Update::Ready("1".into())] {
            assert!(!may_check_quietly(&busy, false), "{busy:?}");
        }
        assert!(may_check_quietly(&Update::Failed { version: None, why: Why::Download }, false), "a failure may be tried again");
    }

    #[test]
    fn the_pill_follows_the_state() {
        assert_eq!(pill(&Update::Idle), None);
        assert_eq!(pill(&Update::Downloading("0.49.0".into())).as_deref(), Some("새 버전 0.49.0 받는 중…"));
        assert_eq!(pill(&Update::Ready("0.49.0".into())).as_deref(), Some("다시 시작해 업데이트 · 0.49.0"));
        assert_eq!(pill(&Update::Failed { version: None, why: Why::Download }).as_deref(), Some("업데이트 받기 실패 · 다시 시도"));
        assert_eq!(pill(&Update::Failed { version: None, why: Why::Move }).as_deref(), Some("Applications 폴더로 옮겨 주세요"));
    }

    #[test]
    fn an_app_run_from_the_disk_image_or_a_download_cant_update_itself() {
        assert_eq!(misplaced_at("/Applications/equipe.app", true), None);
        assert_eq!(misplaced_at("/Volumes/equipe 0.49.0/equipe.app", false), Some(Misplaced::DiskImage));
        assert_eq!(misplaced_at("/private/var/folders/x/T/AppTranslocation/1A2B/d/equipe.app", false), Some(Misplaced::Translocated));
        assert_eq!(misplaced_at("/Applications/equipe.app", false), Some(Misplaced::ReadOnly));
    }

    #[test]
    fn log_lines_carry_a_readable_time() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(stamp(1_792_123_624), "2026-10-16 04:07:04Z");
    }
}
