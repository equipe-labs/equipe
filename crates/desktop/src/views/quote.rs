//! Asking about words in the conversation, as Claude's apps do: select a
//! stretch of a reply (a call's output, a message of your own) and, a beat
//! after the button lets go, a small bar floats under the selection's end —
//! "인용해서 묻기" and "복사". Asking (a click, ⏎, ⌘⇧R) opens a box just
//! above the selection: the quote on top (its first two lines, quiet, a bar
//! at its left), a box that grows for the question, its send at the corner.
//! ⏎ or ⌘⏎ sends — a message to the member on screen, as the composer sends
//! one, the quote first as a Markdown blockquote — and Esc puts it away.
//!
//! The person's message shows the quote above their words (`in_bubble`); a
//! click on it goes back to the words quoted and flashes them, if they are
//! still there. The rules — trimming, the message and reading it back,
//! finding the words, where things float — are `crate::quote`'s.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use equipe_core::protocol::SessionId;
use equipe_core::t;
use gpui_kit::base::{ElementExt as _, TextSelection};
use gpui_kit::base::text::RangeHighlight;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use crate::compose::EnterGuard;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::quote::{self as rules, Rect};
use crate::theme::{ICON_SM, META, Palette, R_LG, R_MD, R_SM, SMALL};
use crate::tips::tip;
use crate::{Screen, Shell};

/// How long after the button lets go the bar shows: a drag that goes on
/// doesn't make it flicker.
const SETTLE: Duration = Duration::from_millis(150);
/// The quoted words flash this long, strongly, then fade out over `FADE`.
const FLASH: Duration = Duration::from_millis(900);
const FADE: Duration = Duration::from_millis(500);
/// The bar's size, for keeping it inside the window.
const BAR: (f32, f32) = (196., 30.);
/// The box's height before it is laid out, for whether it fits above.
const BOX_HEIGHT: f32 = 132.;

#[derive(Default)]
pub struct QuoteUi {
    /// The words selected in the conversation, offered to quote.
    pub offer: Option<Offer>,
    /// The box open on a quote.
    pub ask: Option<Ask>,
    /// The row whose words are lit (the box's quote, or a flash), to unlight.
    lit: Option<usize>,
    _settle: Option<Task<()>>,
    _flash: Option<Task<()>>,
}

pub struct Offer {
    session: SessionId,
    text: String,
    /// Where the selection's ends were last drawn.
    ends: Cell<Option<(Rect, Rect)>>,
    copied: bool,
}

pub struct Ask {
    session: SessionId,
    quote: String,
    input: Entity<TextareaState>,
    start: Rect,
    end: Rect,
    /// The box's height as last laid out.
    height: Rc<Cell<f32>>,
    guard: EnterGuard,
    _subscriptions: Vec<Subscription>,
}

/// The selection's ends, in the window's points.
fn ends(window: &Window, cx: &App) -> Option<(Rect, Rect)> {
    if TextSelection::is_selecting(window, cx) {
        return None;
    }
    let (start, end) = TextSelection::ends(window, cx)?;
    let rect = |b: Bounds<Pixels>| Rect { x: f32::from(b.origin.x), y: f32::from(b.origin.y), w: f32::from(b.size.width), h: f32::from(b.size.height) };
    Some((rect(start), rect(end)))
}

fn window_size(window: &Window) -> (f32, f32) {
    let size = window.viewport_size();
    (f32::from(size.width), f32::from(size.height))
}

