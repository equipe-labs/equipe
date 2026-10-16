//! The browser panel: the shared Chrome the members browse in, live, beside
//! the conversation (⌘⇧B, or the header's 브라우저 button). Like Claude
//! Code's browser pane, but the browser is equipe's own — one Chrome on the
//! daemon's machine, signed in once — so what shows here is exactly what the
//! members see, sign-ins and all.
//!
//! A row of tabs grouped by whose they are (each member a dot and its name,
//! then its tabs; the person's own, 내 탭, last); back, forward, reload, the
//! address, 따라가기, and "Chrome에서 열기" for what a picture can't do (a
//! passkey, a permission prompt); the page itself; and a control bar saying
//! who has it in hand, with one button to take it or give it back. It follows
//! the conversation on screen — that member's browser, the tab it last worked
//! in — until the person picks another tab.
//!
//! The page is a stream of pictures (`crate::stream`), only ever of the tab
//! picked (`browse::Picture`). A page the person drives is laid out at the
//! picture's size; a member's page at its own size is fitted, its shape kept.
//! The person's mouse and keys go back to the page, mapped through the
//! picture's frame. Touching a member's page (or 직접 하기) takes it into the
//! person's hands (its tools wait); 돌려주기, Esc, or two idle minutes give it
//! back. A page that waits for a person — a "verify you are human" check, a
//! sign-in, a code — says so over the picture, with 직접 하기 (`browse::Hand`).
//! The rules are `crate::browse`, tested without a window.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use equipe_core::protocol::stream::{self, Fit, FrameHeader, InputEvent, StreamIn};
use equipe_core::protocol::{BrowserNavAction, BrowserPage, Command, Event, HumanCheck, ScreenBounds, SessionId};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent as FieldEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::browse::{self, Boxed, Hand, LaneRef, Mode, Picture, Settle};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::stream::{Link, Update};
use crate::target::Endpoint;
use crate::theme::{CONTROL, ICON_SM, Palette, R_LG, R_MD, R_SM, SANS, SMALL, UI};
use crate::tips::tip;
use crate::ui::{self, Kind};

/// Where the page's picture was last drawn (window coordinates), and of what frame.
type Placed = Rc<RefCell<Option<(Bounds<Pixels>, FrameHeader)>>>;

/// The person's own tabs, as lanes go.
const MINE: &str = "";
/// How wide the panel opens, and its limits.
pub const WIDTH: f32 = 560.;
const MIN_WIDTH: f32 = 360.;
/// Room the conversation keeps beside it.
const CHAT_MIN: f32 = 420.;
/// Mouse moves go no more often than this (60 a second).
const MOVE_EVERY: Duration = Duration::from_millis(16);
/// The line down this long before the panel says so (a reconnect is quick).
const DOWN_AFTER: Duration = Duration::from_millis(1500);

/// What the panel asks of the window.
pub enum PanelEvent {
    Command(Command),
    /// The width was dragged: remember it.
    Resized(f32),
    /// It opened on its own (a member asked the person in): the window shows
    /// the conversation beside it.
    Opened,
}

/// One lane's tabs, as the daemon last told.
#[derive(Debug, Clone, PartialEq)]
struct Lane {
    squad: Option<String>,
    session: Option<SessionId>,
    /// The host's label: `frontend`, `pm (site)`.
    member: Option<String>,
    /// `r, g, b`.
    color: Option<String>,
    pages: Vec<BrowserPage>,
    agent_page: Option<u64>,
}

/// A page's question waiting for the person.
#[derive(Debug, Clone)]
struct Dialog {
    page: u64,
    kind: String,
    message: String,
}

/// A member's browser that went away while on screen: who, and where it was.
#[derive(Debug, Clone)]
struct Gone {
    name: String,
    url: Option<String>,
}

/// Who a conversation is, for its lane's label and colour, and whether it is
/// at work now.
#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub name: String,
    pub hue: usize,
    pub working: bool,
}

pub struct BrowserPanel {
    pub open: bool,
    /// The window shows it: open, beside the conversation.
    shown: bool,
    pub width: f32,
    drag: Option<(f32, f32)>,
    pub focus: FocusHandle,
    palette: Palette,
    squad: String,
    url: Entity<InputState>,
    /// The page whose address the field shows.
    url_of: Option<(u64, String)>,
    prompt: Entity<InputState>,
    lanes: BTreeMap<String, Lane>,
    /// Who has each member's browser in hand.
    hands: HashMap<String, Hand>,
    dialogs: Vec<Dialog>,
    selected: Option<(String, u64)>,
    follow: bool,
    /// The conversation on screen, and who each conversation is.
    viewed: Option<SessionId>,
    who: HashMap<SessionId, Who>,
    /// A new tab of the person's own was asked for: picked when it comes.
    new_tab: bool,
    /// The person closed this tab: its going is no news.
    closing: Option<u64>,
    gone: Option<Gone>,
    /// The page's keys go to it at the next draw (the person chose 직접 하기).
    focus_next: bool,
    /// A page picked from 나에게 or a notification, shown at the next draw.
    pending_reveal: Option<(String, u64)>,
    // The picture.
    picture: Picture<Arc<RenderImage>>,
    previous: Option<Arc<RenderImage>>,
    /// Where the picture was drawn, in window coordinates, and of what frame.
    placed: Placed,
    /// The viewport's box, as drawn.
    viewport: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// The page's size asked for, once the box holds still.
    settle: Settle,
    /// When the page on screen was first asked for: its frames late past
    /// `browse::STALL`, the panel offers to try again.
    asked_at: Option<Instant>,
    _wake: Option<Task<()>>,
    _down_wake: Option<Task<()>>,
    tabs_scroll: ScrollHandle,
    /// The picked tab is scrolled into sight at the next draw.
    reveal_tab: std::cell::Cell<bool>,
    // The line.
    endpoint: Option<Endpoint>,
    link: Option<Link>,
    connected: bool,
    /// When the line dropped, having been up.
    down_since: Option<Instant>,
    watching: Option<StreamIn>,
    // The hand.
    marked: Option<String>,
    keys_down: std::collections::HashSet<String>,
    buttons: u32,
    last_move: Instant,
    /// Wheel turns not sent yet, when the last went, and the send to come.
    wheel: Option<InputEvent>,
    last_wheel: Instant,
    _wheel_flush: Option<Task<()>>,
    last_click: Option<Point<Pixels>>,
    _pump: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for BrowserPanel {}

impl BrowserPanel {
    /// The language changed: the address box's hint follows.
    pub fn relocalize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url.update(cx, |s, cx| s.set_placeholder(t!("browser.address_placeholder"), window, cx));
    }

