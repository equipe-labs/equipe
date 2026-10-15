//! The header (52 tall, T-54 §3.1): who the conversation is with, the project
//! chips that filter the squad (and pick the PM's thread), the context fill, and
//! the two tabs, conversation and board. Still text; nothing here moves.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::{Member, Presence};
use crate::theme::{PROJECT_HUES, Palette, SANS, SMALL, UI, look};
use crate::{Screen, Shell};

pub const HEIGHT: f32 = 52.;
/// Past this many projects the rest fold into a `+N` chip.
pub const CHIPS_SHOWN: usize = 8;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let member = model.selected_member();
    let projects = model.project_names();

    let who = member.map(|m| title(p, m));
    let context = member.and_then(|m| m.context).map(|c| {
        div()
            .text_size(px(SMALL))
            .text_color(if c >= 50 { p.text2 } else { p.muted })
            .when(c >= 50, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(t!("header.context", c = c))
    });

    let mut chips = Vec::new();
    if !projects.is_empty() {
        chips.push(chip(p, "all", t!("header.all").into(), Some("⌘0"), None, model.focus.is_none(), cx.listener(|this, _, window, cx| this.focus_project(None, window, cx))));
        for (ix, name) in projects.iter().take(CHIPS_SHOWN).enumerate() {
            let target = name.clone();
            chips.push(chip(
                p,
                name,
                name.clone(),
                (ix < 9).then(|| ["⌘1", "⌘2", "⌘3", "⌘4", "⌘5", "⌘6", "⌘7", "⌘8", "⌘9"][ix]),
                Some(p.member(PROJECT_HUES[ix % PROJECT_HUES.len()])),
                model.focus.as_deref() == Some(name.as_str()),
                cx.listener(move |this, _, window, cx| this.focus_project(Some(target.clone()), window, cx)),
            ));
        }
        if projects.len() > CHIPS_SHOWN {
            // The rest are reached with `/project <name>`; the chip says how many.
            chips.push(
                div()
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .text_size(px(SMALL))
                    .text_color(p.muted)
                    .child(format!("+{} · /project", projects.len() - CHIPS_SHOWN))
                    .into_any_element(),
            );
        }
    }

    h_flex()
        .h(px(HEIGHT))
        .flex_none()
        .px_5()
        .gap_4()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .font_family(SANS)
        .children(who)
        .child(h_flex().flex_1().min_w_0().gap_1().overflow_hidden().children(chips))
        .children(context)
        .child(tabs(p, shell, cx))
}

/// `◆ pm`, in the member's colour; a member who left is dimmed.
fn title(p: Palette, member: &Member) -> impl IntoElement + use<> {
    let (mark, hue) = look(&member.role, &member.name);
    let color = if member.presence == Presence::Closed { p.muted } else { p.member(hue) };
    h_flex()
        .gap_2()
        .items_center()
        .flex_none()
        .text_color(color)
        .child(div().text_size(px(14.)).child(mark))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(member.name.clone()))
}

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

fn chip(
    p: Palette,
    id: &str,
    label: String,
    shortcut: Option<&'static str>,
    hue: Option<Rgba>,
    on: bool,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let click: Click = Box::new(click);
    h_flex()
        .id(SharedString::from(format!("chip-{id}")))
        .h(px(28.))
        .px_3()
        .gap_2()
        .flex_none()
        .items_center()
        .rounded(px(14.))
        .text_size(px(SMALL))
        .cursor_pointer()
        .when(on, |c| c.bg(p.wash).font_weight(FontWeight::SEMIBOLD).text_color(p.text))
        .when(!on, |c| c.text_color(p.text2).hover(move |s| s.bg(p.wash)))
        .on_click(move |e, w, cx| click(e, w, cx))
        .when_some(hue, |c, hue| c.child(div().text_size(px(9.)).text_color(hue).child("●")))
        .child(label)
        .when_some(shortcut, |c, key| c.child(div().text_size(px(12.)).text_color(p.muted).child(key)))
        .into_any_element()
}

fn tabs(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let tab = |id: &'static str, label: &'static str, screen: Screen| {
        let on = shell.screen == screen;
        div()
            .id(id)
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(UI))
            .cursor_pointer()
            .when(on, |t| t.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |t| t.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .on_click(cx.listener(move |this, _, _, cx| this.show(screen, cx)))
            .child(label)
    };
    let browser_on = shell.browser.read(cx).open;
    let browser = div()
        .id("tab-browser")
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(UI))
        .cursor_pointer()
        .when(browser_on, |t| t.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
        .when(!browser_on, |t| t.text_color(p.text2).hover(move |s| s.bg(p.wash)))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_browser(window, cx)))
        .child(t!("header.browser"));
    h_flex()
        .flex_none()
        .gap_1()
        .child(tab("tab-chat", t!("header.tab_chat"), Screen::Chat))
        .child(tab("tab-board", t!("header.tab_board"), Screen::Board))
        .child(browser)
}
