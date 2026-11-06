//! Asking about — and replying to — words in the conversation, as Claude's
//! apps do: select a stretch of a reply (a call's output, a message of your
//! own) and, the moment the button lets go, a small bar floats over the
//! selection — "Reply", "Ask about this" and "Copy", each acting on the
//! press.
//!
//! Replying opens a small box by the selection: what the person says to that
//! passage, ⏎ making it a card in a row over the composer (Esc: nothing).
//! They do this for as many passages as they like; each card shows their
//! words beside a thin bar — the passage over it while the pointer is on it,
//! and a click goes back to it — with ✕ and Reply (the box again, to change
//! the words). Asking (a press, ⏎, ⌘⇧R) makes a card of the passage alone:
//! the composer's words are the question. The cards are kept with the draft
//! and go, in order, with the next message — the composer's words after
//! them — even from an empty box.
//!
//! The person's message shows each passage folded over the words to it
//! (`in_bubble`); a click on it goes back to the words quoted and flashes
//! them, if they are still there. The rules — trimming, the message and
//! reading it back, finding the words, where things float — are
//! `crate::quote`'s.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use equipe_core::protocol::SessionId;
use equipe_core::t;
use gpui_kit::base::text::RangeHighlight;
use gpui_kit::base::{SelectableText, TextSelection};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::decide::Took;
use crate::compose::EnterGuard;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::quote::{self as rules, Rect, Reply};
use crate::theme::{BODY, BODY_LINE, META, Palette, R_LG, R_MD, R_SM, SMALL};
use crate::tips::tip;
use crate::{Screen, Shell};

/// Looked at again this often, this many times, while the selection ends
/// (it is first looked at as soon as the release has been handled).
const SETTLE_AGAIN: Duration = Duration::from_millis(16);
const SETTLE_TRIES: usize = 12;
/// The quoted words flash this long, strongly, then fade out over `FADE`.
const FLASH: Duration = Duration::from_millis(900);
const FADE: Duration = Duration::from_millis(500);
/// The bar's size, for keeping it inside the window.
const BAR: (f32, f32) = (340., 30.);
/// A card over the composer: this wide, its words this many lines at most.
const CARD_W: f32 = 180.;
const CARD_LINES: usize = 4;
const CARD_LINE: f32 = 18.;
/// The box's height before it is first drawn.
const BOX_HEIGHT: f32 = 96.;

#[derive(Default)]
pub struct QuoteUi {
    /// The words selected in the conversation, offered to quote.
    pub offer: Option<Offer>,
    /// The row whose words are lit (a flash), to unlight.
    lit: Option<usize>,
    /// A press began in the conversation: its release, wherever it lands
    /// (on the composer, a card, outside the window), offers what it selected.
    pressed: bool,
    /// The box open for what the person says to a passage.
    pub writing: Option<Writing>,
    _settle: Option<Task<()>>,
    _flash: Option<Task<()>>,
}

pub struct Offer {
    session: SessionId,
    text: String,
    /// Where the selection's ends were last drawn.
    ends: Cell<Option<(Rect, Rect)>>,
    /// Where the button let go: the bar's place when the selection doesn't
    /// say where its ends are (a word picked by a double click).
    released: Rect,
    copied: bool,
}

/// The box for what the person says to a passage ("Reply", or a card's
/// Reply): ⏎ makes — or changes — the card, Esc closes it.
pub struct Writing {
    session: SessionId,
    quote: String,
    /// The card it was opened from; `None`, a passage just selected.
    card: Option<usize>,
    /// What it floats by: the selection's ends (a card's: where it is drawn).
    ends: (Rect, Rect),
    input: Entity<TextareaState>,
    height: Rc<Cell<f32>>,
    guard: EnterGuard,
    _subscriptions: Vec<Subscription>,
}

fn rect(b: Bounds<Pixels>) -> Rect {
    Rect { x: f32::from(b.origin.x), y: f32::from(b.origin.y), w: f32::from(b.size.width), h: f32::from(b.size.height) }
}

/// Where card `n` was last drawn, if it was.
fn card_rect(shell: &Shell, n: usize) -> Option<Rect> {
    shell.composer.card_rects.borrow().get(n).filter(|b| b.size.width > px(0.)).map(|b| rect(*b))
}

/// The selection's ends, in the window's points.
fn ends(window: &Window, cx: &App) -> Option<(Rect, Rect)> {
    if TextSelection::is_selecting(window, cx) {
        return None;
    }
    let (start, end) = TextSelection::ends(window, cx)?;
    Some((rect(start), rect(end)))
}

