//! The browser panel: the shared Chrome the members browse in, live, beside
//! the conversation (⌘⇧B, or the header's 브라우저 button). Like Claude
//! Code's browser pane, but the browser is equipe's own — one Chrome on the
//! daemon's machine, signed in once — so what shows here is exactly what the
//! members see, sign-ins and all.
//!
//! A strip of tabs grouped by whose they are (each member's browser in its
//! colour, then the person's own, 내 탭); back, forward, reload, the address,
//! and "Chrome에서 열기" for what a picture can't do (a passkey, a permission
//! prompt); the page itself; and a line saying who has it in hand. It follows
//! the conversation on screen — that member's browser, the tab it last worked
//! in — until the person picks another tab.
//!
//! The page is a stream of pictures (`crate::stream`); the person's mouse and
//! keys go back to the page, mapped through the picture's frame. Touching a
//! member's page takes it into the person's hands (its tools wait); 돌려주기,
//! Esc, or two idle minutes give it back.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use equipe_core::protocol::stream::{self, FrameHeader, InputEvent, StreamIn};
use equipe_core::protocol::{BrowserNavAction, BrowserPage, Command, Event, ScreenBounds, SessionId};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent as FieldEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::stream::{Link, Update};
use crate::target::Endpoint;
use crate::icons::{Icon, icon};
use crate::motion;
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

