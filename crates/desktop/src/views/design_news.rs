//! A design to look at, wherever the person is likely to be: a design a
//! designer handed in or the PM accepted (`crate::designs::news`) is a light
//! line in Needs you (Review) — "New design: <title>" — and a card on the
//! Status screen, each opening its card on the Designs tab; looking at it,
//! from either, or from the tab, ends its being new.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::render;
use crate::theme::{META, Palette, R_LG, R_MD, SMALL};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::visuals::{Ground, Size};
use crate::Shell;

use super::visuals::Shots;

const THUMB_W: f32 = 120.;
const THUMB_H: f32 = 76.;

pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("design-news-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Eye, 13., p.accent)), p, t!("design.results_new"), None).into_any_element()
}

/// What a design to look at says of itself: "New design: <title>", and how it stands.
pub fn say(n: &crate::designs::News) -> (String, &'static str) {
    (t!("design.news_title", title = n.title.clone()), if n.accepted { t!("design.news_accepted") } else { t!("design.news_handed_in") })
}

/// Needs you's row for design `id`: its words, and how it stands.
pub fn row_text(shell: &Shell, id: u32) -> Option<(String, String)> {
    let news = shell.model.new_designs().find(|n| n.ticket == id)?;
    let (title, said) = say(news);
    Some((title, said.to_owned()))
}

/// The peek of the row: its picture and the way to the card.
pub fn peek(shell: &Shell, id: u32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let news = shell.model.new_designs().find(|n| n.ticket == id).cloned();
    v_flex()
        .id("peek")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("design.results_new")))
        .children(news.map(|n| {
            let (title, said) = say(&n);
            v_flex()
                .gap_2()
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
                .child(div().text_size(px(SMALL)).text_color(p.text2).child(said))
                .child(ui::button(p, "design-news-open", Kind::Primary, t!("design.news_open")).icon(Icon::Eye).on_click(cx.listener(move |this, _, _, cx| this.designs_show(id, cx))))
        }))
        .into_any_element()
}

/// The Status screen's card: each design to look at — its picture, its words, a button.
pub fn status_card(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let news: Vec<crate::designs::News> = shell.model.new_designs().cloned().collect();
    if news.is_empty() {
        return None;
    }
    let p = shell.palette;
    let handle = cx.entity().downgrade();
    let shots = Shots::of(shell, handle, cx);
    let rows: Vec<AnyElement> = news
        .iter()
        .map(|n| {
            let id = n.ticket;
            let (title, said) = say(n);
            let thumb = n.hero.as_ref().map(|path| {
                let look = shots.ask(path, Size::Thumb, cx);
                super::visuals::picture_on(&shots, render::hash_of(("design-news", id)), path, look, (THUMB_W, THUMB_H), R_MD, false, 0., Ground::Surface).border_1().border_color(p.hair)
            });
            h_flex()
                .id(SharedString::from(format!("status-design-{id}")))
                .w_full()
                .gap_3()
                .px_3()
                .py_2()
                .items_center()
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .children(thumb)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(div().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(title))
                        .child(div().min_w_0().truncate().text_size(px(META + 0.5)).text_color(p.muted).child(format!("T-{id} · {said}"))),
                )
                .child(ui::button(p, SharedString::from(format!("status-design-open-{id}")), Kind::Secondary, t!("design.news_open")).icon(Icon::Eye).on_click(cx.listener(move |this, _, _, cx| this.designs_show(id, cx))))
                .on_click(cx.listener(move |this, _, _, cx| this.designs_show(id, cx)))
                .into_any_element()
        })
        .collect();
    Some(super::status::card(p, "status-designs", t!("design.news_status_heading").to_owned(), None).children(rows).rounded(px(R_LG)).into_any_element())
}
