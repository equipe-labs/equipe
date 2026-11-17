//! A reference gallery in the conversation (`crate::refcards`): the member's
//! components, screens and patterns side by side in a row that scrolls
//! sideways — each a thumbnail that opens big, its title, a chip of where it
//! is from that opens the page, why it fits and what to take and leave, and
//! the person's reactions: a heart, "not this" with its reasons, kept among
//! the project's references. A picture that can't be had says so with a way
//! to ask again, never an endless placeholder.

use equipe_core::protocol::design::PickReason;
use equipe_core::protocol::references::{RefCard, RefKind};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::visuals::{Shots, picture};
use crate::Shell;
use crate::icons::{Icon, icon};
use crate::refcards::{Gallery, Reaction, Reactions};
use crate::render::{Tones, hash_of};
use crate::theme::{MEASURE, META, Palette, R_LG, R_MD, R_SM, SMALL};
use crate::tips::tip;
use crate::visuals::{Item, Look, Owner, Size, Viewer};

/// A card's width, and the narrowest it gets in a narrow column.
const TILE: f32 = 232.;
const TILE_NARROW: f32 = 188.;
/// The room the card's own padding and border take of the column.
const INSET: f32 = 30.;

/// What a gallery is drawn with.
pub struct Draw<'a> {
    pub p: Palette,
    pub tones: Tones,
    pub shots: &'a Shots,
    pub handle: WeakEntity<Shell>,
    pub reactions: &'a Reactions,
    /// The row's key in the conversation.
    pub key: u64,
    /// The project the row is about: where a kept card goes.
    pub project: Option<String>,
}

/// How wide each card is in a column `column` wide: a little narrower than
/// the room, so the next one peeks in and says the row scrolls.
pub fn tile_width(column: f32) -> f32 {
    let room = (column - INSET).max(TILE_NARROW);
    if room >= TILE * 2. + 12. { TILE } else { (room * 0.78).clamp(TILE_NARROW, TILE) }
}

fn kind_words(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Component => t!("refcards.kind_component"),
        RefKind::Screen => t!("refcards.kind_screen"),
        RefKind::Pattern => t!("refcards.kind_pattern"),
    }
}

/// The big view over every card of the gallery, open at card `n`.
fn viewer(key: u64, g: &Gallery, n: usize) -> Viewer {
    let items = g
        .cards
        .iter()
        .map(|c| Item { path: c.picture.clone(), choice: None, label: c.title.clone(), tile: None, note: [c.site.as_str(), c.why.as_str()].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" · "), before: false })
        .collect();
    Viewer { owner: Owner::Reply(key), items, at: n.min(g.cards.len().saturating_sub(1)) }
}

/// The gallery's card.
pub fn row(d: Draw<'_>, g: &Gallery, cx: &mut App) -> AnyElement {
    let p = d.p;
    let sites = g.sites();
    let head = h_flex()
        .gap(px(8.))
        .items_center()
        .text_size(px(SMALL))
        .child(icon(Icon::Sparkles, 14., p.text2))
        .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("refcards.kicker")))
        .child(div().flex_none().text_color(p.muted).child(format!("· {}", g.name)))
        .child(div().flex_1())
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(META)).text_color(p.muted).child(t!("refcards.count", count = g.cards.len(), sites = sites.join(" · "))));
    let width = tile_width(d.shots.column());
    let tiles: Vec<AnyElement> = g.cards.iter().enumerate().map(|(n, card)| tile(&d, g, n, card, width, cx)).collect();
    let strip = h_flex().id(("refcards-strip", d.key)).w_full().gap(px(12.)).items_start().pb(px(6.)).overflow_x_scroll().children(tiles);
    let study = g.cards.iter().any(RefCard::study_only).then(|| {
        h_flex().gap(px(6.)).items_center().text_size(px(META)).text_color(p.muted).child(icon(Icon::Lock, 11., p.muted)).child(t!("refcards.study_note"))
    });
    v_flex()
        .w_full()
        .max_w(px(MEASURE))
        .px(px(14.))
        .py(px(12.))
        .gap(px(10.))
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .child(head)
        .child(strip)
        .children(study)
        .into_any_element()
}

