//! The Screen panel: the desktop of the machine the window's daemon runs on,
//! live, over the window — and, when the person takes control, their mouse,
//! keyboard and wheel on it (`crate::screen` carries the line).
//!
//! Looking takes nothing from anyone. Taking control takes the screen from the
//! members: the window sends `/desktop stop` (`Command::Desktop { open: false
//! }`) so no member's pointer or keys go in while the person drives, and
//! giving control back — the button, closing the panel, the line dropping —
//! sends `/desktop resume`, but only if it was this panel that stopped it.
//!
//! Text goes as text: what the Mac's input source composes (Korean, Japanese…)
//! is shown here while it is being composed and sent, once committed, as the
//! characters themselves (X's Unicode keysyms), so the far machine needs no
//! input method of its own. Keys that type nothing (arrows, ⏎, ⌫, shortcuts)
//! go as keys, ⌘ as Control on Linux (as the Mac's shortcuts mean there).

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use equipe_core::protocol::{Command, DesktopState};
use equipe_core::t;
use equipe_rfb::keysym;
use equipe_rfb::wire::button;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::screen::{End, Hello, Session, Update};
use crate::target::Endpoint;
use crate::theme::{Palette, SMALL, Sans, UI};
use crate::ui::{self, Kind};

pub enum ScreenEvent {
    /// A command for the window's daemon (`/desktop stop`, `/desktop resume`).
    Command(Command),
}

impl EventEmitter<ScreenEvent> for ScreenPanel {}

/// Where the line stands.
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// This window is on this Mac: its screen is the one in front of the person.
    Here,
    Connecting,
    Live(Hello),
    /// Ended; `retry_at`: tried again then, on its own.
    Ended { end: End, retry_at: Option<Instant> },
}

/// Modifier keys held down on the far machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_: bool,
}

impl Mods {
    /// What the Mac's modifiers mean on a machine running `os`: ⌘ is Control
    /// on Linux (copy, paste, select all are Control there), Command (Super)
    /// on a Mac.
    pub fn of(m: &Modifiers, os: &str) -> Mods {
        let mac = os == "macos";
        Mods { shift: m.shift, control: m.control || (m.platform && !mac), alt: m.alt, super_: m.platform && mac }
    }

    fn keys(self) -> [(bool, u32); 4] {
        [(self.shift, keysym::SHIFT_L), (self.control, keysym::CONTROL_L), (self.alt, keysym::ALT_L), (self.super_, keysym::SUPER_L)]
    }
}

/// The key events that take the far machine from `held` to `want`.
pub fn mods_change(held: Mods, want: Mods) -> Vec<(bool, u32)> {
    held.keys().into_iter().zip(want.keys()).filter(|((a, _), (b, _))| a != b).map(|(_, (down, k))| (down, k)).collect()
}

/// The keysym a keystroke presses. Named keys by name; with ⌃, ⌥ or ⌘ held,
/// the key itself (⌘C is Control + c, not whatever ⌥ would type); else the
/// character it types (shift included: `A`, `!`).
pub fn keysym_of(stroke: &Keystroke) -> Option<u32> {
    if let Some(k) = keysym::of_key(&stroke.key) {
        return Some(k);
    }
    let single = |s: &str| {
        let mut chars = s.chars();
        let c = chars.next()?;
        chars.next().is_none().then_some(c)
    };
    let m = &stroke.modifiers;
    let c = if m.control || m.platform || m.alt {
        single(&stroke.key)?
    } else {
        stroke.key_char.as_deref().and_then(single).filter(|c| !c.is_control()).or_else(|| single(&stroke.key))?
    };
    Some(keysym::of_char(c))
}

