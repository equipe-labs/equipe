//! Where the person's screen is: on the squad's Mac, or on another machine.
//!
//! The daemon runs on one Mac. The person may follow a squad from the equipe
//! app on that same Mac, from the app on another Mac (over SSH, through
//! `equipe attach --stdio`), from a terminal over SSH, or from a phone
//! through the relay. Whatever equipe shows them — a page, a prototype, a
//! file — and whatever a member asks them to open or run has to reach the
//! screen they are actually at. This module is the one notion of it every
//! such path asks: [`Screens`] keeps the screens that follow each squad now
//! and where each is ([`Place`]); [`Following`] is what one squad's add up to.
//!
//! A screen says where it is in its `hello` (`remote`); a connection that
//! `equipe attach --stdio` bridges is marked by the bridge itself
//! (`bridged`), so an older app on another Mac still counts as remote. See
//! docs/REMOTE.md.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Where a screen is, seen from the daemon's machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// On the daemon's own machine: its files and `localhost` are the screen's too.
    Here,
    /// On another machine (the app over SSH, a terminal over SSH).
    Elsewhere,
    /// A phone, through the relay.
    Phone,
}

impl Place {
    /// Where a screen that said hello is: a phone, else another machine when
    /// it said so or came through a bridge, else here.
    pub fn of(phone: bool, remote: bool, bridged: bool) -> Self {
        match (phone, remote || bridged) {
            (true, _) => Place::Phone,
            (false, true) => Place::Elsewhere,
            (false, false) => Place::Here,
        }
    }
}

/// One screen following a squad.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Screen {
    squad: String,
    place: Place,
    /// It shows pages beside the conversation (the app's live preview and
    /// browser panel) — a terminal or a phone shows none.
    shows: bool,
}

/// The screens following each squad now, and where the last one of each
/// squad was — what the person most likely comes back on.
#[derive(Debug, Default)]
pub struct Screens {
    next: AtomicU64,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    open: HashMap<u64, Screen>,
    last: HashMap<String, Place>,
}

/// A screen's seat while it follows its squad: it leaves when this drops.
#[derive(Debug)]
pub struct Seat {
    screens: Arc<Screens>,
    id: u64,
}

impl Drop for Seat {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.screens.inner.lock() {
            inner.open.remove(&self.id);
        }
    }
}

impl Screens {
    /// A screen at `place` follows `squad` until the seat drops. `shows`: it
    /// can show a page beside the conversation.
    pub fn sit(self: &Arc<Self>, squad: &str, place: Place, shows: bool) -> Seat {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut inner) = self.inner.lock() {
            inner.open.insert(id, Screen { squad: squad.to_owned(), place, shows });
            inner.last.insert(squad.to_owned(), place);
        }
        Seat { screens: self.clone(), id }
    }

    /// What follows `squad` now.
    pub fn following(&self, squad: &str) -> Following {
        let Ok(inner) = self.inner.lock() else { return Following::default() };
        let mut out = Following { last: inner.last.get(squad).copied(), ..Following::default() };
        for screen in inner.open.values().filter(|s| s.squad == squad) {
            match screen.place {
                Place::Here => out.here += 1,
                Place::Elsewhere => out.elsewhere += 1,
                Place::Phone => out.phones += 1,
            }
            out.showing += usize::from(screen.shows);
        }
        out
    }
}

/// The screens following one squad.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Following {
    pub here: usize,
    pub elsewhere: usize,
    pub phones: usize,
    /// Of them, those that show pages beside the conversation.
    pub showing: usize,
    /// Where the last screen to follow the squad was, gone or not.
    pub last: Option<Place>,
}

impl Following {
    /// A screen that can show the person a page follows the squad now.
    pub fn can_show(&self) -> bool {
        self.showing > 0
    }

