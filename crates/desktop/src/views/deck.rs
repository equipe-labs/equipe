//! A deck a member made, shown as a presentation over the window: one slide
//! at a time, big, on a dark stage — ←/→ or a click go through it, Esc puts it
//! away (the document stays open beside the conversation). `deck.rs` reads it.

use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::Shell;
use crate::deck::Part;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::render::view::Look;
use crate::theme::{CONTROL, ICON, MONO};
use crate::tips::tip;

/// A slide's shape: a screen's, as decks are made for.
const ASPECT: f32 = 16. / 9.;
/// Room kept around the slide, and under it for the bar.
const MARGIN: f32 = 32.;
const BAR: f32 = 64.;

/// The slide's size in a window of `viewport`: as big as fits, 16:9.
pub fn slide_size(viewport: (f32, f32)) -> (f32, f32) {
    let room = ((viewport.0 - MARGIN * 2.).max(160.), (viewport.1 - MARGIN - BAR).max(90.));
    let w = room.0.min(room.1 * ASPECT);
    (w, w / ASPECT)
}

/// The words' size on a slide `width` wide: read from across a room, but
/// never past what a short slide wants.
pub fn words_size(width: f32) -> f32 {
    (width / 36.).clamp(14., 36.)
}

pub fn presentation(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let doc = shell.doc.as_ref()?;
    let at = doc.presenting?;
    let slides = doc.slides()?;
    let n = slides.len();
    let at = at.min(n.checked_sub(1)?);
    let p = shell.palette;
    let viewport = window.viewport_size();
    let (w, h) = slide_size((viewport.width.into(), viewport.height.into()));
    let size = words_size(w);
    let look = Look { size, line: size * 1.45, color: p.prose, live: false };
    let slide = div()
        .id(("deck-slide", at))
        .w(px(w))
        .h(px(h))
        .flex_none()
        .relative()
        .rounded(px(14.))
        .overflow_hidden()
        .bg(p.bg)
        .border_1()
        .border_color(p.hair)
        .cursor_pointer()
        .role(Role::Document)
        .aria_label(t!("deck.slide_of", at = at + 1, of = n))
        // A click on its left third goes back, anywhere else on.
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            let left = f32::from(e.position().x) < f32::from(window.viewport_size().width) / 3.;
            this.step_slide(if left { -1 } else { 1 }, cx);
        }))
        .child(
            div()
                .id("deck-words")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(size * 0.8))
                .px(px(w * 0.07))
                .py(px(h * 0.08))
                .children(crate::deck::parts(&slides[at]).into_iter().enumerate().map(|(i, part)| {
                    let key = crate::render::hash_of((&doc.path, doc.read, at, i));
                    match part {
                        Part::Words(words) => div().flex_none().child(crate::render::view::document(p, ("deck", key), &words, look)).into_any_element(),
                        // As big as the slide leaves room for, side by side.
                        Part::Pictures(sources) => h_flex()
                            .flex_1()
                            .min_h(px(h * 0.25))
                            .gap(px(size))
                            .justify_center()
                            .children(sources.iter().map(|src| {
                                let at = crate::deck::picture_at(&doc.path, src);
                                let source = if at.contains("://") { ImageSource::Resource(Resource::Uri(at.into())) } else { ImageSource::from(std::path::PathBuf::from(at)) };
                                div().flex_1().h_full().min_w_0().flex().items_center().justify_center().child(img(source).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(10.)))
                            }))
                            .into_any_element(),
                        Part::Columns(columns) => h_flex()
                            .flex_none()
                            .gap(px(size * 2.))
                            .justify_center()
                            .children(columns.into_iter().map(|lines| {
                                let mut lines = lines.into_iter();
                                v_flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(size * 0.3))
                                    .children(lines.next().map(|first| div().text_size(px(size * 2.6)).line_height(px(size * 2.9)).font_weight(FontWeight::EXTRA_BOLD).text_color(p.text).child(first)))
                                    .children(lines.map(|line| div().text_size(px(size * 0.8)).text_color(p.muted).text_center().child(line)))
                            }))
                            .into_any_element(),
                    }
                })),
        )
        // How far through: a hairline of the accent along the bottom.
        .child(div().absolute().left_0().bottom_0().h(px(3.)).w(px(w * (at + 1) as f32 / n as f32)).bg(p.accent));
    let round = |b: Stateful<Div>, glyph: Icon, label: SharedString, keys: &'static str| {
        tip(
            b.size(px(CONTROL)).flex().items_center().justify_center().rounded_full().cursor_pointer().hover(|s| s.bg(rgba(0xFFFFFF1F))).role(Role::Button).child(icon(glyph, ICON, rgb(0xFFFFFF))),
            p,
            label,
            Some(keys),
        )
    };
    let bar = h_flex()
        .id("deck-bar")
        .flex_none()
        .h(px(44.))
        .px_2()
        .gap_3()
        .items_center()
        .rounded(px(22.))
        .bg(rgba(0x2A2A2EF2))
        .border_1()
        .border_color(rgba(0xFFFFFF1F))
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(round(div().id("deck-prev").on_click(cx.listener(|this, _, _, cx| this.step_slide(-1, cx))), Icon::ArrowLeft, t!("deck.prev").into(), "←"))
        .child(div().min_w(px(56.)).flex().justify_center().font_family(MONO).text_size(px(12.5)).text_color(rgba(0xFFFFFFCC)).child(format!("{} / {n}", at + 1)))
        .child(round(div().id("deck-next").on_click(cx.listener(|this, _, _, cx| this.step_slide(1, cx))), Icon::ArrowRight, t!("deck.next").into(), "→"))
        .child(div().w(px(1.)).h(px(18.)).bg(rgba(0xFFFFFF26)))
        .child(round(div().id("deck-as-doc").on_click(cx.listener(|this, _, _, cx| this.stop_presenting(cx))), Icon::FileText, t!("deck.as_doc").into(), "Esc"))
        .child(round(div().id("deck-close").on_click(cx.listener(|this, _, _, cx| this.close_deck(cx))), Icon::X, t!("deck.close").into(), "⌘W"));
    let stage = div()
        .id("deck")
        .absolute()
        .inset_0()
        .bg(rgb(0x0B0B0C))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .occlude()
        .role(Role::Group)
        .aria_label(t!("deck.title", name = doc.name()))
        .child(motion::rise(slide, ElementId::Name(format!("deck-{at}").into())))
        .child(bar);
    Some(motion::fade(stage, "deck-in", motion::BASE).into_any_element())
}

