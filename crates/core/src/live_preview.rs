//! The live preview: a page a member serves on the daemon's machine (a dev
//! server, a comparison board at `http://localhost:4899/…`, a design
//! session's page) shown to the person in a real web view of their own
//! screen — big, sharp at the screen's resolution, and fully theirs to use —
//! instead of a picture of the members' automated browser.
//!
//! Only loopback pages of the daemon's machine are previewed. With the
//! daemon on another Mac, the screen reaches that port through its own SSH
//! link: a forward equipe opens itself, at a local port it picks, and takes
//! down when the preview closes. While a preview is open the member that
//! serves the page is told to keep its server running.
//!
//! Here, with no I/O and no UI: which addresses qualify ([`loopback`]), how
//! a preview moves between its states ([`Preview`]), which local port a
//! forward takes ([`local_port`]), and what the members are told while the
//! person looks ([`Watched`], [`open_note`]).

use std::collections::{BTreeMap, HashMap};

use crate::protocol::SessionId;

/// A loopback page of the daemon's machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub secure: bool,
    pub port: u16,
    /// Path, query and fragment, always starting with `/`.
    pub path: String,
}

/// Why an address isn't previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Not an `http` or `https` address.
    NotWeb,
    /// Not the daemon machine's own loopback (another host, a LAN address).
    NotLoopback,
    /// A user name or password in the address.
    Credentials,
    /// No usable port.
    BadPort,
}

/// The page `url` names, when it is a loopback page of the daemon's machine:
/// `localhost` (and `*.localhost`), `127.0.0.0/8`, `::1`, or `0.0.0.0` (what
/// dev servers print when they listen everywhere — reached on loopback).
pub fn loopback(url: &str) -> Result<Target, Refused> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let (secure, rest_len) = match (lower.strip_prefix("http://"), lower.strip_prefix("https://")) {
        (Some(rest), _) => (false, rest.len()),
        (_, Some(rest)) => (true, rest.len()),
        _ => return Err(Refused::NotWeb),
    };
    let rest = &url[url.len() - rest_len..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return Err(Refused::Credentials);
    }
    let (host, port) = split_host_port(authority)?;
    if !loopback_host(&host) {
        return Err(Refused::NotLoopback);
    }
    let port = port.unwrap_or(if secure { 443 } else { 80 });
    if port == 0 {
        return Err(Refused::BadPort);
    }
    let path = if tail.starts_with('/') { tail.to_owned() } else { format!("/{tail}") };
    Ok(Target { secure, port, path })
}

/// Whether `url` is a page the preview can show.
pub fn previewable(url: &str) -> bool {
    loopback(url).is_ok()
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), Refused> {
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let close = inner.find(']').ok_or(Refused::NotLoopback)?;
        let after = &inner[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port),
            None if after.is_empty() => None,
            None => return Err(Refused::BadPort),
        };
        (&inner[..close], port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = port.map(|port| port.parse::<u16>().map_err(|_| Refused::BadPort)).transpose()?;
    Ok((host.trim_end_matches('.').to_ascii_lowercase(), port))
}

fn loopback_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host == "0.0.0.0" {
        return true;
    }
    host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl Target {
    fn scheme(&self) -> &'static str {
        if self.secure { "https" } else { "http" }
    }

    /// The page reached at `port` of this machine's loopback: the forward's,
    /// or (the daemon on this Mac) the page's own.
    pub fn at(&self, port: u16) -> String {
        format!("{}://localhost:{port}{}", self.scheme(), self.path)
    }

    /// The page as the daemon's machine names it: what the member is told of.
    pub fn page(&self) -> String {
        self.at(self.port)
    }
}

/// The port on this machine a forward to the daemon machine's `remote` port
/// listens at: the same one when it is free here — a page that names its own
/// port (a dev server's live reload) keeps working — else one the system
/// picks (`any`). Never a privileged port.
pub fn local_port(remote: u16, free: impl Fn(u16) -> bool, any: impl FnOnce() -> Option<u16>) -> Option<u16> {
    if remote >= 1024 && free(remote) { Some(remote) } else { any() }
}

/// Where a preview stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The forward to the daemon's machine is being opened.
    Linking,
    /// Asking whether the page answers.
    Checking,
    /// On screen.
    Live,
    /// The page doesn't answer: its server stopped, or never started.
    Down,
    /// The daemon's machine couldn't be reached for the page (no forward).
    Unreachable,
    Closed,
}