/// Who a conversation is, for its lane's label and colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub name: String,
    pub hue: usize,
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
    /// Lanes in the person's hands.
    held: HashSet<String>,
    /// Lanes whose member asked the person in, and what for.
    asked: HashMap<String, String>,
    dialogs: Vec<Dialog>,
    selected: Option<(String, u64)>,
    follow: bool,
    /// The conversation on screen, and who each conversation is.
    viewed: Option<SessionId>,
    who: HashMap<SessionId, Who>,
    /// A new tab of the person's own was asked for: picked when it comes.
    new_tab: bool,
    // The picture.
    frame: Option<(FrameHeader, Arc<RenderImage>)>,
    previous: Option<Arc<RenderImage>>,
    /// Where the picture was drawn, in window coordinates, and of what frame.
    placed: Placed,
    /// The viewport's size, as drawn.
    viewport: Rc<RefCell<Option<Bounds<Pixels>>>>,
    // The line.
    endpoint: Option<Endpoint>,
    link: Option<Link>,
    connected: bool,
    watching: Option<StreamIn>,
    // The hand.
    marked: Option<String>,
    keys_down: HashSet<String>,
    buttons: u32,
    last_move: Instant,
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
        let mut subscriptions = vec![cx.subscribe_in(&url, window, |this, input, event: &FieldEvent, window, cx| {
            if matches!(event, FieldEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                if let Some((_, page)) = this.selected.clone() {
                    this.command(Command::BrowserNav { page, action: BrowserNavAction::Go, url: Some(text) }, cx);
                    this.url_of = None;
                    this.focus_page(window, cx);
                }
            }
        })];
        subscriptions.push(cx.subscribe_in(&prompt, window, |this, input, event: &FieldEvent, _, cx| {
            if matches!(event, FieldEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                this.answer(true, Some(text), cx);
            }
        }));
        // Out of focus, the page streams slower; back in, at full speed.
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            this.rewatch(window);
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
            held: HashSet::new(),
            asked: HashMap::new(),
            dialogs: Vec::new(),
            selected: None,
            follow: true,
            viewed: None,
            who: HashMap::new(),
            new_tab: false,
            frame: None,
            previous: None,
            placed: Rc::default(),
            viewport: Rc::default(),
            endpoint: None,
            link: None,
            connected: false,
            watching: None,
            marked: None,
            keys_down: HashSet::new(),
            buttons: 0,
            last_move: Instant::now(),
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
        self.lanes.clear();
        self.held.clear();
        self.asked.clear();
        self.dialogs.clear();
        self.selected = None;
        self.follow = true;
        self.drop_pictures(window);
        self.link = None;
        self._pump = None;
        self.watching = None;
        self.connected = false;
        self.endpoint = Some(endpoint.clone());
        self.squad = squad.to_owned();
        self.ensure_link(window, cx);
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
            self.rewatch(window);
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
                let mine_before: HashSet<u64> = self.lanes.get(MINE).map(|l| l.pages.iter().map(|p| p.id).collect()).unwrap_or_default();
                if pages.is_empty() {
                    self.lanes.remove(lane);
                } else {
                    self.lanes.insert(
                        lane.clone(),
                        Lane { squad: squad.clone(), session: *session, member: member.clone(), color: color.clone(), pages: pages.clone(), agent_page: *agent_page },
                    );
                }
                // A tab of the person's own just asked for: shown.
                if lane == MINE && self.new_tab
                    && let Some(new) = pages.iter().find(|p| !mine_before.contains(&p.id))
                {
                    self.new_tab = false;
                    self.follow = false;
                    self.select(MINE.to_owned(), new.id, window, cx);
                }
                let moved = before != Some(*agent_page);
                // The tab picked closed: the next one in its lane, or the conversation's.
                if let Some((sel_lane, page)) = self.selected.clone()
                    && !self.lanes.get(&sel_lane).is_some_and(|l| l.pages.iter().any(|p| p.id == page))
                {
                    let next = self.lanes.get(&sel_lane).and_then(|l| l.pages.last()).map(|p| p.id);
                    match next {
                        Some(next) => self.select(sel_lane, next, window, cx),
                        None => {
                            self.selected = None;
                            self.follow = true;
                            self.drop_pictures(window);
                            self.rewatch(window);
                        }
                    }
                }
                self.follow_viewed(moved, window, cx);
                self.refresh_address(window, cx);
                cx.notify();
            }
            Event::BrowserControl { squad, lane, by, why } => {
                if squad.as_deref().is_some_and(|s| s != self.squad) {
                    return;
                }
                if by.is_some() {
                    self.held.insert(lane.clone());
                } else {
                    self.held.remove(lane);
                    self.asked.remove(lane);
                }
                // A member asks the person in: the panel opens on its browser.
                if let Some(why) = why {
                    self.asked.insert(lane.clone(), why.clone());
                    self.follow = false;
                    self.open = true;
                    self.shown = true;
                    cx.emit(PanelEvent::Opened);
                    self.ensure_link(window, cx);
                    if let Some(page) = self.lanes.get(lane).and_then(|l| l.agent_page.or(l.pages.last().map(|p| p.id))) {
                        self.select(lane.clone(), page, window, cx);
                    } else {
                        self.selected = Some((lane.clone(), 0));
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
        let here = self.selected.as_ref().is_some_and(|(s, p)| s == lane && *p != 0);
        if !here || moved {
            self.select(lane.clone(), page, window, cx);
        }
    }

    fn select(&mut self, lane: String, page: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&(lane.clone(), page)) {
            return;
        }
        self.selected = Some((lane, page));
        self.drop_pictures(window);
        self.url_of = None;
        self.refresh_address(window, cx);
        self.rewatch(window);
        cx.notify();
    }

    fn page(&self) -> Option<&BrowserPage> {
        let (lane, page) = self.selected.as_ref()?;
        self.lanes.get(lane)?.pages.iter().find(|p| p.id == *page)
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
        self.rewatch(window);
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
        self.rewatch(window);
    }

    fn on_update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::Line(up) => {
                self.connected = up;
                if up {
                    // A new line starts from nothing: what to watch is asked again.
                    self.watching = None;
                    self.rewatch(window);
                }
                cx.notify();
            }
            Update::Picture(picture) => {
                let ours = self.selected.as_ref().is_some_and(|(_, p)| *p == picture.header.page);
                if ours {
                    if let Some(old) = self.previous.take() {
                        let _ = window.drop_image(old);
                    }
                    self.previous = self.frame.take().map(|(_, image)| image);
                    self.frame = Some((picture.header, picture.image));
                    cx.notify();
                } else {
                    let _ = window.drop_image(picture.image);
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
        for image in [self.frame.take().map(|(_, i)| i), self.previous.take()].into_iter().flatten() {
            let _ = window.drop_image(image);
        }
        *self.placed.borrow_mut() = None;
    }

    /// Tell the stream what to show now: the picked page while the panel is in
    /// sight — fast with the window in front, slow behind — or nothing.
    pub fn rewatch(&mut self, window: &mut Window) {
        let Some(link) = &self.link else { return };
        let page = self.selected.as_ref().map(|(_, p)| *p).filter(|p| *p != 0 && self.open && self.shown);
        let want = match page {
            Some(page) => {
                let scale = window.scale_factor();
                let size = self.viewport.borrow().map(|b| b.size).unwrap_or(size(px(self.width), px(700.)));
                // In steps, so a resize doesn't restart the page's stream at every pixel.
                let step = |v: f32| ((v * scale / 128.).ceil() * 128.).clamp(256., 2560.) as u32;
                let fps = if window.is_window_active() { 30 } else { 5 };
                StreamIn::Watch { page, fps, max_w: step(f32::from(size.width)), max_h: step(f32::from(size.height)) }
            }
            None => StreamIn::Unwatch,
        };
        if self.watching.as_ref() != Some(&want) {
            link.send(want.clone());
            self.watching = Some(want);
        }
    }

    // ── the person's hand ──

    fn lane_of_selected(&self) -> Option<&str> {
        self.selected.as_ref().map(|(l, _)| l.as_str())
    }

    /// The person touched the page: it is theirs until handed back.
    fn hold(&mut self) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.held.insert(lane);
        }
    }

    fn give_back(&mut self, cx: &mut Context<Self>) {
        if let Some(lane) = self.lane_of_selected().filter(|l| !l.is_empty()).map(str::to_owned) {
            self.held.remove(&lane);
            self.command(Command::BrowserControl { lane, take: false }, cx);
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

    fn scroll(&mut self, e: &ScrollWheelEvent) {
        let Some((x, y)) = self.to_page(e.position) else { return };
        let delta = e.delta.pixel_delta(px(20.));
        let k = self.css_per_point();
        self.hold();
        self.send_input(InputEvent::Wheel {
            x,
            y,
            dx: -f64::from(f32::from(delta.x)) * k,
            dy: -f64::from(f32::from(delta.y)) * k,
            modifiers: keys::modifier_bits(&e.modifiers),
        });
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &e.keystroke;
        let m = &stroke.modifiers;
        // The panel's own keys.
        if stroke.key == "escape" && !m.modified() && !self.lane_of_selected().unwrap_or(MINE).is_empty() {
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
        let width = self.width.max(720.);
        let bounds = ScreenBounds { left: (x + w - width).max(0.) as i32, top: y as i32, width: width as u32, height: h as u32 };
        self.command(Command::BrowserPopOut { page, bounds: Some(bounds) }, cx);
    }

    // ── drawing ──

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
            None => (l.member.clone().unwrap_or_else(|| lane.rsplit('/').next().unwrap_or(lane).to_owned()), l.color.as_deref().and_then(parse_color)),
        }
    }

    fn member_name(&self, lane: &str) -> String {
        self.lanes.get(lane).map(|l| self.lane_label(lane, l).0).unwrap_or_else(|| lane.rsplit('/').next().unwrap_or(lane).to_owned())
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let mut groups: Vec<(&String, &Lane)> = self.lanes.iter().filter(|(lane, l)| lane.as_str() == MINE || l.squad.as_deref() == Some(self.squad.as_str())).collect();
        // The conversation's browser first, the person's own last.
        groups.sort_by_key(|(lane, l)| (lane.is_empty(), l.session != self.viewed || self.viewed.is_none(), (*lane).clone()));
        let mut row = h_flex().id("browser-tabs").gap_1().px_2().py_1().items_center().overflow_x_scroll();
        for (lane, l) in groups {
            let (label, color) = self.lane_label(lane, l);
            let dot = color.unwrap_or(p.muted);
            row = row.child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .px_1()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(dot)
                    .child(div().child("●"))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(label)),
            );
            for page in &l.pages {
                let on = self.selected.as_ref() == Some(&(lane.clone(), page.id));
                let (lane_id, page_id) = (lane.clone(), page.id);
                let title = if page.title.is_empty() { short_url(&page.url) } else { page.title.clone() };
                let close_lane = lane.clone();
                row = row.child(
                    h_flex()
                        .id(SharedString::from(format!("tab-{}", page.id)))
                        .flex_none()
                        .max_w(px(180.))
                        .h(px(CONTROL))
                        .px_2()
                        .gap_1()
                        .items_center()
                        .rounded(px(R_SM))
                        .border_1()
                        .border_color(if on { dot } else { p.hair })
                        .when(on, |t| t.bg(p.wash))
                        .text_size(px(12.))
                        .text_color(if on { p.text } else { p.text2 })
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.wash))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // Picking a tab of the conversation's own browser keeps following it.
                            let followed = this.viewed.is_some() && this.lanes.get(&lane_id).is_some_and(|l| l.session == this.viewed);
                            this.follow = followed;
                            this.select(lane_id.clone(), page_id, window, cx);
                        }))
                        .when(page.loading, |t| t.child({ use gpui_kit::component::Sizable as _; gpui_kit::component::spinner::Spinner::new().with_size(px(11.)) }.color(Hsla::from(p.muted))))
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
                        .when(close_lane.is_empty(), |t| {
                            t.child(
                                div()
                                    .id(SharedString::from(format!("close-{}", page.id)))
                                    .size(px(18.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .hover(move |s| s.bg(p.wash))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.command(Command::BrowserCloseTab { page: page_id }, cx);
                                    }))
                                    .child(icon(Icon::X, 12., p.muted)),
                            )
                        }),
                );
            }
        }
        row.child(ui::icon_button(p, "browser-new-tab", Icon::Plus, t!("browser.new_tab"), None).on_click(cx.listener(|this, _, _, cx| {
            this.new_tab = true;
            this.command(Command::BrowserNewTab { url: None }, cx);
        })))
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let loading = self.page().is_some_and(|pg| pg.loading);
        h_flex()
            .px_2()
            .pb_2()
            .gap_1()
            .items_center()
            .child(ui::icon_button(p, "browser-back", Icon::ArrowLeft, t!("browser.back"), Some("⌘[")).disabled(self.page().is_none()).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Back, cx))))
            .child(ui::icon_button(p, "browser-forward", Icon::ArrowRight, t!("browser.forward"), Some("⌘]")).disabled(self.page().is_none()).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Forward, cx))))
            .child(if loading {
                ui::icon_button(p, "browser-reload", Icon::X, t!("browser.stop"), None).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Stop, cx)))
            } else {
                ui::icon_button(p, "browser-reload", Icon::Reload, t!("browser.reload"), Some("⌘R")).on_click(cx.listener(|this, _, _, cx| this.nav(BrowserNavAction::Reload, cx)))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(CONTROL))
                    .pl(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(R_MD))
                    .bg(p.code_bg)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(SMALL))
                    .child(Input::new(&self.url).appearance(false).bordered(false)),
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
                    .child(t!("browser.follow")),
                p,
                t!("browser.follow_tip"),
                None,
            ))
            .child(ui::button(p, "browser-pop-out", Kind::Ghost, t!("browser.open_in_chrome")).icon(Icon::External).on_click(cx.listener(|this, _, window, cx| this.pop_out(window, cx))))
    }

    fn viewport(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let entity = cx.entity();
        let focus = self.focus.clone();
        let frame = self.frame.clone();
        let placed = self.placed.clone();
        let viewport = self.viewport.clone();
        let note = if self.lanes.values().all(|l| l.pages.is_empty()) {
            Some(t!("browser.no_tabs").to_owned())
        } else if self.selected.is_none() {
            Some(t!("browser.pick_a_tab").to_owned())
        } else if !self.connected {
            Some(t!("browser.connecting").to_owned())
        } else if frame.is_none() {
            Some(t!("browser.loading").to_owned())
        } else {
            None
        };
        let dialog = self.selected.as_ref().and_then(|(_, page)| self.dialogs.iter().find(|d| d.page == *page)).cloned();
        div()
            .id("browser-viewport")
            .flex_1()
            .min_h_0()
            .relative()
            .bg(p.code_bg)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| this.key_up(e)))
            .on_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| this.mouse_down(e, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, e: &MouseUpEvent, _, _| this.mouse_up(e)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, _| this.mouse_move(e)))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, _| this.scroll(e)))
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        *viewport.borrow_mut() = Some(bounds);
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
                        let rect = Bounds::new(point(px(o.0 as f32), px(o.1 as f32)), size(px(s.0 as f32), px(s.1 as f32)));
                        let _ = window.paint_image(rect, rect, Corners::default(), image, 0, false);
                        *placed.borrow_mut() = Some((rect, header));
                    },
                )
                .size_full(),
            )
            .children(note.map(|text| {
                div().absolute().inset_0().flex().items_center().justify_center().text_size(px(SMALL)).text_color(p.muted).child(text)
            }))
            .children(dialog.map(|d| self.dialog_card(d, cx)))
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

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let lane = self.lane_of_selected().map(str::to_owned);
        let mut bar = h_flex().h(px(30.)).flex_none().px_3().gap_2().items_center().border_t_1().border_color(p.hair).text_size(px(12.)).text_color(p.text2);
        let Some(lane) = lane else {
            return bar.child(div().text_color(p.muted).child(t!("browser.status_idle")));
        };
        if lane.is_empty() {
            return bar.child(div().text_color(p.muted).child(t!("browser.status_mine")));
        }
        let name = self.member_name(&lane);
        let color = self.lanes.get(&lane).and_then(|l| self.lane_label(&lane, l).1).unwrap_or(p.accent);
        let back = div()
            .id("browser-give-back")
            .px_2()
            .h(px(22.))
            .flex()
            .items_center()
            .rounded(px(R_SM))

            .border_1()
            .border_color(p.hair)
            .text_color(p.text)
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(|this, _, _, cx| this.give_back(cx)));
        if let Some(why) = self.asked.get(&lane) {
            bar = bar
                .child(div().text_color(color).child("●"))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(t!("browser.status_asked", name = name, why = why)))
                .child(back.child(t!("browser.done_give_back", name = name)));
        } else if self.held.contains(&lane) {
            bar = bar
                .child(div().text_color(p.text).child("●"))
                .child(div().flex_1().min_w_0().child(t!("browser.status_mine_hand")))
                .child(back.child(t!("browser.give_back", name = name)))
                .child(div().text_color(p.muted).child("Esc"));
        } else {
            bar = bar
                .child(div().text_color(color).child("●"))
                .child(div().flex_1().min_w_0().child(t!("browser.status_member", name = name)))
                .child(div().text_color(p.muted).child(t!("browser.click_to_take")));
        }
        bar
    }
}

