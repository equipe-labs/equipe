//! The live preview: a page a member serves on the daemon's machine — a
//! comparison board at `http://localhost:4899/…`, a design session's page, a
//! dev server — in a real web view of this screen (`crate::preview_view`),
//! big and sharp at the screen's own resolution, and fully the person's to
//! click, type and scroll in. Not the members' automated browser, and no
//! address to copy or tunnel to set up by hand.
//!
//! It opens from wherever a member shows a page ("Open preview" in a design
//! session, "Live preview" in the browser panel when the tab on screen is a
//! loopback page there — a member's `show_my_browser`, a reference's Try
//! it) beside the conversation, and goes full-window (⌘⇧F, Esc back) or full
//! screen. Its bar: the page's width (phone 390, tablet 768, desktop 1440,
//! or the pane's), the page's light or dark look, back, reload, the page in
//! the default browser, and — in a design session — pins on the page.
//!
//! With the daemon on another Mac, the page's port there is forwarded here
//! over the window's own SSH link (`crate::tunnel`), at a port equipe picks,
//! and closed with the preview. While it is open the daemon tells the member
//! to keep the page's server running (`Command::LivePreview`); a page that
//! stops answering says so, with "Ask <member> to start it again", and shows
//! again by itself once it answers. The rules are `equipe_core::live_preview`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use equipe_core::live_preview::{self as rules, Phase, Preview, Step};
use equipe_core::protocol::{Command, SessionId};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::preview_view::{PreviewWebView, WebEvent};
use crate::theme::{CONTROL, META, R_MD, SMALL, UI};
use crate::ui::{self, Kind};

/// How often a page on screen (or down) is looked at again.
const WATCH_EVERY: Duration = Duration::from_secs(3);
/// The least the pane takes beside the conversation.
const LEAST: f32 = 420.;
/// What the conversation keeps beside a docked pane.
const TALK_KEEPS: f32 = 360.;
/// Room left around a phone or tablet width that fits the pane.
const DEVICE_MARGIN: f32 = 12.;

/// The width the page is laid out at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Width {
    Phone,
    Tablet,
    Desktop,
    /// The pane's own.
    #[default]
    Fit,
}

impl Width {
    pub const ALL: [Width; 4] = [Width::Phone, Width::Tablet, Width::Desktop, Width::Fit];

    /// The page's width in CSS pixels; `None`: the pane's.
    pub fn css(self) -> Option<f32> {
        match self {
            Width::Phone => Some(390.),
            Width::Tablet => Some(768.),
            Width::Desktop => Some(1440.),
            Width::Fit => None,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "phone" | "390" => Some(Width::Phone),
            "tablet" | "768" => Some(Width::Tablet),
            "desktop" | "1440" => Some(Width::Desktop),
            "fit" => Some(Width::Fit),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Width::Phone => t!("preview.width_phone"),
            Width::Tablet => t!("preview.width_tablet"),
            Width::Desktop => t!("preview.width_desktop"),
            Width::Fit => t!("preview.width_fit"),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Width::Phone => "preview-width-phone",
            Width::Tablet => "preview-width-tablet",
            Width::Desktop => "preview-width-desktop",
            Width::Fit => "preview-width-fit",
        }
    }
}

/// Where the web view goes in the page's area (`w`×`h` points, from the
/// area's top left) at `width`, and the page's zoom: a width that fits sits
/// in the middle at its own size, one wider than the pane is drawn smaller,
/// laid out at its width all the same.
pub fn place(area: (f32, f32), width: Width) -> ((f32, f32, f32, f32), f64) {
    let (w, h) = (area.0.max(1.), area.1.max(1.));
    match width.css() {
        None => ((0., 0., w, h), 1.),
        Some(css) if css <= w => {
            let margin = if css + 2. * DEVICE_MARGIN <= w && h > 4. * DEVICE_MARGIN { DEVICE_MARGIN } else { 0. };
            ((((w - css) / 2.).floor(), margin, css, h - 2. * margin), 1.)
        }
        Some(css) => ((0., 0., w, h), f64::from(w / css)),
    }
}

/// How wide the pane is beside the conversation, in a room `room` points wide
/// (the window beside the sidebar): most of it, the conversation keeping its
/// least.
pub fn docked_width(room: f32) -> f32 {
    (room * 0.62).max(room - 640.).min(room - TALK_KEEPS).max(room.min(LEAST))
}

