//! equipe sets the desktop up by itself, so the person only does what macOS
//! keeps for them.
//!
//! What members need to work desktop apps: the accessibility engine (the
//! pinned agent-desktop release, downloaded and checksum-verified), the desktop
//! helper (equipe.app itself when it is installed, else a signed copy of
//! equipe that holds the permission — `install`), and — on a Mac — one switch
//! only the person may flip: Accessibility for "equipe". The daemon does every
//! step that needs no one:
//!
//! 1. the engine is installed, or updated when the pinned version changed, in
//!    the background; a failure is reported once, in a word the screen
//!    translates, with a retry, and is tried again quietly with a growing wait;
//! 2. once a member works (or asks for) the desktop, the helper is installed
//!    and kept current. The daemon never touches the permission database,
//!    never clicks in System Settings and never makes macOS ask anything by
//!    itself: [`DesktopSetup::Grant`] is the card a screen shows, whose button
//!    (a person's click) asks macOS and opens System Settings at the right
//!    pane (`DesktopCheck { guide }`);
//! 3. the grant is watched for lightly, and when it appears the status check
//!    runs by itself and the person is told once ([`DesktopSetup::JustReady`]).
//!
//! 4. a Mac that had the older helper bundle moves to equipe.app by itself:
//!    once the app holds the permission, the bundle is moved aside. Nobody is
//!    told: its stale row in System Settings is harmless. While the app isn't
//!    allowed yet the old bundle keeps working (the app hands over to it), and
//!    the app is started again now and then ([`RECHECK`]) to see whether it is.
//!
//! A card shows only when work is blocked and the person can unblock it with
//! one action (docs/PROMPTS.md has every card and prompt, and when):
//!
//! - Grant: a member was refused for want of the switch ([`Auto::refusal`]),
//!   or the desktop was set up for the first time and never skipped;
//! - Failed: the set-up failed and a member needs it, or it keeps failing
//!   ([`FAILURE_GRACE`]).
//!
//! Never for what resolves itself (installing, restarts, an update, a locked
//! screen). The person puts a card away ([`Auto::dismiss`]): that is kept in
//! `setup.json`, per card and identity ("grant:app", "failed:download"), so a
//! restart or an update doesn't bring it back. A dismissed Grant card returns
//! only after the switch was seen on (the condition cleared) and was taken
//! back while a member needs it; a Failed card, for another failure.
//!
//! macOS's own prompts are never made by this background work: only the
//! person's click on the card does, once, through the prompt budget
//! (`equipe_desktop_control::prompts`).
//!
//! All the machine is behind [`Ops`], so the whole of it is tested with fakes
//! that touch no permission, no screen and no network.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use equipe_desktop_control::engine::pin;
use equipe_desktop_control::helper;
use equipe_protocol::{DesktopFailure, DesktopReadiness, DesktopSetup};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use super::ready;
use crate::keychain::prompts;

/// A set-up failure nobody is blocked on is told only once it has lasted this
/// long (seconds): the quiet retries usually mend it first.
pub const FAILURE_GRACE: u64 = 10 * 60;

/// How often the grant is looked for while the card is up: quickly at first,
/// then lazily.
const POLL_FAST: Duration = Duration::from_secs(4);
const POLL_SLOW: Duration = Duration::from_secs(30);
const FAST_FOR: Duration = Duration::from_secs(10 * 60);
/// Between looks when nothing is going on, and when the desktop isn't wanted yet.
const STEADY: Duration = Duration::from_secs(10 * 60);
const UNWANTED: Duration = Duration::from_secs(2 * 60);
/// Between looks while a failure waits for its next try.
const FAILING: Duration = Duration::from_secs(30);
/// How long after the daemon starts before the engine is fetched: the
/// start-up has its own work.
const START_AFTER: Duration = Duration::from_secs(15);
/// While equipe.app hands over to the old helper bundle (it isn't allowed
/// yet), how often the app is started again to see whether it is.
pub const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);

pub type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Why a step failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub reason: DesktopFailure,
    pub detail: String,
}

impl Failure {
    pub fn new(reason: DesktopFailure, detail: impl Into<String>) -> Self {
        Self { reason, detail: detail.into() }
    }
}

/// Everything the setup does to the machine.
pub trait Ops: Send + Sync + 'static {
    /// The machine can run the desktop helper at all (macOS, Windows).
    fn supported(&self) -> bool;
    /// What is installed of the pinned engine (hashes every file).
    fn engine(&self) -> pin::Installed;
    /// Download, verify and install the pinned engine.
    fn install_engine(&self) -> Fut<Result<(), Failure>>;
    /// Someone is logged in at the Mac's screen.
    fn gui_session(&self) -> bool;
    /// Install the helper and keep it current (macOS), or start it when it
    /// doesn't answer (`answering` false; Windows).
    fn sync_helper(&self, answering: bool) -> Fut<Result<(), Failure>>;
    /// Ask the helper (`_status`, `_request_accessibility`): its answer, or `None`.
    fn helper(&self, tool: &'static str) -> Fut<Option<Value>>;
    /// A member works the desktop (the person turned it on for someone, or set the helper up).
    fn wanted(&self) -> bool;
    /// Start the helper again (no change to its LaunchAgent).
    fn restart_helper(&self) -> bool {
        false
    }
    /// The older helper bundle is still in `~/Applications`.
    fn old_bundle(&self) -> bool {
        false
    }
    /// Move the older helper bundle aside; `true` when it was there.
    fn retire_old(&self) -> bool {
        false
    }
    /// The version of the equipe.app that hosts the helper, if one does.
    fn app_version(&self) -> Option<String> {
        None
    }
}