    pub fn new(width: Option<f32>, palette: Palette, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(t!("browser.address_placeholder")));
        let prompt = cx.new(|cx| InputState::new(window, cx));
        let mut subscriptions = vec![cx.subscribe_in(&url, window, |this, input, event: &FieldEvent, window, cx| match event {
            FieldEvent::PressEnter { .. } => {
                let text = input.read(cx).value().to_string();
                if let Some((_, page)) = this.selected.clone() {
                    this.command(Command::BrowserNav { page, action: BrowserNavAction::Go, url: Some(text) }, cx);
                    this.url_of = None;
                    this.focus_page(window, cx);
                }
            }
            // Out of the field, it shows the page's address again.
            FieldEvent::Blur => {
                this.url_of = None;
                this.refresh_address(window, cx);
                cx.notify();
            }
            _ => {}
        })];
        subscriptions.push(cx.subscribe_in(&prompt, window, |this, input, event: &FieldEvent, _, cx| {
            if matches!(event, FieldEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                this.answer(true, Some(text), cx);
            }
        }));
        // Out of focus, the page streams slower; back in, at full speed.
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            this.rewatch(window, cx);
            cx.notify();
        }));
        Self {
            open: false,
            shown: false,
            width: width.unwrap_or(WIDTH),
            drag: None,
            focus: cx.focus_handle(),
            palette,
            squad: String::new(),
            url,
            url_of: None,
            prompt,
            lanes: BTreeMap::new(),
            hands: HashMap::new(),
            dialogs: Vec::new(),
            selected: None,
            follow: true,
            viewed: None,
            who: HashMap::new(),
            new_tab: false,
            closing: None,
            gone: None,
            focus_next: false,
            pending_reveal: None,
            picture: Picture::default(),
            previous: None,
            placed: Rc::default(),
            viewport: Rc::default(),
            settle: Settle::default(),
            asked_at: None,
            _wake: None,
            _down_wake: None,
            tabs_scroll: ScrollHandle::new(),
            reveal_tab: std::cell::Cell::new(false),
            endpoint: None,
            link: None,
            connected: false,
            down_since: None,
            watching: None,
            marked: None,
            keys_down: Default::default(),
            buttons: 0,
            last_move: Instant::now(),
            wheel: None,
            last_wheel: Instant::now(),
            _wheel_flush: None,
            last_click: None,
            _pump: None,
            _subscriptions: subscriptions,
        }
    }

    fn command(&self, command: Command, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Command(command));
    }

    // ── what the window tells it ──

    /// Which daemon and squad the window follows: a new one, and what was known
    /// of the last is gone.
    fn attach(&mut self, endpoint: &Endpoint, squad: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.endpoint.as_ref() == Some(endpoint) && self.squad == squad {
            return;
        }
        self.forget(window);
        self.link = None;
        self._pump = None;
        self.watching = None;
        self.connected = false;
        self.down_since = None;
        self.endpoint = Some(endpoint.clone());
        self.squad = squad.to_owned();
        self.ensure_link(window, cx);
        cx.notify();
    }

    /// Everything known of the tabs goes: a new daemon, or the same one
    /// attached again (it tells its tabs afresh right after).
    fn forget(&mut self, window: &mut Window) {
        self.lanes.clear();
        self.hands.clear();
        self.dialogs.clear();
        self.selected = None;
        self.follow = true;
        self.gone = None;
        self.closing = None;
        self.drop_pictures(window);
        self.asked_at = None;
    }

    /// Attached to the daemon again (it may have started again, its browser
    /// with it): the tabs it tells next are all there are.
    pub fn reattached(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget(window);
        self.rewatch(window, cx);
        cx.notify();
    }

    /// What the window knows: the daemon and squad it follows, the
    /// conversation on screen, who each conversation is, the colours, and
    /// whether the panel is in sight (`shown`: open, beside the conversation).
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        endpoint: &Endpoint,
        squad: &str,
        viewed: Option<SessionId>,
        who: HashMap<SessionId, Who>,
        palette: Palette,
        chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attach(endpoint, squad, window, cx);
        self.palette = palette;
        self.who = who;
        let shown = self.open && chat;
        if self.shown != shown {
            self.shown = shown;
            self.rewatch(window, cx);
        }
        if self.viewed != viewed {
            self.viewed = viewed;
            self.follow = true;
            self.follow_viewed(false, window, cx);
        }
        cx.notify();
    }

    /// One of the daemon's browser events.
    pub fn on_event(&mut self, event: &Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::BrowserTabs { squad, lane, session, member, color, pages, agent_page } => {
                if squad.as_deref().is_some_and(|s| s != self.squad) {
                    return;
                }
                let before = self.lanes.get(lane).map(|l| l.agent_page);
                let mine_before: std::collections::HashSet<u64> = self.lanes.get(MINE).map(|l| l.pages.iter().map(|p| p.id).collect()).unwrap_or_default();
                let name = self.member_name(lane);
                let last_url = self.selected.as_ref().filter(|(l, _)| l == lane).and_then(|_| self.page()).map(|p| p.url.clone());
                if pages.is_empty() {
                    self.lanes.remove(lane);
                } else {
                    self.lanes.insert(
                        lane.clone(),
                        Lane { squad: squad.clone(), session: *session, member: member.clone(), color: color.clone(), pages: pages.clone(), agent_page: *agent_page },
                    );
                }
                // What the lane's page waits for, for the hand on it.
                if lane != MINE {
                    let waiting = pages.iter().filter(|p| p.check.is_some()).min_by_key(|p| Some(p.id) != *agent_page).and_then(|p| p.check);
                    self.hands.entry(lane.clone()).or_default().on_page(waiting);
                }
                // A tab of the person's own just asked for: shown.
                if lane == MINE
                    && self.new_tab
                    && let Some(new) = pages.iter().find(|p| !mine_before.contains(&p.id))
                {
                    self.new_tab = false;
                    self.follow = false;
                    self.select(MINE.to_owned(), new.id, window, cx);
                }
                let moved = before != Some(*agent_page);
                // The tab picked closed: the next one in its lane — or, the whole
                // browser gone without the person closing it, say so.
                if let Some((sel_lane, page)) = self.selected.clone()
                    && !self.lanes.get(&sel_lane).is_some_and(|l| l.pages.iter().any(|p| p.id == page))
                {
                    let next = self.lanes.get(&sel_lane).and_then(|l| l.pages.last()).map(|p| p.id);
                    match next {
                        Some(next) => self.select(sel_lane, next, window, cx),
                        None => {
                            let closed_here = self.closing == Some(page);
                            self.selected = None;
                            self.drop_pictures(window);
                            if closed_here || sel_lane == MINE {
                                self.follow = true;
                            } else {
                                self.gone = Some(Gone { name, url: last_url });
                                self.follow = false;
                            }
                            self.rewatch(window, cx);
                        }
                    }
                }
                self.follow_viewed(moved, window, cx);
                self.refresh_address(window, cx);
                cx.notify();
            }
            Event::BrowserControl { squad, lane, by, why, check } => {
                if squad.as_deref().is_some_and(|s| s != self.squad) {
                    return;
                }
                self.hands.entry(lane.clone()).or_default().on_control(by.is_some(), why.clone(), *check);
                // A member asks the person in: the panel opens on its browser.
                if why.is_some() {
                    self.follow = false;
                    self.open = true;
                    self.shown = true;
                    cx.emit(PanelEvent::Opened);
                    self.ensure_link(window, cx);
                    if let Some(page) = self.lanes.get(lane).and_then(|l| l.agent_page.or(l.pages.last().map(|p| p.id))) {
                        self.select(lane.clone(), page, window, cx);
                    }
                    window.activate_window();
                }
                cx.notify();
            }
            Event::BrowserDialog { squad, page, kind, message, .. } => {
                if squad.as_deref().is_some_and(|s| s != self.squad) {
                    return;
                }
                self.dialogs.retain(|d| d.page != *page);
                self.dialogs.push(Dialog { page: *page, kind: kind.clone(), message: message.clone() });
                if kind == "prompt" {
                    let default = match event {
                        Event::BrowserDialog { default, .. } => default.clone().unwrap_or_default(),
                        _ => String::new(),
                    };
                    self.prompt.update(cx, |s, cx| s.set_value(default, window, cx));
                }
                cx.notify();
            }
            Event::BrowserDialogClosed { page } => {
                self.dialogs.retain(|d| d.page != *page);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Show the conversation's browser — the tab its member last worked in —
    /// while following. `moved`: its member just moved to another tab.
    fn follow_viewed(&mut self, moved: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.follow {
            return;
        }
        let Some(viewed) = self.viewed else { return };
        let Some((lane, l)) = self.lanes.iter().find(|(_, l)| l.session == Some(viewed)) else { return };
        let page = l.agent_page.filter(|a| l.pages.iter().any(|p| p.id == *a)).or(l.pages.last().map(|p| p.id));
        let Some(page) = page else { return };
        // Another browser on screen, or its member moved on: show where it works.
        let here = self.selected.as_ref().is_some_and(|(s, _)| s == lane);
        if !here || moved {
            self.select(lane.clone(), page, window, cx);
        }
    }

    fn select(&mut self, lane: String, page: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&(lane.clone(), page)) {
            return;
        }
        self.selected = Some((lane, page));
        self.reveal_tab.set(true);
        self.gone = None;
        // Nothing of the last tab shows meanwhile: a skeleton until this one's first frame.
        if let Some(old) = self.picture.pick(Some(page)) {
            let _ = window.drop_image(old);
        }
        if let Some(old) = self.previous.take() {
            let _ = window.drop_image(old);
        }
        *self.placed.borrow_mut() = None;
        self.asked_at = None;
        self.url_of = None;
        self.refresh_address(window, cx);
        self.rewatch(window, cx);
        cx.notify();
    }

    fn page(&self) -> Option<&BrowserPage> {
        let (lane, page) = self.selected.as_ref()?;
        self.lanes.get(lane)?.pages.iter().find(|p| p.id == *page)
    }

    /// Whether a page is one of the tabs as last told.
    fn listed(&self, page: u64) -> bool {
        self.lanes.values().any(|l| l.pages.iter().any(|p| p.id == page))
    }

    /// The address field shows the page's address, unless the person is typing in it.
    fn refresh_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page().cloned() else { return };
        if self.url.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        if self.url_of.as_ref() != Some(&(page.id, page.url.clone())) {
            let shown = if page.url == "about:blank" { String::new() } else { page.url.clone() };
            self.url.update(cx, |s, cx| s.set_value(shown, window, cx));
            self.url_of = Some((page.id, page.url));
        }
    }

    // ── open, close, size ──

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.shown = self.open;
        if self.open {
            self.ensure_link(window, cx);
            self.follow_viewed(false, window, cx);
        } else {
            self.drop_pictures(window);
            if self.focus.is_focused(window) {
                window.blur(cx);
            }
        }
        self.rewatch(window, cx);
        cx.notify();
    }

    fn ensure_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || self.link.is_some() {
            return;
        }
        let Some(endpoint) = self.endpoint.clone() else { return };
        let (link, mut updates) = crate::stream::spawn(endpoint);
        self.link = Some(link);
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let alive = this.update_in(cx, |this, window, cx| this.on_update(update, window, cx));
                if alive.is_err() {
                    break;
                }
            }
        }));
        self.watching = None;
        self.settle.reset();
        self.rewatch(window, cx);
    }

    /// 다시 시도: the line again, from nothing.
    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link = None;
        self._pump = None;
        self.connected = false;
        self.down_since = None;
        self.drop_pictures(window);
        self.asked_at = None;
        self.ensure_link(window, cx);
        cx.notify();
    }

    fn on_update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::Line(up) => {
                self.connected = up;
                self.down_since = if up { None } else { Some(Instant::now()) };
                if up {
                    // A new line starts from nothing: what to watch is asked again.
                    self.watching = None;
                    self.settle.reset();
                    self.asked_at = None;
                    self.rewatch(window, cx);
                } else {
                    // Drawn again once it has been down long enough to say so.
                    self._down_wake = Some(cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(DOWN_AFTER + Duration::from_millis(20)).await;
                        let _ = this.update(cx, |_, cx| cx.notify());
                    }));
                }
                cx.notify();
            }
            Update::Picture(picture) => {
                let listed = self.listed(picture.header.page);
                match self.picture.offer(picture.header, picture.image, listed) {
                    Ok(replaced) => {
                        // The one before stays a frame longer: it may still be on screen.
                        if let Some(old) = std::mem::replace(&mut self.previous, replaced) {
                            let _ = window.drop_image(old);
                        }
                        cx.notify();
                    }
                    Err(image) => {
                        let _ = window.drop_image(image);
                    }
                }
                // Drawn — the next one may come.
                let credit = self.link.as_ref().map(Link::sender);
                window.on_next_frame(move |_, _| {
                    if let Some(credit) = credit {
                        let _ = credit.send(StreamIn::Credit { n: 1 });
                    }
                });
            }
        }
    }

    fn drop_pictures(&mut self, window: &mut Window) {
        for image in [self.picture.clear(), self.previous.take()].into_iter().flatten() {
            let _ = window.drop_image(image);
        }
        // Frames from now are the picked tab's, or none's.
        let _ = self.picture.pick(self.selected.as_ref().map(|(_, p)| *p));
        *self.placed.borrow_mut() = None;
    }

    /// Draw again after `wait`: a size held still, a line down long enough to
    /// say so, frames late enough to offer to try again.
    fn wake_in(&mut self, wait: Duration, window: &mut Window, cx: &mut Context<Self>) {
        self._wake = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    /// The box the page shows in now, as drawn.
    fn boxed(&self, window: &Window) -> Option<Boxed> {
        let b = (*self.viewport.borrow())?;
        let (w, h) = (f32::from(b.size.width).round() as u32, f32::from(b.size.height).round() as u32);
        // The window zoomed (⌘+), the page is laid out at the zoomed box and drawn
        // at the zoomed scale: larger, as a browser zooms, and still crisp.
        (w >= 64 && h >= 64).then(|| Boxed { w, h, scale: window.scale_factor() * window.content_zoom() })
    }

    /// Tell the stream what to show now: the picked page while the panel is in
    /// sight — fast with the window in front, slow behind — or nothing. Its
    /// size goes with it, once the panel's box holds still.
    pub fn rewatch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = &self.link else { return };
        let page = self.selected.as_ref().map(|(_, p)| *p).filter(|_| self.open && self.shown);
        let want = match (page, self.boxed(window)) {
            (Some(page), Some(boxed)) => {
                let now = Instant::now();
                self.settle.see(boxed, now);
                let size = self.settle.sent().unwrap_or(boxed);
                let (max_w, max_h) = size.device();
                let fps = if window.is_window_active() { 30 } else { 5 };
                Some(StreamIn::Watch { page, fps, max_w, max_h, fit: Some(Fit { w: size.w, h: size.h, dpr: size.scale }) })
            }
            // Not laid out yet: asked once it is.
            (Some(_), None) => None,
            (None, _) => Some(StreamIn::Unwatch),
        };
        let Some(want) = want else { return };
        if self.watching.as_ref() != Some(&want) {
            let new_page = match (&self.watching, &want) {
                (Some(StreamIn::Watch { page: a, .. }), StreamIn::Watch { page: b, .. }) => a != b,
                _ => true,
            };
            link.send(want.clone());
            if new_page {
                self.asked_at = matches!(want, StreamIn::Watch { .. }).then(Instant::now);
            }
            self.watching = Some(want);
        }
        if let Some(wait) = self.settle.wait(Instant::now()) {
            self.wake_in(wait + Duration::from_millis(5), window, cx);
        } else if self.picture.waiting() && let Some(at) = self.asked_at {
            let left = browse::STALL.saturating_sub(at.elapsed());
            if !left.is_zero() {
                self.wake_in(left + Duration::from_millis(50), window, cx);
            }
        }
    }

    /// 다시 시도 on a page whose pictures stopped: asked for from nothing.
    fn rewatch_fresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(link) = &self.link {
            link.send(StreamIn::Unwatch);
        }
        self.watching = None;
        self.asked_at = None;
        self.rewatch(window, cx);
        cx.notify();
    }

    // ── the person's hand ──

    fn lane_of_selected(&self) -> Option<&str> {
        self.selected.as_ref().map(|(l, _)| l.as_str())
    }

    /// The person touched the page: it is theirs until handed back.
    fn hold(&mut self) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.hands.entry(lane).or_default().take();
        }
    }

    /// 직접 하기: the member's browser into the person's hands, its keys to the page.
    fn take(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.hands.entry(lane.clone()).or_default().take();
            self.command(Command::BrowserControl { lane, take: true }, cx);
        }
        self.focus_page(window, cx);
        cx.notify();
    }

    fn give_back(&mut self, cx: &mut Context<Self>) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.hands.entry(lane.clone()).or_default().give_back();
            self.command(Command::BrowserControl { lane, take: false }, cx);
            cx.notify();
        }
    }

    /// Past the check, the person keeps the browser.
    fn keep(&mut self, cx: &mut Context<Self>) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.hands.entry(lane).or_default().keep();
            cx.notify();
        }
    }

    fn send_input(&mut self, ev: InputEvent) {
        if let (Some(link), Some((_, page))) = (&self.link, &self.selected) {
            link.send(StreamIn::Input { page: *page, ev });
        }
    }

    /// Where a window point falls on the page, in CSS pixels.
    fn to_page(&self, at: Point<Pixels>) -> Option<(f64, f64)> {
        let placed = self.placed.borrow();
        let (rect, header) = placed.as_ref()?;
        stream::to_page(
            (f64::from(f32::from(at.x)), f64::from(f32::from(at.y))),
            (f64::from(f32::from(rect.origin.x)), f64::from(f32::from(rect.origin.y))),
            (f64::from(f32::from(rect.size.width)), f64::from(f32::from(rect.size.height))),
            header,
        )
    }

    /// CSS pixels per window point, across.
    fn css_per_point(&self) -> f64 {
        let placed = self.placed.borrow();
        placed.as_ref().map_or(1., |(rect, h)| h.css_w / f64::from(f32::from(rect.size.width)).max(1.))
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some((x, y)) = self.to_page(e.position) else { return };
        self.hold();
        self.last_click = Some(e.position);
        self.buttons |= keys::button_bit(e.button);
        self.send_input(InputEvent::Mouse {
            kind: "mousePressed".into(),
            x,
            y,
            button: keys::button_name(e.button).into(),
            buttons: self.buttons,
            click_count: e.click_count as u32,
            modifiers: keys::modifier_bits(&e.modifiers),
        });
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent) {
        let Some((x, y)) = self.to_page(e.position) else {
            self.buttons &= !keys::button_bit(e.button);
            return;
        };
        self.buttons &= !keys::button_bit(e.button);
        self.send_input(InputEvent::Mouse {
            kind: "mouseReleased".into(),
            x,
            y,
            button: keys::button_name(e.button).into(),
            buttons: self.buttons,
            click_count: e.click_count as u32,
            modifiers: keys::modifier_bits(&e.modifiers),
        });
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent) {
        if self.last_move.elapsed() < MOVE_EVERY {
            return;
        }
        let Some((x, y)) = self.to_page(e.position) else { return };
        self.last_move = Instant::now();
        // Moves alone don't take the page: only a press or a key does.
        self.send_input(InputEvent::Mouse {
            kind: "mouseMoved".into(),
            x,
            y,
            button: e.pressed_button.map_or("none", keys::button_name).into(),
            buttons: self.buttons,
            click_count: 0,
            modifiers: keys::modifier_bits(&e.modifiers),
        });
    }

    /// A turn of the wheel: sent at once, then — as a trackpad sends 60–120
    /// a second — the turns that follow added up and sent a frame apart.
    fn scroll(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some((x, y)) = self.to_page(e.position) else { return };
        let delta = e.delta.pixel_delta(px(20.));
        let k = self.css_per_point();
        self.hold();
        let (dx, dy, modifiers) = (-f64::from(f32::from(delta.x)) * k, -f64::from(f32::from(delta.y)) * k, keys::modifier_bits(&e.modifiers));
        match &mut self.wheel {
            Some(InputEvent::Wheel { x: at_x, y: at_y, dx: sum_x, dy: sum_y, modifiers: held }) if *held == modifiers => {
                (*at_x, *at_y) = (x, y);
                *sum_x += dx;
                *sum_y += dy;
            }
            _ => {
                self.flush_wheel();
                self.wheel = Some(InputEvent::Wheel { x, y, dx, dy, modifiers });
            }
        }
        let since = self.last_wheel.elapsed();
        if since >= MOVE_EVERY {
            self.flush_wheel();
        } else if self._wheel_flush.is_none() {
            let wait = MOVE_EVERY - since;
            self._wheel_flush = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |this, _| {
                    this._wheel_flush = None;
                    this.flush_wheel();
                });
            }));
        }
    }

    fn flush_wheel(&mut self) {
        if let Some(wheel) = self.wheel.take() {
            self.last_wheel = Instant::now();
            self.send_input(wheel);
        }
    }

    fn held(&self) -> bool {
        self.lane_of_selected().filter(|l| !l.is_empty()).is_some_and(|l| self.hands.get(l).is_some_and(|h| h.held))
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &e.keystroke;
        let m = &stroke.modifiers;
        // The panel's own keys: Esc gives a member's browser back.
        if stroke.key == "escape" && !m.modified() && self.held() {
            self.give_back(cx);
            cx.stop_propagation();
            return;
        }
        if m.platform && !m.control && !m.alt {
            match (stroke.key.as_str(), m.shift) {
                ("v", false) => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.hold();
                        self.send_input(InputEvent::Insert { text });
                    }
                    cx.stop_propagation();
                    return;
                }
                ("l", false) => {
                    self.url.update(cx, |s, cx| s.focus(window, cx));
                    cx.stop_propagation();
                    return;
                }
                ("r", false) => {
                    self.nav(BrowserNavAction::Reload, cx);
                    cx.stop_propagation();
                    return;
                }
                ("[", false) => {
                    self.nav(BrowserNavAction::Back, cx);
                    cx.stop_propagation();
                    return;
                }
                ("]", false) => {
                    self.nav(BrowserNavAction::Forward, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        let Some(ev) = keys::key_down(stroke) else { return };
        if let InputEvent::Key { key, .. } = &ev {
            self.keys_down.insert(key.clone());
        }
        self.hold();
        self.send_input(ev);
        cx.stop_propagation();
        cx.notify();
    }

    fn key_up(&mut self, e: &KeyUpEvent) {
        let Some(ev) = keys::key_up(&e.keystroke) else { return };
        if let InputEvent::Key { key, .. } = &ev
            && self.keys_down.remove(key)
        {
            self.send_input(ev);
        }
    }

    fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn nav(&mut self, action: BrowserNavAction, cx: &mut Context<Self>) {
        if let Some((_, page)) = self.selected.clone() {
            self.command(Command::BrowserNav { page, action, url: None }, cx);
        }
    }

    fn new_tab(&mut self, url: Option<String>, cx: &mut Context<Self>) {
        self.new_tab = true;
        self.command(Command::BrowserNewTab { url }, cx);
    }

    fn answer(&mut self, accept: bool, text: Option<String>, cx: &mut Context<Self>) {
        let Some(page) = self.selected.as_ref().map(|(_, p)| *p) else { return };
        if self.dialogs.iter().any(|d| d.page == page) {
            self.dialogs.retain(|d| d.page != page);
            self.command(Command::BrowserDialog { page, accept, text }, cx);
            cx.notify();
        }
    }

    /// The real Chrome window, over where the panel is.
    fn pop_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, page)) = self.selected.clone() else { return };
        let b = window.bounds();
        let (x, y, w, h) = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
        // The panel's width in screen points, the window zoomed or not.
        let width = (self.width * window.content_zoom()).max(720.);
        let bounds = ScreenBounds { left: (x + w - width).max(0.) as i32, top: y as i32, width: width as u32, height: h as u32 };
        self.command(Command::BrowserPopOut { page, bounds: Some(bounds) }, cx);
    }

    /// A page waiting for a person, from 나에게 or a notification: the panel
    /// opens on it, in the person's hands, its keys to the page.
    pub fn reveal(&mut self, page: u64, cx: &mut Context<Self>) {
        let Some(lane) = self.lanes.iter().find(|(_, l)| l.pages.iter().any(|p| p.id == page)).map(|(lane, _)| lane.clone()) else { return };
        self.open = true;
        self.shown = true;
        self.follow = false;
        self.selected = None;
        self.pending_reveal = Some((lane, page));
        cx.notify();
    }
}