/// A window point on the picture drawn at `rect`, in the far screen's pixels
/// (`w` × `h`): `None` off the picture.
pub fn to_screen(at: (f32, f32), rect: (f32, f32, f32, f32), w: u16, h: u16) -> Option<(u16, u16)> {
    let (x, y, rw, rh) = rect;
    if rw <= 0. || rh <= 0. || at.0 < x || at.1 < y || at.0 >= x + rw || at.1 >= y + rh {
        return None;
    }
    let fx = ((at.0 - x) / rw * f32::from(w)).floor().clamp(0., f32::from(w.saturating_sub(1)));
    let fy = ((at.1 - y) / rh * f32::from(h)).floor().clamp(0., f32::from(h.saturating_sub(1)));
    Some((fx as u16, fy as u16))
}

/// The picture fitted into a box, its shape kept, centred: `(x, y, w, h)`.
/// Never larger than its own size times `max_scale`.
pub fn fit(boxed: (f32, f32, f32, f32), w: u16, h: u16, max_scale: f32) -> (f32, f32, f32, f32) {
    let (bx, by, bw, bh) = boxed;
    let (w, h) = (f32::from(w.max(1)), f32::from(h.max(1)));
    let scale = (bw / w).min(bh / h).min(max_scale).max(0.01);
    let (dw, dh) = ((w * scale).round(), (h * scale).round());
    ((bx + (bw - dw) / 2.).round(), (by + (bh - dh) / 2.).round(), dw, dh)
}

/// How far the wheel turns per notch sent, in window points.
const WHEEL_STEP: f32 = 24.;

/// Notches to send for the turns so far (`acc`, kept between events): up and
/// down, left and right, each as many as whole steps were turned.
pub fn wheel_notches(acc: &mut (f32, f32), dx: f32, dy: f32) -> Vec<u8> {
    acc.0 += dx;
    acc.1 += dy;
    let mut out = Vec::new();
    while acc.1 >= WHEEL_STEP {
        out.push(button::WHEEL_UP);
        acc.1 -= WHEEL_STEP;
    }
    while acc.1 <= -WHEEL_STEP {
        out.push(button::WHEEL_DOWN);
        acc.1 += WHEEL_STEP;
    }
    while acc.0 >= WHEEL_STEP {
        out.push(button::WHEEL_LEFT);
        acc.0 -= WHEEL_STEP;
    }
    while acc.0 <= -WHEEL_STEP {
        out.push(button::WHEEL_RIGHT);
        acc.0 += WHEEL_STEP;
    }
    out
}

fn button_bit(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => button::LEFT,
        MouseButton::Middle => button::MIDDLE,
        MouseButton::Right => button::RIGHT,
        _ => 0,
    }
}

/// Whether to stop the members when the person takes control: only if they
/// aren't stopped already (then giving control back leaves them stopped).
pub fn stops_members(state: &DesktopState) -> bool {
    !state.stopped
}

pub struct ScreenPanel {
    palette: Palette,
    pub open: bool,
    pub focus: FocusHandle,
    endpoint: Option<Endpoint>,
    host: String,
    session: Option<Session>,
    _pump: Option<Task<()>>,
    _retry: Option<Task<()>>,
    attempts: u32,
    phase: Phase,
    /// The picture shown, and the one before it (it may still be on screen a frame).
    picture: Option<(Arc<RenderImage>, u16, u16)>,
    previous: Option<Arc<RenderImage>>,
    /// Where the picture was drawn last: `(x, y, w, h)` in window points.
    placed: Rc<Cell<Option<(f32, f32, f32, f32)>>>,
    /// The person drives the far screen.
    pub controlling: bool,
    /// This panel stopped the members, and resumes them when control goes back.
    stopped_members: bool,
    desktop: DesktopState,
    buttons: u8,
    held: Mods,
    keys_down: HashMap<String, u32>,
    last_pointer: Option<(u16, u16)>,
    wheel: (f32, f32),
    /// What the input source is composing, shown over the picture.
    marked: Option<String>,
    /// Where the person last clicked: where the input source's window goes.
    caret: Option<Point<Pixels>>,
    /// When the last pictures came: the rate the header shows.
    shown_at: VecDeque<Instant>,
}