/// A preview open.
pub struct Open {
    pub preview: Preview,
    /// The address as the member gave it.
    pub url: String,
    /// The conversation of the member serving the page, when one does.
    pub session: Option<SessionId>,
    pub squad: String,
    /// Opened from a design session: its pins go on the page.
    pub design: bool,
}

/// The live preview's state this launch.
#[derive(Default)]
pub struct PreviewUi {
    pub open: Option<Open>,
    /// Over the whole window (⌘⇧F; Esc back).
    pub full: bool,
    /// The window went full screen for the preview: it comes back with it.
    fullscreen: bool,
    pub width: Width,
    /// The page's own dark look (`Some(true)`), light, or the Mac's.
    pub dark: Option<bool>,
    /// The next click on the page drops a pin.
    pub pinning: bool,
    view: Option<PreviewWebView>,
    /// No web view could open here: the pane offers the default browser.
    no_view: bool,
    /// The address the view shows, or is to show once it opens.
    pub shows: Option<String>,
    /// The page's area, as last drawn.
    area: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// The window drew the pane in its latest frame.
    drawn: bool,
    tunnel: Option<crate::tunnel::Tunnel>,
    /// The pins last put on the page: put again when they change, or a page loads.
    said: Option<String>,
    _events: Option<Task<()>>,
    _watch: Option<Task<()>>,
}

impl PreviewUi {
    /// Beside the conversation (open, not over the whole window).
    pub fn docked(&self) -> bool {
        self.open.is_some() && !self.full
    }

    /// Over the whole window.
    pub fn over_all(&self) -> bool {
        self.open.is_some() && self.full
    }

    /// The window drew the pane this frame, or didn't: undrawn, the web view
    /// hides at once and the area measured for it is forgotten.
    pub fn set_drawn(&mut self, drawn: bool) {
        if self.drawn == drawn {
            return;
        }
        self.drawn = drawn;
        if !drawn {
            *self.area.borrow_mut() = None;
            if let Some(view) = self.view.as_mut() {
                view.set_visible(false);
            }
        }
    }
}

impl Shell {
    /// Open a live preview of `url` — a loopback page of the daemon's machine
    /// that `session`'s member serves. Whether it could open (any other
    /// address: no).
    pub fn open_live_preview(&mut self, url: &str, session: Option<SessionId>, design: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Ok(target) = rules::loopback(url) else { return false };
        if self.preview.open.as_ref().is_some_and(|o| o.preview.target == target && o.session == session) {
            cx.notify();
            return true;
        }
        self.close_preview(window, cx);
        let (preview, step) = Preview::open(target, self.endpoint.is_remote());
        let squad = self.model.squad.clone();
        self.send_command(Command::LivePreview { squad: squad.clone(), session, url: preview.target.page(), open: true });
        self.preview.open = Some(Open { preview, url: url.to_owned(), session, squad, design });
        self.preview.pinning = false;
        self.preview.no_view = false;
        self.preview_run(step, window, cx);
        self.preview_watch(window, cx);
        cx.notify();
        true
    }