impl Shell {
    /// Show the open deck as a presentation, from slide `at`.
    pub fn present(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_mut() {
            doc.presenting = doc.slide_at(at);
            cx.notify();
        }
    }

    /// `by` slides on (back, when negative), kept to the deck.
    pub fn step_slide(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(at) = self.doc.as_ref().and_then(|d| d.presenting) else { return };
        self.present(at.saturating_add_signed(by), cx);
    }

    /// Back to the document, read beside the conversation.
    pub fn stop_presenting(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_mut() {
            doc.presenting = None;
            cx.notify();
        }
    }

    /// Put the deck away altogether: presentation and panel.
    pub fn close_deck(&mut self, cx: &mut Context<Self>) {
        self.close_doc(cx);
    }

    /// The keys of a deck on show; `true` when one was its.
    pub fn on_deck_key(&mut self, key: &str, m: &Modifiers, cx: &mut Context<Self>) -> bool {
        if self.doc.as_ref().and_then(|d| d.presenting).is_none() {
            return false;
        }
        if m.secondary() && key == "w" {
            self.close_deck(cx);
            return true;
        }
        if m.platform || m.control || m.alt {
            return false;
        }
        match key {
            "right" | "down" | "space" | "pagedown" | "enter" => self.step_slide(1, cx),
            "left" | "up" | "pageup" | "backspace" => self.step_slide(-1, cx),
            "home" => self.present(0, cx),
            "end" => self.present(usize::MAX, cx),
            "escape" => self.stop_presenting(cx),
            // Nothing typed into the composer underneath.
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ASPECT, BAR, MARGIN, slide_size, words_size};

    #[test]
    fn a_slide_is_as_big_as_the_window_allows_at_sixteen_by_nine() {
        let (w, h) = slide_size((1440., 900.));
        assert!((w / h - ASPECT).abs() < 0.001);
        assert!(h <= 900. - MARGIN - BAR && w <= 1440. - MARGIN * 2.);
        // A wide window: the height decides.
        let (w, h) = slide_size((3000., 900.));
        assert_eq!(h, 900. - MARGIN - BAR);
        assert!(w < 3000.);
    }

    #[test]
    fn words_on_a_slide_read_from_across_a_room_within_reason() {
        assert_eq!(words_size(1080.), 30.);
        assert_eq!(words_size(200.), 14.);
        assert_eq!(words_size(4000.), 36.);
    }
}