/// What is kept between daemon runs, in `<home>/desktop/setup.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    /// The person was told the desktop is ready (and it hasn't stopped being since).
    #[serde(default)]
    told_ready: bool,
    /// The switch was seen on at some time.
    #[serde(default)]
    ever_granted: bool,
    /// The person skipped the desktop in the first run: no card until a member needs it.
    #[serde(default)]
    skipped: bool,
    /// Cards the person put away, by what and for whom (`grant:<identity>`,
    /// `failed:<reason>`). A key goes when its condition clears.
    #[serde(default)]
    dismissed: Vec<String>,
    /// A member was refused for want of the switch, and it is still off.
    #[serde(default)]
    needs_grant: bool,
    /// A member was refused for want of the set-up, and it hasn't mended.
    #[serde(default)]
    blocked: bool,
    /// Since when the set-up has been failing (unix seconds).
    #[serde(default)]
    failing_since: Option<u64>,
    /// Before dismissals were kept per card: when the person put the card
    /// away (0: skipped). Read once, then folded into the fields above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dismissed_at: Option<u64>,
    /// The older helper bundle was moved aside (equipe.app holds the permission).
    #[serde(default)]
    retired_old: bool,
}

fn saved_path(home: &Path) -> PathBuf {
    helper::dir(home).join("setup.json")
}

impl Saved {
    fn load(home: &Path) -> Self {
        std::fs::read_to_string(saved_path(home)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self, home: &Path) {
        if std::fs::create_dir_all(helper::dir(home)).is_ok()
            && let Ok(text) = serde_json::to_string(self)
        {
            let _ = std::fs::write(saved_path(home), text);
        }
    }
}

#[derive(Default)]
struct Inner {
    /// The latest look, as every screen was told it.
    last: Option<DesktopReadiness>,
    /// A member used or asked for the desktop since the daemon started.
    wanted: bool,
    retry: bool,
    dismiss: bool,
    /// A member needed the desktop and couldn't have it: for want of the switch.
    needed_grant: bool,
    /// …or for want of the set-up.
    needed_setup: bool,
    /// The person was told the desktop was ready before this daemon started.
    granted_before: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    wake: Notify,
}

impl Shared {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The daemon's handle on its desktop setup.
#[derive(Clone)]
pub struct Auto {
    shared: Arc<Shared>,
}

/// Where news of the setup goes: the daemon's screens.
pub type Sink = Arc<dyn Fn(DesktopReadiness) + Send + Sync>;

impl Auto {
    /// Start setting the desktop up in the background. `sink` hears each change.
    pub fn start(home: PathBuf, ops: Arc<dyn Ops>, sink: Sink) -> Self {
        let (auto, worker) = Self::new(home, ops, sink, Box::new(unix_now));
        tokio::spawn(async move {
            tokio::time::sleep(START_AFTER).await;
            worker.run().await;
        });
        auto
    }

    /// Set the desktop up on a thread of its own, at once: the first run's
    /// screens, which have no daemon to do it. It ends when every handle is dropped.
    pub fn start_on_thread(home: PathBuf, ops: Arc<dyn Ops>, sink: Sink) -> Self {
        let (auto, worker) = Self::new(home, ops, sink, Box::new(unix_now));
        let spawned = std::thread::Builder::new().name("desktop-setup".into()).spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                runtime.block_on(worker.run());
            }
        });
        drop(spawned);
        auto
    }

    fn new(home: PathBuf, ops: Arc<dyn Ops>, sink: Sink, clock: Box<dyn Fn() -> u64 + Send + Sync>) -> (Self, Worker) {
        let saved = Saved::load(&home);
        let shared = Arc::new(Shared { inner: Mutex::new(Inner { granted_before: saved.told_ready, ..Inner::default() }), wake: Notify::new() });
        let worker = Worker {
            saved,
            home,
            ops,
            sink,
            clock,
            shared: shared.clone(),
            engine_verified: false,
            engine_failure: None,
            engine_attempts: 0,
            engine_next: None,
            helper_synced: false,
            helper_failure: None,
            helper_attempts: 0,
            helper_next: None,
            sticky_wanted: false,
            waiting_since: None,
            last_recheck: None,
            restarted_for: None,
            card: None,
        };
        (Self { shared }, worker)
    }

    /// The latest look at the desktop, if there has been one.
    pub fn latest(&self) -> Option<DesktopReadiness> {
        self.shared.inner().last.clone()
    }

    /// Whether the person's Accessibility switch is on: as the latest look
    /// saw it, or — before the first look — as the last daemon left it.
    /// Every member works the desktop by default once it is.
    pub fn granted(&self) -> bool {
        let inner = self.shared.inner();
        match &inner.last {
            Some(last) if last.helper => last.accessibility == Some(true),
            _ => inner.granted_before,
        }
    }

    /// Put the setup's state into a readiness a screen asked for.
    pub fn decorate(&self, ready: &mut DesktopReadiness) {
        if let Some(last) = self.latest() {
            ready.setup = last.setup;
        }
    }

    /// Set up again now, after a failure.
    pub fn retry(&self) {
        self.shared.inner().retry = true;
        self.shared.wake.notify_one();
    }

    /// The person put the card away.
    pub fn dismiss(&self) {
        self.shared.inner().dismiss = true;
        self.shared.wake.notify_one();
    }

    /// A member works the desktop: set it up now, and offer the person the
    /// switch if it is still off.
    pub fn wanted(&self) {
        let mut inner = self.shared.inner();
        if !inner.wanted {
            inner.wanted = true;
            drop(inner);
            self.shared.wake.notify_one();
        }
    }

    /// Whether the desktop tool `tool` of a member can't work now: the short,
    /// actionable thing to tell the member, and a note that the person is
    /// needed (their dismissed card comes back). `None`: nothing known to be in
    /// the way, so the call goes on (the helper says the rest itself).
    pub fn refusal(&self, tool: &str) -> Option<String> {
        let last = self.latest()?;
        if !last.supported {
            return None;
        }
        let tool = tool.rsplit("__").next().unwrap_or(tool);
        let engine_tool = equipe_desktop_control::engine::handles(tool);
        // A picture needs Screen Recording, which only the person's own action
        // asks for: until it is on, members read the accessibility tree, and
        // nothing here makes macOS ask (no card either: it is not the switch
        // the setup is about).
        if tool == "screenshot" && last.helper && last.screen == Some(false) {
            return Some(equipe_desktop_control::controller::NO_PICTURE.to_owned());
        }
        // Pictures of the screen need no Accessibility.
        let needs_grant = !matches!(tool, "screenshot" | "list_windows");
        let mut grant = false;
        let message = if engine_tool && !last.engine {
            Some(match last.setup {
                DesktopSetup::Failed { .. } => "equipe couldn't install its accessibility engine (the person was told and can retry). Don't retry; use the screenshot tools meanwhile, or tell the person.".to_owned(),
                _ => "equipe is still setting up the accessibility engine in the background (nothing for the person to do). Call again in a minute, or use the screenshot tools meanwhile.".to_owned(),
            })
        } else if engine_tool && !last.helper {
            Some("equipe is starting the desktop helper (nothing for the person to do). Call again in a minute.".to_owned())
        } else if needs_grant && last.helper && last.accessibility == Some(false) {
            grant = true;
            let name = if last.helper_name.is_empty() { equipe_desktop_control::install::DISPLAY_NAME } else { last.helper_name.as_str() };
            Some(format!(
                "Desktop apps need one switch only the person can flip: Accessibility for \"{name}\" (System Settings › Privacy & Security › Accessibility). equipe has told them (a card in the app, unless they put it away). It works by itself the moment the switch is on, so don't retry now: tell the person if your work waits on it, or do something else first."
            ))
        } else {
            None
        };
        if message.is_some() {
            let mut inner = self.shared.inner();
            inner.wanted = true;
            if grant {
                inner.needed_grant = true;
            } else {
                inner.needed_setup = true;
            }
            drop(inner);
            self.shared.wake.notify_one();
        }
        message
    }
}