/// One card: its picture, what it is, where from, and the person's reactions.
fn tile(d: &Draw<'_>, g: &Gallery, n: usize, card: &RefCard, w: f32, cx: &mut App) -> AnyElement {
    let (p, tones) = (d.p, d.tones);
    let id = d.key ^ hash_of((n, card.picture.as_str()));
    let h = (w * crate::visuals::RATIO).round();
    let reaction = d.reactions.of(&g.key, n);
    d.shots.in_row(&card.picture, d.key);
    let look = d.shots.ask(&card.picture, Size::Thumb, cx);
    let face: AnyElement = match look {
        Look::Gone(why) => gone(p, d.handle.clone(), id, &card.picture, super::visuals::why_words(why), w, h),
        look => {
            let handle = d.handle.clone();
            let open = viewer(d.key, g, n);
            div()
                .id(("refcard-pic", id))
                .relative()
                .rounded(px(R_MD))
                .border_1()
                .border_color(p.hair)
                .overflow_hidden()
                .cursor_pointer()
                .child(picture(d.shots, id, &card.picture, look, w - 2., h, R_MD, false, 0.))
                .when(card.study_only(), |c| c.child(study_badge()))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    let open = open.clone();
                    handle.update(cx, |shell, cx| shell.open_viewer(open, cx)).ok();
                })
                .into_any_element()
        }
    };
    let link = card.link.clone();
    let source = div()
        .id(("refcard-src", id))
        .flex_none()
        .h(px(22.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(11.))
        .border_1()
        .border_color(p.hair)
        .text_size(px(META))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(|s| s.bg(p.wash))
        .child(div().max_w(px(w * 0.55)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(card.site.clone()))
        .child(icon(Icon::External, 11., p.muted))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            cx.open_url(&link);
        });
    let source = tip(source, p, card.link.clone(), None);
    let line = |glyph: Icon, color: Rgba, text: &str| {
        (!text.is_empty()).then(|| {
            h_flex()
                .gap(px(6.))
                .items_start()
                .text_size(px(META))
                .line_height(px(17.))
                .child(div().pt(px(3.)).flex_none().child(icon(glyph, 11., color)))
                .child(div().flex_1().min_w_0().text_color(p.text2).line_clamp(2).text_ellipsis().child(text.to_owned()))
        })
    };
    v_flex()
        .flex_none()
        .w(px(w))
        .gap(px(7.))
        .child(face)
        .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).line_clamp(2).text_ellipsis().child(card.title.clone()))
        .child(h_flex().gap(px(6.)).items_center().child(source).child(div().text_size(px(META)).text_color(p.muted).child(kind_words(card.kind))))
        .children((!card.why.is_empty()).then(|| div().text_size(px(META)).line_height(px(17.)).text_color(p.muted).line_clamp(3).text_ellipsis().child(card.why.clone())))
        .children(line(Icon::Plus, tones.added, &card.take))
        .children(line(Icon::Minus, tones.danger, &card.leave))
        .child(actions(d, g, n, card, &reaction))
        .children(reaction.not.then(|| reasons(d, g, n, &reaction)))
        .into_any_element()
}

/// "Study only" over a picture that is private study material (Mobbin's).
fn study_badge() -> impl IntoElement {
    div()
        .absolute()
        .top(px(6.))
        .left(px(6.))
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(R_SM))
        .bg(rgba(0x000000B3))
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(0xFFFFFF))
        .child(icon(Icon::Lock, 9., rgb(0xFFFFFF)))
        .child(t!("refcards.study_only"))
}

/// A picture that couldn't be had: why, calmly, and a way to ask again.
fn gone(p: Palette, handle: WeakEntity<Shell>, id: u64, path: &str, why: &str, w: f32, h: f32) -> AnyElement {
    let path = path.to_owned();
    v_flex()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .gap(px(6.))
        .items_center()
        .justify_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .bg(p.wash)
        .text_size(px(META))
        .text_color(p.muted)
        .child(icon(Icon::File, 16., p.muted))
        .child(div().px(px(10.)).text_center().child(format!("{} · {why}", t!("refcards.not_loaded"))))
        .child(crate::ui::button(p, ("refcard-retry", id), crate::ui::Kind::Secondary, t!("refcards.retry")).icon(Icon::Retry).on_click(move |_, _, cx| {
            cx.stop_propagation();
            let path = path.clone();
            handle.update(cx, |shell, cx| shell.retry_shots(&[path], cx)).ok();
        }))
        .into_any_element()
}

