//! What this screen shows the app in — its language, light or dark, the
//! window's and the sidebar's size, what is folded, the zoom — and which
//! build it is, told to the daemon (`rpc::SCREEN_LOOK`): as it attaches, and
//! again a moment after any of it changes or once the person uses the window.
//! The daemon keeps the latest of each machine; the UI map's real view is
//! shot in the look of the screen the person used last (`readonly.rs`),
//! wherever the daemon runs.
//!
//! The real view tells none: it isn't the person's screen.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use equipe_core::protocol::look::ScreenLook;

/// When the person last used this screen: a key, the pointer, the window
/// brought forward.
static USED: Mutex<Option<Instant>> = Mutex::new(None);
/// The look as last drawn, for the hello of the next connection.
static SHOWN: Mutex<Option<ScreenLook>> = Mutex::new(None);

/// A look changed is told once it holds this long (a drag sends many).
pub const SETTLE: Duration = Duration::from_millis(800);
/// The person at work is told at most this often.
pub const USED_EVERY: Duration = Duration::from_secs(15);
/// How often the look is looked at.
pub const LOOK_EVERY: Duration = Duration::from_millis(500);

/// The person used this screen just now.
pub fn used() {
    *USED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// When the person last used it.
pub fn used_at() -> Option<Instant> {
    *USED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The look as it was last drawn.
pub fn keep(look: ScreenLook) {
    *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(look);
}

/// What to tell a daemon as this screen attaches: the look last drawn and
/// how long since the person used it. None in the real view, or before the
/// first frame.
pub fn shown() -> Option<ScreenLook> {
    if crate::readonly::on() {
        return None;
    }
    let look = SHOWN.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    Some(ScreenLook { idle: used_at().map(|at| at.elapsed().as_secs()), ..look })
}

/// This Mac's name as its owner gave it (System Settings ▸ General ▸
/// Sharing), else its host name.
pub fn machine() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let read = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                .filter(|name| !name.is_empty())
        };
        read("/usr/sbin/scutil", &["--get", "ComputerName"]).or_else(|| read("/bin/hostname", &[])).unwrap_or_else(|| "Mac".into())
    })
}

/// When to tell the daemon this screen's look: a change once it settles,
/// and the person's use now and then.
#[derive(Debug, Default)]
pub struct Teller {
    /// What was told last, when, and the use it said.
    told: Option<(ScreenLook, Instant, Option<Instant>)>,
    /// The look drawn now, and since when it is so.
    held: Option<(ScreenLook, Instant)>,
}

impl Teller {
    /// The look drawn at `now` (its `idle` unset), the person having last
    /// used the screen at `used`: what to tell now, if anything.
    pub fn next(&mut self, look: ScreenLook, used: Option<Instant>, now: Instant) -> Option<ScreenLook> {
        if self.held.as_ref().is_none_or(|(held, _)| *held != look) {
            self.held = Some((look.clone(), now));
        }
        let since = self.held.as_ref().map_or(now, |(_, since)| *since);
        let changed = self.told.as_ref().is_none_or(|(told, _, _)| *told != look) && now.saturating_duration_since(since) >= SETTLE;
        let used_since = match &self.told {
            Some((_, at, said)) => used > *said && now.saturating_duration_since(*at) >= USED_EVERY,
            None => false,
        };
        if !changed && !used_since {
            return None;
        }
        self.told = Some((look.clone(), now, used));
        Some(ScreenLook { idle: used.map(|at| now.saturating_duration_since(at).as_secs()), ..look })
    }

    /// What was told didn't reach a daemon: told again at the next look.
    pub fn forget(&mut self) {
        self.told = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::look::Theme;

    fn look(lang: &str, sidebar: Option<u32>) -> ScreenLook {
        ScreenLook { machine: "MacBook".into(), app: "equipe-desktop".into(), version: "0.103.25".into(), lang: lang.into(), theme: Theme::Light, width: 1200, height: 760, sidebar_width: sidebar, zoom: 100, ..ScreenLook::default() }
    }

    #[test]
    fn a_change_is_told_once_it_settles_and_use_now_and_then() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut teller = Teller::default();
        // The first look, once it holds.
        assert_eq!(teller.next(look("en", None), None, at(0)), None, "not yet settled");
        let first = teller.next(look("en", None), None, at(900)).expect("told");
        assert_eq!((first.lang.as_str(), first.idle), ("en", None));
        assert_eq!(teller.next(look("en", None), None, at(1400)), None, "nothing new");
        // The sidebar dragged: many sizes, told once, at the last.
        for (n, width) in [300, 260, 220, 180].into_iter().enumerate() {
            assert_eq!(teller.next(look("en", Some(width)), None, at(2000 + n as u64 * 100)), None, "still moving");
        }
        assert_eq!(teller.next(look("en", Some(180)), None, at(3200)).map(|l| l.sidebar_width), Some(Some(180)));
        // Dragged back to what was told meanwhile: nothing to tell.
        assert_eq!(teller.next(look("en", Some(200)), None, at(3300)), None);
        assert_eq!(teller.next(look("en", Some(180)), None, at(4500)), None, "as told");
        // The person types: told, saying how long ago — then not again for a while.
        let typed = at(20_000);
        let told = teller.next(look("en", Some(180)), Some(typed), at(20_400)).expect("their use, told");
        assert_eq!(told.idle, Some(0));
        assert_eq!(teller.next(look("en", Some(180)), Some(at(21_000)), at(21_000)), None, "not on every key");
        assert!(teller.next(look("en", Some(180)), Some(at(36_000)), at(36_000)).is_some(), "and again once it's been a while");
        // A change while in use goes as soon as it settles.
        assert_eq!(teller.next(look("ko", Some(180)), Some(at(36_100)), at(36_100)), None);
        assert_eq!(teller.next(look("ko", Some(180)), Some(at(36_100)), at(37_000)).map(|l| l.lang), Some("ko".into()));
        // Not delivered: told again at the next look.
        teller.forget();
        assert!(teller.next(look("ko", Some(180)), None, at(37_100)).is_some());
    }

    #[test]
    fn the_hello_carries_the_look_last_drawn_and_how_long_since_use() {
        keep(look("en", Some(180)));
        used();
        let shown = shown().expect("outside the real view, told");
        assert_eq!((shown.lang.as_str(), shown.sidebar_width, shown.idle), ("en", Some(180), Some(0)));
        assert!(!machine().is_empty());
    }
}