/// The person skipped the desktop in the first run: the card stays away until
/// a member first needs the desktop, which offers it once.
pub fn skip(home: &Path) {
    let mut saved = Saved::load(home);
    saved.skipped = true;
    saved.save(home);
}

/// Run `f` holding the setup's lock, so the app's first run and the daemon
/// never install the engine or the helper at the same moment.
fn locked<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    use fs2::FileExt as _;
    let file = std::fs::create_dir_all(helper::dir(home))
        .ok()
        .and_then(|()| std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(helper::dir(home).join("setup.lock")).ok());
    if let Some(file) = &file {
        let _ = file.lock_exclusive();
    }
    let out = f();
    drop(file);
    out
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The wait before the engine (or helper) is tried again after `attempts` failures.
fn backoff(attempts: u32) -> Duration {
    Duration::from_secs(match attempts {
        0 | 1 => 2 * 60,
        2 => 10 * 60,
        3 => 60 * 60,
        _ => 6 * 60 * 60,
    })
}

struct Worker {
    home: PathBuf,
    saved: Saved,
    ops: Arc<dyn Ops>,
    sink: Sink,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    shared: Arc<Shared>,
    engine_verified: bool,
    engine_failure: Option<Failure>,
    engine_attempts: u32,
    engine_next: Option<Instant>,
    helper_synced: bool,
    helper_failure: Option<Failure>,
    helper_attempts: u32,
    helper_next: Option<Instant>,
    sticky_wanted: bool,
    waiting_since: Option<Instant>,
    /// When equipe.app, handing over to the old bundle, was last started again.
    last_recheck: Option<Instant>,
    /// The app version the helper was last restarted for (after an update).
    restarted_for: Option<String>,
    /// The card on screen now, as a dismissal key.
    card: Option<String>,
}

impl Worker {
    async fn run(mut self) {
        loop {
            // Nobody holds a handle any more (a first run's screen closed): done.
            if Arc::strong_count(&self.shared) == 1 {
                return;
            }
            let delay = self.step().await;
            // A member needing the desktop, a retry or a dismissal cut the wait short.
            let _ = tokio::time::timeout(delay, self.shared.wake.notified()).await;
        }
    }

    async fn blocking(&self, f: impl FnOnce(&dyn Ops) -> bool + Send + 'static) -> bool {
        let ops = self.ops.clone();
        tokio::task::spawn_blocking(move || f(ops.as_ref())).await.unwrap_or(false)
    }

    /// Change what is kept, writing it only when it did change.
    fn edit(&mut self, f: impl FnOnce(&mut Saved)) {
        let before = self.saved.clone();
        f(&mut self.saved);
        if self.saved != before {
            self.saved.save(&self.home);
        }
    }