// ── drawing ──

impl BrowserPanel {
    fn lane_label(&self, lane: &str, l: &Lane) -> (String, Option<Rgba>) {
        if lane == MINE {
            return (t!("browser.my_tabs").to_owned(), None);
        }
        let p = self.palette;
        match l.session.and_then(|s| self.who.get(&s)) {
            Some(who) => {
                let about = l.member.as_deref().and_then(|m| m.split_once(" (")).map(|(_, a)| a.trim_end_matches(')').to_owned());
                let label = match about {
                    Some(about) if about != "the whole squad" => format!("{} · {about}", who.name),
                    _ => who.name.clone(),
                };
                (label, Some(p.member(who.hue)))
            }
            None => (
                l.member.as_deref().map(|m| m.split_once(" (").map_or(m, |(name, _)| name).to_owned()).unwrap_or_else(|| lane.rsplit('/').next().unwrap_or(lane).to_owned()),
                l.color.as_deref().and_then(parse_color),
            ),
        }
    }

    /// A member's name, bare (no "· project").
    fn member_name(&self, lane: &str) -> String {
        match self.lanes.get(lane) {
            Some(l) => match l.session.and_then(|s| self.who.get(&s)) {
                Some(who) => who.name.clone(),
                None => self.lane_label(lane, l).0,
            },
            None => lane.rsplit('/').next().unwrap_or(lane).split('@').next().unwrap_or(lane).to_owned(),
        }
    }