    /// The person looks from another machine: a screen elsewhere (or a phone)
    /// follows the squad — with one here too, nothing of this Mac is surely
    /// theirs — or none follows and the last one that did was elsewhere.
    pub fn remote(&self) -> bool {
        self.elsewhere + self.phones > 0 || (self.here == 0 && matches!(self.last, Some(Place::Elsewhere | Place::Phone)))
    }
}

/// The daemon's machine as the person knows it (its computer name, else its
/// host name), for "on <Mac>": read once.
pub fn machine_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let read = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|n| !n.is_empty())
        };
        let name = if cfg!(target_os = "macos") { read("/usr/sbin/scutil", &["--get", "ComputerName"]) } else { None };
        name.or_else(|| read("hostname", &[])).unwrap_or_else(|| "the squad's computer".to_owned())
    })
}

/// What a member's turn opens with when where the person looks from changed
/// since it was last told (`told`): from another machine, nothing of `mac`'s
/// is theirs to open; back on `mac`, it is again. `None`: nothing to tell.
pub fn note(remote: bool, told: Option<bool>, mac: &str) -> Option<String> {
    match (remote, told) {
        (true, Some(true)) | (false, None | Some(false)) => None,
        (true, _) => Some(remote_note(mac)),
        (false, Some(true)) => Some(format!("the person is back on {mac} itself: its files and localhost pages open for them as usual.")),
    }
}

/// How a member works for a person who looks from another machine.
pub fn remote_note(mac: &str) -> String {
    format!(
        "the person follows this squad from another machine, not from {mac} where you run — they can't open {mac}'s files \
or its localhost, see its screen, or type in its terminal. So: never hand them a file:// link or a path on {mac} to open; \
show a page with `equipe design session show` (an HTML file or a localhost address: equipe serves it to their screen), a \
picture or visual with your board's tools, a file's contents in your reply. A command you suggest says where it runs — \
\"on {mac}\" or \"on your own computer\". What only {mac}'s own screen can do (a Keychain or macOS permission dialog, a \
native window): say plainly it needs one approval on {mac}'s screen, through Screen Sharing; a sign-in or CAPTCHA in the \
members' browser: they open it in the browser pane of their equipe app. Never wait silently on either."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_counts_while_it_follows_and_where_it_is_decides_remote() {
        let screens = Arc::new(Screens::default());
        assert_eq!(screens.following("acme"), Following::default());
        assert!(!screens.following("acme").remote() && !screens.following("acme").can_show(), "nobody, never anybody: not remote");
        let here = screens.sit("acme", Place::Here, true);
        let f = screens.following("acme");
        assert!(f.can_show() && !f.remote(), "{f:?}");
        let away = screens.sit("acme", Place::of(false, false, true), true);
        assert!(screens.following("acme").remote(), "one elsewhere too: nothing of this Mac is surely theirs");
        assert_eq!(screens.following("beta"), Following::default(), "another squad's screens are its own");
        drop(here);
        drop(away);
        let gone = screens.following("acme");
        assert!(!gone.can_show() && gone.remote(), "none now, the last one was elsewhere: {gone:?}");
        let _phone = screens.sit("acme", Place::of(true, false, false), false);
        let f = screens.following("acme");
        assert!(f.remote() && !f.can_show(), "a phone shows no page beside the conversation: {f:?}");
    }

    #[test]
    fn a_member_hears_where_the_person_looks_from_when_it_changes() {
        assert!(note(false, None, "Studio").is_none(), "here from the start: nothing to say");
        let told = note(true, None, "Studio").unwrap();
        assert!(told.contains("another machine") && told.contains("on Studio") && told.contains("file://") && told.contains("Screen Sharing"), "{told}");
        assert!(note(true, Some(true), "Studio").is_none(), "told once");
        assert!(note(false, Some(true), "Studio").unwrap().contains("back on Studio"));
        assert!(note(false, Some(false), "Studio").is_none());
    }

    #[test]
    fn the_machine_has_a_name() {
        assert!(!machine_name().trim().is_empty());
    }
}
