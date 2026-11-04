//! The app's windows, each its own machine and squad: one on the Mac mini's
//! squad, one beside it on another machine's, both live. A window is opened
//! (New Window, ⌘⇧N; ⌘-click elsewhere; a squad's "open in a new window"),
//! pointed elsewhere without touching the others, and closed without
//! stopping theirs — each has its own line to its daemon (`daemon.rs`).
//!
//! What only one window may do is decided here: per machine and squad (a
//! *seat*), the first window open on it tells the person its notices — each
//! daemon's arrive once, however many windows show it — and the app's first
//! window tells what is the app's own (an update) and takes what other apps
//! share. The Dock's number adds each seat's once.
//!
//! What each window was (its machine, squad and place) is kept beside the
//! app's other state (`<home>/desktop-windows`), in their order, so the next
//! launch opens them all again. A window closed is forgotten — but not the
//! last, which is what a click on the Dock (or the next launch) opens; quitting
//! forgets none.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use equipe_core::protocol::SessionId;
use gpui_kit::{AnyWindowHandle, App, WeakEntity};

use crate::Screen;
use crate::board::Menu;
use crate::target::Endpoint;
use crate::views::elsewhere::Goal;
use crate::views::sidebar::Place;

/// A window's machine and squad: what tells it apart from the others, and
/// whose notices it may be the one to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    pub endpoint: Endpoint,
    /// Empty until the daemon said which (the first connection).
    pub squad: String,
}

impl Seat {
    /// What a notice carries back to a window on this machine and squad —
    /// the same from one launch to the next (FNV-1a, never zero).
    pub fn route(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self.endpoint.saved().bytes().chain(*b"\n").chain(self.squad.bytes()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash.max(1)
    }
}

/// The window's title — what the Mac's Window menu, Mission Control, the
/// tab bar and ⌘` show: "equipe-squad · Mac mini"; before the daemon said
/// which squad, the app's name.
pub fn title(squad: &str, machine: &str) -> String {
    let squad = if squad.trim().is_empty() { "equipe" } else { squad.trim() };
    match machine.trim() {
        "" => squad.to_owned(),
        machine => format!("{squad} · {machine}"),
    }
}

/// Where a window was, to open on again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Spot {
    /// The PM's conversation.
    #[default]
    Pm,
    Member(SessionId),
    /// A ticket beside the conversation.
    Ticket(u32),
    /// The work screens, at a list.
    Board(Menu),
    /// 진행 상황, 디자인 시스템, 로드맵, 지표.
    Screen(Screen),
    /// A project's page.
    Project(String),
}

impl Spot {
    /// The spot of a window at `place`, its project page `project`.
    pub fn of(place: Place, project: Option<&str>) -> Self {
        match place.screen {
            Screen::Chat => match (place.ticket, place.member) {
                (Some(ticket), _) => Spot::Ticket(ticket),
                (None, Some(member)) => Spot::Member(member),
                (None, None) => Spot::Pm,
            },
            Screen::Board => Spot::Board(place.menu),
            Screen::Progress | Screen::Design | Screen::Roadmap | Screen::Metrics => Spot::Screen(place.screen),
            Screen::Project => project.map_or(Spot::Pm, |p| Spot::Project(p.to_owned())),
            // Settings is a visit, not a place to come back to.
            Screen::Settings => Spot::Pm,
        }
    }

    /// Where to go to be here again, once connected.
    pub fn goal(&self) -> Option<Goal> {
        let place = |screen, menu, member, ticket| Goal::Place(Place { screen, menu, member, ticket });
        Some(match self {
            Spot::Pm => return None,
            Spot::Member(member) => place(Screen::Chat, Menu::Inbox, Some(*member), None),
            Spot::Ticket(ticket) => place(Screen::Chat, Menu::Inbox, None, Some(*ticket)),
            Spot::Board(menu) => place(Screen::Board, *menu, None, None),
            Spot::Screen(screen) => place(*screen, Menu::Inbox, None, None),
            Spot::Project(name) => Goal::Project(name.clone()),
        })
    }

    fn code(&self) -> String {
        match self {
            Spot::Pm => "pm".into(),
            Spot::Member(s) => format!("member:{s}"),
            Spot::Ticket(t) => format!("ticket:{t}"),
            Spot::Board(Menu::View(n)) => format!("board:view{n}"),
            Spot::Board(menu) => format!("board:{}", menu_code(*menu)),
            Spot::Screen(screen) => screen_code(*screen).to_owned(),
            Spot::Project(name) => format!("project:{}", name.replace(['\t', '\n'], " ")),
        }
    }