impl ScreenPanel {
    pub fn new(palette: Palette, cx: &mut Context<Self>) -> Self {
        ScreenPanel {
            palette,
            open: false,
            focus: cx.focus_handle(),
            endpoint: None,
            host: String::new(),
            session: None,
            _pump: None,
            _retry: None,
            attempts: 0,
            phase: Phase::Connecting,
            picture: None,
            previous: None,
            placed: Rc::new(Cell::new(None)),
            controlling: false,
            stopped_members: false,
            desktop: DesktopState::default(),
            buttons: 0,
            held: Mods::default(),
            keys_down: HashMap::new(),
            last_pointer: None,
            wheel: (0., 0.),
            marked: None,
            caret: None,
            shown_at: VecDeque::new(),
        }
    }

    /// What the window follows: its daemon's machine, the screen's lease, the colours.
    pub fn sync(&mut self, endpoint: &Endpoint, desktop: &DesktopState, palette: Palette, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = palette;
        if self.endpoint.as_ref() != Some(endpoint) {
            let was = self.endpoint.replace(endpoint.clone());
            self.host = endpoint.label();
            if was.is_some() {
                // Another machine: nothing of the last one stays, and its members get their screen back.
                self.give_back(window, cx);
                self.disconnect(window);
                if self.open {
                    self.connect(window, cx);
                }
            }
        }
        // Someone resumed the members meanwhile (`/desktop resume`): theirs to say.
        if self.stopped_members && !desktop.stopped && self.desktop.stopped {
            self.stopped_members = false;
        }
        if self.desktop != *desktop {
            self.desktop = desktop.clone();
            cx.notify();
        }
    }

