//! The browser panel's rules, with no UI types (`views/browser.rs` draws
//! them): which picture may show for the tab picked, how tabs group and read,
//! who has a browser in hand and what the control bar says of it, the address
//! as shown, and when the page is asked to take the panel's size. Tested
//! without a window.

use std::time::{Duration, Instant};

use equipe_core::protocol::stream::FrameHeader;
use equipe_core::protocol::{BrowserPage, HumanCheck, SessionId};

// ── the picture ──

/// The picture on screen, kept to the tab picked: a frame shows only if it
/// is of that tab, and picking another tab shows nothing (a skeleton) until
/// that tab's own first frame — never the last tab's, never another's.
#[derive(Debug)]
pub struct Picture<T> {
    page: Option<u64>,
    frame: Option<(FrameHeader, T)>,
}

impl<T> Default for Picture<T> {
    fn default() -> Self {
        Self { page: None, frame: None }
    }
}

impl<T> Picture<T> {
    /// The tab picked now. What showed goes, handed back to be let go of.
    pub fn pick(&mut self, page: Option<u64>) -> Option<T> {
        if self.page == page {
            return None;
        }
        self.page = page;
        self.frame.take().map(|(_, image)| image)
    }

    /// A frame came. `Ok(old)`: it shows, and the one it replaces is handed
    /// back; `Err(image)`: it is of another tab (one picked before, or a page
    /// `listed` no longer knows), handed back unshown.
    pub fn offer(&mut self, header: FrameHeader, image: T, listed: bool) -> Result<Option<T>, T> {
        if self.page != Some(header.page) || !listed {
            return Err(image);
        }
        // An older frame than the one showing (out of order across a restart) waits its turn: dropped.
        if self.frame.as_ref().is_some_and(|(h, _)| h.seq > header.seq) {
            return Err(image);
        }
        Ok(self.frame.replace((header, image)).map(|(_, image)| image))
    }

    pub fn frame(&self) -> Option<&(FrameHeader, T)> {
        self.frame.as_ref()
    }

    /// A tab picked whose first frame hasn't come yet.
    pub fn waiting(&self) -> bool {
        self.page.is_some() && self.frame.is_none()
    }

    /// Everything shown goes (the panel closed, the line dropped).
    pub fn clear(&mut self) -> Option<T> {
        self.frame.take().map(|(_, image)| image)
    }
}

// ── tabs ──

/// One lane of the shared Chrome as the strip shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRef<'a> {
    pub lane: &'a str,
    pub session: Option<SessionId>,
    /// The label it goes by (a member's name; empty for the person's own).
    pub name: &'a str,
}

/// The strip's order: the browser of the conversation on screen first, then
/// the other members' by name, the person's own (`""`) last.
pub fn order<'a>(mut lanes: Vec<LaneRef<'a>>, viewed: Option<SessionId>) -> Vec<LaneRef<'a>> {
    lanes.sort_by(|a, b| {
        let key = |l: &LaneRef| (l.lane.is_empty(), viewed.is_none() || l.session != viewed, l.name.to_lowercase(), l.lane.to_owned());
        key(a).cmp(&key(b))
    });
    lanes
}

/// Titles that say nothing of the page: a wall before it, or a page on its
/// way. The tab reads as its site instead.
const NOTHING_TITLES: &[&str] = &[
    "just a moment",
    "잠시만 기다리십시오",
    "잠시만 기다려 주십시오",
    "잠시만 기다려주세요",
    "un instant",
    "einen moment",
    "un momento",
    "しばらくお待ちください",
    "请稍候",
    "請稍候",
    "attention required! | cloudflare",
    "loading",
    "redirecting",
    "please wait",
    "untitled",
    "about:blank",
];

/// What a tab reads as: its title, or its site where the title says nothing
/// (none yet, an address, a wall's "Just a moment…").
pub fn tab_title(page: &BrowserPage) -> String {
    let title = page.title.trim();
    let bare = title.trim_end_matches(['.', '…', ' ']).to_lowercase();
    let says_nothing = title.is_empty()
        || title.starts_with("http://")
        || title.starts_with("https://")
        || page.check == Some(HumanCheck::Challenge)
        || (bare.chars().count() <= 40 && NOTHING_TITLES.iter().any(|t| bare.starts_with(t)));
    if says_nothing { site(&page.url).unwrap_or_default() } else { title.to_owned() }
}