    /// One look, and whatever it calls for. How long to wait for the next.
    async fn step(&mut self) -> Duration {
        let now = (self.clock)();
        if !self.ops.supported() {
            self.publish(DesktopReadiness::default(), DesktopSetup::Idle);
            return STEADY;
        }
        let (retry, dismiss, needed_grant, needed_setup, wanted_flag) = {
            let mut inner = self.shared.inner();
            (
                std::mem::take(&mut inner.retry),
                std::mem::take(&mut inner.dismiss),
                std::mem::take(&mut inner.needed_grant),
                std::mem::take(&mut inner.needed_setup),
                inner.wanted,
            )
        };
        if wanted_flag {
            self.sticky_wanted = true;
        }
        // What an older daemon kept as one time.
        if self.saved.dismissed_at == Some(0) {
            self.edit(|saved| {
                saved.dismissed_at = None;
                saved.skipped = true;
            });
        }
        if self.saved.told_ready && !self.saved.ever_granted {
            self.edit(|saved| saved.ever_granted = true);
        }
        if retry {
            self.engine_next = None;
            self.helper_next = None;
            self.engine_attempts = 0;
            self.helper_attempts = 0;
            self.engine_verified = false;
            // The person asked again: a failure put away is told afresh.
            self.edit(|saved| saved.dismissed.retain(|key| !key.starts_with("failed:")));
        }
        if needed_grant || needed_setup {
            self.edit(|saved| {
                saved.skipped = false;
                saved.needs_grant |= needed_grant;
                saved.blocked |= needed_setup;
            });
        }
        if dismiss && let Some(card) = self.card.take() {
            self.edit(|saved| {
                if !saved.dismissed.contains(&card) {
                    saved.dismissed.push(card);
                }
            });
        }
        let wanted = self.sticky_wanted || self.blocking(|ops| ops.wanted()).await;

        let mut ready =
            DesktopReadiness { supported: true, engine_version: pin::VERSION.to_owned(), work_locked: super::work_locked(&self.home), ..Default::default() };

        // 1. The engine: there and verified, or fetched.
        self.engine_step(wanted, &mut ready).await;
        ready.engine = self.engine_verified;

        // 2. The helper, once someone wants the desktop.
        let mut status = None;
        let mut idle = if wanted { STEADY } else { UNWANTED };
        if wanted {
            status = self.helper_step(&mut ready).await;
        }

        // 3. The one switch.
        let mut setup = DesktopSetup::Idle;
        if let Some(status) = &status {
            ready::apply(&mut ready, status);
            self.host_step(status, &ready, retry).await;
        }
        let mut card = None;
        if let Some(failure) = self.helper_failure.clone().or_else(|| self.engine_failure.clone()) {
            if self.saved.failing_since.is_none() {
                self.edit(|saved| saved.failing_since = Some(now));
            }
            let since = self.saved.failing_since.unwrap_or(now);
            idle = FAILING;
            // Told when a member is blocked on it, or once it has lasted.
            if self.saved.blocked || now.saturating_sub(since) >= FAILURE_GRACE {
                setup = DesktopSetup::Failed { reason: failure.reason, detail: failure.detail.clone() };
                card = Some(format!("failed:{:?}", failure.reason));
            }
        } else {
            let healthy = self.engine_verified && status.is_some();
            self.edit(|saved| {
                saved.failing_since = None;
                if healthy {
                    saved.blocked = false;
                }
            });
            if wanted && status.is_some() && (self.engine_verified || !pin_known()) {
                let identity = prompt_identity(status.as_ref());
                if ready.accessibility == Some(true) {
                    prompts::observe(&self.home, prompts::Kind::Accessibility, &identity, true, now);
                    self.waiting_since = None;
                    if ready.ready() && !self.saved.told_ready {
                        setup = DesktopSetup::JustReady;
                    }
                    let told = ready.ready() || self.saved.told_ready;
                    self.edit(|saved| {
                        saved.told_ready = told;
                        saved.ever_granted = true;
                        saved.needs_grant = false;
                        saved.blocked = false;
                        // The condition cleared: a card put away for it is spent.
                        saved.dismissed.retain(|key| !key.starts_with("grant:"));
                    });
                } else {
                    // The switch is off. A watch for it, light; a card only if
                    // work is blocked on it (or this is the first set-up), and
                    // not on a locked screen, which the person can't reach.
                    self.edit(|saved| saved.told_ready = false);
                    let since = *self.waiting_since.get_or_insert_with(Instant::now);
                    idle = if since.elapsed() < FAST_FOR { POLL_FAST } else { POLL_SLOW };
                    let first_time = !self.saved.ever_granted && !self.saved.skipped;
                    if !ready.locked && (self.saved.needs_grant || first_time) {
                        setup = DesktopSetup::Grant;
                        card = Some(format!("grant:{identity}"));
                    }
                }
            }
        }
        if !wanted {
            setup = DesktopSetup::Idle;
            card = None;
        }
        // A card put away by an older daemon (one time, for whatever card it was).
        if let Some(key) = card.clone()
            && self.saved.dismissed_at.is_some()
        {
            self.edit(|saved| {
                saved.dismissed_at = None;
                saved.dismissed.push(key);
            });
        }
        // A card the person put away stays away.
        if card.as_ref().is_some_and(|key| self.saved.dismissed.contains(key)) {
            setup = DesktopSetup::Idle;
            card = None;
        }
        self.card = card;
        self.publish(ready, setup);
        idle
    }

    /// Where the helper runs, kept right: the old bundle moved aside once
    /// equipe.app holds the permission; the app's helper restarted after the
    /// app was updated; and, while the app hands over to the old bundle, the
    /// app started again now and then to see whether it is allowed by now.
    async fn host_step(&mut self, status: &Value, ready: &DesktopReadiness, retry: bool) {
        let role = status["role"].as_str().unwrap_or_default();
        if role == "app" && ready.accessibility == Some(true) && !self.saved.retired_old && self.blocking(|ops| ops.old_bundle()).await {
            let moved = self.blocking(|ops| ops.retire_old()).await;
            crate::daemon_log!("equipe: desktop: equipe.app holds the permission now; the old helper bundle {}", if moved { "was moved aside" } else { "couldn't be moved aside" });
            self.edit(|saved| saved.retired_old = true);
        }
        if role == "app"
            && let Some(version) = self.ops.app_version()
            && status["version"].as_str().is_some_and(|v| v != version)
            && self.restarted_for.as_deref() != Some(version.as_str())
        {
            self.restarted_for = Some(version);
            let _ = self.blocking(|ops| ops.restart_helper()).await;
        }
        if status["handed_over"].as_bool() == Some(true)
            && self.ops.app_version().is_some()
            && (retry || self.last_recheck.is_none_or(|at| at.elapsed() >= RECHECK))
        {
            self.last_recheck = Some(Instant::now());
            let _ = self.blocking(|ops| ops.restart_helper()).await;
        }
    }

    async fn engine_step(&mut self, wanted: bool, ready: &mut DesktopReadiness) {
        if self.engine_verified {
            return;
        }
        match self.blocking_engine().await {
            pin::Installed::Ready(_) => {
                self.engine_verified = true;
                self.engine_failure = None;
                self.engine_attempts = 0;
            }
            pin::Installed::Unsupported => {}
            pin::Installed::Missing | pin::Installed::Tampered(_) => {
                if self.engine_next.is_some_and(|at| Instant::now() < at) {
                    return;
                }
                // Told while it works, when someone wants the desktop.
                self.publish(ready.clone(), if wanted { DesktopSetup::Installing } else { DesktopSetup::Idle });
                match self.ops.install_engine().await {
                    Ok(()) => {
                        self.engine_verified = matches!(self.blocking_engine().await, pin::Installed::Ready(_));
                        self.engine_failure = None;
                        self.engine_attempts = 0;
                        self.engine_next = None;
                    }
                    Err(failure) => {
                        self.engine_attempts += 1;
                        self.engine_next = Some(Instant::now() + backoff(self.engine_attempts));
                        self.engine_failure = Some(failure);
                    }
                }
            }
        }
    }

    async fn blocking_engine(&self) -> pin::Installed {
        let ops = self.ops.clone();
        tokio::task::spawn_blocking(move || ops.engine()).await.unwrap_or(pin::Installed::Unsupported)
    }

