//! The Mac's screen, shared by the members: one at a time (`lease`), the person
//! first (`presence`), never input while locked or logged out (`mac`).
//!
//! The daemon keeps one [`Desktop`] for every squad and asks it before each
//! desktop tool call goes to the helper: [`Desktop::ask`] says yes or why not,
//! [`Desktop::done`] records when the call ran, so the members' own input isn't
//! taken for the person's. Kept in memory only: a restarted daemon starts with
//! a free screen.

pub mod auto;
pub mod lease;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod presence;
pub mod ready;

use std::time::Instant;

use equipe_protocol::{DesktopState, MemberRef};

use lease::{Lease, Turn};
use presence::Presence;

/// The desktop helper's `desktop serve` arguments (`--home <dir>`), if these
/// are they: what equipe.app's executable is started with by the helper's
/// LaunchAgent. `None` for anything else.
pub fn serve_args(args: &[String]) -> Option<std::path::PathBuf> {
    if args.get(1).map(String::as_str) != Some("desktop") || args.get(2).map(String::as_str) != Some("serve") {
        return None;
    }
    let home = args.iter().position(|a| a == "--home").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    Some(home.unwrap_or_else(equipe_providers::credentials::equipe_home))
}

/// The person's desktop settings file (`<home>/desktop/settings.json`).
fn settings_path(home: &std::path::Path) -> std::path::PathBuf {
    equipe_desktop_control::helper::dir(home).join("settings.json")
}

/// Whether the person lets members act through the accessibility tree while
/// the Mac's screen is locked ("Work while the screen is locked"). Off unless
/// they turned it on.
pub fn work_locked(home: &std::path::Path) -> bool {
    std::fs::read_to_string(settings_path(home))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["work_locked"].as_bool())
        .unwrap_or(false)
}

/// Turn "Work while the screen is locked" on or off: the person's alone.
pub fn set_work_locked(home: &std::path::Path, on: bool) -> std::io::Result<()> {
    let path = settings_path(home);
    let mut value = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()).unwrap_or_else(|| serde_json::json!({}));
    value["work_locked"] = serde_json::json!(on);
    equipe_desktop_control::helper::ensure_dir(home)?;
    std::fs::write(path, value.to_string())
}

/// Serve the desktop helper as equipe.app ([`equipe_desktop_control::serve_as_app`]).
pub fn serve_as_app(home: &std::path::Path) -> std::io::Result<()> {
    equipe_desktop_control::serve_as_app(home)
}

/// A member's hold on the screen ends when it hasn't used a desktop tool for
/// this long (seconds), so a member that wandered off doesn't keep the line waiting.
pub const LEASE_IDLE: f64 = 120.0;

/// How long a member refused for a locked screen counts as waiting for the
/// person to unlock it, when it doesn't ask again (seconds).
pub const LOCK_HELD_FOR: f64 = 600.0;

/// How long the person must have left mouse and keyboard alone before a member's
/// input goes in again (seconds).
pub const PERSON_QUIET: f64 = 30.0;

/// The person's GUI session, as macOS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Logged in at the console, unlocked: members may use it.
    Open,
    /// The screen is locked; members can't unlock it.
    Locked,
    /// Nobody logged in at the console (or FileVault after a reboot), or the
    /// daemon isn't in the GUI session.
    NoSession,
}

/// What the Mac says about the person's session.
pub trait Probe: Send {
    /// Seconds since the last mouse or keyboard event of any source; `None`
    /// when it can't be told.
    fn idle(&self) -> Option<f64>;
    fn screen(&self) -> Screen;
}

/// The Mac this daemon runs on.
pub struct ThisMac;

impl Probe for ThisMac {
    #[cfg(target_os = "macos")]
    fn idle(&self) -> Option<f64> {
        mac::idle()
    }
    #[cfg(target_os = "macos")]
    fn screen(&self) -> Screen {
        mac::screen()
    }
    #[cfg(not(target_os = "macos"))]
    fn idle(&self) -> Option<f64> {
        None
    }
    #[cfg(not(target_os = "macos"))]
    fn screen(&self) -> Screen {
        Screen::NoSession
    }
}

