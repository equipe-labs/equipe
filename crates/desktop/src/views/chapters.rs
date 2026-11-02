//! The rail at a conversation's left (`crate::chapters`): a short bar for
//! each thing the person asked, the one being read longer and brighter.
//! The pointer over it opens the list of them, in the margin it sits in; a
//! click on one brings its message to the top. It sits in the rows' left
//! padding, so it covers no words however narrow the window.

use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::chapters::{self, Chapter};
use crate::motion;
use crate::theme::SMALL;
use crate::tips::tip_at;

/// A bar's length: the one being read, and the rest.
const LONG: f32 = 18.;
const SHORT: f32 = 10.;

pub fn rail(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let all = chapters::of(&shell.chat.blocks);
    if all.len() < 2 {
        return None;
    }
    let p = shell.palette;
    let top = shell.chat.top_row(cx);
    let current = chapters::current(&all, top);
    let open = shell.chat.rail_open;
    let bars = chapters::barred(all.len(), current).map(|i| {
        let here = Some(i) == current;
        div().h(px(2.)).w(px(if here { LONG } else { SHORT })).rounded_full().bg(if here { p.text } else { p.muted.opacity(0.6) })
    });
    let list = open.then(|| {
        let rows = all.iter().enumerate().map(|(i, chapter)| entry(shell, i, chapter, Some(i) == current, cx));
        motion::fade(
            v_flex()
                .id("chapter-list")
                .absolute()
                .top(px(-8.))
                .left_0()
                .w(px(300.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .p_1p5()
                .gap_0p5()
                .rounded(px(12.))
                .bg(p.popover())
                .border_1()
                .border_color(p.hair)
                .shadow(motion::float_shadow(0.8))
                .occlude()
                .role(Role::List)
                .aria_label(t!("chat.chapters"))
                .children(rows),
            "chapter-list-in",
            motion::FAST,
        )
    });
    Some(
        div()
            .id("chapters")
            .absolute()
            .top(px(24.))
            .left(px(8.))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.chat.rail_open != *hovered {
                    this.chat.rail_open = *hovered;
                    cx.notify();
                }
            }))
            .child(v_flex().id("chapter-bars").py_1().gap(px(9.)).cursor_pointer().role(Role::Button).aria_label(t!("chat.chapters")).aria_expanded(open).children(bars))
            .children(list)
            .into_any_element(),
    )
}

/// A chapter in the list: its dash and title; how the answer began on hover.
fn entry(shell: &Shell, i: usize, chapter: &Chapter, here: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let row = chapter.row;
    let item = h_flex()
        .id(("chapter", i))
        .h(px(30.))
        .px_2()
        .gap(px(10.))
        .items_center()
        .rounded(px(8.))
        .cursor_pointer()
        .when(here, |r| r.bg(p.wash))
        .hover(move |s| s.bg(p.popover_hover()))
        .role(Role::ListItem)
        .aria_label(chapter.title.clone())
        .aria_selected(here)
        .child(div().flex_none().h(px(2.)).w(px(if here { 14. } else { 9. })).rounded_full().bg(if here { p.text } else { p.muted }))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(if here { p.text } else { p.text2 }).child(chapter.title.clone()))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.chat.rail_open = false;
            this.chat.scroll_to(row, cx);
            cx.notify();
        }));
    match &chapter.more {
        Some(more) => tip_at(item, p, more.clone(), None, gpui_kit::base::Placement::Right).into_any_element(),
        None => item.into_any_element(),
    }
}