/// What the screen does next for a preview. Each answer comes back with the
/// attempt it was asked in: one for an older attempt is ignored (and a
/// forward that came up for nothing is taken down).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Open a forward to the daemon machine's `remote` port: answer with [`Preview::linked`].
    Link { remote: u16 },
    /// Ask whether the page answers at this machine's `port`: answer with [`Preview::checked`].
    Check { port: u16 },
    /// Take the forward at this machine's `port` down.
    Unlink { port: u16 },
    /// Show this address in the web view.
    Load(String),
    Nothing,
}

/// One preview's state, from opening to closing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub target: Target,
    /// The daemon is on another machine: the page is reached through a forward.
    pub remote: bool,
    pub phase: Phase,
    /// The port on this machine the page is reached at: the forward's, or the page's own.
    pub port: Option<u16>,
    /// Bumped at each new attempt and at the close.
    pub attempt: u64,
    /// The member was asked to start the page's server again since it went down.
    pub asked: bool,
}

impl Preview {
    /// A preview of `target` opens: its first step.
    pub fn open(target: Target, remote: bool) -> (Self, Step) {
        let mut preview = Self { target, remote, phase: Phase::Closed, port: None, attempt: 0, asked: false };
        let step = preview.begin();
        (preview, step)
    }

    fn begin(&mut self) -> Step {
        self.attempt += 1;
        match (self.remote, self.port) {
            (true, None) => {
                self.phase = Phase::Linking;
                Step::Link { remote: self.target.port }
            }
            (true, Some(port)) => {
                self.phase = Phase::Checking;
                Step::Check { port }
            }
            (false, _) => {
                let port = self.target.port;
                self.port = Some(port);
                self.phase = Phase::Checking;
                Step::Check { port }
            }
        }
    }

    /// The forward asked for in `attempt` came up at `Ok(port)`, or didn't.
    pub fn linked(&mut self, attempt: u64, result: Result<u16, String>) -> Step {
        if attempt != self.attempt || self.phase != Phase::Linking {
            return match result {
                Ok(port) if Some(port) != self.port => Step::Unlink { port },
                _ => Step::Nothing,
            };
        }
        match result {
            Ok(port) => {
                self.port = Some(port);
                self.phase = Phase::Checking;
                Step::Check { port }
            }
            Err(_) => {
                self.phase = Phase::Unreachable;
                Step::Nothing
            }
        }
    }

    /// Whether the page answered, as asked in `attempt`.
    pub fn checked(&mut self, attempt: u64, up: bool) -> Step {
        if attempt != self.attempt || self.phase != Phase::Checking {
            return Step::Nothing;
        }
        match (up, self.port) {
            (true, Some(port)) => {
                self.phase = Phase::Live;
                self.asked = false;
                Step::Load(self.target.at(port))
            }
            _ => {
                self.phase = Phase::Down;
                Step::Nothing
            }
        }
    }

    /// A look now and then while on screen or down: the page went down, or
    /// came back (its member started it again) and shows at once.
    pub fn watched(&mut self, up: bool) -> Step {
        match (self.phase, up, self.port) {
            (Phase::Live, false, _) => {
                self.phase = Phase::Down;
                Step::Nothing
            }
            (Phase::Down, true, Some(port)) => {
                self.phase = Phase::Live;
                self.asked = false;
                Step::Load(self.target.at(port))
            }
            _ => Step::Nothing,
        }
    }

    /// The port a look now and then goes to: on screen or down only.
    pub fn watch_port(&self) -> Option<u16> {
        if matches!(self.phase, Phase::Live | Phase::Down) { self.port } else { None }
    }

    /// Try again (Reload on a page that is down, or Retry): checked again —
    /// a forward that never came up is asked for again.
    pub fn retry(&mut self) -> Step {
        if self.phase == Phase::Closed {
            return Step::Nothing;
        }
        self.begin()
    }

    /// The person asks the member to start the page again: `true` the first
    /// time while it is down (the message goes once).
    pub fn ask_restart(&mut self) -> bool {
        if self.phase == Phase::Down && !self.asked {
            self.asked = true;
            return true;
        }
        false
    }