/// A heart, "not this" and keep, in a row under the card.
fn actions(d: &Draw<'_>, g: &Gallery, n: usize, card: &RefCard, r: &Reaction) -> impl IntoElement {
    let p = d.p;
    let id = d.key ^ hash_of((n, "actions"));
    let on = |what: &'static str| {
        let (handle, key, card, project, row) = (d.handle.clone(), g.key.clone(), card.clone(), d.project.clone(), d.key);
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            let (key, card, project) = (key.clone(), card.clone(), project.clone());
            handle.update(cx, |shell, cx| shell.refcard(what, row, &key, n, &card, project, cx)).ok();
        }
    };
    let love = div()
        .id(("refcard-love", id))
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(13.))
        .border_1()
        .border_color(if r.love { d.tones.danger } else { p.hair })
        .when(r.love, |c| c.bg(Rgba { a: 0.12, ..d.tones.danger }))
        .cursor_pointer()
        .hover(|s| s.bg(p.wash))
        .child(icon(Icon::Heart, 13., if r.love { d.tones.danger } else { p.text2 }))
        .on_click(on("love"));
    let love = tip(love, p, if r.love { t!("refcards.loved") } else { t!("refcards.love") }, None);
    let chip = |name: &'static str, label: &'static str, glyph: Icon, lit: bool| {
        div()
            .id((name, id))
            .h(px(26.))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(13.))
            .border_1()
            .border_color(if lit { p.accent } else { p.hair })
            .when(lit, |c| c.bg(p.accent_bg))
            .text_size(px(META))
            .text_color(if lit { p.accent } else { p.text2 })
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .child(icon(glyph, 11., if lit { p.accent } else { p.muted }))
            .child(label)
    };
    let not = chip("refcard-not", t!("refcards.not_this"), Icon::X, r.not).on_click(on("not"));
    let keep = if r.kept {
        chip("refcard-kept", t!("refcards.kept"), Icon::Check, true).into_any_element()
    } else {
        tip(chip("refcard-keep", t!("refcards.keep"), Icon::Plus, false).on_click(on("keep")), p, if card.study_only() { t!("refcards.keep_tip_study") } else { t!("refcards.keep_tip") }, None).into_any_element()
    };
    // Live: a registry component's playground, else its page — in the browser panel.
    let live = !card.study_only() && !equipe_core::playground::registry_urls(card).is_empty();
    let try_it = {
        let (handle, card, project) = (d.handle.clone(), card.clone(), d.project.clone());
        let chip = chip("refcard-try", t!("refcards.try"), if live { Icon::Play } else { Icon::External }, false).on_click(move |_, window, cx| {
            cx.stop_propagation();
            let (card, project) = (card.clone(), project.clone());
            handle.update(cx, |shell, cx| shell.refcard_try(card, project, window, cx)).ok();
        });
        tip(chip, p, if live { t!("refcards.try_tip") } else { t!("refcards.try_tip_view") }, None)
    };
    h_flex().gap(px(6.)).items_center().flex_wrap().child(love).child(not).child(try_it).child(keep)
}

/// Why not, in a tap each, under "not this".
fn reasons(d: &Draw<'_>, g: &Gallery, n: usize, r: &Reaction) -> impl IntoElement {
    let p = d.p;
    h_flex().gap(px(5.)).flex_wrap().items_center().children(PickReason::ALL.into_iter().map(|reason| {
        let lit = r.reasons.contains(&reason);
        let (handle, key, card) = (d.handle.clone(), g.key.clone(), g.cards[n].clone());
        div()
            .id(("refcard-reason", d.key ^ hash_of((n, reason as usize))))
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(11.))
            .border_1()
            .border_color(if lit { p.accent } else { p.hair })
            .when(lit, |c| c.bg(p.accent_bg))
            .text_size(px(11.5))
            .text_color(if lit { p.accent } else { p.text2 })
            .cursor_pointer()
            .child(super::design_session::reason_words(reason))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let (key, card) = (key.clone(), card.clone());
                handle.update(cx, |shell, cx| shell.refcard_reason(&key, n, &card, reason, cx)).ok();
            })
    }))
}