    /// Close the preview: its web view, its forward, and the member is told.
    pub fn close_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut open) = self.preview.open.take() else { return };
        if let Step::Unlink { port } = open.preview.close() {
            self.preview_unlink(port);
        }
        if let Some(tunnel) = self.preview.tunnel.take() {
            std::thread::spawn(move || tunnel.close());
        }
        self.preview.view = None;
        self.preview.said = None;
        self.preview.shows = None;
        self.preview._events = None;
        self.preview._watch = None;
        self.preview.pinning = false;
        self.preview.no_view = false;
        *self.preview.area.borrow_mut() = None;
        if self.preview.full {
            self.preview_set_full(false, window, cx);
        }
        if open.design {
            self.preview_pin_cancel(cx);
        }
        self.send_command(Command::LivePreview { squad: open.squad, session: open.session, url: open.preview.target.page(), open: false });
        cx.notify();
    }

    /// Carry out a preview's step.
    fn preview_run(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        if let Step::Unlink { port } = step {
            return self.preview_unlink(port);
        }
        let Some(open) = self.preview.open.as_ref() else { return };
        let (attempt, secure) = (open.preview.attempt, open.preview.target.secure);
        match step {
            Step::Nothing | Step::Unlink { .. } => {}
            Step::Load(url) => {
                if let Some(view) = self.preview.view.as_ref() {
                    view.load(&url);
                }
                self.preview.shows = Some(url);
                cx.notify();
            }
            Step::Check { port } => {
                // The tool's pictures: as if the page answered.
                if cfg!(feature = "snapshot") {
                    let step = self.preview.open.as_mut().map_or(Step::Nothing, |o| o.preview.checked(attempt, true));
                    return self.preview_run(step, window, cx);
                }
                let asked = cx.background_spawn(async move { crate::preview_view::answers(port, secure) });
                cx.spawn_in(window, async move |this, cx| {
                    let up = asked.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        let step = this.preview.open.as_mut().map_or(Step::Nothing, |o| o.preview.checked(attempt, up));
                        this.preview_run(step, window, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            Step::Link { remote } => {
                let host = match &self.endpoint {
                    crate::target::Endpoint::Ssh(host) if !cfg!(feature = "snapshot") => Some(host.clone()),
                    _ => None,
                };
                let Some(host) = host else {
                    // The tool's pictures: as if forwarded at the same port.
                    let result = if cfg!(feature = "snapshot") { Ok(remote) } else { Err("no SSH link to the daemon's machine".to_owned()) };
                    let step = self.preview.open.as_mut().map_or(Step::Nothing, |o| o.preview.linked(attempt, result));
                    return self.preview_run(step, window, cx);
                };
                let opened = cx.background_spawn(async move {
                    match rules::local_port(remote, crate::tunnel::port_free, crate::tunnel::any_free_port) {
                        Some(local) => crate::tunnel::open_to(&host, local, remote),
                        None => Err("no free port on this Mac".to_owned()),
                    }
                });
                cx.spawn_in(window, async move |this, cx| {
                    let opened = opened.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        let port = opened.as_ref().map(|t| t.port()).map_err(Clone::clone);
                        if let Ok(tunnel) = opened
                            && let Some(old) = this.preview.tunnel.replace(tunnel)
                        {
                            std::thread::spawn(move || old.close());
                        }
                        let step = match this.preview.open.as_mut() {
                            Some(o) => o.preview.linked(attempt, port),
                            None => port.map_or(Step::Nothing, |port| Step::Unlink { port }),
                        };
                        this.preview_run(step, window, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    /// Take the forward at `port` here down, when it is the preview's.
    fn preview_unlink(&mut self, port: u16) {
        if self.preview.tunnel.as_ref().is_some_and(|t| t.port() == port)
            && let Some(tunnel) = self.preview.tunnel.take()
        {
            std::thread::spawn(move || tunnel.close());
        }
    }

    /// Look at the page now and then: gone down, or back.
    fn preview_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cfg!(feature = "snapshot") {
            return;
        }
        self.preview._watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCH_EVERY).await;
                let Ok(Some((port, secure))) = this.update(cx, |this, _| this.preview.open.as_ref().map(|o| (o.preview.watch_port(), o.preview.target.secure))) else { break };
                let Some(port) = port else { continue };
                let up = cx.background_executor().spawn(async move { crate::preview_view::answers(port, secure) }).await;
                let done = this.update_in(cx, |this, window, cx| {
                    let step = this.preview.open.as_mut().map_or(Step::Nothing, |o| o.preview.watched(up));
                    this.preview_run(step, window, cx);
                    cx.notify();
                });
                if done.is_err() {
                    break;
                }
            }
        }));
    }

    /// What the page told the pane.
    fn preview_event(&mut self, event: WebEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            WebEvent::Escape => {
                self.preview_escape(window, cx);
            }
            WebEvent::ToggleFull => self.preview_toggle_full(window, cx),
            WebEvent::Loaded(_) => {
                self.preview.said = None;
                self.preview_sync_page();
            }
            WebEvent::Pin { x, y, width, selector } => {
                self.preview.pinning = false;
                if self.preview.open.as_ref().is_some_and(|o| o.design) {
                    self.design_session_pin_at(x, y, width, selector, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// The page's pins and pin mode, as the pane has them.
    fn preview_sync_page(&mut self) {
        let pins = crate::preview_view::pins_call(&self.preview_pins());
        let Some(view) = self.preview.view.as_ref() else { return };
        view.run(&crate::preview_view::pinning_call(self.preview.pinning));
        view.run(&pins);
        self.preview.said = Some(pins);
    }

    /// The design session's pins, when the preview is its page.
    fn preview_pins(&self) -> Vec<(u32, u32, u32, bool)> {
        if !self.preview.open.as_ref().is_some_and(|o| o.design) {
            return Vec::new();
        }
        self.session.on.as_ref().map(|s| s.pins.iter().map(|p| (p.pin.n, p.pin.x, p.pin.y, p.done.is_some())).collect()).unwrap_or_default()
    }

    /// Esc, for the preview: pin mode, a pin's note, then the whole window.
    /// False: not its to take.
    pub fn preview_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.preview.open.is_none() {
            return self.browser_escape_full(cx);
        }
        if self.preview.pinning {
            self.preview_pin_mode(false, cx);
            return true;
        }
        if self.preview_pin_pending() {
            self.preview_pin_cancel(cx);
            return true;
        }
        if self.preview.full {
            self.preview_set_full(false, window, cx);
            return true;
        }
        false
    }

    /// The browser panel over the whole window: Esc puts it back beside the conversation.
    fn browser_escape_full(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.browser.read(cx).full {
            return false;
        }
        self.browser.update(cx, |b, cx| b.set_full(false, cx));
        cx.notify();
        true
    }

    pub fn preview_set_full(&mut self, full: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.preview.full = full;
        if !full && std::mem::take(&mut self.preview.fullscreen) && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        cx.notify();
    }

    /// ⌘⇧F: the preview over the whole window, or back beside the conversation.
    pub fn preview_toggle_full(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let full = !self.preview.full;
        self.preview_set_full(full, window, cx);
    }

    /// Full screen: the window, with the preview over all of it — or back.
    pub fn preview_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview.fullscreen {
            return self.preview_set_full(false, window, cx);
        }
        self.preview.full = true;
        if !window.is_fullscreen() {
            window.toggle_fullscreen();
            self.preview.fullscreen = true;
        }
        cx.notify();
    }

    /// Reload: the page again; a page that is down is asked again.
    pub fn preview_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.preview.open.as_mut() else { return };
        if open.preview.phase == Phase::Live {
            if let Some(view) = self.preview.view.as_ref() {
                view.reload();
            }
            return;
        }
        let step = open.preview.retry();
        self.preview_run(step, window, cx);
        cx.notify();
    }

    pub fn preview_back(&mut self) {
        if let Some(view) = self.preview.view.as_ref() {
            view.back();
        }
    }

    /// The page in the default browser of this Mac (through the same forward,
    /// up while the preview is open).
    pub fn preview_outside(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.preview.open.as_ref() else { return };
        let url = self.preview.shows.clone().unwrap_or_else(|| open.preview.target.at(open.preview.port.unwrap_or(open.preview.target.port)));
        cx.open_url(&url);
    }

    /// "Ask <member> to start it again": the person's message to the member, once.
    pub fn preview_ask_restart(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.preview.open.as_mut() else { return };
        let Some(session) = open.session else { return };
        if !open.preview.ask_restart() {
            return;
        }
        let page = open.preview.target.page();
        self.send_command(Command::Submit { session, text: t!("preview.restart_message", url = page).to_string(), about: None });
        cx.notify();
    }

    pub fn preview_set_width(&mut self, width: Width, cx: &mut Context<Self>) {
        self.preview.width = width;
        cx.notify();
    }

    /// The page's dark look, or its light one.
    pub fn preview_toggle_dark(&mut self, cx: &mut Context<Self>) {
        let dark = self.preview.dark.unwrap_or(self.palette.is_dark());
        self.preview.dark = Some(!dark);
        cx.notify();
    }

    /// Pin mode: the next click on the page drops a pin.
    pub fn preview_pin_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        self.preview.pinning = on;
        if let Some(view) = self.preview.view.as_ref() {
            view.run(&crate::preview_view::pinning_call(on));
            if on {
                view.focus();
            }
        }
        cx.notify();
    }

    /// The name of the member serving the page.
    fn preview_member(&self) -> Option<String> {
        let session = self.preview.open.as_ref()?.session?;
        self.model.members.iter().find(|m| m.session == session).map(|m| m.name.clone())
    }

    /// Whether something drawn over the window would be hidden under the web view.
    fn preview_covered(&self) -> bool {
        self.overlay_open() || self.viewer.is_some()
    }

    /// Before the window is laid out: the web view opened once the page shows
    /// and the pane has an area, and kept over that area at the width picked —
    /// hidden whenever it shouldn't show (not drawn, the page not on screen,
    /// something over the window).
    pub fn preview_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let live = self.preview.open.as_ref().is_some_and(|o| o.preview.phase == Phase::Live);
        let area = *self.preview.area.borrow();
        let visible = live && self.preview.drawn && area.is_some() && !self.preview_covered();
        let zoom = window.content_zoom();
        let placed = area.map(|b| {
            let (x, y, w, h) = (f32::from(b.origin.x) * zoom, f32::from(b.origin.y) * zoom, f32::from(b.size.width) * zoom, f32::from(b.size.height) * zoom);
            let ((dx, dy, dw, dh), page_zoom) = place((w, h), self.preview.width);
            ((x + dx, y + dy, dw, dh), page_zoom)
        });
        if visible && self.preview.view.is_none() && !self.preview.no_view && !cfg!(feature = "snapshot")
            && let (Some(url), Some((at, page_zoom))) = (self.preview.shows.clone(), placed)
        {
            let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
            match PreviewWebView::open(window, &url, at, events) {
                Ok(mut view) => {
                    view.zoom(page_zoom);
                    view.set_dark(self.preview.dark);
                    self.preview.view = Some(view);
                    self.preview._events = Some(cx.spawn_in(window, async move |this, cx| {
                        while let Some(event) = received.recv().await {
                            if this.update_in(cx, |this, window, cx| this.preview_event(event, window, cx)).is_err() {
                                break;
                            }
                        }
                    }));
                }
                Err(_) => self.preview.no_view = true,
            }
            return;
        }
        // A pin kept, or marked done by the designer: on the page too.
        if visible && self.preview.view.is_some() {
            let pins = crate::preview_view::pins_call(&self.preview_pins());
            if self.preview.said.as_ref() != Some(&pins) {
                if let Some(view) = self.preview.view.as_ref() {
                    view.run(&pins);
                }
                self.preview.said = Some(pins);
            }
        }
        let dark = self.preview.dark;
        if let Some(view) = self.preview.view.as_mut() {
            // Placed first, then shown: never a frame at an old place.
            if let Some((at, page_zoom)) = placed.filter(|_| visible) {
                view.set_bounds(at);
                view.zoom(page_zoom);
            }
            view.set_dark(dark);
            view.set_visible(visible);
        }
    }
}

/// The pane, when a preview is open: beside the conversation (`docked`) or
/// over the whole window.
pub fn pane(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let open = shell.preview.open.as_ref()?;
    let p = shell.palette;
    let full = shell.preview.full;
    let logical = f32::from(window.viewport_size().width) / window.content_zoom().max(0.01);
    let room = logical - if full { 0. } else { shell.sidebar_on_screen(cx) };
    let pane = v_flex().id("live-preview").h_full().min_w_0().bg(p.bg).child(bar(shell, open, window, cx)).children(pin_strip(shell, cx)).child(area(shell, open, cx));
    Some(if full { pane.flex_1().into_any_element() } else { pane.flex_none().w(px(docked_width(room))).border_l_1().border_color(p.hair).into_any_element() })
}

/// The bar over the page.
fn bar(shell: &Shell, open: &Open, window: &Window, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let full = shell.preview.full;
    let live = open.preview.phase == Phase::Live;
    // Over the whole window, the traffic lights sit at the bar's left.
    let left = if full && !window.is_fullscreen() { 80. } else { 12. };
    let address = crate::browse::address(&open.url);
    let segment = |w: Width, cx: &mut Context<Shell>| {
        let on = shell.preview.width == w;
        div()
            .id(w.id())
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .child(w.label())
            .on_click(cx.listener(move |this, _, _, cx| this.preview_set_width(w, cx)))
    };
    let dark = shell.preview.dark.unwrap_or(p.is_dark());
    let design = open.design && shell.session.on.is_some();
    h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pl(px(left))
        .pr_2()
        .gap_1()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(ui::icon_button(p, "preview-back", Icon::ArrowLeft, t!("preview.back"), None).disabled(!live).on_click(cx.listener(|this, _, _, _| this.preview_back())))
        .child(ui::icon_button(p, "preview-reload", Icon::Reload, t!("preview.reload"), None).on_click(cx.listener(|this, _, window, cx| this.preview_reload(window, cx))))
        .child(
            h_flex()
                .id("preview-address")
                .flex_1()
                .min_w_0()
                .h(px(CONTROL))
                .px(px(10.))
                .gap(px(6.))
                .items_center()
                .rounded(px(R_MD))
                .bg(p.code_bg)
                .text_size(px(SMALL))
                .overflow_hidden()
                .child(icon(Icon::Eye, 12., p.muted))
                .children(address.map(|a| {
                    h_flex()
                        .min_w_0()
                        .gap(px(4.))
                        .child(div().flex_none().whitespace_nowrap().text_color(p.text).child(a.host))
                        .child(div().min_w_0().flex_1().whitespace_nowrap().overflow_hidden().text_color(p.muted).child(a.rest))
                }))
                .when(open.preview.remote, |d| d.child(div().flex_none().text_size(px(META)).text_color(p.muted).child(t!("preview.on_daemon", host = shell.endpoint.label())))),
        )
        .child(h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).children(Width::ALL.map(|w| segment(w, cx))))
        .child(ui::icon_button(p, "preview-dark", if dark { Icon::Sparkles } else { Icon::Lightbulb }, if dark { t!("preview.light") } else { t!("preview.dark") }, None).on_click(cx.listener(|this, _, _, cx| this.preview_toggle_dark(cx))))
        .when(design, |b| {
            let pinning = shell.preview.pinning;
            b.child(
                ui::icon_button(p, "preview-pin", Icon::Pin, if pinning { t!("preview.pin_cancel") } else { t!("preview.pin") }, None)
                    .selected(pinning)
                    .disabled(!live)
                    .on_click(cx.listener(move |this, _, _, cx| this.preview_pin_mode(!pinning, cx))),
            )
        })
        .child(ui::icon_button(p, "preview-outside", Icon::External, t!("preview.open_outside"), None).on_click(cx.listener(|this, _, _, cx| this.preview_outside(cx))))
        .child(ui::icon_button(p, "preview-full", Icon::Expand, if full { t!("preview.exit_full") } else { t!("preview.full") }, Some("⌘⇧F")).selected(full).on_click(cx.listener(|this, _, window, cx| this.preview_toggle_full(window, cx))))
        .child(ui::icon_button(p, "preview-fullscreen", Icon::AppWindow, t!("preview.fullscreen"), None).selected(shell.preview.fullscreen).on_click(cx.listener(|this, _, window, cx| this.preview_fullscreen(window, cx))))
        .child(ui::icon_button(p, "preview-close", Icon::X, t!("preview.close"), None).on_click(cx.listener(|this, _, window, cx| this.close_preview(window, cx))))
}

/// A pin's note, while one dropped on the page waits for it (a design session).
fn pin_strip(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.preview.open.as_ref().is_some_and(|o| o.design) || !shell.preview_pin_pending() {
        return None;
    }
    let p = shell.palette;
    let session = shell.session.on.as_ref()?;
    let pending = session.pending.as_ref()?;
    let input = super::design_session::pin_note(shell)?;
    let focus = input.read(cx).focus_handle(cx);
    Some(
        v_flex()
            .id("preview-pin-strip")
            .flex_none()
            .px_3()
            .py_2()
            .gap_2()
            .bg(p.raised)
            .border_b_1()
            .border_color(p.hair)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_none().text_size(px(META)).text_color(p.muted).child(t!("session.pin_title", n = crate::design_session::next_number(&session.pins))))
                    .child(div().id("preview-pin-box").flex_1().min_w_0().h(px(CONTROL + 4.)).px_2().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.bg).track_focus(&focus).text_size(px(UI)).child(Input::new(&input).appearance(false).bordered(false)))
                    .child(ui::button(p, "preview-pin-cancel", Kind::Ghost, t!("session.pin_cancel")).on_click(cx.listener(|this, _, _, cx| this.preview_pin_cancel(cx))))
                    .child(ui::button(p, "preview-pin-add", Kind::Primary, t!("session.pin_add")).on_click(cx.listener(|this, _, window, cx| this.design_session_pin_save(window, cx)))),
            )
            .child(super::design_session::reason_strip(p, "preview-pin-reason", &pending.reasons, false, cx))
            .into_any_element(),
    )
}