/// What a desktop tool call does with the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Only looks, at what the screen shows: a screenshot, the window list,
    /// the screenshot tools' element list. Takes the lease, since the
    /// coordinates it hands out are for the input that follows.
    Look,
    /// Clicks, types, scrolls, opens or brings forward an app: the real
    /// pointer and keyboard, or focus taken.
    Input,
    /// An input the person has just said yes to (a send): their being at the
    /// Mac — where they likely answered — doesn't hold it back. The lease, a
    /// locked screen and /desktop stop still do.
    Approved,
    /// Reads through Accessibility (a snapshot, a find, the app list, a wait):
    /// nothing on screen moves, so no lease and no waiting for the person.
    /// Only /desktop stop refuses it.
    Read,
    /// Acts through Accessibility without the pointer, the keyboard or focus
    /// (a headless ref action): several members at once and beside the
    /// person, so no lease and no waiting for their hands. Still never while
    /// the Mac is locked or nobody is logged in, nor after /desktop stop.
    Headless,
}

impl Use {
    /// What the desktop tool `tool` (`screenshot`, `mcp__desktop__click`) does
    /// with `input` (an accessibility tool's `action`, `headed`…). A tool not
    /// named here is taken for an input.
    pub fn of(tool: &str, input: &serde_json::Value) -> Use {
        let text = |key: &str| input.get(key).and_then(serde_json::Value::as_str).unwrap_or_default();
        let headed = input.get("headed").and_then(serde_json::Value::as_bool).unwrap_or(false);
        match tool.rsplit("__").next().unwrap_or(tool) {
            "screenshot" | "list_windows" | "elements" => Use::Look,
            "snapshot" | "find" | "wait" => Use::Read,
            // A double click is physical; a context menu opens on screen and
            // holds the pointer until it closes: both wait for the screen.
            "act" if headed || matches!(text("action"), "double_click" | "right_click") => Use::Input,
            "act" => Use::Headless,
            "apps" => match text("action") {
                "" | "list" => Use::Read,
                // In the background: nothing comes forward.
                "launch" if !input.get("activate").and_then(serde_json::Value::as_bool).unwrap_or(false) => Use::Headless,
                _ => Use::Input,
            },
            "windows" | "notifications" => match text("action") {
                "" | "list" => Use::Read,
                _ => Use::Input,
            },
            // The person's clipboard: read freely, written only while they
            // aren't using the Mac (they may be about to paste).
            "clipboard" => match text("action") {
                "" | "get" => Use::Read,
                _ => Use::Input,
            },
            _ => Use::Input,
        }
    }

    /// Whether it posts real mouse or keyboard events.
    pub fn physical(self) -> bool {
        matches!(self, Use::Input | Use::Approved)
    }
}

/// Why a member can't use the screen now. The text goes back to the model.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Another member has it; this one is `place`th in line.
    Busy {
        holder: MemberRef,
        place: usize,
    },
    /// The person is using the Mac; members wait `left` more seconds at least.
    Person {
        left: f64,
    },
    Locked,
    /// Locked, and the person lets members work while it is — but only
    /// through the accessibility tree: pointer and keyboard input go to the
    /// lock screen, never to an app.
    LockedPhysical,
    NoSession,
    /// The person took the screen back.
    Stopped,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Busy { holder, place } => write!(
                f,
                "{} is using the screen; you are number {place} in line and get it when they are done. Do something else meanwhile, or call again later.",
                holder.member
            ),
            Refusal::Person { left } => write!(
                f,
                "The person is using the Mac: nothing was clicked or typed. Members' input waits until they have left mouse and keyboard alone for {} s (at least {} s more). Looking at the screen is allowed.",
                PERSON_QUIET as u64,
                left.ceil() as u64
            ),
            Refusal::Locked => f.write_str(
                "The Mac's screen is locked: nothing was clicked or typed. Members can't unlock it, and nothing is ever typed at the lock screen. equipe has already shown the person an \"Unlock this Mac\" card (they unlock it themselves), so don't ask them in chat to unlock it and don't retry in a loop: do other work and call again later; it works by itself once they have.",
            ),
            Refusal::LockedPhysical => f.write_str(
                "The Mac's screen is locked: pointer and keyboard input (headed, double/right click, focusing windows, screenshots-and-click) waits until it is unlocked, since macOS sends it to the lock screen. Reading (snapshot, find) and act on a ref without `headed` work while it is locked: use those. equipe has already shown the person an \"Unlock this Mac\" card (they unlock it themselves), so don't ask them in chat to unlock it and don't retry in a loop: do other work and call again later.",
            ),
            Refusal::NoSession => f.write_str(
                "Nobody is logged in at the Mac's screen, or equipe's daemon isn't running in the person's login session: nothing was done. Ask the person to log in at the Mac and run `equipe daemon install`.",
            ),
            Refusal::Stopped => f.write_str(
                "The person has taken the screen back (/desktop stop): no member uses it until they open it again.",
            ),
        }
    }
}