/// The site of an address, as a person names it (`dash.cloudflare.com`);
/// `None` for a blank page.
pub fn site(url: &str) -> Option<String> {
    if url.is_empty() || url == "about:blank" {
        return None;
    }
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    Some(host.strip_prefix("www.").unwrap_or(host).to_owned())
}

/// An address as the field shows it out of focus: whether it is secure, the
/// site large, the rest quiet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub secure: bool,
    pub host: String,
    pub rest: String,
}

pub fn address(url: &str) -> Option<Address> {
    if url.is_empty() || url == "about:blank" {
        return None;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Some(Address { secure: false, host: url.to_owned(), rest: String::new() });
    };
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(cut);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let tail = if tail == "/" { "" } else { tail };
    Some(Address { secure: scheme.eq_ignore_ascii_case("https"), host: host.strip_prefix("www.").unwrap_or(host).to_owned(), rest: tail.to_owned() })
}

// ── whose hand ──

/// Who has a member's browser, as the panel knows it — a small state
/// machine fed by the daemon's events and the person's own acts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hand {
    /// In the person's hands: its member's browser tools wait.
    pub held: bool,
    /// Its member asked the person in, for this.
    pub asked: Option<String>,
    /// Put in their hands by equipe for a page that waits for a person.
    pub check: Option<HumanCheck>,
    /// That page doesn't wait any more: what is left is to give it back.
    pub passed: bool,
    /// The person put a hand on it (took it, clicked, typed).
    pub touched: bool,
}

/// What the control bar (and the banner over the picture) says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The person's own tab: always theirs.
    Mine,
    /// Its member has it; `working`, it is at work now.
    Member { working: bool },
    /// The person has it.
    Person,
    /// Its member asked the person in, for `why`.
    Asked(String),
    /// A page waits for the person; `touched`, they are on it.
    Waiting { check: HumanCheck, touched: bool },
    /// The page that waited passed: give it back.
    Passed,
}

impl Hand {
    /// The daemon says who has it now.
    pub fn on_control(&mut self, by_person: bool, why: Option<String>, check: Option<HumanCheck>) {
        if !by_person {
            *self = Hand::default();
            return;
        }
        self.held = true;
        if why.is_some() {
            self.asked = why;
        }
        if check.is_some() {
            self.check = check;
            self.passed = false;
        }
    }

    /// What the lane's page waits for now, as its tabs were told.
    pub fn on_page(&mut self, check: Option<HumanCheck>) {
        match check {
            Some(c) if self.check.is_some() => {
                self.check = Some(c);
                self.passed = false;
            }
            None if self.held && self.check.is_some() => self.passed = true,
            _ => {}
        }
    }

    /// The person takes it (직접 하기, or a hand on the page).
    pub fn take(&mut self) {
        self.held = true;
        self.touched = true;
    }

    /// The person gives it back (돌려주기, Esc).
    pub fn give_back(&mut self) {
        *self = Hand::default();
    }

    /// "Keep using it": past the check, the person stays on.
    pub fn keep(&mut self) {
        self.check = None;
        self.passed = false;
        self.touched = true;
    }

    /// What the bar says: `page_check`, what the page on screen waits for.
    pub fn mode(&self, page_check: Option<HumanCheck>, working: bool) -> Mode {
        if !self.held {
            return match page_check {
                Some(check) => Mode::Waiting { check, touched: false },
                None => Mode::Member { working },
            };
        }
        if let Some(why) = &self.asked {
            return Mode::Asked(why.clone());
        }
        if self.passed && page_check.is_none() {
            return Mode::Passed;
        }
        match page_check.or(if self.passed { None } else { self.check }) {
            Some(check) => Mode::Waiting { check, touched: self.touched },
            None => Mode::Person,
        }
    }
}

// ── a browser handed to the person ──