impl Shell {
    /// The button let go over the conversation: a beat later, words selected
    /// there are offered to quote.
    pub fn quote_pointer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quote.ask.is_some() {
            return;
        }
        self.quote._settle = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update_in(cx, |shell, window, cx| shell.quote_offer(window, cx)).ok();
        }));
    }

    /// The button pressed in the conversation: whatever was offered goes.
    pub fn quote_pointer_down(&mut self, cx: &mut Context<Self>) {
        self.quote._settle = None;
        if self.quote.offer.take().is_some() {
            cx.notify();
        }
    }

    fn quote_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(session), Some(at)) = (self.model.viewed(), ends(window, cx)) else { return };
        if self.screen != Screen::Chat {
            return;
        }
        let Some(text) = rules::trim(&TextSelection::selected_text(window, cx)) else { return };
        self.quote.offer = Some(Offer { session, text, ends: Cell::new(Some(at)), copied: false });
        cx.notify();
    }

    /// The offer, while its selection still shows in the conversation on screen.
    fn live_offer(&self, window: &Window, cx: &App) -> Option<&Offer> {
        let offer = self.quote.offer.as_ref().filter(|o| self.screen == Screen::Chat && Some(o.session) == self.model.viewed() && self.quote.ask.is_none())?;
        // Copied, it stays a moment to say so.
        if !offer.copied {
            offer.ends.set(Some(ends(window, cx)?));
        }
        Some(offer)
    }

    /// "복사": the words selected, to the clipboard; the bar says so a moment.
    pub fn quote_copy(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.quote.offer.as_mut() else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(offer.text.clone()));
        offer.copied = true;
        self.quote._settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(900)).await;
            this.update(cx, |shell, cx| {
                shell.quote.offer = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// "인용해서 묻기" (or ⏎, ⌘⇧R): the box opens above the selection, the
    /// keys in its question.
    pub fn quote_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.quote.offer.take() else { return };
        let Some((start, end)) = offer.ends.get() else { return };
        let who = self.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false).placeholder(t!("quote.placeholder", name = who)));
        let mut subscriptions = vec![cx.subscribe_in(&input, window, |_: &mut Shell, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        subscriptions.push(cx.observe_in(&input, window, |shell: &mut Shell, input, window, cx| {
            let composing = input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
            if let Some(ask) = shell.quote.ask.as_mut() {
                ask.guard.observe(composing, Instant::now());
            }
        }));
        input.update(cx, |s, cx| s.focus(window, cx));
        let quote = offer.text;
        let lit = self.quote_light(&quote, None, false, cx);
        self.quote.lit = lit;
        self.quote.ask = Some(Ask { session: offer.session, quote, input, start, end, height: Rc::new(Cell::new(BOX_HEIGHT)), guard: EnterGuard::default(), _subscriptions: subscriptions });
        cx.notify();
    }

    /// Esc, or a click away: the box goes, the keys back to the composer.
    pub fn quote_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quote.ask.take().is_none() {
            return;
        }
        self.quote_unlight(cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// ⏎ or the send button: the quote and the question, to the member on
    /// screen, as the composer sends a message.
    pub fn quote_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.quote.ask.as_mut() else { return };
        if !ask.guard.allows_send(Instant::now()) {
            return;
        }
        let (question, session) = (ask.input.read(cx).value().to_string(), ask.session);
        let text = rules::message(&ask.quote, &question);
        if question.trim().is_empty() || !self.can_act() || Some(session) != self.model.viewed() {
            return;
        }
        let plan = crate::compose::plan(&self.model, &text, false);
        self.composer.flash = plan.flash;
        if plan.commands.is_empty() {
            cx.notify();
            return;
        }
        for command in plan.commands {
            self.send_command(command);
        }
        self.quote.ask = None;
        self.quote_unlight(cx);
        TextSelection::clear(window, cx);
        self.composer.focus(window, cx);
        self.chat.follow_tail(cx);
        cx.notify();
    }

    /// A click on the quote in a message of yours (row `ix`): back to the
    /// words quoted — the latest row before it that holds them — flashing
    /// them there.
    pub fn quote_jump(&mut self, ix: usize, quote: &str, cx: &mut Context<Self>) {
        let blocks = self.chat.blocks.clone();
        let Some(found) = blocks[..ix.min(blocks.len())].iter().rposition(|b| b.searchable().is_some_and(|text| rules::holds(&text, quote))) else { return };
        self.chat.scroll_to(found, cx);
        let quote = quote.to_owned();
        // Once the row is laid out, its text views can be found and lit.
        self.quote._flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(60)).await;
            let lit = this.update(cx, |shell, cx| shell.quote_light(&quote, Some(found), true, cx)).ok().flatten();
            if lit.is_none() {
                return;
            }
            cx.background_executor().timer(FLASH).await;
            this.update(cx, |shell, cx| shell.quote_light(&quote, Some(found), false, cx)).ok();
            cx.background_executor().timer(FADE).await;
            this.update(cx, |shell, cx| shell.quote_unlight(cx)).ok();
        }));
        cx.notify();
    }

    /// Light the quote where it shows: in row `ix` (or, `None`, the latest
    /// row drawn that holds it), strongly or softly, scrolled into view when
    /// strong. The row lit.
    fn quote_light(&mut self, quote: &str, ix: Option<usize>, strong: bool, cx: &mut Context<Self>) -> Option<usize> {
        // ⌘F's marks are in the same views; they win.
        if self.chat.find.open {
            return None;
        }
        let rows: Vec<usize> = match ix {
            Some(ix) => vec![ix],
            None => (0..self.chat.blocks.len()).rev().filter(|ix| self.chat.blocks[*ix].searchable().is_some_and(|text| rules::holds(&text, quote))).collect(),
        };
        let p = self.palette;
        let tone: Hsla = Rgba { a: if strong { 0.34 } else { 0.16 }, ..p.accent }.into();
        for ix in rows {
            let mut lit = false;
            for view in self.chat.row_views(ix) {
                view.update(cx, |s, cx| {
                    let text = s.rendered_text();
                    if let Some(range) = rules::locate(text.as_str(), quote) {
                        lit |= s.set_range_highlights([RangeHighlight::new(range.clone(), tone)], cx).is_ok();
                        if strong {
                            let _ = s.reveal_range(range, cx);
                        }
                    }
                });
            }
            if lit {
                if self.quote.lit.is_some_and(|was| was != ix) {
                    self.quote_unlight(cx);
                }
                self.quote.lit = Some(ix);
                return Some(ix);
            }
        }
        None
    }

    fn quote_unlight(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.quote.lit.take() else { return };
        if self.chat.find.open {
            return;
        }
        for view in self.chat.row_views(ix) {
            view.update(cx, |s, cx| s.clear_range_highlights(cx));
        }
    }

    /// A keystroke, before anyone else sees it: in the box, ⏎ / ⌘⏎ send and
    /// Esc closes (⇧⏎ is a new line); with words offered, ⏎ or ⌘⇧R opens
    /// the box and Esc puts the bar away.
    pub fn on_quote_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        if let Some(ask) = &self.quote.ask {
            if !ask.input.read(cx).focus_handle(cx).is_focused(window) {
                return Took::Pass;
            }
            let input = ask.input.clone();
            return match key {
                "escape" => {
                    self.quote_close(window, cx);
                    Took::Taken
                }
                "enter" if !m.shift && !m.alt && !m.control => {
                    // The Enter that ends a Korean syllable is only that.
                    if input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some()) {
                        return Took::Typing;
                    }
                    self.quote_send(window, cx);
                    Took::Taken
                }
                _ => Took::Typing,
            };
        }
        if self.live_offer(window, cx).is_none() {
            return Took::Pass;
        }
        let plain = !(m.platform || m.control || m.alt || m.shift || m.function);
        match key {
            "r" if m.platform && m.shift && !m.control && !m.alt => {
                self.quote_open(window, cx);
                Took::Taken
            }
            "enter" if plain && !self.composer.is_focused(window, cx) => {
                self.quote_open(window, cx);
                Took::Taken
            }
            "escape" => {
                self.quote.offer = None;
                cx.notify();
                Took::Pass
            }
            _ => Took::Pass,
        }
    }

    /// Dev tool (`snapshot.rs`): words into the box's question.
    #[cfg(feature = "snapshot")]
    pub fn quote_type(&mut self, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ask) = &self.quote.ask {
            ask.input.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
            cx.notify();
        }
    }

    /// Dev tool (`snapshot.rs`): a click on the quote of the latest message of yours that has one.
    #[cfg(feature = "snapshot")]
    pub fn quote_jump_latest(&mut self, cx: &mut Context<Self>) {
        let found = self.chat.blocks.iter().enumerate().rev().find_map(|(ix, b)| match b {
            crate::convo::Block::User(text) => rules::split(&crate::sent::parts(text).text).map(|(q, _)| (ix, q)),
            _ => None,
        });
        if let Some((ix, quote)) = found {
            self.quote_jump(ix, &quote, cx);
        }
    }
}