    /// The preview closes: its forward, if it has one, goes down.
    pub fn close(&mut self) -> Step {
        let was = std::mem::replace(&mut self.phase, Phase::Closed);
        self.attempt += 1;
        match (was, self.remote, self.port.take()) {
            (Phase::Closed, _, _) => Step::Nothing,
            (_, true, Some(port)) => Step::Unlink { port },
            _ => Step::Nothing,
        }
    }
}

// ── what the members are told ──

/// How long a preview counts as open without being closed (a screen that
/// went away without saying so).
pub const OPEN_FOR_SECS: u64 = 8 * 3600;

/// The pages each member's conversation has open in the person's previews:
/// by address, how many screens show it and since when.
#[derive(Debug, Default, Clone)]
pub struct Watched {
    pages: HashMap<SessionId, BTreeMap<String, (u32, u64)>>,
}

impl Watched {
    /// A screen opened `page` of `session`'s member: `true` when it is news
    /// to the member (no screen had it open).
    pub fn open(&mut self, session: SessionId, page: &str, now: u64) -> bool {
        let pages = self.pages.entry(session).or_default();
        let entry = pages.entry(page.to_owned()).or_insert((0, now));
        let fresh = entry.0 == 0 || now.saturating_sub(entry.1) > OPEN_FOR_SECS;
        if fresh {
            *entry = (0, now);
        }
        entry.0 += 1;
        fresh
    }

    /// A screen closed it: `true` when no screen has it open any more.
    pub fn close(&mut self, session: SessionId, page: &str) -> bool {
        let Some(pages) = self.pages.get_mut(&session) else { return false };
        let Some(entry) = pages.get_mut(page) else { return false };
        entry.0 = entry.0.saturating_sub(1);
        let gone = entry.0 == 0;
        if gone {
            pages.remove(page);
        }
        if pages.is_empty() {
            self.pages.remove(&session);
        }
        gone
    }

    /// The member's conversation is gone.
    pub fn forget(&mut self, session: SessionId) {
        self.pages.remove(&session);
    }

    /// The pages open for `session`'s member now.
    pub fn open_now(&self, session: SessionId, now: u64) -> Vec<String> {
        self.pages.get(&session).map(|pages| pages.iter().filter(|(_, (n, since))| *n > 0 && now.saturating_sub(*since) <= OPEN_FOR_SECS).map(|(page, _)| page.clone()).collect()).unwrap_or_default()
    }
}

/// What a member's turn opens with while the person has its pages open.
pub fn open_note(pages: &[String]) -> Option<String> {
    if pages.is_empty() {
        return None;
    }
    Some(format!(
        "the person has {} open in equipe's live preview right now — keep the server behind it running (same port) until you are told they closed it; if it stopped, start it again",
        pages.join(", ")
    ))
}

/// What a member hears when the person opens one of its pages.
pub fn opened_line(page: &str) -> String {
    format!("The person opened {page} in equipe's live preview: keep its server running (same port) until you are told they closed it.")
}