/// What a lane put in the person's hands was handed over for, if it was —
/// its member's words, or the check its page waits for; `None` for the
/// person taking it themselves, or giving it back.
pub fn handed_for(by_person: bool, why: Option<&str>, check: Option<HumanCheck>) -> Option<String> {
    if !by_person {
        return None;
    }
    why.map(str::to_owned).or_else(|| check.map(|c| format!("{c:?}")))
}

/// Where the panel opens for a hand-over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    /// On this tab of the lane, the person driving.
    Now(String, u64),
    /// Its tabs aren't told yet (the lane just opened): when they are.
    Later,
}

/// The members' browsers handed to the person that a panel opened itself
/// for — every screen's panel does, on the tab its member works in — once
/// each: the same hand-over told again (a screen attaching again) doesn't
/// open it again after the person closed the panel.
#[derive(Debug, Default)]
pub struct Handovers {
    opened: std::collections::HashSet<(String, String)>,
    waiting: Option<String>,
}

impl Handovers {
    /// The daemon says who has `lane` now. `page`: the tab its member works
    /// in, if the lane's tabs are known.
    pub fn on_control(&mut self, lane: &str, by_person: bool, why: Option<&str>, check: Option<HumanCheck>, page: Option<u64>) -> Option<Open> {
        let Some(what) = handed_for(by_person, why, check) else {
            if !by_person {
                self.given_back(lane);
            }
            return None;
        };
        if lane.is_empty() || !self.opened.insert((lane.to_owned(), what)) {
            return None;
        }
        Some(match page {
            Some(page) => Open::Now(lane.to_owned(), page),
            None => {
                self.waiting = Some(lane.to_owned());
                Open::Later
            }
        })
    }

    /// A lane's tabs told (`page`, the tab its member works in): a hand-over
    /// waiting for them opens now.
    pub fn on_tabs(&mut self, lane: &str, page: Option<u64>) -> Option<(String, u64)> {
        let page = page.filter(|_| self.waiting.as_deref() == Some(lane))?;
        self.waiting = None;
        Some((lane.to_owned(), page))
    }

    /// Back to its member (here, or on another screen): a next hand-over opens again.
    pub fn given_back(&mut self, lane: &str) {
        self.opened.retain(|(l, _)| l != lane);
        if self.waiting.as_deref() == Some(lane) {
            self.waiting = None;
        }
    }
}

/// Whether a wheel over the page goes to it: always on the person's own tab;
/// on a member's only once it is in their hands — a wheel never takes it.
pub fn wheel_goes(members: bool, held: bool) -> bool {
    !members || held
}

/// Whether closing a tab asks first: a member's, while it is at work on it.
/// The person's own tabs, and an idle member's, close at once.
pub fn close_asks(members: bool, working: bool) -> bool {
    members && working
}

// ── the page's size ──

/// The box the page shows in, as the stream is asked for it: CSS pixels and
/// the screen's scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Boxed {
    pub w: u32,
    pub h: u32,
    pub scale: f32,
}

impl Boxed {
    /// The device pixels the picture needs at most to be sharp.
    pub fn device(&self) -> (u32, u32) {
        let px = |v: u32| ((v as f32 * self.scale).round() as u32).clamp(64, 4096);
        (px(self.w), px(self.h))
    }
}

/// How long a new size holds before the page is asked to take it: a drag of
/// the divider asks once, at its end.
pub const SETTLE: Duration = Duration::from_millis(250);

/// A size asked for only once it has held still for [`SETTLE`] — the first
/// at once.
#[derive(Debug, Clone, Default)]
pub struct Settle {
    seen: Option<(Boxed, Instant)>,
    sent: Option<Boxed>,
}

impl Settle {
    /// The box as drawn now; `Some` when it is time to ask for it.
    pub fn see(&mut self, size: Boxed, now: Instant) -> Option<Boxed> {
        if self.sent == Some(size) {
            self.seen = None;
            return None;
        }
        let since = match self.seen {
            Some((s, at)) if s == size => at,
            _ => {
                self.seen = Some((size, now));
                now
            }
        };
        if self.sent.is_none() || now.duration_since(since) >= SETTLE {
            self.sent = Some(size);
            self.seen = None;
            return Some(size);
        }
        None
    }