    /// Open or close (the menu, ⌘⇧D, `/screen`).
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open { self.close(window, cx) } else { self.show(window, cx) }
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        if self.session.is_none() && !matches!(self.phase, Phase::Ended { retry_at: None, .. }) {
            self.connect(window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.give_back(window, cx);
        self.disconnect(window);
        self.phase = Phase::Connecting;
        self.open = false;
        cx.notify();
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(endpoint) = self.endpoint.clone() else { return };
        self._retry = None;
        let Some(source) = crate::screen::source(&endpoint) else {
            self.phase = Phase::Here;
            cx.notify();
            return;
        };
        let (session, mut updates) = crate::screen::spawn(source);
        self.session = Some(session);
        self.phase = Phase::Connecting;
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(update) = updates.recv().await {
                if this.update_in(cx, |this, window, cx| this.on_update(update, window, cx)).is_err() {
                    return;
                }
            }
        }));
        cx.notify();
    }

    fn disconnect(&mut self, window: &mut Window) {
        self.session = None;
        self._pump = None;
        self._retry = None;
        for image in [self.picture.take().map(|(i, _, _)| i), self.previous.take()].into_iter().flatten() {
            let _ = window.drop_image(image);
        }
        self.placed.set(None);
        self.shown_at.clear();
        self.marked = None;
    }

    /// Try again now (the guide's button).
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect(window);
        self.attempts = 0;
        self.connect(window, cx);
    }

    fn on_update(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::Ready(hello) => {
                self.attempts = 0;
                self.phase = Phase::Live(hello);
            }
            Update::Resized(w, h) => {
                if let Phase::Live(hello) = &mut self.phase {
                    (hello.width, hello.height) = (w, h);
                }
            }
            Update::Picture => {
                if let Some(frame) = self.session.as_ref().and_then(|s| s.latest.take()) {
                    self.show_frame(frame, window, cx);
                }
            }
            Update::Ended(end) => {
                // The person's hands come off a screen that is gone.
                self.give_back(window, cx);
                let retry_at = end.retries().then(|| {
                    let wait = Duration::from_millis(500 * 2u64.pow(self.attempts.min(4)));
                    self.attempts += 1;
                    Instant::now() + wait
                });
                self.session = None;
                self._pump = None;
                if let Some(at) = retry_at {
                    self._retry = Some(cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(at.saturating_duration_since(Instant::now())).await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if this.open && this.session.is_none() {
                                this.connect(window, cx);
                            }
                        });
                    }));
                }
                self.phase = Phase::Ended { end, retry_at };
            }
        }
        cx.notify();
    }

    fn show_frame(&mut self, frame: crate::screen::Frame, window: &mut Window, cx: &mut Context<Self>) {
        let (w, h) = (frame.width, frame.height);
        let Some(image) = image::RgbaImage::from_raw(u32::from(w), u32::from(h), frame.data) else { return };
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(image)]));
        if let Some(old) = std::mem::replace(&mut self.previous, self.picture.replace((image, w, h)).map(|(i, _, _)| i)) {
            let _ = window.drop_image(old);
        }
        let now = Instant::now();
        self.shown_at.push_back(now);
        while self.shown_at.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(2)) {
            self.shown_at.pop_front();
        }
        // Drawn: the next one may come — or is waiting already.
        let latest = self.session.as_ref().map(|s| s.latest.clone());
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let Some(frame) = latest.and_then(|l| l.next()) else { return };
            let _ = this.update(cx, |this, cx| {
                if this.session.is_some() {
                    this.show_frame(frame, window, cx);
                }
            });
        });
        cx.notify();
    }

    /// Pictures per second, over the last two seconds.
    fn rate(&self) -> f32 {
        if self.shown_at.len() < 2 {
            return 0.;
        }
        let span = self.shown_at.back().unwrap().duration_since(*self.shown_at.front().unwrap()).as_secs_f32().max(0.5);
        (self.shown_at.len() - 1) as f32 / span
    }

    fn os(&self) -> &str {
        match &self.phase {
            Phase::Live(hello) => &hello.os,
            _ => "linux",
        }
    }

    // ── control ──

    fn take_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.controlling || !matches!(self.phase, Phase::Live(_)) {
            return;
        }
        self.controlling = true;
        if stops_members(&self.desktop) {
            self.stopped_members = true;
            cx.emit(ScreenEvent::Command(Command::Desktop { open: false }));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Hands off: every key and button let go on the far side, and the
    /// members resumed if this panel stopped them.
    fn give_back(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            for (_, keysym) in self.keys_down.drain() {
                session.key(false, keysym);
            }
            for (down, keysym) in mods_change(self.held, Mods::default()) {
                session.key(down, keysym);
            }
            if self.buttons != 0
                && let Some((x, y)) = self.last_pointer
            {
                session.pointer(0, x, y);
            }
        }
        self.keys_down.clear();
        self.held = Mods::default();
        self.buttons = 0;
        self.marked = None;
        self.wheel = (0., 0.);
        if self.controlling {
            self.controlling = false;
            if std::mem::take(&mut self.stopped_members) {
                cx.emit(ScreenEvent::Command(Command::Desktop { open: true }));
            }
            cx.notify();
        }
    }

    fn sync_mods(&mut self, want: Mods) {
        if let Some(session) = &self.session {
            for (down, keysym) in mods_change(self.held, want) {
                session.key(down, keysym);
            }
        }
        self.held = want;
    }

    fn at(&self, position: Point<Pixels>) -> Option<(u16, u16)> {
        let (_, w, h) = self.picture.as_ref()?;
        to_screen((f32::from(position.x), f32::from(position.y)), self.placed.get()?, *w, *h)
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if !self.controlling {
            return;
        }
        let Some((x, y)) = self.at(e.position) else { return };
        self.caret = Some(e.position);
        self.sync_mods(Mods::of(&e.modifiers, self.os()));
        self.buttons |= button_bit(e.button);
        self.last_pointer = Some((x, y));
        if let Some(s) = &self.session {
            s.pointer(self.buttons, x, y);
        }
        cx.stop_propagation();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent) {
        if !self.controlling {
            return;
        }
        self.buttons &= !button_bit(e.button);
        let at = self.at(e.position).or(self.last_pointer);
        if let (Some(s), Some((x, y))) = (&self.session, at) {
            s.pointer(self.buttons, x, y);
        }
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent) {
        if !self.controlling {
            return;
        }
        let Some(at) = self.at(e.position) else { return };
        if self.last_pointer != Some(at) {
            self.last_pointer = Some(at);
            if let Some(s) = &self.session {
                s.pointer(self.buttons, at.0, at.1);
            }
        }
    }

    fn scroll(&mut self, e: &ScrollWheelEvent) {
        if !self.controlling {
            return;
        }
        let Some((x, y)) = self.at(e.position) else { return };
        let d = e.delta.pixel_delta(px(WHEEL_STEP));
        let notches = wheel_notches(&mut self.wheel, f32::from(d.x), f32::from(d.y));
        if let Some(s) = &self.session {
            for notch in notches {
                s.pointer(self.buttons | notch, x, y);
                s.pointer(self.buttons, x, y);
            }
        }
    }

    fn modifiers_changed(&mut self, e: &ModifiersChangedEvent) {
        if self.controlling {
            self.sync_mods(Mods::of(&e.modifiers, self.os()));
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &e.keystroke;
        if !self.controlling {
            if stroke.key == "escape" && !stroke.modifiers.modified() {
                self.close(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let Some(keysym) = keysym_of(stroke) else { return };
        self.sync_mods(Mods::of(&stroke.modifiers, self.os()));
        if let Some(s) = &self.session {
            if let Some(before) = self.keys_down.insert(stroke.key.clone(), keysym)
                && before != keysym
            {
                s.key(false, before);
            }
            s.key(true, keysym);
        }
        cx.stop_propagation();
    }

    fn key_up(&mut self, e: &KeyUpEvent) {
        if let Some(keysym) = self.keys_down.remove(&e.keystroke.key)
            && let Some(s) = &self.session
        {
            s.key(false, keysym);
        }
    }

    /// Committed text from the input source: the characters, as they are.
    fn type_text(&mut self, text: &str) {
        if !self.controlling || text.is_empty() || text.chars().all(char::is_control) {
            return;
        }
        // Whatever the input source composed, it composed without the modifiers.
        self.sync_mods(Mods::default());
        if let Some(s) = &self.session {
            s.text(text);
        }
    }
}

impl ScreenPanel {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let live = matches!(self.phase, Phase::Live(_));
        let status: String = match &self.phase {
            Phase::Here => t!("screen.status_here").to_owned(),
            Phase::Connecting => t!("screen.status_connecting").to_owned(),
            Phase::Ended { retry_at: Some(_), .. } => t!("screen.status_reconnecting").to_owned(),
            Phase::Ended { .. } => t!("screen.status_ended").to_owned(),
            Phase::Live(hello) => {
                let rate = self.rate();
                let size = format!("{} × {}", hello.width, hello.height);
                let fps = if rate >= 0.5 { format!(" · {rate:.0} fps") } else { String::new() };
                if self.controlling { t!("screen.status_control", size = size, fps = fps) } else { t!("screen.status_look", size = size, fps = fps) }
            }
        };
        let dot = div().size(px(8.)).flex_none().rounded_full().bg(if self.controlling {
            p.accent
        } else if live {
            p.muted
        } else {
            p.amber
        });
        let action = if self.controlling {
            ui::button(p, "screen-give-back", Kind::Primary, t!("screen.give_back")).tip(t!("screen.give_back_tip"), None).on_click(cx.listener(|this, _, window, cx| this.give_back(window, cx))).into_any_element()
        } else if live {
            ui::button(p, "screen-take", Kind::Secondary, t!("screen.take")).icon(Icon::Hand).tip(t!("screen.take_tip"), None).on_click(cx.listener(|this, _, window, cx| this.take_control(window, cx))).into_any_element()
        } else {
            div().into_any_element()
        };
        h_flex()
            .h(px(44.))
            .flex_none()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .child(icon(Icon::Laptop, 15., p.text2))
            .child(div().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).whitespace_nowrap().child(t!("screen.title", host = self.host.clone())))
            .child(dot)
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(SMALL)).text_color(if self.controlling { p.accent } else { p.muted }).child(status))
            .child(action)
            .child(ui::icon_button(p, "screen-close", Icon::X, t!("screen.close"), None).on_click(cx.listener(|this, _, window, cx| this.close(window, cx))))
    }

    /// In place of the picture: why there is none, and what to do.
    fn cover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
                .child(div().max_w(px(520.)).text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).text_center().child(body))
        };
        let code = |text: String| {
            div().max_w(px(640.)).px_3().py_2().rounded(px(6.)).bg(p.code_bg).text_size(px(SMALL)).font_family("Menlo").text_color(p.text).child(text)
        };
        let host = self.host.clone();
        let retry = |cx: &mut Context<Self>| ui::button(p, "screen-retry", Kind::Secondary, t!("screen.retry")).icon(Icon::Retry).on_click(cx.listener(|this, _, window, cx| this.retry(window, cx)));
        match &self.phase {
            Phase::Live(_) if self.picture.is_some() => None,
            Phase::Live(_) | Phase::Connecting => Some(card(t!("screen.connecting_title", host = host), t!("screen.connecting_body").to_owned()).into_any_element()),
            Phase::Here => Some(card(t!("screen.here_title").to_owned(), t!("screen.here_body").to_owned()).into_any_element()),
            Phase::Ended { end, retry_at } => {
                let (title, body, line): (String, String, Option<String>) = match end {
                    End::NoScreen { kind, detail } => match kind.as_str() {
                        "needs_password" => (t!("screen.password_title", host = host), t!("screen.password_body").to_owned(), Some("equipe screen password".into())),
                        "auth_failed" => (t!("screen.auth_title", host = host), t!("screen.password_body").to_owned(), Some("equipe screen password".into())),
                        "unsupported" => (t!("screen.unsupported_title", host = host), format!("{}\n{detail}", t!("screen.unsupported_body")), None),
                        "no_server" => (t!("screen.none_title", host = host), format!("{}\n{detail}", t!("screen.none_body")), Some(NO_SERVER_LINE.into())),
                        _ => (t!("screen.none_title", host = host), detail.clone(), None),
                    },
                    End::Outdated => (t!("screen.outdated_title", host = host), t!("screen.outdated_body").to_owned(), Some("equipe update".into())),
                    End::NoEquipe => (t!("screen.no_equipe_title", host = host), t!("screen.no_equipe_body").to_owned(), None),
                    End::Ssh(said) => (t!("screen.ssh_title", host = host), said.clone(), None),
                    End::Lost(said) if retry_at.is_some() => (t!("screen.lost_title").to_owned(), said.clone(), None),
                    End::Lost(said) => (t!("screen.lost_title").to_owned(), said.clone(), None),
                };
                Some(card(title, body).children(line.map(code)).child(div().h(px(6.))).child(retry(cx)).into_any_element())
            }
        }
    }

    fn viewport(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.palette;
        let entity = cx.entity();
        let focus = self.focus.clone();
        let picture = self.picture.clone();
        let placed = self.placed.clone();
        let ring = self.controlling.then_some(p.accent);
        let marked = self.marked.clone().filter(|m| !m.is_empty());
        let caret = self.caret;
        div()
            .id("screen-viewport")
            .flex_1()
            .min_h_0()
            .relative()
            .bg(p.side)
            .track_focus(&self.focus)
            .when(self.controlling, |d| d.cursor(CursorStyle::Arrow))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| this.key_up(e)))
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, _| this.modifiers_changed(e)))
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
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        let Some((image, w, h)) = picture else {
                            placed.set(None);
                            return;
                        };
                        // Shrunk to fit, never drawn larger than a point per pixel.
                        let (x, y, dw, dh) = fit((f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height)), w, h, 1.);
                        let rect = Bounds::new(point(px(x), px(y)), size(px(dw), px(dh)));
                        let _ = window.paint_image(rect, rect, Corners::default(), image, 0, false);
                        if let Some(color) = ring {
                            let mut quad = outline(rect, color, BorderStyle::default());
                            quad.border_widths = px(2.).into();
                            window.paint_quad(quad);
                        }
                        placed.set(Some((x, y, dw, dh)));
                    },
                )
                .size_full(),
            )
            .children(marked.map(|m| {
                // What the input source is composing, where the person last clicked.
                let at = caret.unwrap_or(point(px(24.), px(24.)));
                div()
                    .absolute()
                    .left(at.x)
                    .top(at.y - px(28.))
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.accent)
                    .text_size(px(UI))
                    .text_color(p.text)
                    .underline()
                    .child(m)
            }))
            .children(self.cover(cx))
    }

    fn footer(&self) -> Option<AnyElement> {
        let p = self.palette;
        let text = match &self.phase {
            Phase::Live(_) if self.controlling => t!("screen.hint_control").to_owned(),
            Phase::Live(_) => t!("screen.hint_look").to_owned(),
            _ => return None,
        };
        Some(h_flex().h(px(30.)).flex_none().px_3().items_center().border_t_1().border_color(p.hair).text_size(px(SMALL)).text_color(p.muted).child(text).into_any_element())
    }
}