impl Shell {
    /// A pin dropped on the preview waits for its note.
    pub fn preview_pin_pending(&self) -> bool {
        self.session.on.as_ref().and_then(|s| s.pending.as_ref()).is_some_and(|p| p.width.is_some())
    }

    fn preview_pin_cancel(&mut self, cx: &mut Context<Self>) {
        if self.preview_pin_pending() {
            self.design_session_pin_cancel(cx);
        }
    }
}

/// The page's area: the web view goes over it; what it says until the page shows.
fn area(shell: &Shell, open: &Open, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let measured = shell.preview.area.clone();
    let width = shell.preview.width;
    // A phone or tablet width: its frame, drawn behind the web view.
    let device = width.css().is_some();
    div()
        .id("preview-area")
        .flex_1()
        .min_h_0()
        .relative()
        .bg(if device { p.side } else { p.bg })
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    // A new box — moved, or sized anew: drawn again, so the web view goes where it is now.
                    let moved = measured.borrow().is_none_or(|b| b != bounds);
                    *measured.borrow_mut() = Some(bounds);
                    if moved {
                        window.refresh();
                    }
                    if device {
                        let zoom = window.content_zoom().max(0.01);
                        let ((x, y, w, h), _) = place((f32::from(bounds.size.width) * zoom, f32::from(bounds.size.height) * zoom), width);
                        let frame = Bounds::new(point(bounds.origin.x + px(x / zoom), bounds.origin.y + px(y / zoom)), size(px(w / zoom), px(h / zoom)));
                        let mut quad = fill(frame, p.bg);
                        quad.corner_radii = gpui_kit::Corners::all(px(6.));
                        quad.border_widths = gpui_kit::Edges::all(px(1.));
                        quad.border_color = p.hair.into();
                        window.paint_quad(quad);
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .children(state_card(shell, open, cx))
}

/// What the area says when the page isn't on screen: reaching it, checking
/// it, down (with "Ask <member> to start it again"), out of reach — or, in
/// the tool's pictures, where the page draws.
fn state_card(shell: &Shell, open: &Open, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let page = open.preview.target.page();
    let member = shell.preview_member();
    let card = |title: String, body: String| {
        v_flex()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .gap_2()
            .px_6()
            .child(div().max_w(px(440.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).text_center().child(title))
            .child(div().max_w(px(440.)).text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).text_center().child(body))
    };
    let element = match open.preview.phase {
        Phase::Live if shell.preview.no_view => card(t!("preview.no_view_title").to_owned(), t!("preview.no_view_body").to_owned())
            .child(ui::button(p, "preview-no-view-outside", Kind::Primary, t!("preview.open_outside")).on_click(cx.listener(|this, _, _, cx| this.preview_outside(cx))))
            .into_any_element(),
        // The tool draws no web view: where it goes, said.
        Phase::Live if cfg!(feature = "snapshot") => card(t!("preview.live_title").to_owned(), page.clone()).into_any_element(),
        Phase::Live | Phase::Closed => return None,
        Phase::Linking => card(t!("preview.linking_title").to_owned(), t!("preview.linking_body", host = shell.endpoint.label()))
            .child(crate::motion::spinner(16., p.muted))
            .into_any_element(),
        Phase::Checking => card(t!("preview.checking_title").to_owned(), page.clone()).child(crate::motion::spinner(16., p.muted)).into_any_element(),
        Phase::Unreachable => card(t!("preview.unreachable_title").to_owned(), t!("preview.unreachable_body", host = shell.endpoint.label()))
            .child(ui::button(p, "preview-retry", Kind::Secondary, t!("preview.try_again")).on_click(cx.listener(|this, _, window, cx| this.preview_reload(window, cx))))
            .into_any_element(),
        Phase::Down => {
            let body = match &member {
                Some(name) => t!("preview.down_body_member", url = page.clone(), name = name.clone()),
                None => t!("preview.down_body", url = page.clone()),
            };
            let asked = open.preview.asked;
            let mut buttons = h_flex().gap_2().pt_1();
            if let Some(name) = member.filter(|_| open.session.is_some()) {
                let label = if asked { t!("preview.asked", name = name) } else { t!("preview.ask_restart", name = name) };
                buttons = buttons.child(ui::button(p, "preview-ask", Kind::Primary, label).icon(if asked { Icon::Check } else { Icon::Reply }).disabled(asked).on_click(cx.listener(|this, _, _, cx| this.preview_ask_restart(cx))));
            }
            buttons = buttons.child(ui::button(p, "preview-retry", Kind::Secondary, t!("preview.try_again")).on_click(cx.listener(|this, _, window, cx| this.preview_reload(window, cx))));
            card(t!("preview.down_title").to_owned(), body).child(div().size(px(36.)).rounded_full().bg(p.wash).flex().items_center().justify_center().child(icon(Icon::Plug, 16., p.muted))).child(buttons).into_any_element()
        }
    };
    Some(element)
}

/// For `--features snapshot` scripts: `preview:<open:<url>|down|up|full|width:<w>|dark|pin|close>`.
#[cfg(feature = "snapshot")]
impl Shell {
    pub fn preview_script(&mut self, arg: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (what, rest) = arg.split_once(':').unwrap_or((arg, ""));
        match what {
            "open" => {
                let session = self.model.members.iter().find(|m| m.role == "designer").or(self.model.members.first()).map(|m| m.session);
                self.open_live_preview(rest, session, false, window, cx);
            }
            "down" | "up" => {
                let up = what == "up";
                if let Some(open) = self.preview.open.as_mut() {
                    let step = open.preview.watched(up);
                    self.preview_run(step, window, cx);
                }
            }
            "ask" => self.preview_ask_restart(cx),
            "full" => self.preview_toggle_full(window, cx),
            "width" => {
                if let Some(width) = Width::parse(rest) {
                    self.preview_set_width(width, cx);
                }
            }
            "dark" => self.preview_toggle_dark(cx),
            "close" => self.close_preview(window, cx),
            _ => {}
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude brings a `test` macro of its own.
    use super::{DEVICE_MARGIN, LEAST, TALK_KEEPS, Width, docked_width, place};

    #[test]
    fn a_width_that_fits_sits_in_the_middle_and_a_wider_one_is_drawn_smaller() {
        assert_eq!(place((1000., 800.), Width::Fit), ((0., 0., 1000., 800.), 1.));
        let ((x, y, w, h), zoom) = place((1000., 800.), Width::Phone);
        assert_eq!((x, w, zoom), (305., 390., 1.));
        assert_eq!((y, h), (DEVICE_MARGIN, 800. - 2. * DEVICE_MARGIN));
        let ((x, _, w, _), zoom) = place((720., 800.), Width::Desktop);
        assert_eq!((x, w), (0., 720.));
        assert!((zoom - 0.5).abs() < 1e-6, "1440 laid out, drawn at half: {zoom}");
        let ((_, _, w, _), zoom) = place((768., 600.), Width::Tablet);
        assert_eq!((w, zoom), (768., 1.), "exactly fits: no margin to keep");
    }

    #[test]
    fn docked_the_page_takes_most_of_the_room_and_the_conversation_keeps_its_least() {
        assert_eq!(docked_width(1200.), 744.);
        assert_eq!(docked_width(2000.), 1360.);
        assert_eq!(docked_width(700.), LEAST, "a narrow window: the pane's least");
        assert!(docked_width(1000.) <= 1000. - TALK_KEEPS);
    }

    #[test]
    fn widths_are_named_as_scripts_name_them() {
        assert_eq!(Width::parse("phone"), Some(Width::Phone));
        assert_eq!(Width::parse("1440"), Some(Width::Desktop));
        assert_eq!(Width::parse("huge"), None);
        assert_eq!(Width::ALL.map(Width::css), [Some(390.), Some(768.), Some(1440.), None]);
    }
}