impl Shell {
    /// A card's heart, "not this" or keep, clicked: kept, and the taste or the references told.
    #[allow(clippy::too_many_arguments)]
    pub fn refcard(&mut self, what: &str, row: u64, key: &str, n: usize, card: &RefCard, project: Option<String>, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        let session = self.model.viewed();
        let reactions = std::rc::Rc::make_mut(&mut self.refcards);
        let command = match what {
            "love" => reactions.love(&squad, key, n, card),
            "not" => reactions.not_this(&squad, key, n, card),
            "keep" => reactions.keep(&squad, session, project, key, n, card),
            _ => None,
        };
        if let Some(command) = command {
            self.send_command(command);
        }
        // "Not this" brings its reasons under the card: the row is measured again.
        self.chat.remeasure_row(row, cx);
        cx.notify();
    }

    /// "Try it": the browser panel opens on the card's playground (or its
    /// page), as a tab of the person's own the daemon is about to open.
    pub fn refcard_try(&mut self, card: RefCard, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browser.read(cx).open {
            self.browser.update(cx, |b, cx| b.toggle(window, cx));
            self.sync_browser(window, cx);
        }
        self.browser.update(cx, |b, cx| b.expect_new_tab(cx));
        self.send_command(equipe_core::protocol::Command::TryReference { squad: self.model.squad.clone(), project, card });
        cx.notify();
    }

    /// A reason why not, tapped.
    pub fn refcard_reason(&mut self, key: &str, n: usize, card: &RefCard, reason: PickReason, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        if let Some(command) = std::rc::Rc::make_mut(&mut self.refcards).reason(&squad, key, n, card, reason) {
            self.send_command(command);
        }
        cx.notify();
    }
}

/// Dev tool (`snapshot.rs`): `refcard:<love|not|keep|view>:<n>` or
/// `refcard:reason:<n>,<k>` on the nth card of the latest gallery in the
/// conversation (k: the kth reason, from 1).
#[cfg(feature = "snapshot")]
impl Shell {
    pub fn refcard_script(&mut self, arg: &str, cx: &mut Context<Self>) {
        let (what, rest) = arg.split_once(':').unwrap_or((arg, "1"));
        let mut nums = rest.split(',').filter_map(|x| x.trim().parse::<usize>().ok());
        let n = nums.next().unwrap_or(1).saturating_sub(1);
        let found = self.chat.blocks.iter().enumerate().rev().find_map(|(ix, b)| match b {
            crate::convo::Block::References(g) => Some((ix, (**g).clone())),
            _ => None,
        });
        let Some((ix, g)) = found else { return };
        let Some(card) = g.cards.get(n).cloned() else { return };
        let row = self.chat.serial << 24 | ix as u64;
        match what {
            "view" => self.open_viewer(viewer(row, &g, n), cx),
            "reason" => {
                if let Some(reason) = nums.next().and_then(|k| PickReason::ALL.get(k.saturating_sub(1)).copied()) {
                    self.refcard_reason(&g.key, n, &card, reason, cx);
                }
            }
            what => self.refcard(what, row, &g.key, n, &card, None, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{INSET, TILE, TILE_NARROW, tile_width};

    #[test]
    fn cards_fill_a_wide_column_and_the_next_peeks_in_a_narrow_one() {
        assert_eq!(tile_width(760.), TILE);
        let narrow = tile_width(360.);
        assert!((TILE_NARROW..TILE).contains(&narrow), "{narrow}");
        assert!(narrow < 360. - INSET, "the next card peeks in");
        assert_eq!(tile_width(120.), TILE_NARROW, "never narrower than this");
    }
}