/// The line the guide offers for a Linux desktop with no VNC server: a second
/// server on the display that is already there, on a private socket.
pub const NO_SERVER_LINE: &str = "x0vncserver -display :0 -rfbport -1 -rfbunixpath ~/.equipe/screen/vnc.sock -rfbunixmode 0600 -SecurityTypes None";

impl Render for ScreenPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        v_flex()
            .id("screen-panel")
            .absolute()
            .inset_0()
            .sans()
            .bg(p.bg)
            .occlude()
            .child(self.header(cx))
            .child(self.viewport(cx))
            .children(self.footer())
    }
}

/// The text box the platform types into: what the person composes shows here,
/// and goes to the far screen once committed.
impl EntityInputHandler for ScreenPanel {
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

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.marked.take().filter(|t| !t.is_empty()) {
            self.type_text(&text);
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.type_text(text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        if !self.controlling {
            return;
        }
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, element: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let at = self.caret.unwrap_or(point(element.origin.x + px(16.), element.origin.y + px(16.)));
        Some(Bounds::new(at, size(px(2.), px(18.))))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.controlling
    }
}

/// For `--features snapshot` scripts: the panel driven as the person would.
#[cfg(feature = "snapshot")]
impl ScreenPanel {
    pub fn hand(&mut self, take: bool, window: &mut Window, cx: &mut Context<Self>) {
        if take { self.take_control(window, cx) } else { self.give_back(window, cx) }
    }