    fn lane_color(&self, lane: &str) -> Rgba {
        self.lanes.get(lane).and_then(|l| self.lane_label(lane, l).1).unwrap_or(self.palette.accent)
    }

    fn working(&self, lane: &str) -> bool {
        self.lanes.get(lane).and_then(|l| l.session).and_then(|s| self.who.get(&s)).is_some_and(|w| w.working)
    }

    /// What the bar and banner say of the tab on screen.
    fn mode(&self) -> Option<Mode> {
        let lane = self.lane_of_selected()?;
        if lane == MINE {
            return Some(Mode::Mine);
        }
        let check = self.page().and_then(|p| p.check);
        Some(self.hands.get(lane).cloned().unwrap_or_default().mode(check, self.working(lane)))
    }

    /// The lanes in the strip's order.
    fn ordered(&self) -> Vec<String> {
        let names: Vec<(String, Option<SessionId>, String)> = self
            .lanes
            .iter()
            .filter(|(lane, l)| lane.as_str() == MINE || l.squad.as_deref() == Some(self.squad.as_str()))
            .map(|(lane, l)| (lane.clone(), l.session, if lane == MINE { String::new() } else { self.member_name(lane) }))
            .collect();
        let refs = names.iter().map(|(lane, session, name)| LaneRef { lane, session: *session, name }).collect();
        browse::order(refs, self.viewed).into_iter().map(|l| l.lane.to_owned()).collect()
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let mut row = h_flex().id("browser-tabs").w_full().h(px(40.)).gap_1().pl_2().pr_1().items_center().overflow_x_scroll().track_scroll(&self.tabs_scroll);
        // Which child the picked tab is, to scroll it into sight once picked.
        let (mut children, mut picked) = (0usize, None);
        for (n, lane) in self.ordered().into_iter().enumerate() {
            let Some(l) = self.lanes.get(&lane) else { continue };
            let (label, color) = self.lane_label(&lane, l);
            let dot = color.unwrap_or(p.muted);
            // The group's label: a dot and the member's name.
            children += 1;
            row = row.child(
                h_flex()
                    .flex_none()
                    .gap(px(5.))
                    .when(n > 0, |g| g.ml_2())
                    .pr_1()
                    .items_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text2)
                    .child(div().size(px(7.)).flex_none().rounded_full().bg(dot))
                    .child(div().whitespace_nowrap().child(label)),
            );
            for page in &l.pages {
                let on = self.selected.as_ref() == Some(&(lane.clone(), page.id));
                let (lane_id, page_id) = (lane.clone(), page.id);
                let title = browse::tab_title(page);
                let mine = lane == MINE;
                let group = SharedString::from(format!("tab-{}", page.id));
                let waits = page.check.is_some();
                if on {
                    picked = Some(children);
                }
                children += 1;
                row = row.child(
                    h_flex()
                        .id(SharedString::from(format!("tab-{}", page.id)))
                        .group(group.clone())
                        .flex_none()
                        .max_w(px(184.))
                        .h(px(CONTROL))
                        .pl(px(8.))
                        .pr(px(if mine { 4. } else { 10. }))
                        .gap(px(6.))
                        .items_center()
                        .rounded(px(R_MD))
                        .border_1()
                        .border_color(if on { p.hair } else { rgba(0) })
                        .when(on, |t| t.bg(p.raised))
                        .text_size(px(12.))
                        .text_color(if on { p.text } else { p.text2 })
                        .cursor_pointer()
                        .when(!on, |t| t.hover(move |s| s.bg(p.wash)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // Picking a tab of the conversation's own browser keeps following it.
                            let followed = this.viewed.is_some() && this.lanes.get(&lane_id).is_some_and(|l| l.session == this.viewed);
                            this.follow = followed;
                            this.select(lane_id.clone(), page_id, window, cx);
                        }))
                        .child(favicon(p, page))
                        .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
                        .when(page.loading, |t| t.child({ use gpui_kit::component::Sizable as _; gpui_kit::component::spinner::Spinner::new().with_size(px(11.)) }.color(Hsla::from(p.muted))))
                        .when(waits && !page.loading, |t| t.child(div().size(px(6.)).flex_none().rounded_full().bg(p.amber)))
                        .when(mine, |t| {
                            t.child(
                                div()
                                    .id(SharedString::from(format!("close-{}", page.id)))
                                    .size(px(18.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .when(!on, |c| c.invisible().group_hover(group.clone(), |s| s.visible()))
                                    .hover(move |s| s.bg(p.wash))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.closing = Some(page_id);
                                        this.command(Command::BrowserCloseTab { page: page_id }, cx);
                                    }))
                                    .child(icon(Icon::X, 12., p.muted)),
                            )
                        }),
                );
            }
        }
        let row = row.child(ui::icon_button(p, "browser-new-tab", Icon::Plus, t!("browser.new_tab"), None).on_click(cx.listener(|this, _, _, cx| this.new_tab(None, cx))));
        // Fading where there is more to scroll to.
        if self.reveal_tab.take()
            && let Some(ix) = picked
        {
            self.tabs_scroll.scroll_to_item(ix);
        }
        let (off, max) = (f32::from(self.tabs_scroll.offset().x), f32::from(self.tabs_scroll.max_offset().x));
        let clear = Hsla::from(p.bg).alpha(0.);
        let fade = |left: bool| {
            let (from, to) = if left { (Hsla::from(p.bg), clear) } else { (clear, Hsla::from(p.bg)) };
            div().absolute().top_0().bottom_0().w(px(24.)).when(left, |d| d.left_0()).when(!left, |d| d.right_0()).bg(linear_gradient(90., linear_color_stop(from, 0.), linear_color_stop(to, 1.)))
        };
        div()
            .relative()
            .flex_none()
            .w_full()
            .overflow_hidden()
            .child(row)
            .when(off < -1., |d| d.child(fade(true)))
            .when(max > 1. && -off < max - 1., |d| d.child(fade(false)))
    }

    fn toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let page = self.page().cloned();
        let loading = page.as_ref().is_some_and(|pg| pg.loading);
        let none = page.is_none();
        let typing = self.url.read(cx).focus_handle(cx).is_focused(window);
        let shown = page.as_ref().and_then(|pg| browse::address(&pg.url)).filter(|_| !typing);
        // Room for the words beside the icons: 따라가기 from 420 points, Chrome에서 열기 from 640.
        let (wide, wider) = (self.width >= 420., self.width >= 640.);
        h_flex()
            .px_2()
            .pb_2()
            .gap_1()
            .items_center()
            .child(ui::icon_button(p, "browser-back", Icon::ArrowLeft, t!("browser.back"), Some("⌘[")).disabled(none).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Back, cx))))
            .child(ui::icon_button(p, "browser-forward", Icon::ArrowRight, t!("browser.forward"), Some("⌘]")).disabled(none).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Forward, cx))))
            .child(if loading {
                ui::icon_button(p, "browser-reload", Icon::X, t!("browser.stop"), None).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Stop, cx)))
            } else {
                ui::icon_button(p, "browser-reload", Icon::Reload, t!("browser.reload"), Some("⌘R")).disabled(none).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Reload, cx)))
            })
            .child(
                div()
                    .id("browser-address")
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(CONTROL))
                    .pl(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(R_MD))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(if typing { p.accent } else { p.hair })
                    .text_size(px(SMALL))
                    .child(Input::new(&self.url).appearance(false).bordered(false))
                    // Out of focus: the site large, the rest quiet, a lock for https. A click types.
                    .children(shown.map(|a| {
                        h_flex()
                            .id("browser-address-shown")
                            .absolute()
                            .inset_0()
                            .px(px(10.))
                            .gap(px(6.))
                            .items_center()
                            .rounded(px(R_MD))
                            .bg(p.code_bg)
                            .cursor_text()
                            .overflow_hidden()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.url.update(cx, |s, cx| s.focus(window, cx));
                                cx.notify();
                            }))
                            .when(a.secure, |r| r.child(icon(Icon::Lock, 12., p.muted)))
                            .child(div().flex_none().whitespace_nowrap().text_color(p.text).child(a.host))
                            .child(div().min_w_0().flex_1().whitespace_nowrap().overflow_hidden().text_color(p.muted).child(a.rest))
                    })),
            )
            .child(tip(
                div()
                    .id("browser-follow")
                    .flex_none()
                    .h(px(CONTROL))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded(px(R_MD))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(self.follow, |b| b.bg(p.accent_bg).text_color(p.accent))
                    .when(!self.follow, |b| b.text_color(p.muted).hover(move |s| s.bg(p.wash)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.follow = !this.follow;
                        this.follow_viewed(true, window, cx);
                        cx.notify();
                    }))
                    .child(icon(Icon::Locate, ICON_SM, if self.follow { p.accent } else { p.muted }))
                    .when(wide, |b| b.child(t!("browser.follow"))),
                p,
                t!("browser.follow_tip"),
                None,
            ))
            .child(if wider {
                ui::button(p, "browser-pop-out", Kind::Ghost, t!("browser.open_in_chrome"))
                    .icon(Icon::External)
                    .tip(t!("browser.open_in_chrome_tip"), None)
                    .disabled(none)
                    .on_click(cx.listener(|this, _, window, cx| this.pop_out(window, cx)))
            } else {
                ui::icon_button(p, "browser-pop-out", Icon::External, t!("browser.open_in_chrome"), None).disabled(none).on_click(cx.listener(|this, _, window, cx| this.pop_out(window, cx)))
            })
    }

    fn viewport(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let entity = cx.entity();
        let focus = self.focus.clone();
        let frame = self.picture.frame().cloned();
        let placed = self.placed.clone();
        let viewport = self.viewport.clone();
        let mode = self.mode();
        let page = self.page().cloned();
        let lane = self.lane_of_selected().map(str::to_owned);
        // The person drives: the picture says so with the accent; a member at
        // work, faintly in its colour (the bar's tooltip says what it means).
        let ring: Option<(Rgba, f32)> = match &mode {
            Some(Mode::Person | Mode::Waiting { touched: true, .. } | Mode::Passed | Mode::Asked(_)) => Some((p.accent, 2.)),
            Some(Mode::Member { working: true }) => lane.as_deref().map(|l| (Rgba { a: 0.55, ..self.lane_color(l) }, 1.)),
            _ => None,
        };
        let cover = self.cover(page.as_ref(), cx);
        let dialog = self.selected.as_ref().and_then(|(_, page)| self.dialogs.iter().find(|d| d.page == *page)).cloned();
        let banner = mode.as_ref().and_then(|m| self.banner(m, page.as_ref(), cx));
        div()
            .id("browser-viewport")
            .flex_1()
            .min_h_0()
            .relative()
            .bg(p.side)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| this.key_up(e)))
            .on_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| this.mouse_down(e, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| this.mouse_move(e)))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.scroll(e, cx)))
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        // A new box: drawn again, so the page is asked for at it.
                        let moved = viewport.borrow().is_none_or(|b| b.size != bounds.size);
                        *viewport.borrow_mut() = Some(bounds);
                        if moved {
                            window.refresh();
                        }
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        let Some((header, image)) = frame else {
                            *placed.borrow_mut() = None;
                            return;
                        };
                        let (o, s) = stream::fit(
                            (f64::from(f32::from(bounds.origin.x)), f64::from(f32::from(bounds.origin.y))),
                            (f64::from(f32::from(bounds.size.width)), f64::from(f32::from(bounds.size.height))),
                            f64::from(header.w),
                            f64::from(header.h),
                        );
                        // Whole points: no hairline of the background at an edge.
                        let rect = Bounds::new(point(px(o.0.round() as f32), px(o.1.round() as f32)), size(px(s.0.round() as f32), px(s.1.round() as f32)));
                        let _ = window.paint_image(rect, rect, Corners::default(), image, 0, false);
                        if let Some((color, width)) = ring {
                            let mut quad = outline(rect, color, BorderStyle::default());
                            quad.border_widths = px(width).into();
                            window.paint_quad(quad);
                        }
                        *placed.borrow_mut() = Some((rect, header));
                    },
                )
                .size_full(),
            )
            .children(cover)
            .children(banner)
            .children(dialog.map(|d| self.dialog_card(d, cx)))
    }

    /// What covers the picture when there is none to show: what the panel is
    /// for, a skeleton while the page comes, or what went wrong and a way on.
    fn cover(&self, page: Option<&BrowserPage>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette;
        let card = |title: String, body: String| {
            v_flex()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .px_6()
                .gap_2()
                .bg(p.side)
                .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).text_center().child(title))
                .child(div().max_w(px(340.)).text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).text_center().child(body))
        };
        let empty = self.lanes.values().all(|l| l.pages.is_empty());
        if self.selected.is_none() {
            if let Some(gone) = self.gone.clone() {
                let url = gone.url.clone();
                return Some(
                    card(t!("browser.gone_title", name = gone.name), t!("browser.gone_body").to_owned())
                        .child(div().h(px(6.)))
                        .child(ui::button(p, "browser-gone-retry", Kind::Secondary, t!("browser.gone_retry")).icon(Icon::Retry).on_click(cx.listener(move |this, _, _, cx| {
                            this.gone = None;
                            this.new_tab(url.clone(), cx);
                        })))
                        .into_any_element(),
                );
            }
            if empty {
                return Some(
                    card(t!("browser.empty_title").to_owned(), t!("browser.empty_body").to_owned())
                        .child(div().h(px(6.)))
                        .child(ui::button(p, "browser-empty-new", Kind::Secondary, t!("browser.new_tab_button")).icon(Icon::Plus).on_click(cx.listener(|this, _, _, cx| this.new_tab(None, cx))))
                        .into_any_element(),
                );
            }
            return Some(card(t!("browser.pick_a_tab").to_owned(), String::new()).into_any_element());
        }
        // The line down a while (it is tried again meanwhile).
        if self.down_since.is_some_and(|at| at.elapsed() >= DOWN_AFTER) {
            return Some(
                card(t!("browser.down_title").to_owned(), t!("browser.down_body").to_owned())
                    .child(div().h(px(6.)))
                    .child(ui::button(p, "browser-reconnect", Kind::Secondary, t!("browser.retry")).icon(Icon::Retry).on_click(cx.listener(|this, _, window, cx| this.reconnect(window, cx))))
                    .into_any_element(),
            );
        }
        if page.is_some_and(|pg| pg.crashed) {
            return Some(
                card(t!("browser.crashed_title").to_owned(), t!("browser.crashed_body").to_owned())
                    .child(div().h(px(6.)))
                    .child(ui::button(p, "browser-crashed-reload", Kind::Primary, t!("browser.reload")).icon(Icon::Reload).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Reload, cx))))
                    .into_any_element(),
            );
        }
        if !self.picture.waiting() {
            return None;
        }
        // Asked, and nothing came for a while: offer to ask again.
        let stalled = self.connected && page.is_some_and(|pg| !pg.loading) && self.asked_at.is_some_and(|at| at.elapsed() >= browse::STALL);
        if stalled {
            return Some(
                card(t!("browser.stalled_title").to_owned(), t!("browser.stalled_body").to_owned())
                    .child(div().h(px(6.)))
                    .child(ui::button(p, "browser-stalled-retry", Kind::Secondary, t!("browser.retry")).icon(Icon::Retry).on_click(cx.listener(|this, _, window, cx| this.rewatch_fresh(window, cx))))
                    .into_any_element(),
            );
        }
        let what = if self.connected { t!("browser.loading") } else { t!("browser.connecting") };
        Some(skeleton(p, what).into_any_element())
    }

    /// Over the picture: a page waiting for a person, or past it.
    fn banner(&self, mode: &Mode, page: Option<&BrowserPage>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette;
        let lane = self.lane_of_selected()?.to_owned();
        let name = self.member_name(&lane);
        let site = page.and_then(|pg| browse::site(&pg.url)).unwrap_or_default();
        let shell = |id: &'static str| {
            h_flex()
                .id(id)
                .absolute()
                .top(px(12.))
                .left(px(12.))
                .right(px(12.))
                .p_3()
                .gap_3()
                .items_center()
                .rounded(px(R_LG))
                .bg(p.popover())
                .border_1()
                .border_color(p.hair)
                .shadow(motion::float_shadow(if p.is_dark() { 0.5 } else { 0.14 }))
                // The page underneath doesn't take the banner's clicks.
                .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        };
        match mode {
            Mode::Waiting { check, touched: false } => {
                let detail = match check {
                    HumanCheck::Challenge | HumanCheck::Captcha => t!("browser.check_human", site = site, name = name),
                    HumanCheck::SignIn => t!("browser.check_sign_in", site = site, name = name),
                    HumanCheck::TwoFactor => t!("browser.check_code", site = site, name = name),
                };
                Some(
                    motion::rise(
                        shell("browser-check")
                            .child(div().size(px(28.)).flex_none().rounded_full().bg(p.amber.alpha(0.16)).flex().items_center().justify_center().child(icon(Icon::Hand, ICON_SM, p.amber)))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("browser.check_title")))
                                    .child(div().text_size(px(12.)).text_color(p.muted).child(detail)),
                            )
                            .child(ui::button(p, "browser-check-take", Kind::Primary, t!("browser.take")).icon(Icon::Hand).on_click(cx.listener(|this, _, window, cx| this.take(window, cx)))),
                        "browser-check-rise",
                    )
                    .into_any_element(),
                )
            }
            // On it: a quiet line at the top, out of the way of the check itself.
            Mode::Waiting { touched: true, .. } => Some(
                h_flex()
                    .absolute()
                    .top(px(8.))
                    .left_0()
                    .right_0()
                    .justify_center()
                    .child(
                        h_flex()
                            .px_3()
                            .h(px(26.))
                            .gap_2()
                            .items_center()
                            .rounded_full()
                            .bg(p.popover())
                            .border_1()
                            .border_color(p.hair)
                            .text_size(px(12.))
                            .text_color(p.text2)
                            .child(div().size(px(6.)).rounded_full().bg(p.amber))
                            .child(t!("browser.check_on_it", name = name)),
                    )
                    .into_any_element(),
            ),
            Mode::Passed => Some(
                motion::rise(
                    shell("browser-passed")
                        .child(div().size(px(28.)).flex_none().rounded_full().bg(p.accent_bg).flex().items_center().justify_center().child(icon(Icon::Done, ICON_SM, p.accent)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("browser.passed_title")))
                                .child(div().text_size(px(12.)).text_color(p.muted).child(t!("browser.passed_body", name = name))),
                        )
                        .child(ui::button(p, "browser-passed-keep", Kind::Ghost, t!("browser.keep")).on_click(cx.listener(|this, _, _, cx| this.keep(cx))))
                        .child(ui::button(p, "browser-passed-back", Kind::Primary, t!("browser.give_back_short")).on_click(cx.listener(|this, _, _, cx| this.give_back(cx)))),
                    "browser-passed-rise",
                )
                .into_any_element(),
            ),
            _ => None,
        }
    }

    fn dialog_card(&self, d: Dialog, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let asks = d.kind != "alert";
        div().absolute().inset_0().flex().items_start().justify_center().pt(px(48.)).child(motion::dialog(
            v_flex()
                .w(px(360.))
                .p_4()
                .gap_3()
                .rounded(px(R_LG))
                .bg(p.popover())
                .border_1()
                .border_color(p.hair)
                .child(div().text_size(px(12.)).text_color(p.muted).child(t!("browser.page_asks")))
                .child(div().text_size(px(UI)).text_color(p.text).child(d.message.clone()))
                .when(d.kind == "prompt", |c| {
                    c.child(
                        div()
                            .h(px(36.))
                            .pl(px(2.))
                            .flex()
                            .items_center()
                            .rounded(px(R_MD))
                            .bg(p.code_bg)
                            .border_1()
                            .border_color(p.hair)
                            .child(Input::new(&self.prompt).appearance(false).bordered(false)),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .when(asks, |r| r.child(ui::button(p, "dialog-cancel", Kind::Secondary, t!("browser.cancel")).on_click(cx.listener(|this, _, _, cx| this.answer(false, None, cx)))))
                        .child(ui::button(p, "dialog-ok", Kind::Primary, t!("browser.ok")).on_click(cx.listener(|this, _, _, cx| {
                            let text = this.prompt.read(cx).value().to_string();
                            this.answer(true, Some(text), cx)
                        }))),
                ),
            ("browser-dialog", d.page),
        ))
    }

    /// The control bar: who has the page in hand, doing what, and the one
    /// button that takes it or gives it back.
    fn control_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let bar = h_flex().h(px(44.)).flex_none().px_3().gap_2().items_center().border_t_1().border_color(p.hair).text_size(px(12.)).text_color(p.text2);
        let Some(mode) = self.mode() else {
            return bar.child(div().text_color(p.muted).child(t!("browser.status_idle")));
        };
        let lane = self.lane_of_selected().unwrap_or(MINE).to_owned();
        let name = self.member_name(&lane);
        let color = self.lane_color(&lane);
        let what = self.page().map(browse::tab_title).filter(|s| !s.is_empty()).unwrap_or_else(|| t!("browser.new_tab").to_owned());
        let dot = |c: Rgba| div().size(px(8.)).flex_none().rounded_full().bg(c);
        let line = |text: String, c: Rgba| div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(c).child(text);
        let take = || ui::button(p, "browser-take", Kind::Secondary, t!("browser.take")).icon(Icon::Hand).tip(t!("browser.take_tip", name = name), None).on_click(cx.listener(|this, _, window, cx| this.take(window, cx)));
        let back = |kind: Kind, label: String| ui::button(p, "browser-give-back", kind, label).tip(t!("browser.give_back_tip", name = name), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.give_back(cx)));
        match mode {
            Mode::Mine => bar.child(dot(p.muted)).child(line(t!("browser.status_mine").to_owned(), p.muted)),
            Mode::Member { working } => {
                let text = if working { t!("browser.bar_member", name = name, what = what) } else { t!("browser.bar_member_idle", name = name) };
                let tipped = tip(
                    h_flex().id("browser-bar-who").flex_1().min_w_0().gap_2().items_center().child(dot(color)).child(line(text, p.text2)),
                    p,
                    if working { t!("browser.bar_member_tip", name = name) } else { t!("browser.take_tip", name = name) },
                    None,
                );
                bar.child(tipped).child(take())
            }
            Mode::Person => bar
                .child(dot(p.accent))
                .child(line(t!("browser.bar_mine_hand").to_owned(), p.accent))
                .child(back(Kind::Secondary, t!("browser.give_back", name = name))),
            Mode::Asked(why) => bar
                .child(dot(color))
                .child(line(t!("browser.status_asked", name = name, why = why), p.text))
                .child(back(Kind::Primary, t!("browser.done_give_back", name = name))),
            Mode::Waiting { touched, .. } => {
                let b = bar.child(dot(p.amber)).child(line(t!("browser.bar_waiting", name = name), p.text2));
                if touched { b.child(back(Kind::Secondary, t!("browser.give_back", name = name))) } else { b.child(take()) }
            }
            Mode::Passed => bar
                .child(dot(p.accent))
                .child(line(t!("browser.passed_title").to_owned(), p.text))
                .child(back(Kind::Primary, t!("browser.give_back", name = name))),
        }
    }
}

