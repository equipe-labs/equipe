//! The two things a command can put over the window: a sheet of rows (usage,
//! help, changes, memory) and a question asked before something hard to take
//! back. Both are drawn from plain data (`sheet.rs`); Esc closes them, ⏎
//! answers a question.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::connect::{button, modal_sized};
use crate::sheet::{Button, Row};
use crate::theme::{BODY, MONO, SMALL};
use crate::Shell;

pub fn question(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ask = shell.confirm.as_ref()?;
    Some(modal_sized(
        p,
        480.,
        v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).child(ask.title.clone()))
            .children(ask.body.iter().filter(|l| !l.is_empty()).map(|l| div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(l.clone())))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(button(p, "ask-cancel", "취소", false).on_click(cx.listener(|this, _, window, cx| this.answer_question(false, window, cx))))
                    .child(button(p, "ask-yes", ask.yes.clone(), true).on_click(cx.listener(|this, _, window, cx| this.answer_question(true, window, cx)))),
            ),
    ))
}

pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let sheet = shell.sheet.as_ref()?;
    let key_width = if sheet.rows.iter().any(|r| r.head.starts_with('/')) { 300. } else { 120. };
    let rows: Vec<AnyElement> = sheet.rows.iter().enumerate().map(|(ix, r)| row(shell, ix, r, key_width, cx)).collect();
    let buttons = sheet.buttons.iter().cloned().enumerate().map(|(ix, b)| match b {
        Button::Copy(label, text) => button(p, "sheet-button", label, false).id(("sheet-button", ix)).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))),
        Button::Run(label, line) => button(p, "sheet-button", label, false).id(("sheet-button", ix)).on_click(cx.listener(move |this, _, window, cx| this.run_line(&line, window, cx))),
        Button::Open(label, path) => button(p, "sheet-button", label, false).id(("sheet-button", ix)).on_click(move |_, _, _| drop(std::process::Command::new("open").arg(&path).spawn())),
    });
    Some(modal_sized(
        p,
        680.,
        v_flex()
            .gap_3()
            .min_h_0()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(sheet.title.clone()))
            .children(sheet.lead.clone().map(|l| div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(l)))
            .child(v_flex().id("sheet-rows").flex_1().min_h_0().max_h(px(420.)).overflow_y_scroll().gap_1().children(rows))
            .children(sheet.foot.clone().map(|l| div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(l)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .children(buttons)
                    .child(button(p, "sheet-close", "닫기", true).on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx)))),
            ),
    ))
}

fn row(shell: &Shell, ix: usize, r: &Row, key_width: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    if r.heading {
        return div().pt_2().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(r.text.clone()).into_any_element();
    }
    let undo = r.undo;
    let squad = shell.model.squad.clone();
    h_flex()
        .gap_3()
        .items_start()
        .text_size(px(SMALL))
        .line_height(px(20.))
        .child(div().w(px(key_width)).flex_none().font_weight(FontWeight::SEMIBOLD).text_color(p.text).when(r.head.starts_with('/'), |d| d.font_family(MONO)).child(r.head.clone()))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_color(p.text2).child(r.text.clone()))
                .children(r.bar.map(|pct| {
                    div().h(px(4.)).w_full().rounded(px(2.)).bg(p.wash).child(div().h_full().w(relative(f32::from(pct) / 100.)).rounded(px(2.)).bg(p.accent))
                })),
        )
        .children(undo.map(|n| {
            button(p, "undo", "되돌리기", false).id(("undo", ix)).on_click(cx.listener(move |this, _, window, cx| this.ask_undo(squad.clone(), n, window, cx)))
        }))
        .into_any_element()
}
