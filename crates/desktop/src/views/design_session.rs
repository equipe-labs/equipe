//! A design session: the designer's conversation at the left, the page it is
//! making live at the right — at 1440 wide or 390 — and the person working on
//! it with the designer in short rounds. A click on the page drops a numbered
//! pin with a short note (and one-tap reasons); the pins go with the next
//! message, each with a crop of the page, and get a check when the designer
//! marks them done. Thumbs and "more / less like this" on what is shown are
//! kept in the project's style profile. The header says the page, the tokens
//! the session used, and whether rounds run fast or the polish step runs on
//! the stronger model. The rules are `crate::design_session`.
//!
//! The page is drawn from the daemon's shared browser over the browser
//! stream (`crate::stream`), so a page on the daemon's machine — a dev server
//! in the designer's worktree, a file there — shows here on this Mac or
//! another; a static render comes as any picture does (`crate::visuals`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use equipe_core::protocol::design::{PickReason, QuickReaction, TasteSignal};
use equipe_core::protocol::design_session::SessionFrom;
use equipe_core::protocol::stream::{self, FrameHeader, StreamIn};
use equipe_core::protocol::{BrowserNavAction, BrowserPage, Command, Event};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::design_session::{self as rules, Live, Pending, Route, Session, Tab, Viewport};
use crate::icons::{Icon, icon};
use crate::stream::{Link, Update};
use crate::theme::{CONTROL, META, Palette, R_MD, SMALL, UI};
use crate::tips::TipExt as _;
use crate::{Screen, Shell, ui};

/// The conversation's column: its width, and the least the page keeps.
const TALK_WIDTH: f32 = 420.;
/// A pin's marker.
const PIN: f32 = 22.;
/// The crop that goes with a pin, in device pixels.
const CROP: u32 = 480;

/// Where the page was last drawn: the box it sits in and the picture's
/// rectangle in it (window coordinates), and of what frame.
type Placed = Rc<RefCell<Option<(Bounds<Pixels>, Bounds<Pixels>, FrameHeader)>>>;

/// A session waiting for its page's name.
pub struct Setup {
    pub project: Option<String>,
    pub from: SessionFrom,
    pub input: Entity<InputState>,
    _enter: Subscription,
}

/// The design session's state this launch.
#[derive(Default)]
pub struct SessionUi {
    pub on: Option<Session>,
    pub setup: Option<Setup>,
    /// Started from this screen: its screen opens when the daemon says it began.
    pub asked: bool,
    /// The person's own tabs in the shared browser.
    pub mine: Vec<BrowserPage>,
    link: Option<Link>,
    _pump: Option<Task<()>>,
    frame: Option<(FrameHeader, Arc<RenderImage>)>,
    previous: Option<Arc<RenderImage>>,
    watching: Option<StreamIn>,
    placed: Placed,
    /// The box a static render is drawn in.
    render_area: Rc<RefCell<Option<Bounds<Pixels>>>>,
    /// How long the designer's conversation was when last read, and how many of its commands had ended.
    scanned: (usize, usize),
    pub shown: rules::Shown,
    /// The pin's note being written.
    note: Option<(Entity<InputState>, Subscription)>,
    /// Reasons tapped under a design choice, for "Again": by its set.
    pub rejections: std::collections::HashMap<u64, Vec<PickReason>>,
    /// Reasons tapped for a design draft sent back ("Ask again").
    pub send_back: Vec<PickReason>,
}

/// The screen.
pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let body: AnyElement = match (&shell.session.on, &shell.session.setup) {
        // The page in the live preview beside it: the conversation takes the room.
        (Some(session), _) if shell.preview.open.as_ref().is_some_and(|o| o.design) => h_flex().flex_1().min_h_0().child(talk(shell, session, cx)).into_any_element(),
        (Some(session), _) => h_flex()
            .flex_1()
            .min_h_0()
            .child(talk(shell, session, cx))
            .child(live(shell, session, cx))
            .into_any_element(),
        (None, Some(setup)) => setup_view(shell, setup, cx),
        (None, None) => div().flex_1().into_any_element(),
    };
    v_flex().id("design-session").flex_1().min_h_0().bg(p.bg).child(header(shell, cx)).child(body).into_any_element()
}

/// The title bar: the designer, the page, the tokens used, fast or polish, End.
fn header(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let mut bar = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("session.title")));
    let Some(session) = &shell.session.on else {
        return bar.child(div().flex_1()).child(ui::button(p, "session-cancel", ui::Kind::Ghost, t!("session.cancel")).on_click(cx.listener(|this, _, _, cx| this.design_session_close(cx))));
    };
    let designer = shell.model.members.iter().find(|m| m.session == session.designer);
    if let Some(m) = designer {
        bar = bar.child(h_flex().flex_none().gap_1p5().items_center().child(crate::theme::avatar(p, &m.role, &m.name, 18.)).child(div().text_size(px(SMALL)).text_color(p.text2).child(m.name.clone())));
    }
    let used = rules::tokens_used(shell.model.spent.get(&session.designer).map_or(0, |s| s.tokens), session.tokens_at_start);
    bar = bar
        .child(div().min_w_0().truncate().text_size(px(UI)).text_color(p.text).child(session.page.clone()))
        .children(session.project.clone().map(|project| div().flex_none().text_size(px(META)).text_color(p.muted).child(project)))
        .child(div().flex_1())
        .child(
            h_flex()
                .id("session-tokens")
                .flex_none()
                .h(px(24.))
                .px_2()
                .gap_1()
                .items_center()
                .rounded(px(12.))
                .bg(p.wash)
                .text_size(px(META))
                .text_color(p.text2)
                .child(t!("session.tokens", count = rules::tokens_label(used)))
                .tip(p, t!("session.tokens_tip"), None),
        );
    // Fast rounds, or the polish step on the stronger model.
    let segment = |id: &'static str, label: &'static str, tip: &'static str, on: bool, polish: bool, cx: &mut Context<Shell>| {
        div()
            .id(id)
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .child(label)
            .tip(p, tip, None)
            .on_click(cx.listener(move |this, _, _, cx| this.design_session_polish(polish, cx)))
    };
    bar.child(
        h_flex()
            .flex_none()
            .h(px(26.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(R_MD))
            .bg(p.wash)
            .child(segment("session-fast", t!("session.fast"), t!("session.fast_tip"), !session.polish, false, cx))
            .child(segment("session-polish", t!("session.polish"), t!("session.polish_tip"), session.polish, true, cx)),
    )
    .child(ui::button(p, "session-end", ui::Kind::Secondary, t!("session.end")).tip(t!("session.end_tip"), None).on_click(cx.listener(|this, _, _, cx| this.design_session_end(cx))))
}