/// The screen: the lease, the person's hands and the session, on the daemon's clock.
pub struct Desktop {
    lease: Lease<MemberRef>,
    presence: Presence,
    probe: Box<dyn Probe>,
    epoch: Instant,
    /// Where the person's "Work while the screen is locked" is kept; `None`: off.
    home: Option<std::path::PathBuf>,
    /// Members refused for a locked screen, and when (on [`Desktop::now`]'s
    /// clock): the ones waiting for the person to unlock it.
    lock_held: Vec<(MemberRef, f64)>,
}

impl Desktop {
    pub fn new(probe: Box<dyn Probe>) -> Self {
        Self::with(probe, LEASE_IDLE, PERSON_QUIET)
    }

    pub fn with(probe: Box<dyn Probe>, idle: f64, quiet: f64) -> Self {
        Self { lease: Lease::new(idle), presence: Presence::new(quiet), probe, epoch: Instant::now(), home: None, lock_held: Vec::new() }
    }

    /// Read the person's "Work while the screen is locked" from `home` (at each ask while locked).
    pub fn with_home(mut self, home: &std::path::Path) -> Self {
        self.home = Some(home.to_owned());
        self
    }

    fn locked_refusal(&self) -> Refusal {
        if self.home.as_deref().is_some_and(work_locked) { Refusal::LockedPhysical } else { Refusal::Locked }
    }

    /// Seconds on the daemon's clock, for [`Desktop::done`].
    pub fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// May `who` make a call that does `what` now? On yes it has the screen
    /// until its turn ends, it lets go, or it stays away from it too long.
    pub fn ask(&mut self, who: &MemberRef, what: Use) -> Result<(), Refusal> {
        let now = self.now();
        self.ask_at(who, what, now)
    }

    fn ask_at(&mut self, who: &MemberRef, what: Use, now: f64) -> Result<(), Refusal> {
        let answer = self.decide(who, what, now);
        match &answer {
            // Held by the lock: it waits for the person, who is shown a card.
            Err(Refusal::Locked | Refusal::LockedPhysical) => {
                match self.lock_held.iter_mut().find(|(m, _)| m == who) {
                    Some((_, at)) => *at = now,
                    None => self.lock_held.push((who.clone(), now)),
                }
            }
            // The screen is open and it got through: nothing holds it.
            Ok(()) if !self.lock_held.is_empty() && self.probe.screen() == Screen::Open => self.lock_held.retain(|(m, _)| m != who),
            _ => {}
        }
        answer
    }

    /// Members held by a locked screen now: refused for it, not since let
    /// through, and the screen is still locked.
    pub fn locked_waiting(&self) -> Vec<MemberRef> {
        if self.lock_held.is_empty() || self.probe.screen() != Screen::Locked {
            return Vec::new();
        }
        let now = self.now();
        self.lock_held.iter().filter(|(_, at)| now - at < LOCK_HELD_FOR).map(|(m, _)| m.clone()).collect()
    }