    /// How long until the size seen may be asked for.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.seen.map(|(_, at)| SETTLE.saturating_sub(now.duration_since(at)))
    }

    /// The size last asked for.
    pub fn sent(&self) -> Option<Boxed> {
        self.sent
    }

    /// Asked again from nothing (a new line).
    pub fn reset(&mut self) {
        *self = Settle::default();
    }
}

/// A watched page whose frames stopped this long is said to be stuck, with a
/// way to try again.
pub const STALL: Duration = Duration::from_secs(8);

// ── the pointer ──

/// The person's pointer as it goes to the page, in the page's CSS pixels: a
/// move no more often than `every`, the last of a burst never lost (it goes
/// once its time comes), and before a press, a move to the press's own point
/// if the page last saw the pointer elsewhere — as a real mouse always moves
/// to where it presses, and the page's hover follows it. Nothing here makes
/// anything up: every point sent is one the person's pointer was at.
#[derive(Debug, Clone)]
pub struct Pointer {
    every: Duration,
    /// The last point the page was sent, and when.
    sent: Option<((f64, f64), Instant)>,
    /// A move held back by the pace, to go when its time comes.
    held: Option<(f64, f64)>,
}

impl Pointer {
    pub fn new(every: Duration) -> Self {
        Self { every, sent: None, held: None }
    }

    /// The pointer moved to `at`: `Some` when the move goes now.
    pub fn moved(&mut self, at: (f64, f64), now: Instant) -> Option<(f64, f64)> {
        if self.sent.is_some_and(|(p, _)| p == at) {
            self.held = None;
            return None;
        }
        match self.sent {
            Some((_, when)) if now.duration_since(when) < self.every => {
                self.held = Some(at);
                None
            }
            _ => {
                self.held = None;
                self.sent = Some((at, now));
                Some(at)
            }
        }
    }