    fn parse(code: &str) -> Self {
        let (kind, rest) = code.split_once(':').unwrap_or((code, ""));
        let spot = match kind {
            "member" => rest.parse().ok().map(Spot::Member),
            "ticket" => rest.parse().ok().map(Spot::Ticket),
            "board" => Some(Spot::Board(match rest {
                "working" => Menu::Working,
                "all" => Menu::All,
                view if view.starts_with("view") => view[4..].parse().map(Menu::View).unwrap_or_default(),
                _ => Menu::Inbox,
            })),
            "project" if !rest.trim().is_empty() => Some(Spot::Project(rest.to_owned())),
            screen => [Screen::Progress, Screen::Design, Screen::Roadmap, Screen::Metrics].into_iter().find(|s| screen_code(*s) == screen).map(Spot::Screen),
        };
        spot.unwrap_or_default()
    }
}

fn menu_code(menu: Menu) -> &'static str {
    match menu {
        Menu::Inbox => "inbox",
        Menu::Working => "working",
        Menu::All => "all",
        Menu::View(_) => "view",
    }
}

fn screen_code(screen: Screen) -> &'static str {
    match screen {
        Screen::Progress => "progress",
        Screen::Design => "design",
        Screen::Roadmap => "roadmap",
        Screen::Metrics => "metrics",
        _ => "pm",
    }
}

/// A window as kept between launches: its machine, its squad (none: as the
/// daemon picks), where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub endpoint: Endpoint,
    pub squad: Option<String>,
    pub spot: Spot,
}

impl Kept {
    /// How it opens again.
    pub fn opening(&self) -> Opening {
        Opening { endpoint: Some(self.endpoint.clone()), squad: self.squad.clone(), goal: self.spot.goal(), chooser: false }
    }
}

pub fn path(home: &Path) -> PathBuf {
    home.join("desktop-windows")
}

/// `window\t<machine>\t<squad>\t<spot>` lines, front-most first as opened.
fn parse(text: &str) -> Vec<Kept> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.strip_prefix("window\t")?.split('\t');
            let endpoint = Endpoint::from_saved(parts.next()?)?;
            let squad = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
            let spot = parts.next().map(Spot::parse).unwrap_or_default();
            Some(Kept { endpoint, squad, spot })
        })
        .collect()
}

fn render(kept: &[Kept]) -> String {
    kept.iter()
        .map(|k| format!("window\t{}\t{}\t{}\n", k.endpoint.saved(), k.squad.as_deref().unwrap_or("").replace(['\t', '\n'], " "), k.spot.code()))
        .collect()
}

/// The windows the last launch left open, in order (none: one window, as
/// the app's other state says).
pub fn load(home: &Path) -> Vec<Kept> {
    std::fs::read_to_string(path(home)).map(|text| parse(&text)).unwrap_or_default()
}

fn save(home: &Path, kept: &[Kept]) {
    let path = path(home);
    if crate::readonly::blocked(&path.display().to_string()) {
        return;
    }
    let text = render(kept);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return;
    }
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::write(path, text);
}

/// What a new window is opened with: its machine and squad (none: as the
/// app was left), where to go once connected, and whether the machine
/// chooser opens over it at once ("connect to another machine…" in a new
/// window).
#[derive(Debug, Clone, Default)]
pub struct Opening {
    pub endpoint: Option<Endpoint>,
    pub squad: Option<String>,
    pub goal: Option<Goal>,
    pub chooser: bool,
}

/// The open windows, in the order they opened — the plain facts the
/// once-only duties are decided from.
#[derive(Debug, Clone, Default)]
pub struct Windows {
    open: Vec<Entry>,
    /// The last window closed while no other was open: kept, to open again.
    last_closed: Option<Kept>,
}

#[derive(Debug, Clone)]
struct Entry {
    id: u64,
    seat: Seat,
    spot: Spot,
    /// What waits for the person in it (나에게), for the Dock.
    needs_you: usize,
}

impl Windows {
    pub fn opened(&mut self, id: u64, seat: Seat) {
        self.open.retain(|e| e.id != id);
        self.open.push(Entry { id, seat, spot: Spot::Pm, needs_you: 0 });
        self.last_closed = None;
    }

    /// The window went; the last one open is kept to open again.
    pub fn closed(&mut self, id: u64) {
        let Some(at) = self.open.iter().position(|e| e.id == id) else { return };
        let entry = self.open.remove(at);
        if self.open.is_empty() {
            self.last_closed = Some(Kept { endpoint: entry.seat.endpoint, squad: Some(entry.seat.squad).filter(|s| !s.is_empty()), spot: entry.spot });
        }
    }

