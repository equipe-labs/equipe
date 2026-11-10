//! The two things a command can put over the window: a sheet of rows (usage,
//! help, changes, memory) and a question asked before something hard to take
//! back. Both are drawn from plain data (`sheet.rs`); Esc closes them, ⏎
//! answers a question.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use equipe_core::t;
use gpui_kit::*;

use std::rc::Rc;

use super::connect::{button, modal_w};
use crate::sheet::{Button, Row};
use crate::theme::{BODY, MONO, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

pub fn question(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ask = shell.confirm.as_ref()?;
    Some(modal_w(
        p,
        "question",
        480.,
        v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).child(ask.title.clone()))
            .children(ask.body.iter().filter(|l| !l.is_empty()).map(|l| div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(l.clone())))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(button(p, "ask-cancel", t!("sheet.cancel"), false).on_click(cx.listener(|this, _, window, cx| this.answer_question(false, window, cx))))
                    // Taking something away: red, not the window's primary.
                    .child(
                        button(p, "ask-yes", ask.yes.clone(), true)
                            .when(ask.destructive(), |b| b.bg(p.red).text_color(p.on_accent))
                            .on_click(cx.listener(|this, _, window, cx| this.answer_question(true, window, cx))),
                    ),
            ),
    ))
}

/// What a sheet's button does, from a click or from a key.
type Act = Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;

pub fn sheet(shell: &Shell, window: &mut Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let sheet = shell.sheet.as_ref()?;
    let key_width = if sheet.rows.iter().any(|r| r.head.starts_with('/')) { 300. } else { 120. };
    // Every button the keys can reach, in order — a row's Undo, the sheet's
    // own, Close last — each with its focus; Tab turns inside this list only.
    let mut stops: Vec<(FocusHandle, Act)> = Vec::new();
    let mut stop = |ix: usize, act: Act, window: &mut Window, cx: &mut Context<Shell>| -> FocusHandle {
        let focus = window.use_keyed_state(("sheet-focus", ix), cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone();
        stops.push((focus.clone(), act));
        focus
    };
    let squad = shell.model.squad.clone();
    let rows: Vec<AnyElement> = sheet
        .rows
        .iter()
        .enumerate()
        .map(|(ix, r)| {
            let undo = r.undo.map(|n| {
                let squad = squad.clone();
                let act: Act = Rc::new(move |this, window, cx| this.ask_undo(squad.clone(), n, window, cx));
                (stop(ix, act.clone(), window, cx), act)
            });
            row(shell, ix, r, key_width, undo, cx)
        })
        .collect();
    let buttons: Vec<AnyElement> = sheet
        .buttons
        .iter()
        .cloned()
        .enumerate()
        .map(|(ix, b)| {
            let (label, act): (String, Act) = match b {
                Button::Copy(label, text) => (label, Rc::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))),
                Button::Run(label, line) => (label, Rc::new(move |this, window, cx| this.run_line(&line, window, cx))),
                Button::Open(label, path) => (label, Rc::new(move |_, _, _| drop(equipe_providers::platform::open(&path)))),
                Button::Ask(label, ask) => (
                    label,
                    Rc::new(move |this, _, cx| {
                        this.sheet = None;
                        this.confirm = Some((*ask).clone());
                        cx.notify();
                    }),
                ),
            };
            let focus = stop(1000 + ix, act.clone(), window, cx);
            let go = cx.listener(move |this, _: &ClickEvent, window, cx| act(this, window, cx));
            ui::button(p, ("sheet-button", ix), Kind::Secondary, label).focus(&focus).on_click(go).into_any_element()
        })
        .collect();
    // Close is the first the keys land on (`sheet_focus`), and the last in the turn.
    let close: Act = Rc::new(|this, window, cx| this.close_info(window, cx));
    stops.push((shell.sheet_focus.clone(), close.clone()));
    let close_button = ui::button(p, "sheet-close", Kind::Primary, t!("sheet.close")).focus(&shell.sheet_focus).on_click(cx.listener(|this, _, window, cx| this.close_info(window, cx)));
    *shell.sheet_stops.borrow_mut() = stops.iter().map(|(f, _)| f.clone()).collect();
    let shell_ref = cx.entity().downgrade();
    let on_key = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
        let k = &event.keystroke;
        let at = stops.iter().position(|(f, _)| f.is_focused(window));
        match k.key.as_str() {
            "space" | "enter" if !k.modifiers.modified() => {
                let Some(i) = at else { return };
                let act = stops[i].1.clone();
                shell_ref.update(cx, |this, cx| act(this, window, cx)).ok();
            }
            _ => return,
        }
        cx.stop_propagation();
    };
    Some(modal_w(
        p,
        "sheet",
        680.,
        v_flex()
            .id("sheet-card")
            .role(Role::Dialog)
            .aria_label(sheet.title.clone())
            .on_key_down(on_key)
            .gap_3()
            .min_h_0()
            .child(div().id("sheet-title").role(Role::Heading).aria_label(sheet.title.clone()).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(sheet.title.clone()))
            .children(sheet.lead.clone().map(|l| div().id("sheet-lead").role(Role::Paragraph).aria_label(l.clone()).text_size(px(SMALL)).line_height(px(20.)).text_color(p.text2).child(l)))
            .child(
                // A long list scrolls: a hairline under it says there is more.
                v_flex()
                    .id("sheet-rows")
                    .flex_1()
                    .min_h_0()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .gap_1()
                    .pb_2()
                    .border_b_1()
                    .border_color(p.hair)
                    .children(rows),
            )
            .children(sheet.foot.clone().map(|l| div().id("sheet-foot").role(Role::Paragraph).aria_label(l.clone()).text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(l)))
            .child(h_flex().gap_2().justify_end().children(buttons).child(close_button)),
    ))
}

fn row(shell: &Shell, ix: usize, r: &Row, key_width: f32, undo: Option<(FocusHandle, Act)>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    if r.heading {
        return div().id(("sheet-row", ix)).role(Role::Heading).aria_label(r.text.clone()).pt_2().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(r.text.clone()).into_any_element();
    }
    h_flex()
        .id(("sheet-row", ix))
        .role(Role::ListItem)
        .aria_label(if r.head.is_empty() { r.text.clone() } else { format!("{}: {}", r.head, r.text) })
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
        .children(undo.map(|(focus, act)| {
            ui::button(p, ("undo", ix), Kind::Secondary, t!("sheet.undo")).focus(&focus).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| act(this, window, cx)))
        }))
        .into_any_element()
}
