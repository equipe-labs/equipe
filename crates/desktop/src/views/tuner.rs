//! The taste tuner, over the design screen: two real product screens side by
//! side, the person picking the one they like more — ← or →, S to skip —
//! with one-tap reasons under them, and how far they are to a first style
//! profile. Each pick goes at once; the profile on the design screen fills in
//! as they go. The rules are `crate::tuner`.

use equipe_core::protocol::design::{PickSide, Taste};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::theme::{META, SMALL, UI};
use crate::tuner::{self as rules, Key};
use crate::visuals::{Look, Size};
use crate::{Shell, ui};

/// The taste the tuner is for.
fn taste_of<'a>(shell: &'a Shell, project: &Option<String>) -> Option<&'a Taste> {
    shell.design.taste.iter().find(|t| t.project == *project)
}

/// The tuner, over everything, while open.
pub fn layer(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let project = shell.tuner.open.clone()?;
    let p = shell.palette;
    let taste = taste_of(shell, &project).cloned().unwrap_or_default();
    let mut ui_state = shell.tuner.clone();
    let pair = ui_state.current(&taste).map(|(a, b, probe)| (a.clone(), b.clone(), probe));
    let (all, left) = rules::progress(&taste, shell.tuner.picks);
    let count = if left > 0 { t!("tuner.to_first", count = left) } else { t!("tuner.refining", count = all) };
    let side = |id: &'static str, item: &equipe_core::protocol::design::TunerItem, key: &'static str, chose: PickSide, cx: &mut Context<Shell>| {
        let look = shell.shots.borrow_mut().look(&item.picture, Size::Full);
        let face: AnyElement = match look {
            Look::Ready(shot) => img(shot.image).max_w_full().max_h_full().object_fit(ObjectFit::Contain).rounded(px(8.)).into_any_element(),
            Look::Gone(_) => icon(Icon::Alert, 16., p.muted).into_any_element(),
            Look::Loading => crate::motion::spinner(16., p.muted),
        };
        v_flex()
            .id(id)
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_2()
            .p_3()
            .rounded(px(12.))
            .border_1()
            .border_color(p.hair)
            .bg(p.bg)
            .cursor_pointer()
            .hover(move |s| s.border_color(p.accent))
            .on_click(cx.listener(move |this, _, _, cx| this.tuner_answer(Some(chose), cx)))
            .child(div().flex_1().min_h_0().flex().items_center().justify_center().child(face))
            .child(h_flex().gap_2().items_center().child(div().flex_none().px_1p5().rounded(px(4.)).bg(p.wash).text_size(px(META)).text_color(p.text2).child(key)).child(div().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(item.source.clone())))
    };
    let body: AnyElement = match pair {
        Some((a, b, probe)) => v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(h_flex().flex_1().min_h_0().gap_4().child(side("tuner-a", &a, "←", PickSide::A, cx)).child(side("tuner-b", &b, "→", PickSide::B, cx)))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(reasons(shell, cx))
                    .child(div().flex_1())
                    .when(probe, |d| d.child(div().text_size(px(META)).text_color(p.muted).child(t!("tuner.probe"))))
                    .child(ui::button(p, "tuner-skip", ui::Kind::Ghost, t!("tuner.skip")).key("S").on_click(cx.listener(|this, _, _, cx| this.tuner_answer(None, cx)))),
            )
            .into_any_element(),
        None => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .child(div().max_w(px(440.)).text_center().text_size(px(UI)).text_color(p.text2).child(if taste.tuner.is_empty() { t!("tuner.no_set") } else { t!("tuner.done") }))
            .when(taste.tuner.is_empty(), |d| d.child(ui::button(p, "tuner-ask", ui::Kind::Primary, t!("tuner.ask_designer")).on_click(cx.listener(|this, _, window, cx| this.tuner_ask_designer(window, cx)))))
            .into_any_element(),
    };
    Some(
        div()
            .id("tuner-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui_kit::Rgba { a: 0.55, ..gpui_kit::rgb(0x000000) })
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .child(
                v_flex()
                    .id("tuner")
                    .w_full()
                    .max_w(px(1180.))
                    .h_full()
                    .max_h(px(780.))
                    .p_5()
                    .gap_4()
                    .rounded(px(16.))
                    .bg(p.raised)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(v_flex().gap_0p5().child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("tuner.title"))).child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("tuner.note"))))
                            .child(div().flex_1())
                            .child(div().text_size(px(SMALL)).text_color(p.text2).child(count))
                            .child(ui::icon_button(p, "tuner-close", Icon::X, t!("tuner.close"), Some("Esc")).on_click(cx.listener(|this, _, _, cx| this.tuner_close(cx)))),
                    )
                    .child(body),
            )
            .into_any_element(),
    )
}

/// The one-tap reasons for the pair on screen.
fn reasons(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.tuner.reasons.clone();
    h_flex().flex_wrap().gap_1p5().children(equipe_core::protocol::design::PickReason::ALL.into_iter().map(move |reason| {
        let lit = on.contains(&reason);
        div()
            .id(("tuner-reason", reason as usize))
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(if lit { p.accent } else { p.hair })
            .when(lit, |d| d.bg(p.accent_bg))
            .text_size(px(META))
            .text_color(if lit { p.accent } else { p.text2 })
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(super::design_session::reason_words(reason))
            .on_click(cx.listener(move |this, _, _, cx| this.tuner_reason(reason, cx)))
    }))
}

impl Shell {
    /// Open the tuner for `project` (`None`: the squad's own).
    pub fn tuner_open(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_secs());
        self.tuner.open(project, seed);
        if self.screen != crate::Screen::Design {
            self.open_design(cx);
        }
        cx.notify();
    }

    pub fn tuner_close(&mut self, cx: &mut Context<Self>) {
        self.tuner.close();
        cx.notify();
    }

    /// The pair on screen picked (`Some`) or skipped: kept at once, the next comes.
    pub fn tuner_answer(&mut self, chose: Option<PickSide>, cx: &mut Context<Self>) {
        let Some(project) = self.tuner.open.clone() else { return };
        let taste = taste_of(self, &project).cloned().unwrap_or_default();
        let Some((a, b)) = self.tuner.current(&taste).map(|(a, b, _)| (a.picture.clone(), b.picture.clone())) else { return };
        let command = self.tuner.answer(&self.model.squad, &a, &b, chose);
        self.send_command(command);
        cx.notify();
    }

    pub fn tuner_reason(&mut self, reason: equipe_core::protocol::design::PickReason, cx: &mut Context<Self>) {
        self.tuner.toggle(reason);
        cx.notify();
    }

    /// The tuner's keys: ←, →, S, Esc. Whether it took the key.
    pub fn tuner_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.tuner.open.is_none() {
            return false;
        }
        match rules::key(key) {
            Some(Key::Pick(side)) => self.tuner_answer(Some(side), cx),
            Some(Key::Skip) => self.tuner_answer(None, cx),
            Some(Key::Close) => self.tuner_close(cx),
            None => return false,
        }
        true
    }

    /// No set yet: the designer is asked to prepare one.
    pub fn tuner_ask_designer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tuner.close();
        self.design_ask_beside(t!("tuner.ask_message").into(), t!("tuner.ask_pm_message").into(), window, cx);
    }
}