impl Render for BrowserPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The viewport's size may have moved since the stream was asked.
        self.rewatch(window);
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
                    .child(self.toolbar(cx))
                    .child(self.viewport(cx))
                    .child(self.status(cx)),
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
        let all: Vec<(String, u64)> = self.lanes.iter().flat_map(|(lane, l)| l.pages.iter().map(move |p| (lane.clone(), p.id))).collect();
        if let Some((lane, page)) = n.checked_sub(1).and_then(|i| all.get(i)).cloned() {
            self.follow = false;
            self.select(lane, page, window, cx);
        }
    }

    /// A tab of the person's own at `url`, picked when it comes.
    pub fn open_tab(&mut self, url: &str, cx: &mut Context<Self>) {
        self.new_tab = true;
        self.command(Command::BrowserNewTab { url: Some(url.to_owned()) }, cx);
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

/// `r, g, b` as the host gives a member's colour.
fn parse_color(text: &str) -> Option<Rgba> {
    let parts: Vec<u8> = text.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    let [r, g, b] = parts[..] else { return None };
    Some(rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)))
}

/// A tab with no title yet: its host, or the address.
fn short_url(url: &str) -> String {
    if url.is_empty() || url == "about:blank" {
        return t!("browser.new_tab").to_owned();
    }
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_owned()
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
    use super::{parse_color, short_url};
    use equipe_core::t;
    use gpui_kit::rgb;

    #[test]
    fn a_members_colour_reads_from_the_hosts_rgb() {
        assert_eq!(parse_color("120, 190, 240"), Some(rgb(0x78BEF0)));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn a_tab_without_a_title_shows_its_host() {
        assert_eq!(short_url("https://vercel.com/dashboard"), "vercel.com");
        assert_eq!(short_url("about:blank"), t!("browser.new_tab"));
    }
}