    /// The window is on `seat` now, at `spot`: whether that changed what is kept.
    pub fn moved(&mut self, id: u64, seat: &Seat, spot: &Spot) -> bool {
        let Some(entry) = self.open.iter_mut().find(|e| e.id == id) else { return false };
        // Until the daemon names the squad, a window keeps the one it asked for.
        let seat = if seat.squad.is_empty() && seat.endpoint == entry.seat.endpoint { Seat { squad: entry.seat.squad.clone(), ..seat.clone() } } else { seat.clone() };
        let changed = entry.seat != seat || entry.spot != *spot;
        entry.seat = seat;
        entry.spot = spot.clone();
        changed
    }

    pub fn set_needs_you(&mut self, id: u64, count: usize) {
        if let Some(entry) = self.open.iter_mut().find(|e| e.id == id) {
            entry.needs_you = count;
        }
    }

    /// Whether `id` is the app's first window: the one that tells what is
    /// the app's own and takes what other apps share.
    pub fn first(&self, id: u64) -> bool {
        self.open.first().is_some_and(|e| e.id == id)
    }

    /// The window that tells `id`'s seat's notices: the first open on the
    /// same machine and squad.
    fn teller(&self, seat: &Seat) -> Option<u64> {
        self.open.iter().find(|e| e.seat == *seat).map(|e| e.id)
    }

    /// Whether `id` tells its seat's notices.
    pub fn tells(&self, id: u64) -> bool {
        self.open.iter().find(|e| e.id == id).is_some_and(|me| self.teller(&me.seat) == Some(id))
    }

    /// The Dock's number: each seat's once, from the window that tells it.
    pub fn needs_you(&self) -> usize {
        self.open.iter().filter(|e| self.teller(&e.seat) == Some(e.id)).map(|e| e.needs_you).sum()
    }

    /// The window a click on a notice goes to: the one telling the seat it
    /// came from, else (a notice from before a restart, the update's) the first.
    pub fn clicked(&self, route: Option<u64>) -> Option<u64> {
        route.and_then(|r| self.open.iter().find(|e| e.seat.route() == r).and_then(|e| self.teller(&e.seat))).or_else(|| self.open.first().map(|e| e.id))
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open.iter().map(|e| e.id).collect()
    }

    /// What the next launch opens.
    pub fn kept(&self) -> Vec<Kept> {
        if self.open.is_empty() {
            return self.last_closed.iter().cloned().collect();
        }
        self.open.iter().map(|e| Kept { endpoint: e.seat.endpoint.clone(), squad: Some(e.seat.squad.clone()).filter(|s| !s.is_empty()), spot: e.spot.clone() }).collect()
    }
}

// ── the app's own list, on its main thread ──

struct Registry {
    windows: Windows,
    handles: Vec<(u64, AnyWindowHandle, WeakEntity<crate::Shell>)>,
    /// Quitting: every window closes, and all of them are kept.
    quitting: bool,
    /// What the Dock shows now, once set.
    badge: Option<Option<usize>>,
}

thread_local! {
    static APP: RefCell<Registry> = RefCell::new(Registry { windows: Windows::default(), handles: Vec::new(), quitting: false, badge: None });
}

fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    APP.with(|app| f(&mut app.borrow_mut()))
}

fn keep(app: &Registry) {
    if !app.quitting {
        save(&equipe_providers::credentials::equipe_home(), &app.windows.kept());
    }
}

/// Whether no window is open yet: the one opening now is the app's first.
pub fn none_open() -> bool {
    with(|app| app.windows.open.is_empty())
}

/// A window opened, on `seat`.
pub fn opened(id: u64, handle: AnyWindowHandle, shell: WeakEntity<crate::Shell>, seat: Seat) {
    with(|app| {
        app.windows.opened(id, seat);
        app.handles.retain(|(i, ..)| *i != id);
        app.handles.push((id, handle, shell));
        keep(app);
    });
}

/// The window is on `seat` at `spot` now.
pub fn moved(id: u64, seat: &Seat, spot: &Spot) {
    with(|app| {
        if app.windows.moved(id, seat, spot) {
            keep(app);
        }
    });
}

/// A window closed: what the others now do in its stead (the shared items,
/// its seat's notices, the Dock's number) — and it is forgotten, unless it
/// was the last.
pub fn closed(id: u64, cx: &mut App) {
    let (first, rest) = with(|app| {
        if app.quitting {
            return (None, Vec::new());
        }
        app.windows.closed(id);
        app.handles.retain(|(i, ..)| *i != id);
        keep(app);
        (app.handles.first().map(|(_, _, s)| s.clone()), app.handles.iter().map(|(_, _, s)| s.clone()).collect::<Vec<_>>())
    });
    if let Some(first) = first.and_then(|s| s.upgrade()) {
        first.update(cx, |shell, cx| shell.take_first_duties(cx));
    }
    for shell in rest.iter().filter_map(WeakEntity::upgrade) {
        shell.update(cx, |shell, _| shell.sync_badge());
    }
}