/// Asking which page: a box, the UI map's screens to pick from, Start.
fn setup_view(shell: &Shell, setup: &Setup, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let screens: Vec<String> = shell.design.systems.iter().find(|s| s.draft.is_none() && s.project == setup.project).or(shell.design.current()).and_then(|s| s.flows.as_ref()).map(|map| map.flows.iter().flat_map(|f| f.screens.iter().map(move |s| format!("{} › {}", if f.title.is_empty() { &f.id } else { &f.title }, if s.title.is_empty() { &s.id } else { &s.title }))).take(8).collect()).unwrap_or_default();
    let chips = screens.into_iter().enumerate().map(|(n, name)| {
        let pick = name.clone();
        div()
            .id(("session-screen", n))
            .h(px(26.))
            .px_2p5()
            .flex()
            .items_center()
            .rounded(px(13.))
            .border_1()
            .border_color(p.hair)
            .text_size(px(SMALL))
            .text_color(p.text2)
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(name)
            .on_click(cx.listener(move |this, _, window, cx| this.design_session_start(Some(pick.clone()), window, cx)))
    });
    let focus = setup.input.read(cx).focus_handle(cx);
    v_flex()
        .flex_1()
        .items_center()
        .pt(px(96.))
        .px_6()
        .child(
            v_flex()
                .w_full()
                .max_w(px(560.))
                .gap_4()
                .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("session.setup_title")))
                .child(div().text_size(px(UI)).text_color(p.text2).child(t!("session.setup_note")))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .id("session-page-box")
                                .flex_1()
                                .h(px(CONTROL + 6.))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(R_MD))
                                .border_1()
                                .border_color(p.hair)
                                .track_focus(&focus)
                                .text_size(px(UI))
                                .child(Input::new(&setup.input).appearance(false).bordered(false)),
                        )
                        .child(ui::button(p, "session-start", ui::Kind::Primary, t!("session.start")).on_click(cx.listener(|this, _, window, cx| this.design_session_start(None, window, cx)))),
                )
                .child(h_flex().flex_wrap().gap_2().children(chips)),
        )
        .into_any_element()
}

/// The designer's conversation and the composer.
fn talk(shell: &Shell, session: &Session, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let viewing = shell.model.shows_own(session.designer);
    let body: AnyElement = if viewing && !shell.chat.blocks.is_empty() {
        shell.panes.conversation.clone().cached(StyleRefinement::default().flex_1().min_h_0().w_full()).into_any_element()
    } else {
        div().flex_1().w_full().flex().items_center().justify_center().px_6().child(div().max_w(px(300.)).text_center().text_size(px(SMALL)).text_color(p.muted).child(t!("session.talk_empty"))).into_any_element()
    };
    let wide = shell.preview.open.as_ref().is_some_and(|o| o.design);
    v_flex()
        .when(!wide, |c| c.w(px(TALK_WIDTH)).flex_none())
        .when(wide, |c| c.flex_1().min_w_0())
        .h_full()
        .min_h_0()
        .border_r_1()
        .border_color(p.hair)
        .child(v_flex().flex_1().min_h_0().w_full().items_center().child(body))
        .children((!session.pins.iter().all(|p| p.sent)).then(|| pins_waiting(p, session)))
        .child(div().flex_none().w_full().px_3().flex().justify_center().child(super::composer::render(shell, cx)))
}

/// The pins that go with the next message, over the composer.
fn pins_waiting(p: Palette, session: &Session) -> impl IntoElement + use<> {
    let n = session.pins.iter().filter(|p| !p.sent).count();
    h_flex()
        .flex_none()
        .w_full()
        .px_4()
        .py_1p5()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .text_size(px(META))
        .text_color(p.text2)
        .child(icon(Icon::Pin, 12., p.accent))
        .child(t!("session.pins_waiting", count = n))
}

