//! The Mac's screen, shared by the members: one at a time (`lease`), the person
//! first (`presence`), never input while locked or logged out (`mac`).
//!
//! The daemon keeps one [`Desktop`] for every squad and asks it before each
//! desktop tool call goes to the helper: [`Desktop::ask`] says yes or why not,
//! [`Desktop::done`] records when the call ran, so the members' own input isn't
//! taken for the person's. Kept in memory only: a restarted daemon starts with
//! a free screen.

pub mod lease;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod presence;

use std::time::Instant;

use equipe_protocol::{DesktopState, MemberRef};

use lease::{Lease, Turn};
use presence::Presence;

/// A member's hold on the screen ends when it hasn't used a desktop tool for
/// this long (seconds), so a member that wandered off doesn't keep the line waiting.
pub const LEASE_IDLE: f64 = 120.0;

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
    /// Only looks: a screenshot, the window list, the accessibility tree.
    Look,
    /// Clicks, types, scrolls, opens or brings forward an app.
    Input,
    /// An input the person has just said yes to (a send): their being at the
    /// Mac — where they likely answered — doesn't hold it back. The lease, a
    /// locked screen and /desktop stop still do.
    Approved,
}

impl Use {
    /// What the desktop tool `tool` (`screenshot`, `mcp__desktop__click`) does.
    pub fn of(tool: &str) -> Use {
        match tool.rsplit("__").next().unwrap_or(tool) {
            "screenshot" | "list_windows" | "elements" => Use::Look,
            _ => Use::Input,
        }
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
                "The Mac's screen is locked: nothing was clicked or typed. Members can't unlock it; ask the person to.",
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
}

impl Desktop {
    pub fn new(probe: Box<dyn Probe>) -> Self {
        Self::with(probe, LEASE_IDLE, PERSON_QUIET)
    }

    pub fn with(probe: Box<dyn Probe>, idle: f64, quiet: f64) -> Self {
        Self { lease: Lease::new(idle), presence: Presence::new(quiet), probe, epoch: Instant::now() }
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
            Screen::Locked => return Err(Refusal::Locked),
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
        self.lease.leave(who, now)
    }

    /// Let go of a holder that stayed away too long. Returns whether anything changed.
    pub fn expire(&mut self) -> bool {
        let now = self.now();
        self.lease.expire(now)
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
        }
    }
}

#[cfg(test)]
mod tests {
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
    fn stop_refuses_everyone_at_once_until_resume() {
        let (mut d, _) = desktop(Some(600.0), Screen::Open);
        let (a, b) = (member("frontend"), member("backend"));
        assert_eq!(d.ask(&a, Use::Input), Ok(()));
        let _ = d.ask(&b, Use::Input);
        assert!(d.stop());
        assert_eq!(d.state(), DesktopState { holder: None, queue: vec![], stopped: true });
        assert_eq!(d.ask(&a, Use::Input), Err(Refusal::Stopped));
        assert_eq!(d.ask(&b, Use::Look), Err(Refusal::Stopped));
        assert!(d.resume());
        assert_eq!(d.ask(&b, Use::Input), Ok(()));
    }
}