    fn decide(&mut self, who: &MemberRef, what: Use, now: f64) -> Result<(), Refusal> {
        // Through Accessibility only: nobody's pointer or keyboard, so no
        // line to stand in, and the person keeps working meanwhile.
        if matches!(what, Use::Read | Use::Headless) {
            if self.lease.stopped() {
                return Err(Refusal::Stopped);
            }
            if what == Use::Read {
                return Ok(());
            }
            return match self.probe.screen() {
                Screen::Open => Ok(()),
                // The person's choice: on an unattended Mac, accessibility
                // actions go on while it is locked (the apps keep running in
                // the locked session and answer their accessibility calls).
                Screen::Locked if self.home.as_deref().is_some_and(work_locked) => Ok(()),
                Screen::Locked => Err(Refusal::Locked),
                Screen::NoSession => Err(Refusal::NoSession),
            };
        }
        match self.lease.ask(who, now) {
            Turn::Yours => {}
            Turn::Waiting { holder, place } => return Err(Refusal::Busy { holder, place }),
            Turn::Stopped => return Err(Refusal::Stopped),
        }
        if what == Use::Look {
            return Ok(());
        }
        match self.probe.screen() {
            Screen::Open => {}
            Screen::Locked => return Err(self.locked_refusal()),
            Screen::NoSession => return Err(Refusal::NoSession),
        }
        if what == Use::Approved {
            return Ok(());
        }
        // Not knowing whether the person is there is not leave to go ahead.
        let Some(idle) = self.probe.idle() else {
            return Err(Refusal::NoSession);
        };
        match self.presence.observe(now, idle) {
            Some(left) => Err(Refusal::Person { left }),
            None => Ok(()),
        }
    }

    /// `who`'s call ran from `start` to `end` (see [`Desktop::now`]); `input`:
    /// it posted mouse or keyboard events.
    pub fn done(&mut self, who: &MemberRef, start: f64, end: f64, input: bool) {
        if input {
            self.presence.posted(start, end);
        }
        self.lease.used(who, end);
    }

    /// `who` lets go of the screen, or leaves the line: it called `release`,
    /// or its turn ended. Returns whether anything changed.
    pub fn leave(&mut self, who: &MemberRef) -> bool {
        let now = self.now();
        let held = self.lock_held.len();
        self.lock_held.retain(|(m, _)| m != who);
        self.lease.leave(who, now) || self.lock_held.len() != held
    }

    /// Let go of a holder that stayed away too long, and of the members that
    /// waited for an unlock that came (or never mattered). Returns whether
    /// anything changed.
    pub fn expire(&mut self) -> bool {
        let now = self.now();
        let held = self.lock_held.len();
        let locked = self.probe.screen() == Screen::Locked;
        self.lock_held.retain(|(_, at)| locked && now - at < LOCK_HELD_FOR);
        self.lease.expire(now) || self.lock_held.len() != held
    }

    pub fn stop(&mut self) -> bool {
        self.lease.stop()
    }

    pub fn resume(&mut self) -> bool {
        self.lease.resume()
    }

