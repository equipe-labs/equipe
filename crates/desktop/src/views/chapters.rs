//! The rail at a conversation's left (`crate::chapters`): a short bar for
//! each thing the person asked, the one being read longer and brighter.
//! The pointer over it opens the list of them, in the margin it sits in; a
//! click on one brings its message to the top. It sits in the rows' left
//! padding, which widens while it shows, so it covers no words and keeps
//! clear of them however narrow the window.

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
/// Where the bars start, from the conversation's left edge.
const LEFT: f32 = 8.;
/// The air between the longest bar and the words.
const CLEAR: f32 = 16.;
/// The rows' padding on each side while the rail shows: past the longest
/// bar, with air before the words (on both sides, so the column stays centred).
pub const GUTTER: f32 = LEFT + LONG + CLEAR;
// The longest bar ends well before the words.
const _: () = assert!(GUTTER - (LEFT + LONG) >= 12.);

/// Whether the rail shows: two things asked or more.
pub fn shows(chapters: &[Chapter]) -> bool {
    chapters.len() >= 2
}

pub fn rail(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let all = shell.chat.chapters();
    if !shows(all) {
        return None;
    }
    let p = shell.palette;
    let top = shell.chat.top_row(cx);
    let current = chapters::current(all, top);
    let open = shell.chat.rail_open;
    let bars = chapters::barred(all.len(), current).map(|i| {
        let here = Some(i) == current;
        div().h(px(2.)).w(px(if here { LONG } else { SHORT })).rounded_full().bg(if here { p.text } else { p.muted.opacity(0.6) })
    });
    let list = open.then(|| {
        let rows: Vec<AnyElement> = all.iter().enumerate().map(|(i, chapter)| entry(shell, i, chapter, Some(i) == current, cx)).collect();
        // Drawn over everything (never cut by the conversation's edge); it
        // covers the bars, so it — not they — says when the pointer left.
        deferred(motion::fade(
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
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.chat.rail_open {
                        this.chat.rail_open = false;
                        cx.notify();
                    }
                }))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.chat.rail_open = false;
                    cx.notify();
                }))
                .children(rows),
            "chapter-list-in",
            motion::FAST,
        ))
        .with_priority(1)
    });
    Some(
        div()
            .id("chapters")
            .absolute()
            .top(px(24.))
            .left(px(LEFT))
            .child(v_flex().id("chapter-bars")
                // The pointer on the bars opens the list; the list, once open, closes itself.
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered && !this.chat.rail_open {
                        this.chat.rail_open = true;
                        cx.notify();
                    }
                })).py_1().gap(px(9.)).cursor_pointer().role(Role::Button).aria_label(t!("chat.chapters")).aria_expanded(open).children(bars))
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