/// Quitting: every window is kept as it is.
pub fn quitting() {
    with(|app| app.quitting = true);
}

pub fn first(id: u64) -> bool {
    with(|app| app.windows.first(id))
}

pub fn tells(id: u64) -> bool {
    with(|app| app.windows.tells(id))
}

/// `id`'s count for the Dock; the number the Dock should show now, if it
/// changed (`Some(None)`: none).
pub fn badge(id: u64, needs_you: usize, pings: &crate::target::Pings) -> Option<Option<usize>> {
    with(|app| {
        app.windows.set_needs_you(id, needs_you);
        let want = crate::notify::badge(app.windows.needs_you(), pings);
        (app.badge != Some(want)).then(|| {
            app.badge = Some(want);
            want
        })
    })
}

/// The window (and its shell) a click on a notice from `route` goes to.
pub fn clicked(route: Option<u64>) -> Option<(AnyWindowHandle, WeakEntity<crate::Shell>)> {
    with(|app| {
        let id = app.windows.clicked(route)?;
        app.handles.iter().find(|(i, ..)| *i == id).map(|(_, h, s)| (*h, s.clone()))
    })
}

/// Every open window's shell, in order.
pub fn shells() -> Vec<(AnyWindowHandle, WeakEntity<crate::Shell>)> {
    with(|app| app.windows.ids().iter().filter_map(|id| app.handles.iter().find(|(i, ..)| i == id).map(|(_, h, s)| (*h, s.clone()))).collect())
}

