//! `/usage` as a panel over the window, live (`crate::usage`): each engine's
//! plan as a card — how full, coloured by it, and when it starts over —
//! engine switching chosen right there, and the members, those that spent
//! the most first, each with its avatar, engine, context and tokens.

use equipe_core::protocol::{Command, Engine, PlanUsage, PlanWindow};
use equipe_core::protocol::engines::Switching;
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::theme::{ICON_SM, Palette, R_LG, R_MD, SMALL, avatar};
use crate::usage::{self, Fill, MemberUse};
use crate::ui;

const WIDTH: f32 = 600.;

pub fn panel(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.usage_open {
        return None;
    }
    let p = shell.palette;
    let model = &shell.model;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let close = ui::icon_button(p, "usage-close", Icon::X, t!("disk.close"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_usage(window, cx)));
    let header = h_flex()
        .gap_3()
        .items_center()
        .child(div().size(px(32.)).flex_none().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Chart, 17., p.text)))
        .child(div().flex_1().min_w_0().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(t!("sheet.usage_title")))
        .child(close);
    let codex = model.engines.plan(Engine::Codex).map(|p| p.usage);
    // Side by side, the same height whatever each has to say.
    let plans = h_flex()
        .gap_3()
        .items_stretch()
        .child(plan_card(p, Engine::Claude, model.plan, now))
        .child(plan_card(p, Engine::Codex, codex, now));
    let mode = model.engines.mode;
    let squad = model.squad.clone();
    let switching = h_flex()
        .gap_3()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("engines.settings_section")))
                .child(div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(crate::switching::mode_about(mode))),
        )
        .child(h_flex().flex_none().p(px(2.)).gap(px(2.)).rounded(px(R_MD)).bg(p.wash).children(Switching::ALL.into_iter().map(|m| {
            let on = m == mode;
            let squad = squad.clone();
            div()
                .id(("usage-switching", m as usize))
                .h(px(26.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(R_MD - 2.))
                .text_size(px(12.5))
                .cursor_pointer()
                .role(Role::RadioButton)
                .aria_selected(on)
                .aria_label(crate::switching::mode_word(m))
                .when(on, |b| b.bg(p.popover()).text_color(p.text).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |b| b.text_color(p.text2).hover(move |s| s.text_color(p.text)))
                .child(crate::switching::mode_word(m))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.send_command(Command::SetEngineSwitching { squad: squad.clone(), mode: m });
                    this.model.engines.mode = m;
                    cx.notify();
                }))
        })));
    let members = usage::members(model);
    let most = members.iter().map(|m| m.tokens).max().unwrap_or(0);
    let rows = members.iter().map(|m| member_row(p, m, most));
    let list = v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .pb_1()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(div().flex_1().child(t!("sheet.members")))
                .child(div().w(px(150.)).flex_none().child(t!("usage.col_context")))
                .child(div().w(px(64.)).flex_none().text_right().child(t!("usage.col_tokens"))),
        )
        .children(rows);
    let foot = h_flex()
        .gap_2()
        .items_start()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .child(div().pt(px(2.)).flex_none().child(icon(Icon::Info, ICON_SM, p.muted)))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("usage.note_short")));
    let section = |el: AnyElement| v_flex().p_4().rounded(px(R_LG)).bg(p.wash).child(el);
    Some(crate::views::connect::modal_w(
        p,
        "usage",
        WIDTH,
        v_flex()
            .id("usage-body")
            .gap_4()
            .min_h_0()
            .overflow_y_scroll()
            .child(header)
            .child(plans)
            .child(section(switching.into_any_element()))
            .child(list)
            .child(foot),
    ))
}