/// What floats over the conversation: the box, or the bar under a selection.
pub fn layer(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    if let Some(ask) = shell.quote.ask.as_ref().filter(|a| Some(a.session) == shell.model.viewed()) {
        return Some(ask_box(shell, ask, window, cx));
    }
    let before = shell.quote.offer.as_ref().and_then(|o| o.ends.get());
    let offer = shell.live_offer(window, cx)?;
    // The ends are last frame's: moved (the conversation scrolled), one more
    // frame puts the bar where they are now.
    if offer.ends.get() != before {
        window.request_animation_frame();
    }
    let (_, end) = offer.ends.get()?;
    let (x, y) = rules::button_at(end, BAR, window_size(window));
    Some(deferred(anchored().position(point(px(x), px(y))).child(motion::fade(bar(shell.palette, offer.copied, cx), "quote-bar-in", motion::FAST))).with_priority(2).into_any_element())
}

/// The bar: ask about the words, or copy them.
fn bar(p: Palette, copied: bool, cx: &mut Context<Shell>) -> impl IntoElement + Styled + use<> {
    let item = |id: &'static str, glyph: Icon, words: &'static str| {
        h_flex()
            .id(id)
            .h_full()
            .px(px(9.))
            .gap(px(6.))
            .items_center()
            .rounded(px(R_SM))
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(icon(glyph, 13., p.text2))
            .child(words)
    };
    let ask = tip(item("quote-ask", Icon::Message, t!("quote.ask")), p, t!("quote.ask_tip"), Some("⌘⇧R")).on_click(cx.listener(|shell, _, window, cx| shell.quote_open(window, cx)));
    let copy = if copied { item("quote-copy", Icon::Check, t!("quote.copied")) } else { item("quote-copy", Icon::Copy, t!("chat.copy")) }.on_click(cx.listener(|shell, _, _, cx| shell.quote_copy(cx)));
    h_flex()
        .id("quote-bar")
        .occlude()
        .h(px(BAR.1))
        .p(px(2.))
        .gap(px(2.))
        .items_center()
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow(motion::float_shadow(0.55))
        .text_size(px(12.5))
        .text_color(p.text)
        // A press here is the bar's, not a new selection's.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(ask)
        .child(div().w(px(1.)).h(px(14.)).bg(p.hair))
        .child(copy)
}