/// A tab's icon: the page's own, else a globe.
fn favicon(p: Palette, page: &BrowserPage) -> AnyElement {
    let globe = move || icon(Icon::Globe, 14., p.muted).into_any_element();
    match page.favicon.clone() {
        Some(url) => img(SharedUri::from(url)).size(px(14.)).flex_none().rounded(px(3.)).with_fallback(globe).with_loading(globe).into_any_element(),
        None => globe(),
    }
}

/// Where the page will be, while it comes: a toolbar's band and lines of text.
fn skeleton(p: Palette, what: &'static str) -> impl IntoElement {
    let block = |w: f32, h: f32| div().w(relative(w)).h(px(h)).rounded(px(R_SM)).bg(p.wash);
    v_flex()
        .absolute()
        .inset_0()
        .bg(p.side)
        .p_5()
        .gap_3()
        .child(block(1., 36.))
        .child(div().h(px(8.)))
        .child(block(0.55, 18.))
        .child(block(0.9, 12.))
        .child(block(0.82, 12.))
        .child(block(0.7, 12.))
        .child(div().h(px(8.)))
        .child(block(1., 120.))
        .child(h_flex().pt_2().justify_center().text_size(px(12.)).text_color(p.muted).child(what))
}

impl Render for BrowserPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A page waiting for the person, picked from 나에게: on screen, in their hands.
        if let Some((lane, page)) = self.pending_reveal.take() {
            self.ensure_link(window, cx);
            self.select(lane, page, window, cx);
            self.take(window, cx);
        }
        if std::mem::take(&mut self.focus_next) {
            self.focus_page(window, cx);
        }
        // The viewport's size may have moved since the stream was asked.
        self.rewatch(window, cx);
        let p = self.palette;
        let dragging = self.drag.is_some();
        h_flex()
            .h_full()
            .w(px(self.width))
            .flex_none()
            .font_family(SANS)
            .child(
                // The divider: drag to resize.
                div()
                    .id("browser-split")
                    .w(px(7.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            this.drag = Some((f32::from(e.position.x), this.width));
                            cx.notify();
                        }),
                    )
                    .child(div().h_full().w(px(if dragging { 2. } else { 1. })).bg(if dragging { p.accent } else { p.hair })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(p.bg)
                    .child(self.tab_strip(cx))
                    .child(self.toolbar(window, cx))
                    .child(self.viewport(cx))
                    .child(self.control_bar(cx)),
            )
    }
}