/// One engine's plan: its name and mark, the fuller of its windows big, a bar
/// in a colour for how full, and when each window starts over.
fn plan_card(p: Palette, engine: Engine, plan: Option<PlanUsage>, now: i64) -> AnyElement {
    let windows: Vec<(&'static str, PlanWindow)> = plan
        .map(|u| [(t!("usage.five_hour"), u.five_hour), (t!("usage.week"), u.seven_day)].into_iter().filter_map(|(l, w)| Some((l, w?))).collect())
        .unwrap_or_default();
    let head = h_flex().gap_2().items_center().child(crate::views::switching::label(p, engine));
    let body: AnyElement = match windows.iter().max_by_key(|(_, w)| w.used_percent) {
        None => div()
            .text_size(px(12.5))
            .line_height(px(18.))
            .text_color(p.muted)
            .child(if engine == Engine::Claude { t!("usage.not_yet") } else { t!("engines.codex_not_read") })
            .into_any_element(),
        Some(_) => v_flex()
            .gap_2p5()
            .children(windows.iter().map(|(label, w)| {
                let color = match usage::fill(w.used_percent) {
                    Fill::Room => p.accent,
                    Fill::Full => p.amber,
                    Fill::Out => p.red,
                };
                let pct = w.used_percent.min(100);
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().text_size(px(22.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(format!("{pct}%")))
                            .child(div().text_size(px(12.)).text_color(p.muted).child(*label)),
                    )
                    .child(div().h(px(6.)).w_full().rounded_full().bg(p.hair).child(div().h_full().w(relative(pct as f32 / 100.)).rounded_full().bg(color)))
                    .child(div().text_size(px(11.5)).text_color(p.muted).child(t!("usage.resets", left = usage::time_left(w.resets_at, now), when = usage::local_time(w.resets_at))))
            }))
            .into_any_element(),
    };
    v_flex().flex_1().min_w_0().gap_3().p_4().rounded(px(R_LG)).bg(p.wash).child(head).child(body).into_any_element()
}

/// A member: avatar and name, its engine, its context as a short bar, and
/// its tokens — a dash where nothing is known yet.
fn member_row(p: Palette, m: &MemberUse, most: u64) -> AnyElement {
    let context: AnyElement = match m.context {
        Some(pct) => h_flex()
            .gap_2()
            .items_center()
            .child(div().w(px(96.)).h(px(4.)).rounded_full().bg(p.hair).child(div().h_full().w(relative(pct as f32 / 100.)).rounded_full().bg(if pct >= 60 { p.amber } else { p.text2 })))
            .child(div().text_size(px(12.)).text_color(p.text2).child(format!("{pct}%")))
            .into_any_element(),
        None => div().text_size(px(12.)).text_color(p.muted).child("—").into_any_element(),
    };
    let tokens = if m.tokens == 0 { "—".to_owned() } else { usage::short_count(m.tokens) };
    let strong = most > 0 && m.tokens * 2 >= most;
    let row = h_flex()
        .id(SharedString::from(format!("usage-member-{}", m.name)))
        .h(px(34.))
        .px_2()
        .gap_2p5()
        .items_center()
        .rounded(px(R_MD))
        .hover(move |s| s.bg(p.wash))
        .child(avatar(p, &m.role, &m.name, 20.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.text).child(m.name.clone()))
        .children(m.engine.map(|e| div().flex_none().w(px(70.)).child(crate::views::switching::label(p, e))))
        .child(div().w(px(150.)).flex_none().child(context))
        .child(div().w(px(64.)).flex_none().text_right().text_size(px(12.5)).text_color(if strong { p.text } else { p.text2 }).child(tokens));
    match m.cache {
        Some(pct) if m.tokens > 0 => crate::tips::tip(row, p, t!("usage.cache_tip", pct = pct), None).into_any_element(),
        _ => row.into_any_element(),
    }
}

impl Shell {
    pub fn open_usage(&mut self, cx: &mut Context<Self>) {
        self.usage_open = true;
        cx.notify();
    }

    pub fn close_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.usage_open = false;
        self.composer.focus(window, cx);
        cx.notify();
    }
}