/// The box: the quote, the question, its send.
fn ask_box(shell: &Shell, ask: &Ask, window: &Window, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (width, (x, y), above) = rules::box_at(ask.start, ask.end, ask.height.get(), window_size(window));
    let can = !ask.input.read(cx).value().trim().is_empty() && shell.can_act();
    let quote = h_flex()
        .gap(px(9.))
        .items_stretch()
        .child(div().flex_none().w(px(2.)).rounded_full().bg(p.hair))
        .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(19.)).text_color(p.muted).line_clamp(2).text_ellipsis().child(ask.quote.clone()));
    let send = div()
        .id("quote-send")
        .size(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(if can { p.primary } else { p.wash })
        .when(can, |b| b.cursor_pointer().on_click(cx.listener(|shell, _, window, cx| shell.quote_send(window, cx))))
        .child(icon(Icon::ArrowUp, ICON_SM, if can { p.on_primary } else { p.muted }));
    let send = tip(send, p, t!("quote.send"), Some("⏎"));
    let field = v_flex()
        .px(px(10.))
        .pt(px(8.))
        .pb(px(6.))
        .gap(px(4.))
        .rounded(px(R_MD))
        .bg(p.bg)
        .border_1()
        .border_color(p.hair)
        .text_size(px(14.))
        .line_height(px(21.))
        .child(Textarea::new(&ask.input).appearance(false).bordered(false))
        .child(h_flex().gap_2().items_center().child(div().flex_1().text_size(px(META)).text_color(p.muted).child(t!("quote.keys"))).child(send));
    let height = ask.height.clone();
    let card = v_flex()
        .id("quote-box")
        .occlude()
        .w(px(width))
        .p(px(10.))
        .gap(px(9.))
        .rounded(px(R_LG))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow(motion::float_shadow(0.7))
        .on_mouse_down_out(cx.listener(|shell, _, window, cx| shell.quote_close(window, cx)))
        .on_prepaint(move |bounds, _, _| height.set(f32::from(bounds.size.height)))
        .child(quote)
        .child(field);
    deferred(anchored().position(point(px(x), px(y))).child(motion::pop(card, "quote-box-in", above))).with_priority(2).into_any_element()
}

/// The quote at the top of a message of yours: a quiet bar at its left, its
/// first lines muted; a click goes back to the words.
pub fn in_bubble(p: Palette, key: u64, ix: usize, quote: &str, handle: WeakEntity<Shell>) -> AnyElement {
    let quote = quote.to_owned();
    let words = quote.clone();
    let row = h_flex()
        .id(("user-quote", key))
        .gap(px(9.))
        .items_stretch()
        .rounded(px(R_SM))
        .cursor_pointer()
        .child(div().flex_none().w(px(2.)).rounded_full().bg(Rgba { a: 0.45, ..p.muted }))
        .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(19.)).text_color(p.muted).line_clamp(3).text_ellipsis().child(words))
        .hover(move |s| s.opacity(0.8))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.quote_jump(ix, &quote, cx)).ok();
        });
    tip(row, p, t!("quote.jump"), None).into_any_element()
}