    /// How long until a held-back move may go.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        let (_, when) = self.sent?;
        self.held.map(|_| self.every.saturating_sub(now.duration_since(when)))
    }

    /// The held-back move, if its time has come.
    pub fn due(&mut self, now: Instant) -> Option<(f64, f64)> {
        let at = self.held?;
        if self.wait(now).is_some_and(|w| !w.is_zero()) {
            return None;
        }
        self.held = None;
        self.sent = Some((at, now));
        Some(at)
    }

    /// A button goes down (or up) at `at`: the move to send first, if the page
    /// last saw the pointer anywhere else. A press is never held back.
    pub fn pressed(&mut self, at: (f64, f64), now: Instant) -> Option<(f64, f64)> {
        self.held = None;
        let there = self.sent.is_some_and(|(p, _)| p == at);
        self.sent = Some((at, now));
        (!there).then_some(at)
    }

    /// The pointer left the picture: where it was is no longer known.
    pub fn left(&mut self) {
        self.sent = None;
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(page: u64, seq: u64) -> FrameHeader {
        FrameHeader { page, seq, w: 10, h: 10, css_w: 10., css_h: 10., sx: 0., sy: 0., top: 0. }
    }

    #[test]
    fn a_tab_only_ever_shows_its_own_frames() {
        let mut p: Picture<&str> = Picture::default();
        p.pick(Some(1));
        assert!(p.waiting(), "a skeleton until its first frame");
        assert_eq!(p.offer(header(1, 1), "mobile-1", true), Ok(None));
        assert_eq!(p.frame().map(|f| f.1), Some("mobile-1"));
        // Another tab picked: nothing of the last shows meanwhile.
        assert_eq!(p.pick(Some(2)), Some("mobile-1"));
        assert!(p.waiting());
        assert_eq!(p.offer(header(1, 2), "mobile-2", true), Err("mobile-2"), "the last tab's late frame");
        assert_eq!(p.offer(header(3, 1), "backend-1", true), Err("backend-1"), "another tab's");
        assert!(p.frame().is_none());
        assert_eq!(p.offer(header(2, 1), "cf-1", true), Ok(None));
        assert_eq!(p.offer(header(2, 2), "cf-2", true), Ok(Some("cf-1")));
        assert_eq!(p.offer(header(2, 1), "cf-old", true), Err("cf-old"), "older than what shows");
        // Picking the same tab again keeps what shows.
        assert_eq!(p.pick(Some(2)), None);
        assert_eq!(p.frame().map(|f| f.1), Some("cf-2"));
    }

    #[test]
    fn a_frame_of_a_page_no_longer_listed_never_shows() {
        // The browser started again: an id the screen still holds names no tab of it.
        let mut p: Picture<&str> = Picture::default();
        p.pick(Some(5));
        assert_eq!(p.offer(header(5, 1), "someone-elses", false), Err("someone-elses"));
        assert!(p.waiting());
    }

    #[test]
    fn the_strip_starts_with_the_conversations_browser_and_ends_with_mine() {
        let lanes = vec![
            LaneRef { lane: "", session: None, name: "" },
            LaneRef { lane: "acme/mobile", session: Some(7), name: "mobile" },
            LaneRef { lane: "acme/backend", session: Some(3), name: "backend" },
            LaneRef { lane: "acme/designer", session: Some(4), name: "designer" },
        ];
        let lanes_of = |v: Vec<LaneRef<'static>>| v.into_iter().map(|l| l.lane).collect::<Vec<_>>();
        assert_eq!(lanes_of(order(lanes.clone(), Some(7))), ["acme/mobile", "acme/backend", "acme/designer", ""]);
        assert_eq!(lanes_of(order(lanes, None)), ["acme/backend", "acme/designer", "acme/mobile", ""]);
    }

    #[test]
    fn a_tab_reads_as_its_site_when_its_title_says_nothing() {
        let page = |url: &str, title: &str| BrowserPage { id: 1, url: url.into(), title: title.into(), ..Default::default() };
        assert_eq!(tab_title(&page("https://appstoreconnect.apple.com/apps", "App Store Connect")), "App Store Connect");
        assert_eq!(tab_title(&page("https://dash.cloudflare.com/", "Just a moment...")), "dash.cloudflare.com");
        assert_eq!(tab_title(&page("https://dash.cloudflare.com/", "잠시만 기다리십시오…")), "dash.cloudflare.com");
        assert_eq!(tab_title(&page("https://www.vercel.com/new", "")), "vercel.com");
        assert_eq!(tab_title(&page("https://x.com/a", "https://x.com/a")), "x.com");
        let walled = BrowserPage { check: Some(HumanCheck::Challenge), ..page("https://shop.test/", "Shop") };
        assert_eq!(tab_title(&walled), "shop.test");
        assert_eq!(tab_title(&page("https://news.test/", "Loading the future: a long essay about where the web goes next")), "Loading the future: a long essay about where the web goes next");
    }

    #[test]
    fn an_address_shows_its_site_large_and_says_whether_it_is_secure() {
        assert_eq!(
            address("https://www.appstoreconnect.apple.com/apps/123?x=1"),
            Some(Address { secure: true, host: "appstoreconnect.apple.com".into(), rest: "/apps/123?x=1".into() })
        );
        assert_eq!(address("http://localhost:3000/"), Some(Address { secure: false, host: "localhost:3000".into(), rest: String::new() }));
        assert_eq!(address("about:blank"), None);
        assert_eq!(site("https://user@dash.cloudflare.com/x").as_deref(), Some("dash.cloudflare.com"));
    }

    #[test]
    fn a_member_drives_until_the_person_takes_it_and_gets_it_back() {
        let mut h = Hand::default();
        assert_eq!(h.mode(None, true), Mode::Member { working: true });
        h.take();
        assert_eq!(h.mode(None, true), Mode::Person);
        h.give_back();
        assert_eq!(h.mode(None, false), Mode::Member { working: false });
        // The daemon says the person has it (they touched the page): the same.
        h.on_control(true, None, None);
        assert_eq!(h.mode(None, true), Mode::Person);
        h.on_control(false, None, None);
        assert_eq!(h, Hand::default());
    }

    #[test]
    fn a_member_asking_the_person_in_says_what_for() {
        let mut h = Hand::default();
        h.on_control(true, Some("로그인해 주세요".into()), None);
        assert_eq!(h.mode(None, true), Mode::Asked("로그인해 주세요".into()));
        // Their hand on it doesn't forget why.
        h.on_control(true, None, None);
        assert_eq!(h.mode(None, true), Mode::Asked("로그인해 주세요".into()));
    }

    /// What every attached screen's panel does with the daemon's word that
    /// a member handed the person its browser: each runs this, so each opens.
    #[test]
    fn a_hand_over_opens_the_panel_on_its_members_tab_in_the_persons_hands_once() {
        let why = "Umbriles Inc. 계정 소유자 Apple ID로 로그인해 주세요";
        let screens = [Handovers::default(), Handovers::default()];
        for mut panel in screens {
            assert_eq!(panel.on_control("acme/pm", true, Some(why), None, Some(7)), Some(Open::Now("acme/pm".into(), 7)), "on the tab pm works in");
            // The person driving it, its banner saying what pm asked.
            let mut hand = Hand::default();
            hand.on_control(true, Some(why.into()), None);
            hand.take();
            assert_eq!(hand.mode(None, false), Mode::Asked(why.into()));
            // Told again (this screen attached again), with or without the words: not opened again.
            assert_eq!(panel.on_control("acme/pm", true, Some(why), None, Some(7)), None);
            assert_eq!(panel.on_control("acme/pm", true, None, None, Some(7)), None, "the host saying the person has it");
            // Given back — here or on another screen — the next hand-over opens again.
            assert_eq!(panel.on_control("acme/pm", false, None, None, Some(7)), None);
            assert_eq!(panel.on_control("acme/pm", true, Some(why), None, Some(7)), Some(Open::Now("acme/pm".into(), 7)));
        }
        // Handed over as its lane opens, before its tabs are told: when they are.
        let mut panel = Handovers::default();
        assert_eq!(panel.on_control("acme/qa", true, Some("2단계 코드를 넣어 주세요"), None, None), Some(Open::Later));
        assert_eq!(panel.on_tabs("acme/pm", Some(3)), None, "another lane's tabs");
        assert_eq!(panel.on_tabs("acme/qa", Some(9)), Some(("acme/qa".into(), 9)));
        assert_eq!(panel.on_tabs("acme/qa", Some(9)), None, "once");
        // A page waiting for a person opens it too; the person taking a tab themselves doesn't.
        assert!(matches!(panel.on_control("acme/backend", true, None, Some(HumanCheck::SignIn), Some(4)), Some(Open::Now(_, 4))));
        assert_eq!(panel.on_control("acme/frontend", true, None, None, Some(5)), None);
        assert_eq!(panel.on_control("", true, Some("x"), None, Some(1)), None, "the person's own tabs");
    }

    #[test]
    fn a_wheel_never_takes_a_members_page_and_closing_one_at_work_asks_first() {
        assert!(wheel_goes(false, false), "the person's own tab scrolls");
        assert!(!wheel_goes(true, false), "a member's it drives: no — a click takes it");
        assert!(wheel_goes(true, true), "in their hands: it scrolls");
        assert!(close_asks(true, true), "a member at work: asked once");
        assert!(!close_asks(true, false) && !close_asks(false, true), "an idle member's, or the person's own: at once");
    }

    #[test]
    fn a_page_waiting_for_a_person_asks_them_then_offers_to_give_it_back() {
        let mut h = Hand::default();
        // Seen before equipe pauses its member: the banner shows already.
        assert_eq!(h.mode(Some(HumanCheck::Challenge), true), Mode::Waiting { check: HumanCheck::Challenge, touched: false });
        h.on_control(true, None, Some(HumanCheck::Challenge));
        assert_eq!(h.mode(Some(HumanCheck::Challenge), false), Mode::Waiting { check: HumanCheck::Challenge, touched: false });
        h.take();
        assert_eq!(h.mode(Some(HumanCheck::Challenge), false), Mode::Waiting { check: HumanCheck::Challenge, touched: true });
        // The challenge becomes a CAPTCHA, then passes.
        h.on_page(Some(HumanCheck::Captcha));
        assert_eq!(h.mode(Some(HumanCheck::Captcha), false), Mode::Waiting { check: HumanCheck::Captcha, touched: true });
        h.on_page(None);
        assert_eq!(h.mode(None, false), Mode::Passed);
        // Kept on: the person's own now.
        let mut kept = h.clone();
        kept.keep();
        assert_eq!(kept.mode(None, false), Mode::Person);
        // Given back: the member's.
        h.give_back();
        assert_eq!(h.mode(None, true), Mode::Member { working: true });
        // Passed with no hand on it: the daemon gives it back itself.
        let mut quiet = Hand::default();
        quiet.on_control(true, None, Some(HumanCheck::Challenge));
        quiet.on_page(None);
        quiet.on_control(false, None, None);
        assert_eq!(quiet.mode(None, true), Mode::Member { working: true });
    }

    #[test]
    fn the_pointer_goes_at_a_pace_and_never_loses_where_it_stopped() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut p = Pointer::new(Duration::from_millis(16));
        assert_eq!(p.moved((10., 10.), t0), Some((10., 10.)), "the first move at once");
        assert_eq!(p.moved((10., 10.), ms(30)), None, "the same point again: nothing");
        assert_eq!(p.moved((12., 10.), ms(40)), Some((12., 10.)));
        // A burst within the pace: held back, only the latest kept.
        assert_eq!(p.moved((13., 11.), ms(44)), None);
        assert_eq!(p.moved((15., 12.), ms(50)), None);
        assert_eq!(p.wait(ms(50)), Some(Duration::from_millis(6)));
        assert_eq!(p.due(ms(52)), None, "not yet");
        // The pointer stopped: where it stopped still goes, once its time comes.
        assert_eq!(p.due(ms(56)), Some((15., 12.)));
        assert_eq!(p.wait(ms(56)), None);
        assert_eq!(p.due(ms(90)), None, "nothing more held");
    }

    #[test]
    fn a_press_comes_after_a_move_to_its_own_point() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut p = Pointer::new(Duration::from_millis(16));
        // Never moved over the picture (the window just came forward): a move there first.
        assert_eq!(p.pressed((40., 30.), t0), Some((40., 30.)));
        assert_eq!(p.pressed((40., 30.), ms(80)), None, "the release where it pressed: no move");
        assert_eq!(p.moved((50., 30.), ms(100)), Some((50., 30.)));
        // A move held back by the pace is not lost before the press: the press goes to it.
        assert_eq!(p.moved((60., 31.), ms(105)), None);
        assert_eq!(p.pressed((60., 31.), ms(106)), Some((60., 31.)), "moved there first, never held back");
        assert_eq!(p.due(ms(200)), None, "the press took the held move's place");
        assert_eq!(p.pressed((60., 31.), ms(180)), None);
        // Out of the picture and back: where it is isn't known, so a move goes again.
        p.left();
        assert_eq!(p.pressed((60., 31.), ms(300)), Some((60., 31.)));
    }

    #[test]
    fn a_new_size_is_asked_for_once_it_holds_still() {
        let t0 = Instant::now();
        let b = |w| Boxed { w, h: 700, scale: 2. };
        let mut s = Settle::default();
        assert_eq!(s.see(b(500), t0), Some(b(500)), "the first at once");
        assert_eq!(s.see(b(500), t0 + Duration::from_millis(10)), None, "the same again: nothing");
        // A drag: sizes come and go; only the last, once still, is asked for.
        assert_eq!(s.see(b(510), t0 + Duration::from_millis(20)), None);
        assert_eq!(s.see(b(530), t0 + Duration::from_millis(60)), None);
        assert_eq!(s.wait(t0 + Duration::from_millis(160)), Some(Duration::from_millis(150)));
        assert_eq!(s.see(b(530), t0 + Duration::from_millis(200)), None);
        assert_eq!(s.see(b(530), t0 + Duration::from_millis(320)), Some(b(530)));
        assert_eq!(s.sent(), Some(b(530)));
        assert_eq!(b(530).device(), (1060, 1400));
    }
}