/// For `--features snapshot` scripts: the panel driven as the person would.
#[cfg(feature = "snapshot")]
impl BrowserPanel {
    /// Where the page's picture is drawn, in window coordinates.
    pub fn picture(&self) -> Option<Bounds<Pixels>> {
        self.placed.borrow().as_ref().map(|(rect, _)| *rect)
    }

    /// Pick the `n`th tab (from 1) in the strip's order.
    pub fn pick(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let all: Vec<(String, u64)> = self.ordered().into_iter().flat_map(|lane| self.lanes.get(&lane).map(|l| l.pages.iter().map(|p| (lane.clone(), p.id)).collect::<Vec<_>>()).unwrap_or_default()).collect();
        if let Some((lane, page)) = n.checked_sub(1).and_then(|i| all.get(i)).cloned() {
            self.follow = false;
            self.select(lane, page, window, cx);
        }
    }

    /// 직접 하기, or 돌려주기, as the bar's button.
    pub fn hand(&mut self, take: bool, window: &mut Window, cx: &mut Context<Self>) {
        if take { self.take(window, cx) } else { self.give_back(cx) }
    }

    /// A tab of the person's own at `url`, picked when it comes.
    pub fn open_tab(&mut self, url: &str, cx: &mut Context<Self>) {
        self.new_tab(Some(url.to_owned()), cx);
    }