/// The live page, its bar, its pins.
fn live(shell: &Shell, session: &Session, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let live = shell.session.shown.live.clone();
    let round = shell.session.shown.count;
    let width = |v: Viewport, id: &'static str, label: &'static str, cx: &mut Context<Shell>| {
        let on = session.viewport == v;
        div()
            .id(id)
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .when(on, |d| d.bg(p.raised).text_color(p.text).shadow_sm())
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.design_session_viewport(v, window, cx)))
    };
    let reacted = session.reactions.iter().rev().find(|(r, _)| *r == round).map(|(_, r)| *r);
    let react = |id: &'static str, glyph: Icon, label: &'static str, reaction: QuickReaction, cx: &mut Context<Shell>| {
        ui::button(p, id, ui::Kind::Ghost, label).icon(glyph).selected(reacted == Some(reaction)).disabled(live.is_none()).on_click(cx.listener(move |this, _, _, cx| this.design_session_react(reaction, cx)))
    };
    let address = match &live {
        Some(Live::Page(url)) => url.clone(),
        Some(Live::Render(path)) => path.rsplit('/').next().unwrap_or(path).to_owned(),
        None => String::new(),
    };
    let outside = match &live {
        Some(Live::Page(url)) => rules::open_here(url, shell.endpoint.is_remote()),
        _ => None,
    };
    let bar = h_flex()
        .flex_none()
        .h(px(44.))
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(h_flex().flex_none().h(px(26.)).p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).child(width(Viewport::Desktop, "session-1440", "1440", cx)).child(width(Viewport::Phone, "session-390", "390", cx)))
        .child(div().flex_1().min_w_0().truncate().text_size(px(META)).text_color(p.muted).child(address))
        .child(react("session-up", Icon::Check, t!("session.up"), QuickReaction::Up, cx))
        .child(react("session-down", Icon::X, t!("session.down"), QuickReaction::Down, cx))
        .child(react("session-more", Icon::Plus, t!("session.more_like"), QuickReaction::MoreLikeThis, cx))
        .child(react("session-less", Icon::Minus, t!("session.less_like"), QuickReaction::LessLikeThis, cx))
        .child(ui::icon_button(p, "session-reload", Icon::Reload, t!("session.reload"), None).disabled(!matches!(live, Some(Live::Page(_)))).on_click(cx.listener(|this, _, _, cx| this.design_session_reload(cx))))
        .children(outside.map(|url| ui::icon_button(p, "session-outside", Icon::External, t!("session.open_outside"), None).on_click(cx.listener(move |_, _, _, _| {
            let _ = equipe_providers::platform::open(&url);
        }))))
        // A page served on the daemon's machine: big, sharp and the person's own, in the live preview.
        .children(match &live {
            Some(Live::Page(url)) if equipe_core::live_preview::previewable(url) => {
                let (url, designer) = (url.clone(), session.designer);
                Some(ui::button(p, "session-preview", ui::Kind::Primary, t!("preview.open")).icon(Icon::Expand).tip(t!("preview.open_tip"), None).on_click(cx.listener(move |this, _, window, cx| {
                    this.open_preview(&url, Some(designer), true, window, cx);
                })))
            }
            _ => None,
        });
    let page: AnyElement = match live.as_ref().map(rules::route) {
        None => waiting(shell, session),
        Some(Route::Stream(_)) => page_view(shell, session, cx),
        Some(Route::Picture(path)) => render_view(shell, session, &path, cx),
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.side)
        .child(bar)
        .children(session.down.as_ref().map(|reasons| reason_strip(p, "session-down-reason", reasons, true, cx)))
        .child(page)
        .children(pin_list(p, session))
}

/// Nothing shown yet.
fn waiting(shell: &Shell, session: &Session) -> AnyElement {
    let p = shell.palette;
    let working = shell.model.members.iter().any(|m| m.session == session.designer && m.presence == crate::state::Presence::Working);
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .px_8()
        .children(working.then(|| crate::motion::spinner(16., p.muted)))
        .child(div().max_w(px(380.)).text_center().text_size(px(UI)).text_color(p.text2).child(t!("session.waiting")))
        .into_any_element()
}

/// The page from the shared browser, fitted to the pane, with its pins.
fn page_view(shell: &Shell, session: &Session, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let frame = shell.session.frame.clone();
    let placed = shell.session.placed.clone();
    let overlay = pins_over(shell, session);
    let pending = pending_card(shell, session, cx);
    div()
        .id("session-page")
        .flex_1()
        .min_h_0()
        .relative()
        .cursor_crosshair()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.design_session_click(e.position, window, cx)))
        .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| this.design_session_wheel(e, window, cx)))
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let Some((header, image)) = frame else {
                        *placed.borrow_mut() = None;
                        return;
                    };
                    let pad = 16.;
                    let (o, s) = stream::fit(
                        (f64::from(f32::from(bounds.origin.x) + pad), f64::from(f32::from(bounds.origin.y) + pad)),
                        (f64::from((f32::from(bounds.size.width) - 2. * pad).max(1.)), f64::from((f32::from(bounds.size.height) - 2. * pad).max(1.))),
                        f64::from(header.w),
                        f64::from(header.h),
                    );
                    let rect = Bounds::new(point(px(o.0.round() as f32), px(o.1.round() as f32)), size(px(s.0.round() as f32), px(s.1.round() as f32)));
                    let _ = window.paint_image(rect, rect, Corners::all(px(6.)), image, 0, false);
                    let changed = placed.borrow().as_ref().is_none_or(|(_, r, h)| *r != rect || h.sx != header.sx || h.sy != header.sy);
                    *placed.borrow_mut() = Some((bounds, rect, header));
                    if changed {
                        window.refresh();
                    }
                },
            )
            .size_full(),
        )
        .when(shell.session.frame.is_none(), |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(crate::motion::spinner(16., p.muted))))
        .children(overlay)
        .children(pending)
        .into_any_element()
}

/// A static render, with its pins.
fn render_view(shell: &Shell, session: &Session, path: &str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let look = shell.shots.borrow_mut().look(path, crate::visuals::Size::Full);
    let area = shell.session.render_area.clone();
    let drawn = render_rect(shell, path).map(|r| (f32::from(r.size.width), f32::from(r.size.height)));
    let face: AnyElement = match look {
        crate::visuals::Look::Ready(shot) => match drawn {
            Some((w, h)) => img(shot.image).w(px(w)).h(px(h)).rounded(px(6.)).into_any_element(),
            None => img(shot.image).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(6.)).into_any_element(),
        },
        crate::visuals::Look::Gone(_) => icon(Icon::Alert, 16., p.muted).into_any_element(),
        crate::visuals::Look::Loading => crate::motion::spinner(16., p.muted),
    };
    let pending = pending_card(shell, session, cx);
    div()
        .id("session-render")
        .flex_1()
        .min_h_0()
        .relative()
        .p_4()
        .flex()
        .items_center()
        .justify_center()
        .cursor_crosshair()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| this.design_session_click(e.position, window, cx)))
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let moved = area.borrow().is_none_or(|b| b != bounds);
                    *area.borrow_mut() = Some(bounds);
                    if moved {
                        window.refresh();
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(face)
        .children(pending)
        .into_any_element()
}