/// What a member hears when the person closed the last preview of its page.
pub fn closed_line(page: &str) -> String {
    format!("The person closed the live preview of {page}: its server may stop once your work no longer needs it.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_pages_of_the_daemons_machine_are_previewed() {
        let t = loopback("http://localhost:4899/board?x=1#top").unwrap();
        assert_eq!((t.secure, t.port, t.path.as_str()), (false, 4899, "/board?x=1#top"));
        assert_eq!(loopback("http://127.0.0.1:3000").unwrap().path, "/");
        assert_eq!(loopback("HTTP://LOCALHOST:5173/").unwrap().port, 5173);
        assert_eq!(loopback("http://[::1]:8080/a").unwrap().port, 8080);
        assert_eq!(loopback("http://0.0.0.0:3000/").unwrap().page(), "http://localhost:3000/");
        assert_eq!(loopback("http://app.localhost:3000/").unwrap().port, 3000);
        assert_eq!(loopback("https://localhost:8443/").unwrap().page(), "https://localhost:8443/");
        assert_eq!(loopback("http://localhost/").unwrap().port, 80);
        assert_eq!(loopback("http://127.0.0.2:9000/").unwrap().port, 9000);
        assert_eq!(loopback("  http://localhost:4899?q  ").unwrap().path, "/?q");

        assert_eq!(loopback("https://example.com/"), Err(Refused::NotLoopback));
        assert_eq!(loopback("http://192.168.1.10:3000/"), Err(Refused::NotLoopback));
        assert_eq!(loopback("http://10.0.0.2:3000/"), Err(Refused::NotLoopback));
        assert_eq!(loopback("http://localhost.example.com:3000/"), Err(Refused::NotLoopback));
        assert_eq!(loopback("http://[fe80::1]:3000/"), Err(Refused::NotLoopback));
        assert_eq!(loopback("file:///Users/me/index.html"), Err(Refused::NotWeb));
        assert_eq!(loopback("javascript:alert(1)"), Err(Refused::NotWeb));
        assert_eq!(loopback("ws://localhost:3000/"), Err(Refused::NotWeb));
        assert_eq!(loopback("http://user:pw@localhost:3000/"), Err(Refused::Credentials));
        assert_eq!(loopback("http://evil.com@localhost:3000/"), Err(Refused::Credentials));
        assert_eq!(loopback("http://localhost:99999/"), Err(Refused::BadPort));
        assert_eq!(loopback("http://localhost:0/"), Err(Refused::BadPort));
        assert_eq!(loopback("http://localhost:abc/"), Err(Refused::BadPort));
        assert!(previewable("http://localhost:4899/") && !previewable("https://github.com/"));
    }

    #[test]
    fn a_forward_keeps_the_pages_own_port_when_it_is_free_here() {
        assert_eq!(local_port(4899, |_| true, || Some(50001)), Some(4899));
        assert_eq!(local_port(4899, |_| false, || Some(50001)), Some(50001), "taken here: one the system picks");
        assert_eq!(local_port(80, |_| true, || Some(50002)), Some(50002), "never a privileged port");
        assert_eq!(local_port(3000, |_| false, || None), None);
    }

    #[test]
    fn a_page_on_this_mac_is_checked_then_shown() {
        let (mut p, step) = Preview::open(loopback("http://localhost:4899/b").unwrap(), false);
        assert_eq!(step, Step::Check { port: 4899 });
        assert_eq!(p.phase, Phase::Checking);
        assert_eq!(p.checked(p.attempt, true), Step::Load("http://localhost:4899/b".into()));
        assert_eq!(p.phase, Phase::Live);
        assert_eq!(p.watch_port(), Some(4899));
        assert_eq!(p.close(), Step::Nothing, "no forward to take down");
        assert_eq!(p.phase, Phase::Closed);
    }

    #[test]
    fn a_page_that_is_down_says_so_asks_once_and_comes_back_by_itself() {
        let (mut p, _) = Preview::open(loopback("http://localhost:4899/").unwrap(), false);
        assert_eq!(p.checked(p.attempt, false), Step::Nothing);
        assert_eq!(p.phase, Phase::Down);
        assert!(p.ask_restart(), "the member is asked");
        assert!(!p.ask_restart(), "once");
        assert!(p.asked);
        assert_eq!(p.watched(false), Step::Nothing);
        assert_eq!(p.watched(true), Step::Load("http://localhost:4899/".into()), "started again: shown at once");
        assert_eq!((p.phase, p.asked), (Phase::Live, false));
        assert_eq!(p.watched(false), Step::Nothing);
        assert_eq!(p.phase, Phase::Down, "stopped while on screen");
        assert_eq!(p.retry(), Step::Check { port: 4899 });
        assert_eq!(p.phase, Phase::Checking);
    }

    /// A stand-in for the SSH forward: what it was asked to open and close.
    #[derive(Default)]
    struct FakeForward {
        up: Vec<u16>,
        next: u16,
        fail: bool,
    }

    impl FakeForward {
        /// Carry out one step as the screen would; a check answers with `answer`.
        fn run(&mut self, p: &mut Preview, step: Step, answer: bool) -> Vec<Step> {
            let mut done = vec![step.clone()];
            let next = match step {
                Step::Link { .. } if self.fail => p.linked(p.attempt, Err("refused".into())),
                Step::Link { remote } => {
                    let port = local_port(remote, |port| !self.up.contains(&port), || Some(self.next)).unwrap();
                    self.up.push(port);
                    p.linked(p.attempt, Ok(port))
                }
                Step::Check { .. } => p.checked(p.attempt, answer),
                Step::Unlink { port } => {
                    self.up.retain(|&u| u != port);
                    Step::Nothing
                }
                Step::Load(_) | Step::Nothing => return done,
            };
            done.extend(self.run(p, next, answer));
            done
        }
    }

    #[test]
    fn a_far_page_is_reached_through_a_forward_taken_down_when_the_preview_closes() {
        let mut fwd = FakeForward { next: 50123, ..Default::default() };
        let (mut p, step) = Preview::open(loopback("http://localhost:4899/board").unwrap(), true);
        assert_eq!(p.phase, Phase::Linking);
        let steps = fwd.run(&mut p, step, true);
        assert_eq!(steps, vec![Step::Link { remote: 4899 }, Step::Check { port: 4899 }, Step::Load("http://localhost:4899/board".into())]);
        assert_eq!(fwd.up, vec![4899]);
        // Reloading checks through the same forward: no second one.
        let step = p.retry();
        assert_eq!(step, Step::Check { port: 4899 });
        fwd.run(&mut p, step, true);
        assert_eq!(fwd.up, vec![4899]);
        let step = p.close();
        fwd.run(&mut p, step, true);
        assert!(fwd.up.is_empty(), "closed with the preview");
        assert_eq!(p.close(), Step::Nothing, "closing twice takes nothing down twice");
    }

    #[test]
    fn a_far_page_whose_port_is_taken_here_gets_another_and_shows_there() {
        let mut fwd = FakeForward { up: vec![3000], next: 50200, ..Default::default() };
        let (mut p, step) = Preview::open(loopback("http://127.0.0.1:3000/x").unwrap(), true);
        let steps = fwd.run(&mut p, step, true);
        assert_eq!(steps.last(), Some(&Step::Load("http://localhost:50200/x".into())));
        let step = p.close();
        fwd.run(&mut p, step, true);
        assert_eq!(fwd.up, vec![3000], "only its own forward goes");
    }

    #[test]
    fn a_forward_that_comes_up_after_the_preview_closed_is_taken_down() {
        let (mut p, step) = Preview::open(loopback("http://localhost:4899/").unwrap(), true);
        assert_eq!(step, Step::Link { remote: 4899 });
        let attempt = p.attempt;
        assert_eq!(p.close(), Step::Nothing, "nothing up yet");
        assert_eq!(p.linked(attempt, Ok(4899)), Step::Unlink { port: 4899 });
        assert_eq!(p.phase, Phase::Closed);
        // A late check answer changes nothing either.
        assert_eq!(p.checked(attempt, true), Step::Nothing);
        assert_eq!(p.retry(), Step::Nothing);
    }

    #[test]
    fn no_forward_is_unreachable_and_try_again_asks_for_one_again() {
        let mut fwd = FakeForward { fail: true, ..Default::default() };
        let (mut p, step) = Preview::open(loopback("http://localhost:4899/").unwrap(), true);
        fwd.run(&mut p, step, true);
        assert_eq!(p.phase, Phase::Unreachable);
        assert_eq!(p.watch_port(), None);
        fwd.fail = false;
        fwd.next = 50300;
        let step = p.retry();
        assert_eq!(step, Step::Link { remote: 4899 });
        fwd.run(&mut p, step, true);
        assert_eq!(p.phase, Phase::Live);
    }

    #[test]
    fn the_member_hears_of_its_page_once_per_opening_and_once_when_all_closed() {
        let mut w = Watched::default();
        assert!(w.open(7, "http://localhost:4899/", 100), "news");
        assert!(!w.open(7, "http://localhost:4899/", 110), "a second screen: not news");
        assert_eq!(w.open_now(7, 120), vec!["http://localhost:4899/".to_owned()]);
        assert!(!w.close(7, "http://localhost:4899/"), "one screen still has it");
        assert!(w.close(7, "http://localhost:4899/"), "the last one closed");
        assert!(w.open_now(7, 130).is_empty());
        assert!(!w.close(7, "http://localhost:4899/"), "closing what isn't open is no news");
        w.open(8, "http://localhost:3000/", 0);
        assert!(w.open_now(8, OPEN_FOR_SECS + 1).is_empty(), "a screen that went away without closing stops counting");
        assert!(w.open(8, "http://localhost:3000/", OPEN_FOR_SECS + 2), "and opening again is news again");
        w.forget(8);
        assert!(w.open_now(8, OPEN_FOR_SECS + 3).is_empty());
        assert_eq!(open_note(&[]), None);
        assert!(open_note(&["http://localhost:4899/".into()]).unwrap().contains("keep the server behind it running"));
    }
}