    /// The helper installed, current and answering: its status, if it does.
    async fn helper_step(&mut self, ready: &mut DesktopReadiness) -> Option<Value> {
        if !self.blocking(|ops| ops.gui_session()).await {
            // Nobody at the Mac's screen: there is no one to set it up for yet.
            ready.helper = false;
            return None;
        }
        let mut status = self.ops.helper(helper::STATUS).await;
        let due = self.helper_next.is_none_or(|at| Instant::now() >= at);
        if (status.is_none() || !self.helper_synced) && due {
            self.publish(ready.clone(), DesktopSetup::Installing);
            match self.ops.sync_helper(status.is_some()).await {
                Ok(()) => {
                    self.helper_synced = true;
                    self.helper_failure = None;
                    self.helper_attempts = 0;
                    self.helper_next = None;
                    // The helper takes a moment to start listening.
                    for _ in 0..40 {
                        status = self.ops.helper(helper::STATUS).await;
                        if status.is_some() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
                Err(failure) => {
                    self.helper_attempts += 1;
                    self.helper_next = Some(Instant::now() + backoff(self.helper_attempts));
                    self.helper_failure = Some(failure);
                }
            }
        }
        if status.is_some() {
            self.helper_failure = None;
        }
        status
    }

    fn publish(&self, mut ready: DesktopReadiness, setup: DesktopSetup) {
        ready.setup = setup;
        {
            let mut inner = self.shared.inner();
            if inner.last.as_ref() == Some(&ready) {
                return;
            }
            inner.last = Some(ready.clone());
        }
        (self.sink)(ready);
    }
}

/// Who macOS's Accessibility prompt would be for, as the prompt budget keeps it.
fn prompt_identity(status: Option<&Value>) -> String {
    match status.and_then(|s| s["role"].as_str()) {
        Some("app") => equipe_desktop_control::APP_PROMPT.to_owned(),
        Some(role) => format!("helper-{role}"),
        None => "helper".to_owned(),
    }
}

/// Whether this machine has an engine pinned (else the grant stage needn't wait for one).
fn pin_known() -> bool {
    pin::this_machine().is_some()
}

/// The real machine.
pub struct ThisMachine {
    pub home: PathBuf,
}

/// Download the pinned engine and install it, checksum-verified.
pub async fn install_engine(home: &Path) -> Result<(), Failure> {
    let Some(archive) = pin::this_machine() else { return Ok(()) };
    let fetched = async {
        let client = reqwest::Client::builder()
            .user_agent(concat!("equipe/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client.get(archive.url()).send().await.map_err(|e| e.to_string())?;
        let response = response.error_for_status().map_err(|e| e.to_string())?;
        response.bytes().await.map_err(|e| e.to_string())
    };
    let bytes = fetched.await.map_err(|why| Failure::new(DesktopFailure::Download, why))?;
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || {
        locked(&home, || {
            // Another process (the app's first run, the daemon) may have been first.
            if matches!(pin::check(&home, Some(archive)), pin::Installed::Ready(_)) {
                return Ok(pin::executable(&home, archive));
            }
            pin::install_from_archive(&home, archive, &bytes)
        })
    })
    .await
        .map_err(|e| Failure::new(DesktopFailure::Other, e.to_string()))?
        .map(|_| ())
        .map_err(|why| Failure::new(if why.contains("doesn't match") { DesktopFailure::Checksum } else { DesktopFailure::Other }, why))
}

impl Ops for ThisMachine {
    fn supported(&self) -> bool {
        cfg!(any(target_os = "macos", windows))
    }

    fn engine(&self) -> pin::Installed {
        pin::check(&self.home, pin::this_machine())
    }

    fn install_engine(&self) -> Fut<Result<(), Failure>> {
        let home = self.home.clone();
        Box::pin(async move { install_engine(&home).await })
    }

    fn gui_session(&self) -> bool {
        if cfg!(target_os = "macos") { equipe_desktop_control::install::gui_session() } else { true }
    }

    fn sync_helper(&self, answering: bool) -> Fut<Result<(), Failure>> {
        let home = self.home.clone();
        Box::pin(async move {
            #[cfg(target_os = "macos")]
            {
                let _ = answering;
                tokio::task::spawn_blocking(move || {
                    use equipe_desktop_control::install::{self, Signer};
                    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                    locked(&home, || install::install(&home, &exe, env!("CARGO_PKG_VERSION"), &Signer::chosen(&home))).map(|_| ())
                })
                .await
                .map_err(|e| Failure::new(DesktopFailure::Other, e.to_string()))?
                .map_err(|why| Failure::new(DesktopFailure::Helper, why))
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                if answering {
                    return Ok(());
                }
                // No permission to ask for on Windows: the helper is just started, detached.
                let exe = std::env::current_exe().map_err(|e| Failure::new(DesktopFailure::Helper, e.to_string()))?;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                std::process::Command::new(exe)
                    .args(["desktop", "serve", "--home"])
                    .arg(&home)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| Failure::new(DesktopFailure::Helper, e.to_string()))
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                let _ = (home, answering);
                Ok(())
            }
        })
    }

    fn helper(&self, tool: &'static str) -> Fut<Option<Value>> {
        let home = self.home.clone();
        Box::pin(async move {
            let result = helper::call(&home, "auto", tool, &Value::Null).await;
            if result.is_error { None } else { serde_json::from_str(&result.text).ok() }
        })
    }

    fn wanted(&self) -> bool {
        // The person set the helper up once (setup, the first run, or a member
        // they turned on before): it is kept current and its switch watched.
        if cfg!(target_os = "macos") && equipe_desktop_control::install::plist_path().is_file() {
            return true;
        }
        let Ok(dirs) = std::fs::read_dir(self.home.join("squads")) else { return false };
        dirs.flatten()
            .filter(|d| d.path().join("squad.db").exists())
            .filter_map(|d| crate::journal::Journal::for_squad(&self.home, &d.file_name().to_string_lossy()).ok())
            .any(|journal| !journal.desktop_members().is_empty())
    }

    fn restart_helper(&self) -> bool {
        cfg!(target_os = "macos") && equipe_desktop_control::install::restart().is_ok()
    }

    fn old_bundle(&self) -> bool {
        cfg!(target_os = "macos") && equipe_desktop_control::install::bundle_executable().is_some()
    }

    fn retire_old(&self) -> bool {
        cfg!(target_os = "macos") && equipe_desktop_control::install::retire_bundle(&self.home)
    }

    fn app_version(&self) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let app = equipe_desktop_control::install::app_host()?;
        let text = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
        equipe_desktop_control::install::plist_value(&text, "CFBundleShortVersionString")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    /// A machine that records what was asked of it and touches nothing.
    struct Fake {
        engine_ready: AtomicBool,
        engine_ok: AtomicBool,
        helper_up: AtomicBool,
        granted: AtomicBool,
        wanted: AtomicBool,
        engine_installs: AtomicUsize,
        helper_syncs: AtomicUsize,
        prompts: AtomicUsize,
        screen_requests: AtomicUsize,
        engine_ready_after_install: AtomicBool,
        locked: AtomicBool,
        /// Who holds the permission, as the helper's status says it ("" = not said).
        role: Mutex<&'static str>,
        old_bundle: AtomicBool,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                engine_ready: AtomicBool::new(false),
                engine_ok: AtomicBool::new(true),
                helper_up: AtomicBool::new(false),
                granted: AtomicBool::new(false),
                wanted: AtomicBool::new(true),
                engine_installs: AtomicUsize::new(0),
                helper_syncs: AtomicUsize::new(0),
                prompts: AtomicUsize::new(0),
                screen_requests: AtomicUsize::new(0),
                engine_ready_after_install: AtomicBool::new(true),
                locked: AtomicBool::new(false),
                role: Mutex::new(""),
                old_bundle: AtomicBool::new(false),
            })
        }
    }

    impl Ops for Fake {
        fn supported(&self) -> bool {
            true
        }
        fn engine(&self) -> pin::Installed {
            if self.engine_ready.load(Ordering::SeqCst) { pin::Installed::Ready(PathBuf::from("/fake/agent-desktop")) } else { pin::Installed::Missing }
        }
        fn install_engine(&self) -> Fut<Result<(), Failure>> {
            self.engine_installs.fetch_add(1, Ordering::SeqCst);
            let ok = self.engine_ok.load(Ordering::SeqCst);
            if ok {
                self.engine_ready.store(self.engine_ready_after_install.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            Box::pin(async move { if ok { Ok(()) } else { Err(Failure::new(DesktopFailure::Download, "offline")) } })
        }
        fn gui_session(&self) -> bool {
            true
        }
        fn sync_helper(&self, _answering: bool) -> Fut<Result<(), Failure>> {
            self.helper_syncs.fetch_add(1, Ordering::SeqCst);
            self.helper_up.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
        fn helper(&self, tool: &'static str) -> Fut<Option<Value>> {
            if tool == helper::REQUEST_ACCESSIBILITY {
                self.prompts.fetch_add(1, Ordering::SeqCst);
            } else if tool != helper::STATUS {
                // Anything else (`REQUEST`) is the person's "Allow…": the
                // background setup must never send it.
                self.screen_requests.fetch_add(1, Ordering::SeqCst);
            }
            let up = self.helper_up.load(Ordering::SeqCst);
            let granted = self.granted.load(Ordering::SeqCst);
            let locked = self.locked.load(Ordering::SeqCst);
            let role = *self.role.lock().unwrap();
            Box::pin(async move {
                up.then(|| json!({ "role": role, "accessibility": granted, "post_events": granted, "screen": false, "session": { "locked": locked }, "engine": { "accessibility": if granted { "granted" } else { "denied" } } }))
            })
        }
        fn old_bundle(&self) -> bool {
            self.old_bundle.load(Ordering::SeqCst)
        }
        fn retire_old(&self) -> bool {
            self.old_bundle.store(false, Ordering::SeqCst);
            true
        }
        fn wanted(&self) -> bool {
            self.wanted.load(Ordering::SeqCst)
        }
    }

    struct Rig {
        fake: Arc<Fake>,
        auto: Auto,
        worker: Worker,
        heard: Arc<Mutex<Vec<DesktopReadiness>>>,
        home: PathBuf,
        // The system prompt's turn (`prompts::begin`) is one per process:
        // rigs take turns so parallel tests don't take it from each other.
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    fn rig() -> Rig {
        static SERIAL: Mutex<()> = Mutex::new(());
        let serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        static N: AtomicUsize = AtomicUsize::new(0);
        let home = std::env::temp_dir().join(format!("eq-auto-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&home);
        let fake = Fake::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let heard = heard.clone();
            Arc::new(move |ready| heard.lock().unwrap().push(ready))
        };
        let clock = Arc::new(AtomicUsize::new(1_000_000));
        let (auto, worker) = Auto::new(home.clone(), fake.clone(), sink, Box::new(move || clock.load(Ordering::SeqCst) as u64));
        Rig { fake, auto, worker, heard, home, _serial: serial }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    impl Rig {
        fn setup(&self) -> DesktopSetup {
            self.auto.latest().unwrap().setup
        }
        fn told(&self) -> Vec<DesktopSetup> {
            self.heard.lock().unwrap().iter().map(|r| r.setup.clone()).collect()
        }
    }

    #[tokio::test]
    async fn the_engine_installs_itself_and_a_second_look_leaves_it_alone() {
        let mut rig = rig();
        rig.fake.wanted.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        assert_eq!(rig.fake.engine_installs.load(Ordering::SeqCst), 1);
        assert!(rig.auto.latest().unwrap().engine);
        rig.worker.step().await;
        assert_eq!(rig.fake.engine_installs.load(Ordering::SeqCst), 1, "verified once, not fetched again");
        // Nobody works the desktop: no helper installed, no prompt, no card.
        assert_eq!(rig.fake.helper_syncs.load(Ordering::SeqCst), 0);
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        assert_eq!(rig.setup(), DesktopSetup::Idle);
    }

    #[tokio::test]
    async fn a_failure_is_told_once_with_a_retry_and_never_blocks() {
        let mut rig = rig();
        rig.fake.engine_ok.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        // Nobody is blocked on it and it may mend by itself: no card yet.
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        rig.worker.clock = Box::new(|| 1_000_000 + FAILURE_GRACE);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Failed { reason: DesktopFailure::Download, detail: "offline".into() });
        // Looking again before the wait is over doesn't try again, nor say it again.
        let told = rig.told().len();
        rig.worker.step().await;
        assert_eq!(rig.fake.engine_installs.load(Ordering::SeqCst), 1);
        assert_eq!(rig.told().len(), told);
        // Retry: tried at once; this time it works.
        rig.fake.engine_ok.store(true, Ordering::SeqCst);
        rig.auto.retry();
        rig.worker.step().await;
        assert_eq!(rig.fake.engine_installs.load(Ordering::SeqCst), 2);
        assert!(rig.auto.latest().unwrap().engine);
        assert!(!matches!(rig.setup(), DesktopSetup::Failed { .. }));
    }

    #[tokio::test]
    async fn the_switch_is_watched_without_a_system_prompt_and_the_person_told_once() {
        let mut rig = rig();
        rig.worker.step().await;
        // Engine and helper in place, the grant missing, first set-up: a card, and no
        // macOS prompt from the background (the card's button asks, on the person's click).
        assert_eq!(rig.fake.helper_syncs.load(Ordering::SeqCst), 1);
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        // Watching doesn't prompt either.
        rig.worker.step().await;
        rig.worker.step().await;
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        // The person flips the switch: the next look sees it and says so.
        rig.fake.granted.store(true, Ordering::SeqCst);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::JustReady);
        assert!(rig.auto.latest().unwrap().ready());
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        assert_eq!(rig.told().iter().filter(|s| **s == DesktopSetup::JustReady).count(), 1);
        // Nor does a new daemon say it again.
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let heard = heard.clone();
            Arc::new(move |ready: DesktopReadiness| heard.lock().unwrap().push(ready.setup))
        };
        let (_, mut again) = Auto::new(rig.home.clone(), rig.fake.clone(), sink, Box::new(|| 1_000_000));
        again.step().await;
        assert!(!heard.lock().unwrap().contains(&DesktopSetup::JustReady));
    }

    /// A second daemon run on the same home, as after a restart or an update.
    fn restarted(rig: &Rig) -> (Auto, Worker, Arc<Mutex<Vec<DesktopSetup>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let heard = heard.clone();
            Arc::new(move |ready: DesktopReadiness| heard.lock().unwrap().push(ready.setup))
        };
        let (auto, worker) = Auto::new(rig.home.clone(), rig.fake.clone(), sink, Box::new(|| 1_000_000));
        (auto, worker, heard)
    }

    #[tokio::test]
    async fn a_dismissed_card_stays_away_even_when_a_member_needs_the_desktop_again() {
        let mut rig = rig();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        rig.auto.dismiss();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle, "no nagging");
        // A member needs it: told how, but the person's card is not put up again.
        let message = rig.auto.refusal("mcp__desktop__snapshot").expect("a member is told");
        assert!(message.contains("Accessibility for \"") && message.contains("System Settings"), "{message}");
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        // Not even much later.
        rig.worker.clock = Box::new(|| 1_000_000 + 30 * 24 * 60 * 60);
        rig.auto.refusal("mcp__desktop__act");
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0, "background work never makes macOS ask");
    }

    #[tokio::test]
    async fn a_dismissal_survives_a_daemon_restart_and_an_update() {
        let mut rig = rig();
        rig.worker.step().await;
        rig.auto.dismiss();
        rig.worker.step().await;
        let (auto, mut again, heard) = restarted(&rig);
        again.step().await;
        auto.refusal("mcp__desktop__snapshot");
        again.step().await;
        again.step().await;
        assert!(!heard.lock().unwrap().contains(&DesktopSetup::Grant), "{:?}", heard.lock().unwrap());
    }

    #[tokio::test]
    async fn a_card_for_another_identity_is_another_question() {
        let mut rig = rig();
        *rig.fake.role.lock().unwrap() = "bundle";
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        rig.auto.dismiss();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        // The app took over and isn't allowed either: a different row, a different card.
        *rig.fake.role.lock().unwrap() = "app";
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
    }

    #[tokio::test]
    async fn an_update_that_takes_the_grant_back_asks_once_and_only_when_work_needs_it() {
        let mut rig = rig();
        rig.fake.granted.store(true, Ordering::SeqCst);
        rig.worker.step().await;
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        // The update changed the code signature: macOS forgot the grant. Nothing needs it: no card.
        rig.fake.granted.store(false, Ordering::SeqCst);
        for _ in 0..3 {
            rig.worker.step().await;
        }
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        // Work reaches for the desktop: one card.
        rig.auto.refusal("mcp__desktop__snapshot");
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        rig.auto.dismiss();
        rig.worker.step().await;
        // And once put away, a restart, another need or the next update don't bring it back.
        let (auto, mut again, heard) = restarted(&rig);
        again.step().await;
        auto.refusal("mcp__desktop__act");
        again.step().await;
        assert!(!heard.lock().unwrap().contains(&DesktopSetup::Grant));
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_card_goes_the_moment_the_grant_arrives_and_a_no_changes_nothing() {
        let mut rig = rig();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        // The person says no in macOS's prompt: still off, still the one card, nothing asked again.
        for _ in 0..3 {
            rig.worker.step().await;
        }
        assert_eq!(rig.setup(), DesktopSetup::Grant);
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        // The switch goes on while the card is up: the next look takes it away.
        rig.fake.granted.store(true, Ordering::SeqCst);
        rig.worker.step().await;
        assert_ne!(rig.setup(), DesktopSetup::Grant);
        rig.worker.step().await;
        assert_eq!(rig.told().last(), Some(&DesktopSetup::Idle));
    }

    #[tokio::test]
    async fn a_card_put_away_on_one_screen_goes_from_every_screen() {
        let mut rig = rig();
        rig.worker.step().await;
        assert_eq!(rig.told().last(), Some(&DesktopSetup::Grant));
        // Both screens hear the daemon (one sink); either may press "Not now".
        rig.auto.dismiss();
        rig.auto.dismiss();
        rig.worker.step().await;
        assert_eq!(rig.told().last(), Some(&DesktopSetup::Idle));
        assert_eq!(rig.setup(), DesktopSetup::Idle, "what a screen asking afterwards is told");
    }

    #[tokio::test]
    async fn a_locked_screen_is_no_reason_for_a_card() {
        let mut rig = rig();
        rig.fake.locked.store(true, Ordering::SeqCst);
        rig.auto.wanted();
        rig.worker.step().await;
        rig.auto.refusal("mcp__desktop__snapshot");
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        // Unlocked, the person can reach it: the card, once.
        rig.fake.locked.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
    }

    #[tokio::test]
    async fn taking_over_from_the_old_helper_tells_nobody_anything() {
        let mut rig = rig();
        rig.fake.granted.store(true, Ordering::SeqCst);
        rig.fake.old_bundle.store(true, Ordering::SeqCst);
        *rig.fake.role.lock().unwrap() = "app";
        for _ in 0..3 {
            rig.worker.step().await;
        }
        assert!(!rig.fake.old_bundle.load(Ordering::SeqCst), "moved aside");
        assert!(rig.heard.lock().unwrap().iter().all(|r| !r.old_helper), "its stale row is harmless: no note");
        assert_eq!(rig.setup(), DesktopSetup::Idle);
    }

    #[tokio::test]
    async fn a_failure_waits_for_a_blocked_member_or_for_time_and_a_put_away_one_stays_away() {
        let mut rig = rig();
        rig.fake.engine_ok.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle, "installing and retrying quietly");
        // A member blocked on it: told at once.
        rig.auto.refusal("mcp__desktop__snapshot");
        rig.worker.step().await;
        assert!(matches!(rig.setup(), DesktopSetup::Failed { .. }));
        rig.auto.dismiss();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        let (_, mut again, heard) = restarted(&rig);
        again.step().await;
        assert!(!heard.lock().unwrap().iter().any(|s| matches!(s, DesktopSetup::Failed { .. })), "not after a restart");
    }

    #[tokio::test]
    async fn a_dismissal_an_older_daemon_kept_is_honoured() {
        let mut rig = rig();
        std::fs::create_dir_all(helper::dir(&rig.home)).unwrap();
        std::fs::write(saved_path(&rig.home), r#"{"prompted":true,"told_ready":false,"dismissed_at":5}"#).unwrap();
        rig.worker.saved = Saved::load(&rig.home);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle, "the card they put away isn't shown again by the update");
    }

    #[tokio::test]
    async fn members_are_told_short_and_true_while_it_isnt_ready() {
        let mut rig = rig();
        assert_eq!(rig.auto.refusal("snapshot"), None, "nothing is known before the first look");
        rig.fake.engine_ok.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        // Still mending quietly: told to call again. A member blocked on it makes the failure known.
        let soft = rig.auto.refusal("mcp__desktop__snapshot").unwrap();
        assert!(soft.contains("Call again in a minute"), "{soft}");
        rig.worker.step().await;
        let message = rig.auto.refusal("mcp__desktop__snapshot").unwrap();
        assert!(message.contains("Don't retry") && message.contains("screenshot"), "{message}");
        // The picture of the screen needs no engine and no Accessibility, only
        // Screen Recording (which the fake machine hasn't got).
        let picture = rig.auto.refusal("mcp__desktop__screenshot").unwrap();
        assert!(picture.contains("Screen Recording"), "{picture}");
        // Once everything is on, nothing stands in the way.
        rig.fake.engine_ok.store(true, Ordering::SeqCst);
        rig.fake.granted.store(true, Ordering::SeqCst);
        rig.auto.retry();
        rig.worker.step().await;
        assert!(rig.auto.latest().unwrap().ready());
        assert_eq!(rig.auto.refusal("mcp__desktop__snapshot"), None);
    }

    #[tokio::test]
    async fn the_grant_is_asked_for_only_once_the_engine_is_ready() {
        let mut rig = rig();
        rig.fake.engine_ok.store(false, Ordering::SeqCst);
        rig.worker.step().await;
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0, "nothing to ask for until the engine is there");
        assert_ne!(rig.setup(), DesktopSetup::Grant);
    }

    #[tokio::test]
    async fn skipping_in_the_first_run_is_undone_by_the_first_member_who_needs_the_desktop() {
        let mut rig = rig();
        skip(&rig.home);
        rig.worker.saved = Saved::load(&rig.home);
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Idle, "skipped: no card");
        rig.auto.refusal("mcp__desktop__snapshot");
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant, "offered again at the first need");
    }

    #[tokio::test]
    async fn an_unwanted_desktop_is_never_prompted_for() {
        let mut rig = rig();
        rig.fake.wanted.store(false, Ordering::SeqCst);
        for _ in 0..3 {
            rig.worker.step().await;
        }
        assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0);
        assert_eq!(rig.setup(), DesktopSetup::Idle);
        // Until a member reaches for it.
        rig.auto.wanted();
        rig.worker.step().await;
        assert_eq!(rig.setup(), DesktopSetup::Grant);
    }

    #[tokio::test]
    async fn background_work_never_asks_for_screen_recording_and_members_read_the_tree() {
        for ax in [true, false] {
            let mut rig = rig();
            rig.fake.granted.store(ax, Ordering::SeqCst);
            // Three days of the daemon looking, as it does when "everyone" works the desktop.
            for hour in 0..72u64 {
                rig.worker.clock = Box::new(move || 1_000_000 + hour * 3600);
                rig.worker.step().await;
                assert_eq!(rig.auto.refusal("mcp__desktop__snapshot").is_some(), !ax);
                let picture = rig.auto.refusal("mcp__desktop__screenshot").expect("no Screen Recording: told, not tried");
                assert!(picture.contains("Screen Recording") && picture.contains("snapshot") && !picture.contains('\n'), "{picture}");
            }
            assert_eq!(rig.fake.screen_requests.load(Ordering::SeqCst), 0, "ax={ax}: nothing but the person's own action asks for Screen Recording");
            assert_eq!(rig.fake.prompts.load(Ordering::SeqCst), 0, "ax={ax}: background work never asks for Accessibility; only the person's click does");
            assert_eq!(prompts::recorded(&rig.home, prompts::Kind::ScreenRecording), None);
        }
    }
}