/// Where a render is drawn (window coordinates): whole, centred in its box.
fn render_rect(shell: &Shell, path: &str) -> Option<Bounds<Pixels>> {
    let area = (*shell.session.render_area.borrow())?;
    let dims = shell.shots.borrow().dims(path)?;
    let pad = 16.;
    let (w, h) = crate::visuals::contain(dims, ((f32::from(area.size.width) - 2. * pad).max(1.), (f32::from(area.size.height) - 2. * pad).max(1.)));
    let origin = point(area.origin.x + (area.size.width - px(w)) / 2., area.origin.y + (area.size.height - px(h)) / 2.);
    Some(Bounds::new(origin, size(px(w), px(h))))
}

/// The pins over the page, where they are as it scrolls: a number, or a check when done.
fn pins_over(shell: &Shell, session: &Session) -> Vec<AnyElement> {
    let p = shell.palette;
    let Some((area, rect, header)) = shell.session.placed.borrow().clone() else { return Vec::new() };
    let drawn = (f32::from(rect.size.width), f32::from(rect.size.height));
    let (dx, dy) = (f32::from(rect.origin.x - area.origin.x), f32::from(rect.origin.y - area.origin.y));
    session
        .pins
        .iter()
        .filter_map(|pin| {
            let (x, y) = rules::spot(&pin.pin, drawn, &header)?;
            let done = pin.done.is_some();
            let tip = match &pin.done {
                Some(what) => t!("session.pin_done", n = pin.pin.n, what = what),
                None => format!("{} · {}", pin.pin.n, pin.pin.note),
            };
            Some(
                div()
                    .id(("session-pin", pin.pin.n as usize))
                    .absolute()
                    .left(px(dx + x - PIN / 2.))
                    .top(px(dy + y - PIN / 2.))
                    .size(px(PIN))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if done { p.good() } else { p.accent })
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .shadow_md()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xffffff))
                    .child(if done { icon(Icon::Check, 12., rgb(0xffffff)).into_any_element() } else { div().child(pin.pin.n.to_string()).into_any_element() })
                    .tip(p, tip, None)
                    .into_any_element(),
            )
        })
        .collect()
}

/// The note for a pin just dropped, beside it: a box, the reasons, Add and Cancel.
fn pending_card(shell: &Shell, session: &Session, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    // One dropped on the live preview is written in the preview's bar.
    let pending = session.pending.as_ref().filter(|p| p.width.is_none())?;
    let (input, _) = shell.session.note.as_ref()?;
    let (x, y) = (pending.at.0 as f32, pending.at.1 as f32);
    let focus = input.read(cx).focus_handle(cx);
    Some(
        deferred(
            v_flex()
                .id("session-pin-card")
                .occlude()
                .absolute()
                .left(px((x - 12.).max(8.)))
                .top(px(y + 18.))
                .w(px(320.))
                .p_3()
                .gap_2()
                .rounded(px(10.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().text_size(px(META)).text_color(p.muted).child(t!("session.pin_title", n = rules::next_number(&session.pins))))
                .child(div().id("session-pin-box").h(px(CONTROL + 4.)).px_2().flex().items_center().rounded(px(R_MD)).border_1().border_color(p.hair).bg(p.bg).track_focus(&focus).text_size(px(UI)).child(Input::new(input).appearance(false).bordered(false)))
                .child(reason_strip(p, "session-pin-reason", &pending.reasons, false, cx))
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(ui::button(p, "session-pin-cancel", ui::Kind::Ghost, t!("session.pin_cancel")).on_click(cx.listener(|this, _, _, cx| this.design_session_pin_cancel(cx))))
                        .child(ui::button(p, "session-pin-add", ui::Kind::Primary, t!("session.pin_add")).on_click(cx.listener(|this, _, window, cx| this.design_session_pin_save(window, cx)))),
                ),
        )
        .into_any_element(),
    )
}

/// The box a pin's note is being written in, while one waits for it.
pub fn pin_note(shell: &Shell) -> Option<Entity<InputState>> {
    shell.session.note.as_ref().map(|(input, _)| input.clone())
}

/// The one-tap reasons: on a pin, or after a thumbs-down.
pub fn reason_strip(p: Palette, id: &'static str, on: &[PickReason], down: bool, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let chips = PickReason::ALL.into_iter().map(|reason| {
        let lit = on.contains(&reason);
        div()
            .id((id, reason as usize))
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(if lit { p.accent } else { p.hair })
            .when(lit, |d| d.bg(p.accent_bg))
            .text_size(px(META))
            .text_color(if lit { p.accent } else { p.text2 })
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(reason_words(reason))
            .on_click(cx.listener(move |this, _, _, cx| this.design_session_reason(reason, down, cx)))
    });
    h_flex()
        .flex_wrap()
        .gap_1p5()
        .when(down, |d| d.px_3().py_2().border_b_1().border_color(p.hair).bg(p.bg).child(div().text_size(px(META)).text_color(p.muted).child(t!("session.why_not"))))
        .children(chips)
        .when(down, |d| d.child(div().flex_1()).child(ui::button(p, "session-down-done", ui::Kind::Secondary, t!("session.reasons_done")).on_click(cx.listener(|this, _, _, cx| this.design_session_down_done(cx)))))
}

/// A reason's words on its chip.
pub fn reason_words(reason: PickReason) -> &'static str {
    match reason {
        PickReason::TooDense => t!("reason.too_dense"),
        PickReason::TooPlain => t!("reason.too_plain"),
        PickReason::Colors => t!("reason.colors"),
        PickReason::Type => t!("reason.type"),
        PickReason::Layout => t!("reason.layout"),
        PickReason::NotUs => t!("reason.not_us"),
    }
}