    /// Where the picture is drawn, for a click at a fraction of it.
    pub fn picture_rect(&self) -> Option<Bounds<Pixels>> {
        let (x, y, w, h) = self.placed.get()?;
        Some(Bounds::new(point(px(x), px(y)), size(px(w), px(h))))
    }

    /// An input source at work, as the platform calls the text box: each step
    /// marked (being composed), `=text` committed.
    pub fn compose(&mut self, steps: &[&str], window: &mut Window, cx: &mut Context<Self>) {
        for step in steps {
            match step.strip_prefix('=') {
                Some(text) => self.replace_text_in_range(None, text, window, cx),
                None => self.replace_and_mark_text_in_range(None, step, None, window, cx),
            }
        }
    }

    pub fn live(&self) -> bool {
        matches!(self.phase, Phase::Live(_)) && self.picture.is_some()
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` attribute would come with it.
    use super::{DesktopState, Mods, button, fit, keysym, keysym_of, mods_change, stops_members, to_screen, wheel_notches};
    use gpui_kit::{Keystroke, Modifiers};

    fn stroke(key: &str, key_char: Option<&str>, m: Modifiers) -> Keystroke {
        Keystroke { modifiers: m, key: key.into(), key_char: key_char.map(str::to_owned) }
    }

    #[test]
    fn keys_type_their_characters_and_shortcuts_their_keys() {
        assert_eq!(keysym_of(&stroke("a", Some("a"), Modifiers::default())), Some('a' as u32));
        assert_eq!(keysym_of(&stroke("a", Some("A"), Modifiers::shift())), Some('A' as u32));
        assert_eq!(keysym_of(&stroke("1", Some("!"), Modifiers::shift())), Some('!' as u32));
        assert_eq!(keysym_of(&stroke("c", Some("c"), Modifiers::command())), Some('c' as u32), "⌘C is Control + c");
        assert_eq!(keysym_of(&stroke("a", Some("å"), Modifiers::alt())), Some('a' as u32), "⌥ is Alt on the far side, not ⌥'s letter");
        assert_eq!(keysym_of(&stroke("enter", None, Modifiers::default())), Some(keysym::RETURN));
        assert_eq!(keysym_of(&stroke("backspace", None, Modifiers::default())), Some(keysym::BACKSPACE));
        assert_eq!(keysym_of(&stroke("left", None, Modifiers::shift())), Some(keysym::LEFT));
        assert_eq!(keysym_of(&stroke("shift", None, Modifiers::shift())), None, "a modifier alone goes as a modifier");
    }

    #[test]
    fn command_is_control_on_linux_and_command_on_a_mac() {
        let cmd = Modifiers::command();
        assert_eq!(Mods::of(&cmd, "linux"), Mods { control: true, ..Mods::default() });
        assert_eq!(Mods::of(&cmd, "macos"), Mods { super_: true, ..Mods::default() });
        let both = Modifiers { control: true, platform: true, ..Modifiers::default() };
        assert_eq!(Mods::of(&both, "linux"), Mods { control: true, ..Mods::default() });
        assert_eq!(mods_change(Mods::default(), Mods { shift: true, control: true, ..Mods::default() }), vec![(true, keysym::SHIFT_L), (true, keysym::CONTROL_L)]);
        assert_eq!(mods_change(Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() }), vec![(false, keysym::SHIFT_L), (true, keysym::ALT_L)]);
        assert!(mods_change(Mods::default(), Mods::default()).is_empty());
    }

    #[test]
    fn a_point_on_the_picture_is_a_pixel_of_the_far_screen() {
        // A 1440×900 screen drawn at half size from (100, 50).
        let rect = (100., 50., 720., 450.);
        assert_eq!(to_screen((100., 50.), rect, 1440, 900), Some((0, 0)));
        assert_eq!(to_screen((460., 275.), rect, 1440, 900), Some((720, 450)));
        assert_eq!(to_screen((819.9, 499.9), rect, 1440, 900), Some((1439, 899)));
        assert_eq!(to_screen((99., 60.), rect, 1440, 900), None, "off the picture");
        assert_eq!(to_screen((820., 60.), rect, 1440, 900), None);
    }

    #[test]
    fn the_picture_keeps_its_shape_and_is_never_blown_up() {
        assert_eq!(fit((0., 44., 1000., 500.), 1440, 900, 1.), (100., 44., 800., 500.));
        assert_eq!(fit((0., 0., 3000., 2000.), 1440, 900, 1.), (780., 550., 1440., 900.), "at most its own size");
    }

    #[test]
    fn the_wheel_sends_a_notch_per_step_turned() {
        let mut acc = (0., 0.);
        assert!(wheel_notches(&mut acc, 0., 10.).is_empty());
        assert_eq!(wheel_notches(&mut acc, 0., 20.), vec![button::WHEEL_UP], "the turns add up");
        assert_eq!(wheel_notches(&mut acc, 0., -60.), vec![button::WHEEL_DOWN, button::WHEEL_DOWN]);
        assert_eq!(wheel_notches(&mut acc, -30., 0.), vec![button::WHEEL_RIGHT]);
    }

    #[test]
    fn taking_control_stops_the_members_only_when_they_run() {
        assert!(stops_members(&DesktopState::default()));
        assert!(!stops_members(&DesktopState { stopped: true, ..DesktopState::default() }), "stopped already: left as it was on give back");
    }
}