    /// Compose each of `steps` in turn (`ㅎ`, `하`, `한`), then commit the last.
    pub fn compose(&mut self, steps: &[&str], window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        for step in steps {
            self.replace_and_mark_text_in_range(None, step, None, window, cx);
        }
        if let Some(last) = steps.last() {
            self.replace_text_in_range(None, last, window, cx);
        }
    }
}

impl BrowserPanel {
    /// The window's mouse moves while the divider is dragged (called from the
    /// window's root, which sees the pointer wherever it goes).
    pub fn drag_to(&mut self, x: f32, pressed: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some((x0, w0)) = self.drag else { return false };
        if !pressed {
            self.drag = None;
            cx.emit(PanelEvent::Resized(self.width));
        } else {
            let max = (f32::from(window.viewport_size().width) - CHAT_MIN - 220.).max(MIN_WIDTH);
            self.width = (w0 + x0 - x).clamp(MIN_WIDTH, max);
        }
        cx.notify();
        true
    }
}

/// The text box the platform types into: what the person composes (Korean,
/// Japanese…) goes to the page as composition, and what they commit as text.
impl EntityInputHandler for BrowserPanel {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let marked = self.marked.as_deref().unwrap_or("");
        let units: Vec<u16> = marked.encode_utf16().collect();
        let range = range.start.min(units.len())..range.end.min(units.len());
        *adjusted = Some(range.clone());
        Some(String::from_utf16_lossy(&units[range]))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let len = self.marked.as_deref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: len..len, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        if let Some(text) = self.marked.take().filter(|t| !t.is_empty()) {
            self.send_input(InputEvent::Insert { text });
        }
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if text.is_empty() {
            self.send_input(InputEvent::Ime { text: String::new() });
            return;
        }
        // Control characters come as keys, not text.
        if text.chars().all(char::is_control) {
            return;
        }
        self.hold();
        self.send_input(InputEvent::Insert { text: text.to_owned() });
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.hold();
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        self.send_input(InputEvent::Ime { text: text.to_owned() });
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, element: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // Where the person last clicked: near the field they type into.
        let at = self.last_click.unwrap_or(point(element.origin.x + px(16.), element.origin.y + px(16.)));
        Some(Bounds::new(at, size(px(2.), px(18.))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

// ── in 나에게: a member's page waiting for the person ──

/// The row's words: what waits, the member's name and colour.
pub fn need_row(shell: &crate::Shell, page: u64) -> Option<(String, String, Rgba)> {
    let wait = shell.model.browser_waits.iter().find(|w| w.page == page)?;
    let member = wait.session.and_then(|s| shell.model.members.iter().find(|m| m.session == s || shell.model.lanes.iter().any(|l| l.session == s && l.of == m.session)));
    let color = member.map_or(shell.palette.muted, |m| shell.palette.member(crate::theme::look(&m.role, &m.name).1));
    Some((need_words(wait.check, &wait.site), wait.member.clone(), color))
}

fn need_words(check: HumanCheck, site: &str) -> String {
    match check {
        HumanCheck::Challenge | HumanCheck::Captcha => t!("browser.need_human", site = site),
        HumanCheck::SignIn => t!("browser.need_sign_in", site = site),
        HumanCheck::TwoFactor => t!("browser.need_code", site = site),
    }
}

/// The row's mark: a hand, in the colour of what waits for the person.
pub fn need_mark(p: Palette) -> AnyElement {
    tip(div().id("browser-need-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Hand, 13., p.amber)), p, t!("browser.check_title"), None).into_any_element()
}

/// The peek: what waits, why no member can, and the one way on.
pub fn need_peek(shell: &crate::Shell, page: u64, cx: &mut Context<crate::Shell>) -> AnyElement {
    let p = shell.palette;
    let Some(wait) = shell.model.browser_waits.iter().find(|w| w.page == page).cloned() else { return div().into_any_element() };
    let detail = match wait.check {
        HumanCheck::Challenge | HumanCheck::Captcha => t!("browser.check_human", site = wait.site, name = wait.member),
        HumanCheck::SignIn => t!("browser.check_sign_in", site = wait.site, name = wait.member),
        HumanCheck::TwoFactor => t!("browser.check_code", site = wait.site, name = wait.member),
    };
    v_flex()
        .id("peek")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("state.status_waiting")))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("browser.check_title")))
        .child(div().text_size(px(12.)).text_color(p.muted).child(need_words(wait.check, &wait.site)))
        .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(detail))
        .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("browser.need_why")))
        .child(h_flex().pt_1().child(ui::button(p, "browser-need-take", Kind::Primary, t!("browser.need_open")).icon(Icon::Hand).on_click(cx.listener(move |shell, _, _, cx| shell.reveal_browser(page, cx)))))
        .into_any_element()
}

impl crate::Shell {
    /// A member's page waiting for the person (from 나에게, a notification):
    /// the conversation with the browser panel beside it, on that page, in
    /// their hands.
    pub fn reveal_browser(&mut self, page: u64, cx: &mut Context<Self>) {
        self.show(crate::Screen::Chat, cx);
        self.browser.update(cx, |b, cx| b.reveal(page, cx));
        cx.notify();
    }
}

/// `r, g, b` as the host gives a member's colour.
fn parse_color(text: &str) -> Option<Rgba> {
    let parts: Vec<u8> = text.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    let [r, g, b] = parts[..] else { return None };
    Some(rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)))
}

/// The person's keys and mouse, as Chrome's `Input` domain takes them — with
/// the editing commands a Mac binds to keys (Chrome on a Mac edits text through
/// them, not through the key itself).
pub mod keys {
    use equipe_core::protocol::stream::{InputEvent, modifiers as bits};
    use gpui_kit::{Keystroke, Modifiers, MouseButton};

    pub fn modifier_bits(m: &Modifiers) -> u32 {
        let mut out = 0;
        if m.alt {
            out |= bits::ALT;
        }
        if m.control {
            out |= bits::CTRL;
        }
        if m.platform {
            out |= bits::META;
        }
        if m.shift {
            out |= bits::SHIFT;
        }
        out
    }