fn window_size(window: &Window) -> (f32, f32) {
    let size = window.viewport_size();
    (f32::from(size.width), f32::from(size.height))
}

impl Shell {
    /// The button let go over the conversation: a beat later, words selected
    /// there are offered to quote.
    pub fn quote_pointer_up(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.quote.pressed) {
            return;
        }
        let released = Rect { x: f32::from(at.x), y: f32::from(at.y), w: 1., h: 1. };
        self.quote._settle = Some(cx.spawn_in(window, async move |this, cx| {
            // At once, once the release has been handled; the selection may
            // still be ending (a slow frame): look again a few times, a frame apart.
            for n in 0..SETTLE_TRIES {
                if n > 0 {
                    cx.background_executor().timer(SETTLE_AGAIN).await;
                }
                let Ok(settled) = this.update_in(cx, |shell, window, cx| shell.quote_offer(released, window, cx)) else { return };
                if settled {
                    return;
                }
            }
        }));
    }

    /// The button pressed in the conversation: whatever was offered goes.
    pub fn quote_pointer_down(&mut self, cx: &mut Context<Self>) {
        self.quote._settle = None;
        self.quote.pressed = true;
        if self.quote.offer.take().is_some() {
            cx.notify();
        }
    }

    /// Offer what is selected, if anything. False while the selection is
    /// still being dragged out, or its words not there yet, to be asked again.
    fn quote_offer(&mut self, released: Rect, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if TextSelection::is_selecting(window, cx) {
            return false;
        }
        let Some(session) = self.model.viewed().filter(|_| self.screen == Screen::Chat) else { return true };
        // Nothing yet: the selection's words may not be drawn yet — looked at again.
        let Some(text) = rules::trim(&TextSelection::selected_text(window, cx)) else { return false };
        self.quote.offer = Some(Offer { session, text, ends: Cell::new(ends(window, cx)), released, copied: false });
        cx.notify();
        true
    }

    /// The offer, while its selection still shows in the conversation on screen.
    fn live_offer(&self, window: &Window, cx: &App) -> Option<&Offer> {
        let offer = self.quote.offer.as_ref().filter(|o| self.screen == Screen::Chat && Some(o.session) == self.model.viewed())?;
        // Copied, it stays a moment to say so.
        if !offer.copied {
            // Where the selection is drawn now; cleared (a press, on the bar
            // too, clears it), where it was; never drawn, where the button let go.
            offer.ends.set(Some(rules::place(ends(window, cx), offer.ends.get(), offer.released)));
        }
        Some(offer)
    }

    /// "Copy": the words selected, to the clipboard; the bar says so a moment.
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

    /// "Ask about this" (or ⏎, ⌘⇧R): the passage a card over the composer,
    /// and the keys there for the question — the person's draft as it was.
    pub fn quote_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.quote.offer.take() else { return };
        if let Some(at) = self.composer.replies.iter().position(|r| r.quote == offer.text) {
            self.composer.reveal_card(at);
        } else {
            self.composer.replies.push(Reply::asked(offer.text));
            self.composer.reveal_card(self.composer.replies.len() - 1);
        }
        TextSelection::clear(window, cx);
        self.composer.focus(window, cx);
        self.save_drafts_soon(cx);
        cx.notify();
    }

    /// "Reply": the box opens by the selection, the keys in it.
    pub fn quote_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.quote.offer.take() else { return };
        let ends = rules::place(None, offer.ends.get(), offer.released);
        // Replied to already: that card's words, to go on with.
        let card = self.composer.replies.iter().position(|r| r.quote == offer.text);
        let words = card.map(|n| self.composer.replies[n].words.clone()).unwrap_or_default();
        self.quote_write(offer.session, offer.text, card, ends, &words, window, cx);
    }

    /// A card's Reply (or ⏎ on it): its box again over it, its words in it.
    pub fn quote_edit(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(reply), Some(session)) = (self.composer.replies.get(n).cloned(), self.model.viewed()) else { return };
        self.composer.reveal_card(n);
        let (w, h) = window_size(window);
        let at = card_rect(self, n).unwrap_or(Rect { x: rules::EDGE, y: h - 200., w: CARD_W.min(w), h: 1. });
        self.quote_write(session, reply.quote, Some(n), (at, at), &reply.words, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn quote_write(&mut self, session: SessionId, quote: String, card: Option<usize>, ends: (Rect, Rect), words: &str, window: &mut Window, cx: &mut Context<Self>) {
        let who = self.model.selected_member().map(|m| m.name.clone()).unwrap_or_default();
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(false).placeholder(t!("quote.reply_placeholder", name = who)));
        if !words.is_empty() {
            input.update(cx, |s, cx| {
                s.set_value(words.to_owned(), window, cx);
                s.set_selected_range(words.len()..words.len(), cx);
            });
        }
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |_: &mut Shell, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // The Enter that ends a composition is only that.
            cx.observe_in(&input, window, |shell: &mut Shell, input, window, cx| {
                let composing = input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some());
                if let Some(w) = shell.quote.writing.as_mut() {
                    w.guard.observe(composing, Instant::now());
                }
            }),
        ];
        input.update(cx, |s, cx| s.focus(window, cx));
        self.quote_unlight(cx);
        self.quote_light(&quote, None, false, cx);
        self.composer.peek = None;
        self.composer.card = None;
        self.quote.writing = Some(Writing { session, quote, card, ends, input, height: Rc::new(Cell::new(BOX_HEIGHT)), guard: EnterGuard::default(), _subscriptions: subscriptions });
        cx.notify();
    }

    /// ⏎ in the box (or its button): the card made — or, the passage replied
    /// to already, its words changed — and the keys back in the composer.
    pub fn quote_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(w) = self.quote.writing.as_mut() else { return };
        if !w.guard.allows_send(Instant::now()) {
            return;
        }
        let Some(w) = self.quote.writing.take() else { return };
        let words = w.input.read(cx).value().trim().to_owned();
        let replies = &mut self.composer.replies;
        let at = match w.card.filter(|n| replies.get(*n).is_some_and(|r| r.quote == w.quote)).or_else(|| replies.iter().position(|r| r.quote == w.quote)) {
            Some(n) => n,
            None => {
                replies.push(Reply::asked(w.quote));
                replies.len() - 1
            }
        };
        replies[at].words = words;
        self.composer.reveal_card(at);
        self.quote_unlight(cx);
        TextSelection::clear(window, cx);
        self.composer.focus(window, cx);
        self.save_drafts_soon(cx);
        cx.notify();
    }

    /// Esc, or a press away: the box goes, nothing made, the keys back to the composer.
    pub fn quote_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quote.writing.take().is_none() {
            return;
        }
        self.quote_unlight(cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// A card's ✕ (or Backspace): it goes; the keys stay on the cards near it.
    pub fn remove_card(&mut self, n: usize, cx: &mut Context<Self>) {
        if n >= self.composer.replies.len() {
            return;
        }
        self.composer.replies.remove(n);
        let left = self.composer.replies.len();
        self.composer.card = self.composer.card.filter(|_| left > 0).map(|at| at.min(left - 1));
        self.composer.peek = None;
        // Its box, open, goes with it; another card's is now one to the left.
        if let Some(w) = self.quote.writing.as_mut() {
            match w.card {
                Some(c) if c == n => self.quote.writing = None,
                Some(c) if c > n => w.card = Some(c - 1),
                _ => {}
            }
        }
        self.save_drafts_soon(cx);
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

    /// A keystroke, before anyone else sees it: in the box, ⏎ makes the card
    /// and Esc closes it (⇧⏎ is a new line); with words offered, ⏎ or ⌘⇧R
    /// quotes them over the composer and Esc puts the bar away.
    pub fn on_quote_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        if let Some(w) = &self.quote.writing {
            if !w.input.read(cx).focus_handle(cx).is_focused(window) {
                return Took::Pass;
            }
            let input = w.input.clone();
            return match key {
                "escape" => {
                    self.quote_cancel(window, cx);
                    Took::Taken
                }
                "enter" if !m.shift && !m.alt && !m.control => {
                    if input.update(cx, |s, cx| gpui_kit::EntityInputHandler::marked_text_range(s, window, cx).is_some()) {
                        return Took::Typing;
                    }
                    self.quote_add(window, cx);
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

    /// Dev tool (`snapshot.rs`): words into the box.
    #[cfg(feature = "snapshot")]
    pub fn quote_type(&mut self, words: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(w) = &self.quote.writing {
            w.input.update(cx, |s, cx| s.set_value(words.to_owned(), window, cx));
            cx.notify();
        }
    }

    /// Dev tool (`snapshot.rs`): a card made at once, of `quote` and `words`.
    #[cfg(feature = "snapshot")]
    pub fn quote_card(&mut self, quote: &str, words: &str, cx: &mut Context<Self>) {
        self.composer.replies.push(Reply { quote: quote.to_owned(), words: words.to_owned() });
        self.composer.reveal_card(self.composer.replies.len() - 1);
        cx.notify();
    }

    /// Dev tool (`snapshot.rs`): a click on the quote of the latest message of yours that has one.
    #[cfg(feature = "snapshot")]
    pub fn quote_jump_latest(&mut self, cx: &mut Context<Self>) {
        let found = self.chat.blocks.iter().enumerate().rev().find_map(|(ix, b)| match b {
            crate::convo::Block::User(text) => rules::read(&crate::design_talk::split(&crate::sent::parts(text).text).1).0.into_iter().find(|r| !r.quote.is_empty()).map(|r| (ix, r.quote)),
            _ => None,
        });
        if let Some((ix, quote)) = found {
            self.quote_jump(ix, &quote, cx);
        }
    }
}

/// What floats over the conversation: the box for a reply, or the bar over a selection.
pub fn layer(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    if let Some(w) = shell.quote.writing.as_ref().filter(|w| Some(w.session) == shell.model.viewed()) {
        return Some(reply_box(shell, w, window, cx));
    }
    let before = shell.quote.offer.as_ref().and_then(|o| o.ends.get());
    let offer = shell.live_offer(window, cx)?;
    // The ends are last frame's: moved (the conversation scrolled), one more
    // frame puts the bar where they are now.
    if offer.ends.get() != before {
        window.request_animation_frame();
    }
    let (start, end) = offer.ends.get()?;
    let (x, y) = rules::button_at(start, end, BAR, window_size(window));
    // At once: no fade to wait through.
    Some(deferred(anchored().position(point(px(x), px(y))).child(bar(shell.palette, offer.copied, cx))).with_priority(2).into_any_element())
}

/// The bar: ask about the words, or copy them — on the press itself: the
/// window's text selection clears on a press, before any click could land.
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
    let reply = tip(item("quote-reply", Icon::Reply, t!("quote.reply")), p, t!("quote.reply_tip"), None).on_mouse_down(
        MouseButton::Left,
        cx.listener(|shell, _, window, cx| {
            cx.stop_propagation();
            shell.quote_reply(window, cx);
        }),
    );
    let ask = tip(item("quote-ask", Icon::Message, t!("quote.ask")), p, t!("quote.ask_tip"), None).on_mouse_down(
        MouseButton::Left,
        cx.listener(|shell, _, window, cx| {
            cx.stop_propagation();
            shell.quote_open(window, cx);
        }),
    );
    let copy = if copied { item("quote-copy", Icon::Check, t!("quote.copied")) } else { item("quote-copy", Icon::Copy, t!("chat.copy")) }.on_mouse_down(
        MouseButton::Left,
        cx.listener(|shell, _, _, cx| {
            cx.stop_propagation();
            shell.quote_copy(cx);
        }),
    );
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
        .child(reply)
        .child(div().w(px(1.)).h(px(14.)).bg(p.hair))
        .child(ask)
        .child(div().w(px(1.)).h(px(14.)).bg(p.hair))
        .child(copy)
}

/// A passage as a quote: a thin bar at its left, its first lines muted.
fn passage(p: Palette, quote: &str, lines: usize) -> Div {
    h_flex()
        .gap(px(9.))
        .items_stretch()
        .child(div().flex_none().w(px(2.)).rounded_full().bg(Rgba { a: 0.45, ..p.muted }))
        .child(div().flex_1().min_w_0().text_size(px(SMALL)).line_height(px(19.)).text_color(p.muted).line_clamp(lines).text_ellipsis().child(quote.to_owned()))
}

/// The box: (opened from a card) its passage, a click away from where it
/// is; what the person says to it; its button, and the keys.
fn reply_box(shell: &Shell, w: &Writing, window: &Window, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    // A card's box follows the card (the row scrolls).
    let (start, end) = w.card.and_then(|n| card_rect(shell, n)).map_or(w.ends, |r| (r, r));
    let (width, (x, y), above) = rules::box_at(start, end, w.height.get(), window_size(window));
    let editing = w.card.is_some();
    let quote = editing.then(|| {
        let q = w.quote.clone();
        let row = passage(p, &w.quote, 3)
            .id("quote-box-passage")
            .rounded(px(R_SM))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .on_click(cx.listener(move |shell, _, _, cx| shell.quote_jump(usize::MAX, &q, cx)));
        tip(row, p, t!("quote.jump"), None)
    });
    let add = div()
        .id("quote-box-add")
        .h(px(26.))
        .px(px(10.))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(R_SM))
        .bg(p.primary)
        .text_color(p.on_primary)
        .text_size(px(12.5))
        .cursor_pointer()
        .child(if editing { t!("quote.reply_save") } else { t!("quote.reply_add") })
        .on_click(cx.listener(|shell, _, window, cx| shell.quote_add(window, cx)));
    let add = tip(add, p, if editing { t!("quote.reply_save") } else { t!("quote.reply_add") }, Some("⏎"));
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
        .child(Textarea::new(&w.input).appearance(false).bordered(false))
        .child(h_flex().gap_2().items_center().child(div().flex_1().text_size(px(META)).text_color(p.muted).child(if editing { t!("quote.reply_keys_save") } else { t!("quote.reply_keys") })).child(add));
    let height = w.height.clone();
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
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(cx.listener(|shell, _, window, cx| shell.quote_cancel(window, cx)))
        .on_prepaint(move |bounds, _, _| height.set(f32::from(bounds.size.height)))
        .children(quote)
        .child(field);
    deferred(anchored().position(point(px(x), px(y))).child(motion::pop(card, "quote-box-in", above))).with_priority(2).into_any_element()
}

/// The cards over the composer, `inset` from its edges: in a row that
/// scrolls sideways when they overflow it, fading where there is more.
pub fn cards(shell: &Shell, inset: f32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let composer = &shell.composer;
    if composer.replies.is_empty() {
        return None;
    }
    let p = shell.palette;
    composer.card_rects.borrow_mut().resize(composer.replies.len(), Bounds::default());
    let editing = shell.quote.writing.as_ref().filter(|w| Some(w.session) == shell.model.viewed()).and_then(|w| w.card);
    let row = h_flex()
        .id("reply-cards")
        .w_full()
        .track_scroll(&composer.cards_scroll)
        .overflow_x_scroll()
        .pl(px(inset))
        .pt(px(inset))
        .gap_2()
        .items_stretch()
        .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
        .children(composer.replies.iter().enumerate().map(|(n, reply)| card(shell, n, reply, editing == Some(n), cx)))
        // The room at the right end: a scrolled row's own padding there is lost.
        .child(div().flex_none().w(px(inset - 8.)));
    let (off, max) = (f32::from(composer.cards_scroll.offset().x), f32::from(composer.cards_scroll.max_offset().x));
    let clear = Hsla::from(p.raised).alpha(0.);
    let fade = |left: bool| {
        let (from, to) = if left { (Hsla::from(p.raised), clear) } else { (clear, Hsla::from(p.raised)) };
        div().absolute().top_0().bottom_0().w(px(40.)).when(left, |d| d.left_0()).when(!left, |d| d.right_0()).bg(linear_gradient(90., linear_color_stop(from, 0.), linear_color_stop(to, 1.)))
    };
    Some(
        div()
            .id("reply-cards-edge")
            .relative()
            .w_full()
            .role(Role::List)
            .aria_label(t!("quote.cards"))
            .child(row)
            // The row's padding scrolls too: within it, its end is reached.
            .when(off < -(inset + 1.), |d| d.child(fade(true)))
            .when(max > 1. && -off < max - (inset + 1.), |d| d.child(fade(false)))
            .into_any_element(),
    )
}

/// A card: the person's words (only asked about: the passage) beside a
/// thin bar, ✕, and Reply. A click on its words goes to the passage; the
/// pointer on it shows the passage over it.
fn card(shell: &Shell, n: usize, reply: &Reply, editing: bool, cx: &mut Context<Shell>) -> AnyElement {
    use gpui_kit::base::ElementExt as _;
    let p = shell.palette;
    let picked = shell.composer.card == Some(n);
    let answered = reply.answered();
    let shown = if answered { reply.words.clone() } else { reply.quote.clone() };
    let quote = reply.quote.clone();
    let face = h_flex()
        .id(("reply-card-face", n))
        .flex_1()
        .min_w_0()
        .gap(px(8.))
        .items_stretch()
        .rounded(px(R_SM))
        .cursor_pointer()
        .child(div().flex_none().w(px(2.)).rounded_full().bg(Rgba { a: 0.45, ..p.muted }))
        // Every card as tall as its most words: the row doesn't move as cards come and go.
        .child(div().flex_1().min_w_0().h(px(CARD_LINE * CARD_LINES as f32)).text_size(px(SMALL)).line_height(px(CARD_LINE)).text_color(if answered { p.text } else { p.muted }).line_clamp(CARD_LINES).text_ellipsis().child(shown))
        .on_hover(cx.listener(move |shell, hovered: &bool, _, cx| {
            let peek = if *hovered { Some(n) } else { shell.composer.peek.filter(|at| *at != n) };
            if peek != shell.composer.peek {
                shell.composer.peek = peek;
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |shell, _, _, cx| shell.quote_jump(usize::MAX, &quote, cx)));
    let remove = crate::ui::x_box(("reply-card-remove", n), p.muted, t!("quote.card_remove"))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |shell, _, _, cx| shell.remove_card(n, cx)));
    let edit = h_flex()
        .id(("reply-card-edit", n))
        .role(Role::Button)
        .h(px(22.))
        .px(px(6.))
        .ml(px(-6.))
        .gap(px(5.))
        .items_center()
        .rounded(px(R_SM))
        .text_size(px(META))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(icon(Icon::Reply, 12., p.text2))
        .child(t!("quote.card_reply"))
        .on_click(cx.listener(move |shell, _, window, cx| shell.quote_edit(n, window, cx)));
    let rects = shell.composer.card_rects.clone();
    v_flex()
        .id(("reply-card", n))
        .role(Role::ListItem)
        .aria_label(if answered { format!("{} — {}", reply.words, reply.quote) } else { reply.quote.clone() })
        .relative()
        .flex_none()
        .w(px(CARD_W))
        .pl(px(10.))
        .pr(px(6.))
        .pt(px(8.))
        .pb(px(4.))
        .gap(px(4.))
        .rounded(px(R_LG))
        .bg(p.bg)
        .border_1()
        .border_color(p.hair)
        .on_prepaint(move |bounds, _, _| {
            if let Some(at) = rects.borrow_mut().get_mut(n) {
                *at = bounds;
            }
        })
        .child(h_flex().w_full().gap(px(4.)).items_start().child(face).child(remove))
        .child(h_flex().mt_auto().child(edit))
        // The keys on it, or its box open: a second point of the accent inside.
        .when(picked || editing, |d| d.child(super::visuals::ring(p, R_LG - 1., 1.)))
        .into_any_element()
}

/// The passage of the card under the pointer, over it.
pub fn peek(shell: &Shell) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let n = shell.composer.peek?;
    let editing = shell.quote.writing.as_ref().and_then(|w| w.card);
    if editing == Some(n) {
        return None;
    }
    let quote = &shell.composer.replies.get(n)?.quote;
    let at = card_rect(shell, n)?;
    let p = shell.palette;
    let card = v_flex()
        .w(px(320.))
        .p(px(10.))
        .gap(px(6.))
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow(motion::float_shadow(0.55))
        .child(passage(p, quote, 6))
        .child(div().text_size(px(META)).text_color(p.muted).child(t!("quote.card_peek")));
    Some(deferred(anchored().anchor(Anchor::BottomLeft).position(point(px(at.x), px(at.y - 6.))).snap_to_window_with_margin(px(rules::EDGE)).child(motion::fade(card, ("quote-peek", n), motion::FAST))).with_priority(2).into_any_element())
}

/// A card in a message of yours: the passage folded — a quiet bar at its
/// left, its first lines muted; a click goes back to the words — and what
/// was said to it under it.
pub fn in_bubble(p: Palette, key: u64, ix: usize, reply: &Reply, handle: WeakEntity<Shell>) -> AnyElement {
    let quote = (!reply.quote.is_empty()).then(|| {
        let q = reply.quote.clone();
        let row = passage(p, &reply.quote, 3).id(("user-quote", key)).rounded(px(R_SM)).cursor_pointer().hover(move |s| s.opacity(0.8)).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.quote_jump(ix, &q, cx)).ok();
        });
        tip(row, p, t!("quote.jump"), None)
    });
    let words = reply.answered().then(|| div().text_size(px(BODY)).line_height(px(BODY_LINE)).text_color(p.prose).child(SelectableText::new(("user-reply", key), reply.words.clone())));
    v_flex().gap(px(4.)).children(quote).children(words).into_any_element()
}