    pub fn state(&self) -> DesktopState {
        DesktopState {
            holder: self.lease.holder().cloned(),
            queue: self.lease.queue().cloned().collect(),
            stopped: self.lease.stopped(),
            locked_waiting: self.locked_waiting(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn with_the_persons_yes_tree_actions_go_on_while_locked_and_input_still_waits() {
        let (d, mac) = desktop(Some(600.0), Screen::Locked);
        let home = std::env::temp_dir().join(format!("eq-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut d = d.with_home(&home);
        let a = MemberRef { squad: "s".into(), member: "a".into() };
        assert_eq!(d.ask(&a, Use::Headless), Err(Refusal::Locked), "off unless the person turned it on");
        super::set_work_locked(&home, true).unwrap();
        assert!(super::work_locked(&home));
        assert_eq!(d.ask(&a, Use::Headless), Ok(()), "act on a ref, headless, while locked");
        assert_eq!(d.ask(&a, Use::Read), Ok(()));
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::LockedPhysical), "pointer and keys would reach the lock screen");
        mac.0.lock().unwrap().1 = Screen::NoSession;
        assert_eq!(d.ask(&a, Use::Headless), Err(Refusal::NoSession), "logged out stays refused");
        super::set_work_locked(&home, false).unwrap();
        mac.0.lock().unwrap().1 = Screen::Locked;
        assert_eq!(d.ask(&a, Use::Headless), Err(Refusal::Locked));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_member_refused_for_a_locked_screen_is_held_until_it_is_unlocked() {
        let (mut d, mac) = desktop(Some(600.0), Screen::Locked);
        let (a, b) = (member("frontend"), member("qa"));
        assert!(d.state().locked_waiting.is_empty(), "locked, but nobody asked");
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Locked));
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Locked));
        assert_eq!(d.ask(&b, Use::Headless), Err(Refusal::Locked));
        assert_eq!(d.state().locked_waiting, vec![a.clone(), b.clone()], "each once, in the order they were refused");
        assert!(!d.state().is_quiet());
        // Reading is fine while locked and isn't held by it.
        let (mut r, _) = desktop(Some(600.0), Screen::Locked);
        assert_eq!(r.ask(&a, Use::Read), Ok(()));
        assert!(r.state().locked_waiting.is_empty());
        // The person unlocks: nobody is held, whether or not anyone asks again.
        mac.0.lock().unwrap().1 = Screen::Open;
        assert!(d.state().locked_waiting.is_empty());
        assert!(d.expire(), "the tick finds the change and tells the screens");
        assert!(!d.expire());
        // Locked once more: held only by a new refusal.
        mac.0.lock().unwrap().1 = Screen::Locked;
        assert!(d.state().locked_waiting.is_empty());
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Locked));
        assert_eq!(d.state().locked_waiting, vec![a.clone()]);
        // A member that leaves the line (its turn ended) is no longer waiting.
        assert!(d.leave(&a));
        assert!(d.state().locked_waiting.is_empty());
    }

    #[test]
    fn a_member_let_through_is_no_longer_held_and_an_old_refusal_runs_out() {
        let (mut d, mac) = desktop(Some(600.0), Screen::Locked);
        let a = member("frontend");
        assert_eq!(d.ask_at(&a, Use::Input, 0.0), Err(Refusal::Locked));
        assert_eq!(d.lock_held.len(), 1);
        mac.0.lock().unwrap().1 = Screen::Open;
        assert_eq!(d.ask_at(&a, Use::Input, 1.0), Ok(()));
        assert!(d.lock_held.is_empty());
        mac.0.lock().unwrap().1 = Screen::Locked;
        assert_eq!(d.ask_at(&a, Use::Input, 2.0), Err(Refusal::Locked));
        d.lock_held[0].1 = d.now() - LOCK_HELD_FOR - 1.0;
        assert!(d.locked_waiting().is_empty(), "it never asked again: not waiting any more");
        assert!(d.expire());
        assert!(d.lock_held.is_empty());
    }

    #[test]
    fn the_refusal_tells_members_the_person_has_the_card_and_not_to_ask_in_chat() {
        for refusal in [Refusal::Locked, Refusal::LockedPhysical] {
            let text = refusal.to_string();
            assert!(text.contains("Unlock this Mac") && text.contains("don't ask them in chat"), "{text}");
        }
    }

    #[test]
    fn the_app_knows_when_it_is_started_as_the_helper() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(super::serve_args(&args(&["x", "desktop", "serve", "--home", "/h/.equipe"])), Some(std::path::PathBuf::from("/h/.equipe")));
        assert!(super::serve_args(&args(&["x", "desktop", "serve"])).is_some());
        assert_eq!(super::serve_args(&args(&["x"])), None);
        assert_eq!(super::serve_args(&args(&["x", "--notify-test"])), None);
    }

    use std::sync::{Arc, Mutex};

    use super::*;

    /// A Mac whose last input and lock state the test sets.
    #[derive(Clone)]
    struct Fake(Arc<Mutex<(Option<f64>, Screen)>>);

    impl Probe for Fake {
        fn idle(&self) -> Option<f64> {
            self.0.lock().unwrap().0
        }
        fn screen(&self) -> Screen {
            self.0.lock().unwrap().1
        }
    }

    fn member(name: &str) -> MemberRef {
        MemberRef { squad: "acme".into(), member: name.into() }
    }

    fn desktop(idle: Option<f64>, screen: Screen) -> (Desktop, Fake) {
        let fake = Fake(Arc::new(Mutex::new((idle, screen))));
        (Desktop::new(Box::new(fake.clone())), fake)
    }

    #[test]
    fn one_member_at_a_time_and_the_next_carries_on() {
        let (mut d, _) = desktop(Some(600.0), Screen::Open);
        let (a, b) = (member("frontend"), member("backend"));
        assert_eq!(d.ask(&a, Use::Input), Ok(()));
        let refused = d.ask(&b, Use::Input).unwrap_err();
        assert_eq!(refused, Refusal::Busy { holder: a.clone(), place: 1 });
        assert!(refused.to_string().starts_with("frontend is using the screen; you are number 1 in line"));
        assert_eq!(d.ask(&b, Use::Look), Err(Refusal::Busy { holder: a.clone(), place: 1 }));
        assert!(d.leave(&a));
        assert_eq!(d.state().holder, Some(b.clone()));
        assert_eq!(d.ask(&b, Use::Input), Ok(()));
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Busy { holder: b, place: 1 }));
    }

    #[test]
    fn the_person_at_the_mac_holds_input_back_but_not_looking() {
        let (mut d, mac) = desktop(Some(2.0), Screen::Open);
        let a = member("frontend");
        assert!(matches!(d.ask(&a, Use::Input), Err(Refusal::Person { .. })));
        assert_eq!(d.ask(&a, Use::Look), Ok(()));
        assert_eq!(d.ask(&a, Use::Approved), Ok(()), "a send they just said yes to");
        // They stopped a while ago, but not long enough ago.
        mac.0.lock().unwrap().0 = Some(PERSON_QUIET - 5.0);
        assert!(matches!(d.ask(&a, Use::Input), Err(Refusal::Person { .. })));
    }

    #[test]
    fn a_members_own_input_is_not_taken_for_the_person_but_the_person_moving_after_it_is() {
        let (mut d, mac) = desktop(Some(600.0), Screen::Open);
        let a = member("frontend");
        assert_eq!(d.ask_at(&a, Use::Input, 1000.0), Ok(()));
        d.done(&a, 1000.0, 1000.3, true);
        // The last event macOS saw is the click equipe just posted.
        mac.0.lock().unwrap().0 = Some(1.0);
        assert_eq!(d.ask_at(&a, Use::Input, 1001.0), Ok(()));
        // Then the person moves the mouse: the next input doesn't go in.
        mac.0.lock().unwrap().0 = Some(0.5);
        assert!(matches!(d.ask_at(&a, Use::Input, 1004.0), Err(Refusal::Person { .. })));
    }

    #[test]
    fn a_locked_screen_or_no_session_gets_no_input() {
        let (mut d, mac) = desktop(Some(600.0), Screen::Locked);
        let a = member("frontend");
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Locked));
        assert_eq!(d.ask(&a, Use::Approved), Err(Refusal::Locked));
        assert_eq!(d.ask(&a, Use::Look), Ok(()));
        mac.0.lock().unwrap().1 = Screen::NoSession;
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::NoSession));
        // Input with no way to tell whether the person is there: no.
        *mac.0.lock().unwrap() = (None, Screen::Open);
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::NoSession));
    }

    #[test]
    fn headless_work_runs_side_by_side_and_beside_the_person_but_never_locked() {
        // The person is typing, and one member holds the screen.
        let (mut d, mac) = desktop(Some(1.0), Screen::Open);
        let (a, b, c) = (member("frontend"), member("backend"), member("designer"));
        mac.0.lock().unwrap().0 = Some(600.0);
        assert_eq!(d.ask(&a, Use::Input), Ok(()));
        mac.0.lock().unwrap().0 = Some(1.0);
        for who in [&b, &c] {
            assert_eq!(d.ask(who, Use::Headless), Ok(()), "no line for a headless ref action");
            assert_eq!(d.ask(who, Use::Read), Ok(()));
        }
        assert_eq!(d.state().queue, Vec::<MemberRef>::new(), "nobody was queued");
        assert_eq!(d.state().holder, Some(a.clone()));
        // Locked: reading still goes, acting doesn't.
        mac.0.lock().unwrap().1 = Screen::Locked;
        assert_eq!(d.ask(&b, Use::Headless), Err(Refusal::Locked));
        assert_eq!(d.ask(&b, Use::Read), Ok(()));
        mac.0.lock().unwrap().1 = Screen::NoSession;
        assert_eq!(d.ask(&b, Use::Headless), Err(Refusal::NoSession));
        // Taken back: nothing at all.
        mac.0.lock().unwrap().1 = Screen::Open;
        d.stop();
        assert_eq!(d.ask(&b, Use::Headless), Err(Refusal::Stopped));
        assert_eq!(d.ask(&c, Use::Read), Err(Refusal::Stopped));
    }

    #[test]
    fn what_each_tool_does_with_the_screen() {
        use serde_json::json;
        let of = |tool: &str, input: serde_json::Value| Use::of(tool, &input);
        assert_eq!(of("mcp__desktop__screenshot", json!({})), Use::Look);
        assert_eq!(of("click", json!({ "x": 1, "y": 2 })), Use::Input);
        assert_eq!(of("snapshot", json!({ "app": "Slack" })), Use::Read);
        assert_eq!(of("act", json!({ "ref": "e1" })), Use::Headless);
        assert_eq!(of("act", json!({ "ref": "e1", "action": "type", "text": "hi" })), Use::Headless);
        assert_eq!(of("act", json!({ "ref": "e1", "headed": true })), Use::Input, "physical input takes the screen");
        assert_eq!(of("act", json!({ "ref": "e1", "action": "double_click" })), Use::Input);
        assert_eq!(of("act", json!({ "ref": "e1", "action": "right_click" })), Use::Input, "a context menu takes the screen");
        assert_eq!(of("apps", json!({})), Use::Read);
        assert_eq!(of("apps", json!({ "action": "launch", "app": "Notes" })), Use::Headless);
        assert_eq!(of("apps", json!({ "action": "launch", "app": "Notes", "activate": true })), Use::Input);
        assert_eq!(of("apps", json!({ "action": "close", "app": "Notes" })), Use::Input);
        assert_eq!(of("windows", json!({ "action": "focus" })), Use::Input);
        assert_eq!(of("notifications", json!({ "action": "act", "index": 1 })), Use::Input);
        assert_eq!(of("clipboard", json!({})), Use::Read);
        assert_eq!(of("clipboard", json!({ "action": "set", "text": "x" })), Use::Input);
        assert_eq!(of("something_new", json!({})), Use::Input);
        // Every tool a member is offered is one this knows.
        for tool in equipe_desktop_control::tools::tools() {
            let name = tool["name"].as_str().unwrap();
            let known = ["screenshot", "list_windows", "elements", "click", "double_click", "drag", "scroll", "type_text", "key", "open_app", "focus_window"];
            assert!(known.contains(&name) || equipe_desktop_control::engine::handles(name), "{name}");
        }
    }

    #[test]
    fn stop_refuses_everyone_at_once_until_resume() {
        let (mut d, _) = desktop(Some(600.0), Screen::Open);
        let (a, b) = (member("frontend"), member("backend"));
        assert_eq!(d.ask(&a, Use::Input), Ok(()));
        let _ = d.ask(&b, Use::Input);
        assert!(d.stop());
        assert_eq!(d.state(), DesktopState { holder: None, queue: vec![], stopped: true, ..Default::default() });
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Stopped));
        assert_eq!(d.ask(&b, Use::Look), Err(Refusal::Stopped));
        assert!(d.resume());
        assert_eq!(d.ask(&b, Use::Input), Ok(()));
    }
}
