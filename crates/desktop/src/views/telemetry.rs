//! Anonymous usage data (docs/TELEMETRY.md): the question the first run ends
//! with — and that a Mac with a squad already is asked once, the first time
//! it opens after the update — and the switch that takes the answer back in
//! settings. The daemon keeps the answer (`telemetry`, `telemetry_set`); this
//! asks only while it says `unset`. Two buttons of the same weight, neither
//! chosen beforehand.

use equipe_core::protocol::telemetry::{Consent, TelemetryStatus};
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::state::Conn;
use crate::theme::{BODY, ICON_SM, Palette, R_LG, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

/// Where the full list lives, for a reader who wants every event.
pub const DOC: &str = "https://github.com/equipe-labs/equipe/blob/main/docs/TELEMETRY.md";

pub struct TelemetryUi {
    /// What the daemon said last; `None` until it has, or while not attached.
    pub status: Option<TelemetryStatus>,
    /// An answer is on its way to the daemon.
    pub saving: bool,
    /// The daemon didn't take the answer.
    pub failed: Option<String>,
    send_focus: FocusHandle,
    keep_focus: FocusHandle,
}

impl TelemetryUi {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self { status: None, saving: false, failed: None, send_focus: cx.focus_handle().tab_stop(true), keep_focus: cx.focus_handle().tab_stop(true) }
    }

    /// Nothing answered yet: the question is up.
    pub fn asking(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.consent == Consent::Unset)
    }

    pub fn on(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.consent == Consent::On)
    }
}

impl Shell {
    /// The question is up in place of the screens: connected, and unanswered.
    pub fn telemetry_asking(&self) -> bool {
        matches!(self.model.conn, Conn::Connected { .. }) && self.telemetry.asking()
    }

    /// Ask the daemon where it stands, after each attach.
    pub fn telemetry_look(&mut self, cx: &mut Context<Self>) {
        let Some(link) = &self.link else { return };
        let uploader = link.uploader();
        self.telemetry.status = None;
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(uploader.telemetry());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(status)) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.telemetry.status = Some(status);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The person's answer, kept by the daemon.
    pub fn telemetry_answer(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(link) = &self.link else { return };
        if self.telemetry.saving {
            return;
        }
        let uploader = link.uploader();
        self.telemetry.saving = true;
        self.telemetry.failed = None;
        cx.notify();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(uploader.telemetry_set(on));
        });
        cx.spawn(async move |this, cx| {
            let done = rx.await.unwrap_or_else(|_| Err("stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.telemetry.saving = false;
                match done {
                    Ok(status) => this.telemetry.status = Some(status),
                    Err(why) => this.telemetry.failed = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// A line of the three: a mark, then its words.
fn point_line(p: Palette, glyph: Icon, ink: Rgba, text: &'static str) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_start()
        .child(div().flex_none().pt(px(4.)).child(icon(glyph, 16., ink)))
        .child(div().flex_1().min_w_0().text_size(px(UI)).line_height(px(23.)).text_color(p.text).child(text))
}

/// The question, across the pane.
pub fn screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.telemetry;
    let button = |id: &'static str, label: &'static str, on: bool, focus: &FocusHandle, cx: &mut Context<Shell>| {
        ui::button(p, id, Kind::Secondary, label).field().min_width(132.).focus(focus).disabled(ui.saving).on_click(cx.listener(move |this, _, _, cx| this.telemetry_answer(on, cx)))
    };
    v_flex()
        .id("telemetry")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .items_center()
        .py_8()
        .child(motion::rise(
            v_flex()
                .w_full()
                .max_w(px(560.))
                .px_4()
                .gap_3()
                .child(div().text_size(px(22.)).line_height(px(30.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("telemetry.title")))
                .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(t!("telemetry.text")))
                .child(
                    v_flex()
                        .gap_3()
                        .p_4()
                        .rounded(px(R_LG))
                        .bg(p.code_bg)
                        .border_1()
                        .border_color(p.hair)
                        .child(point_line(p, Icon::Check, p.accent, t!("telemetry.sent")))
                        .child(point_line(p, Icon::X, p.text2, t!("telemetry.never")))
                        .child(point_line(p, Icon::Shield, p.text2, t!("telemetry.anonymous"))),
                )
                .child(h_flex().child(ui::button(p, "telemetry-doc", Kind::Ghost, t!("telemetry.doc")).icon(Icon::External).on_click(|_, _, cx| cx.open_url(DOC))))
                .children(ui.failed.clone().map(|why| {
                    h_flex().gap_2().items_start().text_size(px(SMALL)).line_height(px(20.)).text_color(p.danger()).child(div().flex_none().pt(px(3.)).child(icon(Icon::Warning, ICON_SM, p.danger()))).child(div().flex_1().min_w_0().child(format!("{}\n{why}", t!("telemetry.failed"))))
                }))
                .child(
                    h_flex()
                        .pt_2()
                        .gap_3()
                        .items_center()
                        .child(button("telemetry-send", t!("telemetry.send"), true, &ui.send_focus, cx))
                        .child(button("telemetry-keep", t!("telemetry.keep"), false, &ui.keep_focus, cx))
                        .when(ui.saving, |r| r.child(motion::spinner(14., p.text2))),
                )
                .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("telemetry.later"))),
            "telemetry-in",
        ))
        .into_any_element()
}
