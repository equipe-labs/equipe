//! Asking about words in the conversation, as Claude's apps do: select a
//! stretch of a reply (a call's output, a message of your own) and, the
//! moment the button lets go, a small bar floats over the selection —
//! "Ask about this" and "Copy", each acting on the press. Asking (a press, ⏎, ⌘⇧R) quotes the words
//! into the composer, over its box, and puts the keys there: the person
//! writes the question as any message — their draft and attachments kept,
//! several quotes at once — and it goes with the quotes first, a Markdown
//! blockquote each.
//!
//! The person's message shows the quote above their words (`in_bubble`); a
//! click on it goes back to the words quoted and flashes them, if they are
//! still there. The rules — trimming, the message and reading it back,
//! finding the words, where things float — are `crate::quote`'s.

use std::cell::Cell;
use std::time::Duration;

use equipe_core::protocol::SessionId;
use equipe_core::t;
use gpui_kit::base::TextSelection;
use gpui_kit::base::text::RangeHighlight;
use gpui_kit::component::*;
use gpui_kit::*;

use super::decide::Took;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::quote::{self as rules, Rect};
use crate::theme::{Palette, R_MD, R_SM, SMALL};
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
const BAR: (f32, f32) = (236., 30.);

#[derive(Default)]
pub struct QuoteUi {
    /// The words selected in the conversation, offered to quote.
    pub offer: Option<Offer>,
    /// The row whose words are lit (a flash), to unlight.
    lit: Option<usize>,
    /// A press began in the conversation: its release, wherever it lands
    /// (on the composer, a card, outside the window), offers what it selected.
    pressed: bool,
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
    /// still being dragged out, to be asked again.
    fn quote_offer(&mut self, released: Rect, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if TextSelection::is_selecting(window, cx) {
            return false;
        }
        let Some(session) = self.model.viewed().filter(|_| self.screen == Screen::Chat) else { return true };
        let Some(text) = rules::trim(&TextSelection::selected_text(window, cx)) else { return true };
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

    /// "Ask about this" (or ⏎, ⌘⇧R): the words quoted into the composer,
    /// over its box, and the keys there — the person's draft as it was.
    pub fn quote_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.quote.offer.take() else { return };
        if !self.composer.quotes.contains(&offer.text) {
            self.composer.quotes.push(offer.text);
        }
        TextSelection::clear(window, cx);
        self.composer.focus(window, cx);
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

    /// A keystroke, before anyone else sees it: with words offered, ⏎ or
    /// ⌘⇧R quotes them into the composer and Esc puts the bar away.
    pub fn on_quote_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
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

    /// Dev tool (`snapshot.rs`): a click on the quote of the latest message of yours that has one.
    #[cfg(feature = "snapshot")]
    pub fn quote_jump_latest(&mut self, cx: &mut Context<Self>) {
        let found = self.chat.blocks.iter().enumerate().rev().find_map(|(ix, b)| match b {
            crate::convo::Block::User(text) => rules::split_all(&crate::sent::parts(text).text).0.into_iter().next().map(|q| (ix, q)),
            _ => None,
        });
        if let Some((ix, quote)) = found {
            self.quote_jump(ix, &quote, cx);
        }
    }
}

/// What floats over the conversation: the bar over a selection.
pub fn layer(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
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
        .child(ask)
        .child(div().w(px(1.)).h(px(14.)).bg(p.hair))
        .child(copy)
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