    pub fn button_name(b: MouseButton) -> &'static str {
        match b {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            _ => "none",
        }
    }

    pub fn button_bit(b: MouseButton) -> u32 {
        match b {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            _ => 0,
        }
    }

    /// A key's DOM name, code and Windows key code — the three Chrome wants.
    /// `None` for keys that only change others (shift alone).
    pub fn dom(key: &str, key_char: Option<&str>) -> Option<(String, String, u32)> {
        let named = |k: &str, code: &str, vk: u32| Some((k.to_owned(), code.to_owned(), vk));
        match key {
            "enter" => named("Enter", "Enter", 13),
            "backspace" => named("Backspace", "Backspace", 8),
            "tab" => named("Tab", "Tab", 9),
            "escape" => named("Escape", "Escape", 27),
            "space" => named(" ", "Space", 32),
            "delete" => named("Delete", "Delete", 46),
            "left" => named("ArrowLeft", "ArrowLeft", 37),
            "up" => named("ArrowUp", "ArrowUp", 38),
            "right" => named("ArrowRight", "ArrowRight", 39),
            "down" => named("ArrowDown", "ArrowDown", 40),
            "home" => named("Home", "Home", 36),
            "end" => named("End", "End", 35),
            "pageup" => named("PageUp", "PageUp", 33),
            "pagedown" => named("PageDown", "PageDown", 34),
            k if k.len() >= 2 && k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                let n: u32 = k[1..].parse().ok()?;
                Some((k.to_uppercase(), k.to_uppercase(), 111 + n))
            }
            k if k.chars().count() == 1 => {
                let c = k.chars().next()?;
                let shown = key_char.filter(|s| !s.is_empty()).unwrap_or(k).to_owned();
                if c.is_ascii_alphabetic() {
                    let up = c.to_ascii_uppercase();
                    return Some((shown, format!("Key{up}"), up as u32));
                }
                if c.is_ascii_digit() {
                    return Some((shown, format!("Digit{c}"), c as u32));
                }
                let (code, vk) = match c {
                    '-' => ("Minus", 189),
                    '=' => ("Equal", 187),
                    '[' => ("BracketLeft", 219),
                    ']' => ("BracketRight", 221),
                    '\\' => ("Backslash", 220),
                    ';' => ("Semicolon", 186),
                    '\'' => ("Quote", 222),
                    ',' => ("Comma", 188),
                    '.' => ("Period", 190),
                    '/' => ("Slash", 191),
                    '`' => ("Backquote", 192),
                    _ => ("", 0),
                };
                Some((shown, code.to_owned(), vk))
            }
            _ => None,
        }
    }

    /// The Mac editing commands for a key with modifiers, as Chrome's DevTools
    /// takes them (`deleteBackward`, `selectAll`…). Those that insert text are
    /// left out: the text goes as text.
    pub fn commands(key: &str, m: &Modifiers) -> Vec<String> {
        let (shift, alt, cmd, ctrl) = (m.shift, m.alt, m.platform, m.control);
        let one = |c: &str| vec![c.to_owned()];
        let pick = |plain: &str, shifted: &str| if shift { one(shifted) } else { one(plain) };
        match (key, alt, cmd, ctrl) {
            ("backspace", false, false, false) => one("deleteBackward"),
            ("backspace", true, false, false) => one("deleteWordBackward"),
            ("backspace", false, true, false) => one("deleteToBeginningOfLine"),
            ("delete", false, false, false) => one("deleteForward"),
            ("delete", true, false, false) => one("deleteWordForward"),
            ("left", false, false, false) => pick("moveLeft", "moveLeftAndModifySelection"),
            ("right", false, false, false) => pick("moveRight", "moveRightAndModifySelection"),
            ("up", false, false, false) => pick("moveUp", "moveUpAndModifySelection"),
            ("down", false, false, false) => pick("moveDown", "moveDownAndModifySelection"),
            ("left", true, false, false) => pick("moveWordLeft", "moveWordLeftAndModifySelection"),
            ("right", true, false, false) => pick("moveWordRight", "moveWordRightAndModifySelection"),
            ("left", false, true, false) | ("left", false, false, true) => pick("moveToLeftEndOfLine", "moveToLeftEndOfLineAndModifySelection"),
            ("right", false, true, false) | ("right", false, false, true) => pick("moveToRightEndOfLine", "moveToRightEndOfLineAndModifySelection"),
            ("up", false, true, false) => pick("moveToBeginningOfDocument", "moveToBeginningOfDocumentAndModifySelection"),
            ("down", false, true, false) => pick("moveToEndOfDocument", "moveToEndOfDocumentAndModifySelection"),
            ("home", false, false, false) => pick("scrollToBeginningOfDocument", "moveToBeginningOfDocumentAndModifySelection"),
            ("end", false, false, false) => pick("scrollToEndOfDocument", "moveToEndOfDocumentAndModifySelection"),
            ("pageup", false, false, false) => pick("scrollPageUp", "pageUpAndModifySelection"),
            ("pagedown", false, false, false) => pick("scrollPageDown", "pageDownAndModifySelection"),
            ("a", false, true, false) if !shift => one("selectAll"),
            ("c", false, true, false) if !shift => one("copy"),
            ("x", false, true, false) if !shift => one("cut"),
            ("z", false, true, false) => pick("undo", "redo"),
            ("a", false, false, true) => pick("moveToBeginningOfParagraph", "moveToBeginningOfParagraphAndModifySelection"),
            ("e", false, false, true) => pick("moveToEndOfParagraph", "moveToEndOfParagraphAndModifySelection"),
            ("k", false, false, true) => one("deleteToEndOfParagraph"),
            ("h", false, false, true) => one("deleteBackward"),
            ("d", false, false, true) => one("deleteForward"),
            _ => Vec::new(),
        }
    }

    /// A key going down. Printable keys carry their text; ⌘ chords and the
    /// rest go as raw keys, with their editing commands.
    pub fn key_down(stroke: &Keystroke) -> Option<InputEvent> {
        let m = &stroke.modifiers;
        let (key, code, vk) = dom(&stroke.key, stroke.key_char.as_deref())?;
        let chord = m.platform || m.control;
        let text = match stroke.key.as_str() {
            "enter" if !chord => Some("\r".to_owned()),
            "space" if !chord => Some(" ".to_owned()),
            k if k.chars().count() == 1 && !chord => stroke.key_char.clone().filter(|c| !c.is_empty() && !c.chars().any(char::is_control)),
            _ => None,
        };
        Some(InputEvent::Key {
            kind: if text.is_some() { "keyDown" } else { "rawKeyDown" }.into(),
            key,
            code,
            text,
            vk,
            modifiers: modifier_bits(m),
            commands: commands(&stroke.key, m),
            repeat: false,
        })
    }

    pub fn key_up(stroke: &Keystroke) -> Option<InputEvent> {
        let (key, code, vk) = dom(&stroke.key, stroke.key_char.as_deref())?;
        Some(InputEvent::Key { kind: "keyUp".into(), key, code, text: None, vk, modifiers: modifier_bits(&stroke.modifiers), commands: Vec::new(), repeat: false })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn stroke(key: &str, key_char: Option<&str>, m: Modifiers) -> Keystroke {
            Keystroke { modifiers: m, key: key.into(), key_char: key_char.map(Into::into) }
        }

        fn parts(ev: InputEvent) -> (String, String, String, Option<String>, u32, u32, Vec<String>) {
            match ev {
                InputEvent::Key { kind, key, code, text, vk, modifiers, commands, .. } => (kind, key, code, text, vk, modifiers, commands),
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn a_letter_goes_with_its_text_and_shift_makes_it_a_capital() {
            let (kind, key, code, text, vk, _, cmds) = parts(key_down(&stroke("a", Some("a"), Modifiers::default())).unwrap());
            assert_eq!((kind.as_str(), key.as_str(), code.as_str(), text.as_deref(), vk), ("keyDown", "a", "KeyA", Some("a"), 65));
            assert!(cmds.is_empty());
            let shifted = Modifiers { shift: true, ..Default::default() };
            let (_, key, _, text, _, mods, _) = parts(key_down(&stroke("a", Some("A"), shifted)).unwrap());
            assert_eq!((key.as_str(), text.as_deref(), mods), ("A", Some("A"), bits::SHIFT));
        }

        #[test]
        fn editing_keys_carry_the_macs_commands() {
            let (kind, key, _, text, vk, _, cmds) = parts(key_down(&stroke("backspace", None, Modifiers::default())).unwrap());
            assert_eq!((kind.as_str(), key.as_str(), text, vk, cmds), ("rawKeyDown", "Backspace", None, 8, vec!["deleteBackward".to_owned()]));
            let alt = Modifiers { alt: true, ..Default::default() };
            assert_eq!(parts(key_down(&stroke("backspace", None, alt)).unwrap()).6, ["deleteWordBackward"]);
            let shift = Modifiers { shift: true, ..Default::default() };
            assert_eq!(parts(key_down(&stroke("left", None, shift)).unwrap()).6, ["moveLeftAndModifySelection"]);
            let cmd = Modifiers { platform: true, ..Default::default() };
            let (kind, _, _, text, _, mods, cmds) = parts(key_down(&stroke("a", Some("a"), cmd)).unwrap());
            assert_eq!((kind.as_str(), text, mods, cmds), ("rawKeyDown", None, bits::META, vec!["selectAll".to_owned()]));
            let cmd_shift = Modifiers { platform: true, shift: true, ..Default::default() };
            assert_eq!(parts(key_down(&stroke("z", Some("Z"), cmd_shift)).unwrap()).6, ["redo"]);
        }

        #[test]
        fn enter_types_a_return_and_named_keys_have_their_codes() {
            let (kind, key, code, text, vk, _, _) = parts(key_down(&stroke("enter", None, Modifiers::default())).unwrap());
            assert_eq!((kind.as_str(), key.as_str(), code.as_str(), text.as_deref(), vk), ("keyDown", "Enter", "Enter", Some("\r"), 13));
            assert_eq!(dom("f5", None), Some(("F5".into(), "F5".into(), 116)));
            assert_eq!(dom("pagedown", None), Some(("PageDown".into(), "PageDown".into(), 34)));
            assert_eq!(dom("/", Some("?")), Some(("?".into(), "Slash".into(), 191)));
            assert_eq!(dom("7", Some("7")), Some(("7".into(), "Digit7".into(), 55)));
            assert_eq!(dom("shift", None), None);
            let up = parts(key_up(&stroke("enter", None, Modifiers::default())).unwrap());
            assert_eq!((up.0.as_str(), up.3), ("keyUp", None));
        }

        #[test]
        fn modifiers_and_buttons_count_as_chrome_counts_them() {
            let all = Modifiers { alt: true, control: true, platform: true, shift: true, function: false };
            assert_eq!(modifier_bits(&all), 15);
            assert_eq!(modifier_bits(&Modifiers { platform: true, ..Default::default() }), 4);
            assert_eq!((button_name(MouseButton::Right), button_bit(MouseButton::Right)), ("right", 2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_color;
    use gpui_kit::rgb;

    #[test]
    fn a_members_colour_reads_from_the_hosts_rgb() {
        assert_eq!(parse_color("120, 190, 240"), Some(rgb(0x78BEF0)));
        assert_eq!(parse_color("nope"), None);
    }
}