/// Every pin, a line each: its number, its note, and what changed once done.
fn pin_list(p: Palette, session: &Session) -> Option<impl IntoElement + use<>> {
    if session.pins.is_empty() {
        return None;
    }
    let rows = session.pins.iter().map(|pin| {
        let done = pin.done.is_some();
        h_flex()
            .gap_2()
            .items_center()
            .text_size(px(META))
            .child(
                div()
                    .flex_none()
                    .size(px(16.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if done { p.good() } else if pin.sent { p.accent } else { p.wash })
                    .text_size(px(9.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if done || pin.sent { rgb(0xffffff).into() } else { p.text2 })
                    .child(if done { icon(Icon::Check, 10., rgb(0xffffff)).into_any_element() } else { div().child(pin.pin.n.to_string()).into_any_element() }),
            )
            .child(div().min_w_0().truncate().text_color(p.text2).child(pin.pin.note.clone()))
            .children(pin.done.clone().map(|what| div().min_w_0().truncate().text_color(p.muted).child(format!("→ {what}"))))
    });
    Some(v_flex().flex_none().max_h(px(120.)).px_4().py_2().gap_1().border_t_1().border_color(p.hair).bg(p.bg).children(rows))
}

// ── what the screen does ──

impl Shell {
    /// "Start a design session" with nothing named yet: the screen asks which page.
    pub fn design_session_setup(&mut self, project: Option<String>, from: SessionFrom, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("session.setup_placeholder")));
        let enter = cx.subscribe_in(&input, window, |shell, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                shell.design_session_start(None, window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.session.setup = Some(Setup { project, from, input, _enter: enter });
        self.show(Screen::DesignSession, cx);
        cx.notify();
    }

    /// The page named (or `page` picked): the daemon starts the session; its screen opens when it says so.
    pub fn design_session_start(&mut self, page: Option<String>, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(setup) = &self.session.setup else { return };
        let page = page.unwrap_or_else(|| setup.input.read(cx).value().trim().to_owned());
        if page.is_empty() {
            self.composer.flash = Some(t!("session.name_the_page").into());
            cx.notify();
            return;
        }
        let (project, from) = (setup.project.clone(), setup.from.clone());
        self.design_session_ask(project, page, from, cx);
    }

    /// Start one on `page` at once — a designer's ticket names its own.
    pub fn design_session_ask(&mut self, project: Option<String>, page: String, from: SessionFrom, cx: &mut Context<Self>) {
        if !self.model.members.iter().any(|m| m.role == "designer") {
            self.composer.flash = Some(t!("session.no_designer").into());
            cx.notify();
            return;
        }
        self.session.asked = true;
        self.send_command(Command::StartDesignSession { squad: self.model.squad.clone(), project, page, from });
        cx.notify();
    }

    /// A designer's ticket's "Start a design session".
    pub fn design_session_for_ticket(&mut self, ticket: equipe_core::protocol::TicketId, cx: &mut Context<Self>) {
        let Some(t) = self.model.tickets.iter().find(|t| t.id == ticket) else { return };
        let project = t.project.as_ref().map(|p| equipe_core::board::project_name(p));
        let page = t.title.clone();
        self.design_session_ask(project, page, SessionFrom::Ticket { ticket }, cx);
    }

    /// The daemon says a session began: opened here when this screen asked for
    /// it or the PM brought it (the person took its proposal, or asked it).
    pub fn design_session_started(&mut self, event: &Event, window: &mut Window, cx: &mut Context<Self>) {
        let Event::DesignSessionStarted { squad, designer, project, page, from } = event else { return };
        if *squad != self.model.squad || !(self.session.asked || matches!(from, SessionFrom::Pm { .. })) {
            return;
        }
        self.session.asked = false;
        self.session.setup = None;
        let since = self.model.convos.get(designer).map_or(0, |c| c.transcript.entries().len());
        let tokens = self.model.spent.get(designer).map_or(0, |s| s.tokens);
        let mut session = Session::new(squad, *designer, project.clone(), page, from.clone(), since, tokens);
        // Rounds run on a fast model; the one before comes back for polish and at the end.
        if let Some(m) = self.model.members.iter().find(|m| m.session == *designer)
            && let Some(fast) = rules::fast_model(m.engine, m.asked.as_deref().or(m.model.as_deref()))
        {
            session.model_before = m.asked.clone().or(m.model.clone());
            self.send_command(Command::SetModel { session: *designer, model: fast.to_owned() });
        }
        self.session.on = Some(session);
        self.session.shown = rules::Shown::default();
        self.session.scanned = (0, 0);
        self.drop_live(window);
        self.model.select(*designer);
        self.refresh(window, cx);
        self.show(Screen::DesignSession, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// Put the setup away without starting.
    pub fn design_session_close(&mut self, cx: &mut Context<Self>) {
        self.session.setup = None;
        self.show(Screen::Design, cx);
    }

    /// End the session: the designer's model as it was, the page let go.
    pub fn design_session_end(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.on.take()
            && let Some(model) = session.model_before.filter(|_| !session.polish)
        {
            self.send_command(Command::SetModel { session: session.designer, model });
        }
        self.session.link = None;
        self.session._pump = None;
        self.session.frame = None;
        self.session.watching = None;
        self.show(Screen::Design, cx);
    }

    /// Fast rounds, or the polish step on the stronger model.
    pub fn design_session_polish(&mut self, polish: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.on.as_mut() else { return };
        if session.polish == polish {
            return;
        }
        session.polish = polish;
        let designer = session.designer;
        let before = session.model_before.clone();
        let engine = self.model.members.iter().find(|m| m.session == designer).and_then(|m| m.engine);
        let model = if polish { before } else { rules::fast_model(engine, before.as_deref()).map(str::to_owned) };
        if let Some(model) = model {
            self.send_command(Command::SetModel { session: designer, model });
        }
        cx.notify();
    }

    pub fn design_session_viewport(&mut self, viewport: Viewport, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.session.on.as_mut() {
            session.viewport = viewport;
        }
        self.session.watching = None;
        self.session.frame = None;
        self.rewatch_live(window);
        cx.notify();
    }

    pub fn design_session_reload(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.session.on.as_ref().and_then(|s| s.tab) {
            self.send_command(Command::BrowserNav { page, action: BrowserNavAction::Reload, url: None });
        }
        cx.notify();
    }

    /// A wheel over the page scrolls it, in the shared browser.
    pub fn design_session_wheel(&mut self, e: &ScrollWheelEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let (Some(link), Some(page)) = (&self.session.link, self.session.on.as_ref().and_then(|s| s.tab)) else { return };
        let delta = e.delta.pixel_delta(px(16.));
        let Some((_, rect, header)) = self.session.placed.borrow().clone() else { return };
        let scale = header.css_w as f32 / f32::from(rect.size.width).max(1.);
        let (x, y) = ((f32::from(e.position.x - rect.origin.x) * scale).max(0.), (f32::from(e.position.y - rect.origin.y) * scale).max(0.));
        link.send(StreamIn::Input { page, ev: stream::InputEvent::Wheel { x: f64::from(x), y: f64::from(y), dx: -f64::from(f32::from(delta.x)) * f64::from(scale), dy: -f64::from(f32::from(delta.y)) * f64::from(scale), modifiers: 0 } });
    }

    /// A click on the page: a pin there, its note asked for beside it.
    pub fn design_session_click(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.on.as_ref() else { return };
        if session.pending.is_some() {
            return self.design_session_pin_cancel(cx);
        }
        let live = self.session.shown.live.clone();
        let (spot, area_at, crop) = match &live {
            Some(Live::Page(_)) => {
                let Some((area, rect, header)) = self.session.placed.borrow().clone() else { return };
                let local = (f32::from(at.x - rect.origin.x), f32::from(at.y - rect.origin.y));
                let Some(spot) = rules::place(local, (f32::from(rect.size.width), f32::from(rect.size.height)), &header) else { return };
                let device = ((local.0 / f32::from(rect.size.width).max(1.) * header.w as f32) as u32, (local.1 / f32::from(rect.size.height).max(1.) * header.h as f32) as u32);
                let crop = self.session.frame.as_ref().and_then(|(_, image)| crop_png(image, device));
                (spot, (f32::from(at.x - area.origin.x), f32::from(at.y - area.origin.y)), crop)
            }
            // A render: where on it, in its own pixels.
            Some(Live::Render(path)) => {
                let (Some(rect), Some(area), Some(dims)) = (render_rect(self, path), *self.session.render_area.borrow(), self.shots.borrow().dims(path)) else { return };
                let local = (f32::from(at.x - rect.origin.x), f32::from(at.y - rect.origin.y));
                let Some(spot) = rules::place_on_render(local, (f32::from(rect.size.width), f32::from(rect.size.height)), dims) else { return };
                (spot, (f32::from(at.x - area.origin.x), f32::from(at.y - area.origin.y)), None)
            }
            None => return,
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("session.pin_placeholder")));
        let enter = cx.subscribe_in(&input, window, |shell, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                shell.design_session_pin_save(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.session.note = Some((input, enter));
        if let Some(session) = self.session.on.as_mut() {
            session.pending = Some(Pending { x: spot.0, y: spot.1, at: (area_at.0.max(0.) as u32, area_at.1.max(0.) as u32), reasons: Vec::new(), crop, width: None, selector: None });
        }
        cx.notify();
    }

    /// A pin dropped on the live preview of the session's page: CSS pixels
    /// on a page `width` wide, on the element `selector` names. Its note is
    /// asked for in the preview's bar.
    pub fn design_session_pin_at(&mut self, x: u32, y: u32, width: u32, selector: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.on.is_none() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("session.pin_placeholder")));
        let enter = cx.subscribe_in(&input, window, |shell, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                shell.design_session_pin_save(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.session.note = Some((input, enter));
        if let Some(session) = self.session.on.as_mut() {
            session.pending = Some(Pending { x, y, at: (0, 0), reasons: Vec::new(), crop: None, width: Some(width), selector });
        }
        cx.notify();
    }

    pub fn design_session_pin_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.on.as_mut() {
            session.pending = None;
        }
        self.session.note = None;
        cx.notify();
    }

    /// A reason tapped on the pin's card, or after a thumbs-down.
    pub fn design_session_reason(&mut self, reason: PickReason, down: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.on.as_mut() else { return };
        let list = if down { session.down.as_mut() } else { session.pending.as_mut().map(|p| &mut p.reasons) };
        if let Some(list) = list {
            if let Some(at) = list.iter().position(|r| *r == reason) {
                list.remove(at);
            } else {
                list.push(reason);
            }
        }
        cx.notify();
    }

    /// The pin kept: numbered, its crop attached to the next message, its
    /// reasons (if any) kept with the person's taste.
    pub fn design_session_pin_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let note = self.session.note.take().map(|(input, _)| input.read(cx).value().to_string()).unwrap_or_default();
        let Some(session) = self.session.on.as_mut() else { return };
        let Some(pending) = session.pending.take() else { return };
        let width = pending.width.unwrap_or(session.viewport.width());
        let Some(n) = session.drop_pin_wide(pending.x, pending.y, width, &note, pending.reasons.clone(), pending.selector.clone()) else {
            cx.notify();
            return;
        };
        let (squad, project, page) = (session.squad.clone(), session.project.clone(), session.page.clone());
        if let Some(bytes) = pending.crop {
            self.attach_sources(vec![crate::attach::Source::Bytes { name: rules::crop_name(n), bytes }], cx);
        }
        if !pending.reasons.is_empty() {
            let what = format!("{page}: {}", equipe_core::design::session::Pin::tidy(&note));
            self.send_command(Command::TasteSignals { squad, project, signals: vec![TasteSignal::Rejected { what, picture: None, reasons: pending.reasons }] });
        }
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A quick reaction to what is shown: kept in the style profile; a
    /// thumbs-down asks for its reasons.
    pub fn design_session_react(&mut self, reaction: QuickReaction, cx: &mut Context<Self>) {
        let round = self.session.shown.count;
        let picture = match &self.session.shown.live {
            Some(Live::Render(path)) => Some(path.clone()),
            _ => None,
        };
        let Some(session) = self.session.on.as_mut() else { return };
        session.reactions.push((round, reaction));
        if reaction == QuickReaction::Down {
            session.down = Some(Vec::new());
            cx.notify();
            return;
        }
        let label = rules::round_label(&session.page, round);
        let (squad, project) = (session.squad.clone(), session.project.clone());
        self.send_command(Command::TasteSignals { squad, project, signals: vec![TasteSignal::React { picture, label, reaction, reasons: Vec::new() }] });
        cx.notify();
    }

    /// The thumbs-down's reasons given (or none): kept.
    pub fn design_session_down_done(&mut self, cx: &mut Context<Self>) {
        let round = self.session.shown.count;
        let picture = match &self.session.shown.live {
            Some(Live::Render(path)) => Some(path.clone()),
            _ => None,
        };
        let Some(session) = self.session.on.as_mut() else { return };
        let reasons = session.down.take().unwrap_or_default();
        let label = rules::round_label(&session.page, round);
        let (squad, project) = (session.squad.clone(), session.project.clone());
        self.send_command(Command::TasteSignals { squad, project, signals: vec![TasteSignal::React { picture, label, reaction: QuickReaction::Down, reasons }] });
        cx.notify();
    }

    /// The person's message in a session: its pins before the words, each
    /// crop's place among the pictures named. Untouched elsewhere.
    pub fn design_session_wrap(&mut self, words: String) -> String {
        if self.screen != Screen::DesignSession {
            return words;
        }
        let Some(session) = self.session.on.as_ref() else { return words };
        if session.pins.iter().all(|p| p.sent) {
            return words;
        }
        let ready: Vec<String> = self.composer.strip.items().iter().filter(|i| matches!(i.stage, crate::attach::Stage::Ready(_))).map(|i| i.name.clone()).collect();
        let crops: Vec<(u32, usize)> = session.pins.iter().filter(|p| !p.sent).filter_map(|p| ready.iter().position(|n| *n == rules::crop_name(p.pin.n)).map(|at| (p.pin.n, at + 1))).collect();
        let page = match &self.session.shown.live {
            Some(Live::Page(url)) => url.clone(),
            Some(Live::Render(path)) => path.clone(),
            None => session.page.clone(),
        };
        let pins = self.session.on.as_mut().map(|s| s.sent(&crops)).unwrap_or_default();
        equipe_core::design::session::with_pins(&page, &pins, &words)
    }

    /// The shared browser's tabs changed: the person's own are kept for the session's page.
    pub fn design_session_tabs(&mut self, event: &Event, window: &mut Window) {
        let Event::BrowserTabs { lane, pages, .. } = event else { return };
        if !lane.is_empty() {
            return;
        }
        self.session.mine = pages.clone();
        let url = match &self.session.shown.live {
            Some(Live::Page(url)) => url.clone(),
            _ => return,
        };
        if let Some(session) = self.session.on.as_mut()
            && let Some(before) = session.asked_tab.take()
        {
            session.tab = rules::opened(&url, &before, pages);
            if session.tab.is_none() {
                session.asked_tab = Some(before);
            }
        }
        self.rewatch_live(window);
    }

    /// Each draw on the session's screen: what the designer showed and
    /// marked done read from its conversation, and the page kept live.
    pub fn design_session_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::DesignSession {
            return;
        }
        let Some(session) = self.session.on.as_ref() else { return };
        let Some(entries) = self.model.convos.get(&session.designer).map(|c| c.transcript.entries()) else { return };
        // A command's output lands in its line after the line came: both counted.
        let finished = entries.iter().skip(session.since).filter(|e| matches!(e, equipe_core::protocol::Entry::Tool { outcome: Some(_), .. })).count();
        if (entries.len(), finished) != self.session.scanned {
            self.session.scanned = (entries.len(), finished);
            let shown = rules::shown(entries, session.since);
            let changed = shown != self.session.shown;
            let was = self.session.shown.live.clone();
            self.session.shown = shown;
            if changed {
                let shown = self.session.shown.clone();
                let Some(session) = self.session.on.as_mut() else { return };
                rules::mark_done(&mut session.pins, &shown.done);
                let reload = session.should_reload(&shown);
                match (&shown.live, reload) {
                    (Some(Live::Render(path)), true) => self.shots.borrow_mut().refresh(std::slice::from_ref(path)),
                    (Some(Live::Page(url)), true) if was == shown.live => {
                        if let Some(page) = session.tab {
                            self.send_command(Command::BrowserNav { page, action: BrowserNavAction::Reload, url: None });
                        }
                        let _ = url;
                    }
                    _ => {}
                }
                // The live preview of the session's page follows what the designer shows.
                if reload && self.preview.open.as_ref().is_some_and(|o| o.design) {
                    let designer = self.session.on.as_ref().map(|s| s.designer);
                    match &shown.live {
                        Some(Live::Page(_)) if was == shown.live => self.preview_reload(window, cx),
                        Some(Live::Page(url)) => {
                            self.open_preview(url, designer, true, window, cx);
                        }
                        _ => {}
                    }
                }
                cx.notify();
            }
        }
        if matches!(self.session.shown.live, Some(Live::Page(_))) {
            self.ensure_live(window, cx);
        }
    }

    /// The page's tab in the shared browser, and the stream watching it.
    fn ensure_live(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Live::Page(url)) = self.session.shown.live.clone() else { return };
        let Some(session) = self.session.on.as_mut() else { return };
        if session.asked_tab.is_none() {
            match rules::tab_for(&url, session.tab, &self.session.mine) {
                Tab::Watch(page) => session.tab = Some(page),
                Tab::Go(page) => {
                    session.tab = Some(page);
                    self.send_command(Command::BrowserNav { page, action: BrowserNavAction::Go, url: Some(url.clone()) });
                }
                Tab::Open => {
                    session.asked_tab = Some(self.session.mine.iter().map(|p| p.id).collect());
                    session.tab = None;
                    self.send_command(Command::BrowserNewTab { url: Some(url.clone()), profile: None });
                }
            }
        }
        if self.session.link.is_none() {
            let (link, mut updates) = crate::stream::spawn(self.endpoint.clone());
            self.session.link = Some(link);
            self.session.watching = None;
            self.session._pump = Some(cx.spawn_in(window, async move |this, cx| {
                while let Some(update) = updates.recv().await {
                    if this.update_in(cx, |this, window, cx| this.design_session_frame(update, window, cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        self.rewatch_live(window);
    }

    /// Ask the stream for the page at the session's width.
    fn rewatch_live(&mut self, window: &mut Window) {
        let (Some(link), Some(session)) = (&self.session.link, &self.session.on) else { return };
        let Some(page) = session.tab else { return };
        let fit = session.viewport.fit();
        let fps = if window.is_window_active() { 20 } else { 4 };
        let (max_w, max_h) = ((fit.w as f32 * fit.dpr) as u32, (fit.h as f32 * fit.dpr) as u32);
        let want = StreamIn::Watch { page, fps, max_w, max_h, fit: Some(fit) };
        if self.session.watching.as_ref() != Some(&want) {
            link.send(want.clone());
            self.session.watching = Some(want);
        }
    }

    /// A frame of the page, or the line up or down.
    fn design_session_frame(&mut self, update: Update, window: &mut Window, cx: &mut Context<Self>) {
        match update {
            Update::Line(up) => {
                if up {
                    self.session.watching = None;
                    self.rewatch_live(window);
                }
            }
            Update::Picture(picture) => {
                let ours = self.session.on.as_ref().and_then(|s| s.tab) == Some(picture.header.page);
                if ours {
                    if let Some((_, old)) = self.session.frame.replace((picture.header, picture.image))
                        && let Some(older) = self.session.previous.replace(old)
                    {
                        let _ = window.drop_image(older);
                    }
                } else {
                    let _ = window.drop_image(picture.image);
                }
                let credit = self.session.link.as_ref().map(Link::sender);
                window.on_next_frame(move |_, _| {
                    if let Some(credit) = credit {
                        let _ = credit.send(StreamIn::Credit { n: 1 });
                    }
                });
                cx.notify();
            }
        }
    }

    fn drop_live(&mut self, window: &mut Window) {
        for image in [self.session.frame.take().map(|(_, i)| i), self.session.previous.take()].into_iter().flatten() {
            let _ = window.drop_image(image);
        }
        self.session.watching = None;
        *self.session.placed.borrow_mut() = None;
    }

    /// The snapshot tool's `dsession:` (`snapshot.rs`): `setup[:<project>]`,
    /// `start:<page>`, `pin:<fx>,<fy>,<note>` (a pin at fractions of the page,
    /// its note kept), `reason:<n>`, `viewport:<desktop|phone>`,
    /// `react:<up|down|more|less>`, `polish`, `send:<words>`, `end`.
    pub fn snapshot_session(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (verb, arg) = what.split_once(':').unwrap_or((what, ""));
        match verb {
            "setup" => self.design_session_setup((!arg.is_empty()).then(|| arg.to_owned()), SessionFrom::DesignScreen, window, cx),
            "start" => {
                if self.session.setup.is_none() {
                    self.design_session_setup(Some("web".into()), SessionFrom::DesignScreen, window, cx);
                }
                self.design_session_start(Some(arg.to_owned()), window, cx);
            }
            "pin" => {
                let mut parts = arg.splitn(3, ',');
                let (fx, fy) = (parts.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.5), parts.next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.5));
                let note = parts.next().unwrap_or_default().to_owned();
                let rect = self.session.placed.borrow().as_ref().map(|(_, r, _)| *r);
                if let Some(rect) = rect {
                    let at = point(rect.origin.x + rect.size.width * fx, rect.origin.y + rect.size.height * fy);
                    self.design_session_click(at, window, cx);
                    if let Some((input, _)) = &self.session.note {
                        input.update(cx, |s, cx| s.set_value(note, window, cx));
                    }
                }
            }
            "reason" => {
                if let Some(reason) = arg.parse::<usize>().ok().and_then(|n| PickReason::ALL.get(n).copied()) {
                    let down = self.session.on.as_ref().is_some_and(|s| s.down.is_some());
                    self.design_session_reason(reason, down, cx);
                }
            }
            "save" => self.design_session_pin_save(window, cx),
            "viewport" => self.design_session_viewport(if arg == "phone" { Viewport::Phone } else { Viewport::Desktop }, window, cx),
            "react" => self.design_session_react(
                match arg {
                    "down" => QuickReaction::Down,
                    "more" => QuickReaction::MoreLikeThis,
                    "less" => QuickReaction::LessLikeThis,
                    _ => QuickReaction::Up,
                },
                cx,
            ),
            "polish" => self.design_session_polish(true, cx),
            "end" => self.design_session_end(cx),
            _ => {}
        }
    }
}

/// A crop of the frame around a click, as PNG bytes.
fn crop_png(image: &RenderImage, at: (u32, u32)) -> Option<Vec<u8>> {
    let size = image.size(0);
    let (w, h) = (u32::try_from(i32::from(size.width)).ok()?, u32::try_from(i32::from(size.height)).ok()?);
    let (x, y, cw, ch) = rules::crop_box(at, (w, h), CROP)?;
    let bgra = image.as_bytes(0)?;
    let mut out = image::RgbaImage::new(cw, ch);
    for row in 0..ch {
        for col in 0..cw {
            let i = (((y + row) * w + (x + col)) * 4) as usize;
            let px = bgra.get(i..i + 4)?;
            out.put_pixel(col, row, image::Rgba([px[2], px[1], px[0], px[3]]));
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(out).write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}