/// Make the menu bar's `index`th menu the Mac's Window menu, whatever its
/// name in the language in use: macOS lists the open windows in it by their
/// titles. (GPUI does so only for a menu named "Window".)
pub fn windows_menu_at(index: isize) {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        let Some(class) = AnyClass::get(c"NSApplication") else { return };
        // SAFETY: NSApplication's shared instance, its main menu, an item in
        // range and its submenu — each documented, each checked for nil.
        unsafe {
            let app: *mut AnyObject = msg_send![class, sharedApplication];
            if app.is_null() {
                return;
            }
            let bar: *mut AnyObject = msg_send![app, mainMenu];
            if bar.is_null() {
                return;
            }
            let count: isize = msg_send![bar, numberOfItems];
            if index < 0 || index >= count {
                return;
            }
            let item: *mut AnyObject = msg_send![bar, itemAtIndex: index];
            if item.is_null() {
                return;
            }
            let menu: *mut AnyObject = msg_send![item, submenu];
            if !menu.is_null() {
                let _: () = msg_send![app, setWindowsMenu: menu];
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = index;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::SshHost;

    fn mini() -> Endpoint {
        Endpoint::Ssh(SshHost::parse("abel@mini").unwrap())
    }

    fn seat(endpoint: Endpoint, squad: &str) -> Seat {
        Seat { endpoint, squad: squad.into() }
    }

    #[test]
    fn a_window_is_told_apart_by_its_squad_and_machine() {
        assert_eq!(title("equipe-squad", "Mac mini"), "equipe-squad · Mac mini");
        assert_eq!(title("", "Mac mini"), "equipe · Mac mini", "before the daemon said which squad");
        assert_eq!(title(" photo-studio ", ""), "photo-studio");
    }

    #[test]
    fn each_seat_has_one_teller_and_the_app_one_first() {
        let mut w = Windows::default();
        w.opened(1, seat(mini(), "equipe-squad"));
        w.opened(2, seat(Endpoint::this_mac(), "photo-studio"));
        w.opened(3, seat(mini(), "equipe-squad"));
        w.opened(4, seat(mini(), "cj-geo-squad"));
        // Each machine's squad tells its notices once: from the first window on it.
        assert_eq!([1, 2, 3, 4].map(|id| w.tells(id)), [true, true, false, true]);
        assert!(w.first(1) && !w.first(2), "the app's own (an update, what other apps share): the first window");
        // The first on a seat closes: the next on it tells now.
        w.closed(1);
        assert!(w.tells(3) && w.first(2));
        // A window pointed elsewhere tells there — and leaves its old seat to the next.
        w.opened(5, seat(Endpoint::this_mac(), "photo-studio"));
        assert!(w.moved(2, &seat(mini(), "hackathon-squad"), &Spot::Pm));
        assert!(w.tells(2) && w.tells(5));
    }

    #[test]
    fn the_dock_adds_each_seats_number_once() {
        let mut w = Windows::default();
        w.opened(1, seat(mini(), "equipe-squad"));
        w.opened(2, seat(mini(), "equipe-squad"));
        w.opened(3, seat(Endpoint::this_mac(), "photo-studio"));
        for (id, n) in [(1, 2), (2, 2), (3, 5)] {
            w.set_needs_you(id, n);
        }
        assert_eq!(w.needs_you(), 7, "the same squad in two windows counts once");
    }

    #[test]
    fn a_click_on_a_notice_goes_to_the_window_it_came_from() {
        let mut w = Windows::default();
        w.opened(1, seat(Endpoint::this_mac(), "photo-studio"));
        w.opened(2, seat(mini(), "equipe-squad"));
        w.opened(3, seat(mini(), "equipe-squad"));
        assert_eq!(w.clicked(Some(seat(mini(), "equipe-squad").route())), Some(2));
        assert_eq!(w.clicked(None), Some(1), "the update's: the first window");
        assert_eq!(w.clicked(Some(seat(mini(), "gone").route())), Some(1), "a seat no window shows any more: the first");
        assert_ne!(seat(mini(), "a").route(), seat(Endpoint::this_mac(), "a").route(), "the same squad name on two machines");
        assert_eq!(seat(mini(), "a").route(), seat(mini(), "a").route(), "the same from one launch to the next");
    }

    #[test]
    fn the_windows_open_again_as_they_were_left() {
        let dir = std::env::temp_dir().join(format!("eq-windows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir).is_empty(), "nothing kept: one window, as before");
        let mut w = Windows::default();
        w.opened(1, seat(mini(), ""));
        w.opened(2, seat(Endpoint::this_mac(), "photo-studio"));
        w.opened(3, seat(mini(), "cj-geo-squad"));
        // The daemon hasn't said yet: the window keeps the squad it asked for.
        assert!(!w.moved(3, &seat(mini(), ""), &Spot::Pm));
        w.moved(1, &seat(mini(), "equipe-squad"), &Spot::Member(7));
        w.moved(2, &seat(Endpoint::this_mac(), "photo-studio"), &Spot::Board(Menu::View(2)));
        w.moved(3, &seat(mini(), "cj-geo-squad"), &Spot::Project("my\tsite".into()));
        save(&dir, &w.kept());
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 3);
        assert_eq!((loaded[0].endpoint.clone(), loaded[0].squad.as_deref(), loaded[0].spot.clone()), (mini(), Some("equipe-squad"), Spot::Member(7)));
        assert_eq!((loaded[1].endpoint.clone(), loaded[1].spot.clone()), (Endpoint::this_mac(), Spot::Board(Menu::View(2))));
        assert_eq!(loaded[2].spot, Spot::Project("my site".into()));
        assert_eq!(loaded[0].opening().goal, Some(Goal::Place(Place { screen: Screen::Chat, menu: Menu::Inbox, member: Some(7), ticket: None })));

        // Closed one by one: the last one closed is still what opens next.
        for id in [2, 3, 1] {
            w.closed(id);
        }
        assert_eq!(w.kept().len(), 1);
        assert_eq!(w.kept()[0].squad.as_deref(), Some("equipe-squad"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_spot_reads_back_and_nonsense_is_the_pm() {
        for spot in [
            Spot::Pm,
            Spot::Member(3),
            Spot::Ticket(41),
            Spot::Board(Menu::Inbox),
            Spot::Board(Menu::Working),
            Spot::Board(Menu::All),
            Spot::Board(Menu::View(0)),
            Spot::Screen(Screen::Progress),
            Spot::Screen(Screen::Design),
            Spot::Screen(Screen::Roadmap),
            Spot::Screen(Screen::Metrics),
            Spot::Project("web".into()),
        ] {
            assert_eq!(Spot::parse(&spot.code()), spot);
        }
        for junk in ["", "member:x", "settings", "project:", "nope:1"] {
            assert_eq!(Spot::parse(junk), Spot::Pm, "{junk:?}");
        }
        let settings = Place { screen: Screen::Settings, menu: Menu::All, member: None, ticket: None };
        assert_eq!(Spot::of(settings, None), Spot::Pm, "settings is a visit: the PM next time");
        assert_eq!(parse("window\tssh -bad\tx\tpm\nwindow\tlocal\t\tpm\njunk\n").len(), 1, "a machine that can't be read is skipped");
    }
}
