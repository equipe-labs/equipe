//! Usage: where an engine's plan went (`crate::usage`). The plan's week as a
//! bar, with when it runs out at this pace against when it refills; then
//! what took it — the person's own sessions outside equipe (opened to their
//! folders), and equipe's by ticket, member or project, the heaviest first,
//! each with its share of the week; the burn hour by hour, a bar picked
//! listing what ran then; the fastest burns in plain words. A ticket opens
//! from any of them. Reached from the plan's meters, the title bar's glance,
//! `/usage` and Settings › AI platforms. Also the notice of a fast burn,
//! over every screen, in the disk banner's place and style.

use std::time::Instant;

use equipe_core::protocol::engines::Switching;
use equipe_core::protocol::plan_use::{Beat, Report, Slice, Span, Who};
use equipe_core::protocol::{Command, Engine, PlanWindow};
use equipe_core::t;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::theme::{ICON_SM, Palette, R_LG, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::usage::{self, Cut};
use crate::{Screen, Shell};

/// The page's column, at its widest.
const COLUMN: f32 = 820.;
/// The burn's height.
const CHART_H: f32 = 112.;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Outside equipe's colour, beside equipe's accent.
fn outside_color(p: Palette) -> Rgba {
    p.orange
}

fn faint(c: Rgba, a: f32) -> Rgba {
    Rgba { a, ..c }
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.plan_use;
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::Chart, 15., p.text2))
        .child(div().min_w_0().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("sheet.usage_title")))
        .child(div().flex_1())
        .children(ui.looking().then(|| Spinner::new().with_size(px(13.)).color(Hsla::from(p.muted))));

    let seg = |id: &'static str, label: &'static str, on: bool| {
        div()
            .id(id)
            .px_2p5()
            .h(px(26.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.5))
            .role(Role::Tab)
            .aria_selected(on)
            .when(on, |d| d.bg(p.popover()).text_color(p.text).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.text_color(p.text)))
            .child(label)
    };
    let group = || h_flex().flex_none().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(p.wash);
    let controls = h_flex()
        .w_full()
        .flex_wrap()
        .gap_2()
        .child(group().children([Engine::Claude, Engine::Codex].into_iter().map(|engine| {
            let id = if engine == Engine::Claude { "plan-use-engine-claude" } else { "plan-use-engine-codex" };
            seg(id, crate::switching::engine_name(engine), ui.engine == engine).on_click(cx.listener(move |this, _, _, cx| {
                let asks = this.plan_use.pick_engine(engine);
                this.plan_use_send(asks, cx);
            }))
        })))
        .child(group().children([(Span::Week, "plan-use-span-week", t!("plan_use.span_week")), (Span::Day, "plan-use-span-day", t!("plan_use.span_day"))].into_iter().map(|(span, id, label)| {
            seg(id, label, ui.span == span).on_click(cx.listener(move |this, _, _, cx| {
                let asks = this.plan_use.pick_span(span);
                this.plan_use_send(asks, cx);
            }))
        })));

    let now = now();
    let mut column = v_flex().w_full().max_w(px(COLUMN)).gap_5().child(controls).child(plan_card(shell, now));
    match ui.report() {
        None => {
            column = column.child(
                h_flex().w_full().py_10().gap_2().justify_center().items_center().text_size(px(13.)).text_color(p.muted).child(Spinner::new().with_size(px(14.)).color(Hsla::from(p.muted))).child(t!("plan_use.adding_up")),
            )
        }
        Some(report) if report.load == 0 => {
            column = column.child(
                v_flex()
                    .w_full()
                    .py_8()
                    .gap_1p5()
                    .items_center()
                    .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(t!("plan_use.empty_title")))
                    .child(div().max_w(px(440.)).text_center().text_size(px(13.)).text_color(p.muted).child(t!("plan_use.empty"))),
            )
        }
        Some(report) => {
            column = column.child(where_it_went(shell, report, cx)).child(burn(shell, report, now, cx));
            if !report.spikes.is_empty() {
                column = column.child(spikes(shell, report, now, cx));
            }
        }
    }
    column = column.child(switching(shell, cx)).child(foot(shell));
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(header)
        .child(div().id("plan-use-body").flex_1().min_h_0().overflow_y_scroll().child(div().w_full().px_5().py_5().flex().justify_center().child(column)))
        .into_any_element()
}

/// The engine's week: how much is used, a meter coloured by it (amber from
/// 80%, red from 95%) with its even-pace mark, when it refills, and when it
/// runs out at this pace; at 100%, "Limit reached" and when it refills.
fn plan_card(shell: &Shell, now: i64) -> AnyElement {
    use crate::switching::{self as rules, Tone};
    use crate::tips::TipExt as _;
    let p = shell.palette;
    let engine = shell.plan_use.engine;
    let plan = match engine {
        Engine::Claude => shell.model.plan.or_else(|| shell.model.engines.plan(engine).map(|p| p.usage)),
        Engine::Codex => shell.model.engines.plan(engine).map(|p| p.usage),
    };
    let report = shell.plan_use.report();
    let week: Option<PlanWindow> = report.and_then(|r| r.week).or_else(|| plan.and_then(|u| u.seven_day).filter(|w| w.resets_at > now));
    let five = plan.and_then(|u| u.five_hour).filter(|w| w.resets_at > now).map(|w| {
        let used = w.used_percent.min(100);
        let tone = rules::tone(used);
        let words = match tone {
            Tone::Out => t!("usage.window_out", window = t!("usage.five_hour"), span = rules::span(w.resets_at, now)),
            _ => t!("usage.window_used", window = t!("usage.five_hour"), used = used),
        };
        div()
            .text_size(px(12.))
            .text_color(crate::views::switching::tone_color(p, tone, p.muted))
            .when(tone >= Tone::Critical, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(words)
    });
    let head = h_flex().gap_2().items_center().child(crate::views::switching::label(p, engine)).child(div().flex_1()).children(five);
    let body: AnyElement = match week {
        None => div()
            .text_size(px(12.5))
            .line_height(px(18.))
            .text_color(p.muted)
            .child(if engine == Engine::Claude { t!("usage.not_yet") } else { t!("engines.codex_not_read") })
            .into_any_element(),
        Some(w) => {
            let pct = w.used_percent.min(100);
            let gauge = rules::Gauge { window: equipe_core::protocol::engines::Window::Weekly, percent: pct, resets_at: w.resets_at, tone: rules::tone(pct) };
            let color = crate::views::switching::tone_color(p, gauge.tone, p.accent);
            let out = gauge.tone == Tone::Out;
            let pace = report.and_then(|r| usage::pace_words(r, now)).map(|(words, warn)| {
                h_flex()
                    .id("plan-use-pace")
                    .gap_1p5()
                    .items_start()
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(if warn { p.amber } else { p.text2 })
                    .when(warn, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(div().pt(px(2.)).flex_none().child(icon(Icon::Clock, 12., if warn { p.amber } else { p.muted })))
                    .child(div().flex_1().min_w_0().child(words))
            });
            let number = if out {
                h_flex().child(div().text_size(px(24.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(t!("usage.limit_reached")))
            } else {
                h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().text_size(px(24.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(if gauge.tone == Tone::Calm { p.text } else { color }).child(format!("{pct}%")))
                    .child(div().text_size(px(12.5)).text_color(p.muted).child(t!("plan_use.week_used")))
            };
            let meter = div()
                .id("plan-use-week")
                .w_full()
                .py(px(2.))
                .role(Role::Meter)
                .aria_numeric_value(f64::from(pct))
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(100.)
                .tip(p, rules::used_words(engine, &gauge, now, usage::local_time), None)
                .aria_label(rules::gauge_aria(engine, &gauge))
                .child(crate::views::switching::meter_bar(p, pct, color, gauge.pace(now), 8.).w_full());
            v_flex()
                .gap_1p5()
                .child(number)
                .child(meter)
                .child(div().text_size(px(11.5)).text_color(p.muted).child(t!("usage.resets", left = usage::time_left(w.resets_at, now), when = usage::local_time(w.resets_at))))
                .children(pace)
                .into_any_element()
        }
    };
    v_flex().w_full().gap_3().p_4().rounded(px(R_LG)).bg(p.wash).child(head).child(body).into_any_element()
}

/// A row's share as a short bar beside its words.
fn share_bar(p: Palette, fraction: f32, color: Rgba) -> AnyElement {
    div().w(px(72.)).flex_none().h(px(4.)).rounded_full().bg(p.hair).child(div().h_full().w(relative(fraction.clamp(0.0, 1.0))).rounded_full().bg(color)).into_any_element()
}

/// One part of the plan: its name (and what it is), a bar for its share,
/// the share in words, and its calls. A ticket of this squad opens.
fn row(shell: &Shell, id: SharedString, slice: &Slice, most: u64, depth: u8, color: Rgba, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ticket = usage::ticket_of(&slice.who, &shell.model.squad);
    let name = usage::who_words(&slice.who, if matches!(slice.who, Who::Ticket { .. }) { slice.label.as_deref() } else { None });
    let aside = match &slice.who {
        // Its role, unless the name says it already.
        Who::Member { member, .. } => slice.label.clone().filter(|role| role != member),
        Who::Outside { folder: None } => Some(t!("plan_use.outside_sub").to_owned()),
        _ => None,
    };
    let fraction = if most == 0 { 0. } else { slice.load as f32 / most as f32 };
    let mut el = h_flex()
        .id(id)
        .w_full()
        .min_h(px(34.))
        .px_2()
        .pl(px(8. + 16. * f32::from(depth)))
        .gap_2p5()
        .items_center()
        .rounded(px(R_MD))
        .hover(move |s| s.bg(p.wash))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(SMALL)).text_color(p.text).child(name))
                .children(aside.map(|a| div().truncate().text_size(px(11.5)).text_color(p.muted).child(a))),
        )
        .child(share_bar(p, fraction, color))
        .child(div().w(px(118.)).flex_none().text_right().text_size(px(12.5)).text_color(p.text).font_weight(FontWeight::MEDIUM).child(usage::share_words(slice)))
        .child(div().w(px(78.)).flex_none().text_right().text_size(px(11.5)).text_color(p.muted).child(t!("plan_use.calls", calls = usage::grouped(slice.calls))));
    if let Some(ticket) = ticket {
        el = el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.open_ticket(ticket, cx)));
    }
    crate::tips::tip(el, p, t!("plan_use.row_tip", why = usage::mostly_words(slice.mostly), tokens = usage::short_count(slice.tokens.total())), None).into_any_element()
}

fn heading(p: Palette, title: &'static str, sub: Option<String>) -> AnyElement {
    v_flex()
        .gap_0p5()
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title))
        .children(sub.map(|s| div().text_size(px(12.)).text_color(p.muted).child(s)))
        .into_any_element()
}

/// Outside equipe, then equipe's by the cut picked.
fn where_it_went(shell: &Shell, report: &Report, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.plan_use;
    let outside_bp = report.outside.as_ref().map_or(0, |o| o.share_bp);
    let sub = t!("plan_use.where_sub", calls = usage::grouped(report.calls), tokens = usage::short_count(report.tokens.total()));
    // Outside equipe and equipe's, side by side in one bar.
    let split = h_flex()
        .w_full()
        .h(px(10.))
        .rounded_full()
        .overflow_hidden()
        .bg(p.hair)
        .child(div().h_full().w(relative(outside_bp as f32 / 10_000.)).bg(outside_color(p)))
        .child(div().h_full().flex_1().bg(p.accent));
    let legend = |color: Rgba, words: String| h_flex().gap_1p5().items_center().child(div().size(px(8.)).rounded_full().bg(color)).child(div().text_size(px(12.)).text_color(p.text2).child(words));
    let legends = h_flex()
        .flex_wrap()
        .gap_4()
        .child(legend(outside_color(p), t!("plan_use.legend_outside", pct = usage::percent(outside_bp))))
        .child(legend(p.accent, t!("plan_use.legend_equipe", pct = usage::percent(10_000 - outside_bp.min(10_000)))));
    let most = report.outside.iter().map(|o| o.load).chain(report.projects.iter().map(|s| s.load)).max().unwrap_or(0);
    let mut list = v_flex().w_full().gap_0p5();
    if let Some(outside) = &report.outside {
        let open = ui.outside_open;
        let toggle = h_flex()
            .id("plan-use-outside")
            .w_full()
            .items_center()
            .cursor_pointer()
            .aria_expanded(open)
            .on_click(cx.listener(|this, _, _, cx| {
                this.plan_use.outside_open = !this.plan_use.outside_open;
                cx.notify();
            }))
            .child(div().w(px(16.)).flex_none().child(icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 12., p.muted)))
            .child(div().flex_1().min_w_0().child(row(shell, "plan-use-outside-row".into(), outside, most, 0, outside_color(p), cx)));
        list = list.child(toggle);
        if open {
            let inner = outside.children.iter().map(|c| c.load).max().unwrap_or(0);
            for (i, child) in outside.children.iter().enumerate() {
                list = list.child(div().pl(px(16.)).child(row(shell, format!("plan-use-folder-{i}").into(), child, inner, 1, faint(outside_color(p), 0.7), cx)));
            }
        }
    }
    let tabs = h_flex().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(p.wash).flex_none().children(Cut::ALL.into_iter().map(|cut| {
        let on = ui.cut == cut;
        div()
            .id(SharedString::from(format!("plan-use-cut-{}", cut.code())))
            .px_2p5()
            .h(px(24.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .role(Role::Tab)
            .aria_selected(on)
            .when(on, |d| d.bg(p.popover()).text_color(p.text).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.text_color(p.text)))
            .child(cut.label())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.plan_use.cut = cut;
                cx.notify();
            }))
    }));
    let (slices, none) = match ui.cut {
        Cut::Tickets => (&report.tickets, t!("plan_use.none_tickets")),
        Cut::Members => (&report.members, t!("plan_use.none_members")),
        Cut::Projects => (&report.projects, t!("plan_use.none_projects")),
    };
    let top = slices.iter().map(|s| s.load).max().unwrap_or(0);
    let cut_rows: Vec<AnyElement> = slices.iter().enumerate().map(|(i, s)| div().pl(px(16.)).child(row(shell, format!("plan-use-{}-{i}", ui.cut.code()).into(), s, top, 0, p.accent, cx)).into_any_element()).collect();
    let equipe = v_flex()
        .w_full()
        .gap_1()
        .pt_2()
        .child(h_flex().w_full().flex_wrap().gap_2().items_center().child(div().flex_1().min_w(px(140.)).text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(p.text2).child(t!("plan_use.by_equipe"))).child(tabs))
        .when(cut_rows.is_empty(), |d| d.child(div().pl(px(24.)).py_2().text_size(px(12.5)).text_color(p.muted).child(none)))
        .children(cut_rows);
    v_flex()
        .w_full()
        .gap_3()
        .child(heading(p, t!("plan_use.where_title"), Some(sub)))
        .child(split)
        .child(legends)
        .child(list)
        .child(equipe)
        .into_any_element()
}

/// The burn, hour by hour: a bar each, outside equipe's part stacked on
/// equipe's; a spike's hours on a tint; a bar picked lists what ran then.
fn burn(shell: &Shell, report: &Report, now: i64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let tallest = usage::tallest(&report.beats).max(1);
    let gap = if report.beats.len() > 48 { 1. } else { 3. };
    let picked = shell.plan_use.hour;
    let bars = h_flex().w_full().h(px(CHART_H)).gap(px(gap)).items_end().children(report.beats.iter().enumerate().map(|(i, beat)| {
        let at = beat.at;
        let equipe = beat.load.saturating_sub(beat.outside);
        let on = picked == Some(at);
        let band = usage::in_spike(report, at);
        div()
            .id(("plan-use-beat", i))
            .flex_1()
            .min_w(px(1.))
            .h_full()
            .flex()
            .flex_col()
            .justify_end()
            .cursor_pointer()
            .rounded_t(px(2.))
            .when(band, |d| d.bg(faint(p.amber, if p.is_dark() { 0.16 } else { 0.12 })))
            .when(on, |d| d.bg(faint(p.text, 0.10)))
            .hover(move |s| s.bg(faint(p.text, 0.06)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.plan_use.pick_hour(at);
                cx.notify();
            }))
            .child(div().w_full().h(relative(beat.outside as f32 / tallest as f32)).bg(outside_color(p)))
            .child(div().w_full().h(relative(equipe as f32 / tallest as f32)).bg(if on { p.text } else { p.accent }))
    }));
    let labels = h_flex().w_full().gap(px(gap)).h(px(16.)).children(report.beats.iter().map(|b| {
        // A label runs on over the hours after its own.
        div().flex_1().min_w(px(1.)).h_full().relative().children(usage::beat_label(b.at, report.span).map(|l| div().absolute().left_0().top_0().whitespace_nowrap().text_size(px(10.5)).text_color(p.muted).child(l)))
    }));
    let chart = v_flex().w_full().gap_1().p_3().rounded(px(R_LG)).border_1().border_color(p.hair).child(bars).child(labels);
    let mut section = v_flex().w_full().gap_3().child(heading(p, t!("plan_use.burn_title"), Some(t!("plan_use.burn_hint").to_owned()))).child(chart);
    if let Some(beat) = shell.plan_use.picked(report) {
        section = section.child(hour(shell, report, beat, now, cx));
    }
    section.into_any_element()
}

/// What ran in the hour picked.
fn hour(shell: &Shell, report: &Report, beat: &Beat, now: i64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let share = beat_share(report, beat);
    let head = h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(usage::hour_words(beat.at, now)))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(share))
        .child(ui::icon_button(p, "plan-use-hour-close", Icon::X, t!("disk.close"), None).on_click(cx.listener(|this, _, _, cx| {
            this.plan_use.hour = None;
            cx.notify();
        })));
    let most = beat.parts.iter().map(|s| s.load).max().unwrap_or(0);
    let rows: Vec<AnyElement> = beat
        .parts
        .iter()
        .enumerate()
        .map(|(i, s)| row(shell, format!("plan-use-hour-{i}").into(), s, most, 0, if s.who.is_outside() { outside_color(p) } else { p.accent }, cx))
        .collect();
    v_flex()
        .id("plan-use-hour")
        .w_full()
        .gap_1()
        .p_3()
        .rounded(px(R_LG))
        .bg(p.wash)
        .child(head)
        .when(rows.is_empty(), |d| d.child(div().py_1().text_size(px(12.5)).text_color(p.muted).child(t!("plan_use.hour_empty"))))
        .children(rows)
        .into_any_element()
}

/// An hour's share: of the week when the plan is read, else of the look.
fn beat_share(report: &Report, beat: &Beat) -> String {
    match report.week.filter(|_| report.week_load > 0) {
        Some(w) => t!("plan_use.of_week", pct = usage::percent((beat.load as u128 * u128::from(w.used_percent) * 100 / u128::from(report.week_load)) as u32)),
        None => t!("plan_use.of_seen", pct = usage::percent(if report.load == 0 { 0 } else { (beat.load as u128 * 10_000 / u128::from(report.load)) as u32 })),
    }
}

/// The fastest burns, in plain words; a click picks its first hour.
fn spikes(shell: &Shell, report: &Report, now: i64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let squad = shell.model.squad.clone();
    let rows = report.spikes.iter().enumerate().map(|(i, spike)| {
        let from = spike.from;
        let ticket = usage::ticket_of(&spike.slice.who, &squad);
        let when = usage::hours_words(spike.from, spike.to, now);
        h_flex()
            .id(("plan-use-spike", i))
            .w_full()
            .gap_2p5()
            .items_start()
            .px_3()
            .py_2()
            .rounded(px(R_MD))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.plan_use.hour = Some(from);
                cx.notify();
            }))
            .child(div().pt(px(2.)).flex_none().child(icon(Icon::Warning, ICON_SM, p.amber)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_size(px(13.)).line_height(px(19.)).text_color(p.text).child(usage::spike_words(&spike.slice, spike.from, spike.to)))
                    .child(div().text_size(px(11.5)).text_color(p.muted).child(when))
                    .children(spike.slice.label.clone().filter(|_| matches!(spike.slice.who, Who::Ticket { .. })).map(|title| div().truncate().text_size(px(11.5)).text_color(p.text2).child(title))),
            )
            .children(ticket.map(|t| {
                ui::button(p, ("plan-use-spike-open", i), Kind::Ghost, t!("plan_use.open_ticket", ticket = t)).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_ticket(t, cx);
                }))
            }))
    });
    v_flex().w_full().gap_1().child(heading(p, t!("plan_use.spikes_title"), None)).children(rows).into_any_element()
}

/// How members move between engines as plans run low — set right here too.
fn switching(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let mode = shell.model.engines.mode;
    let squad = shell.model.squad.clone();
    // The week won't last at this pace and equipe doesn't move members on its own: say Auto would.
    let hint = (mode != Switching::Auto && shell.plan_use.report().and_then(|r| r.pace).is_some_and(|pace| pace.before_reset)).then(|| {
        h_flex()
            .id("usage-auto-hint")
            .gap_1p5()
            .items_start()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(p.amber)
            .child(div().pt(px(2.)).flex_none().child(icon(Icon::Clock, 12., p.amber)))
            .child(div().flex_1().min_w_0().child(t!("plan_use.auto_hint")))
    });
    h_flex()
        .w_full()
        .flex_wrap()
        .gap_3()
        .items_center()
        .p_4()
        .rounded(px(R_LG))
        .border_1()
        .border_color(p.hair)
        .child(
            v_flex()
                .flex_1()
                .min_w(px(220.))
                .gap_0p5()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("engines.settings_section")))
                .child(div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(crate::switching::mode_about(mode)))
                .children(hint),
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
        })))
        .into_any_element()
}

/// How shares are counted, and what this Mac can't see.
fn foot(shell: &Shell) -> AnyElement {
    let p = shell.palette;
    let report = shell.plan_use.report();
    let line = |words: String, strong: bool| {
        h_flex()
            .gap_2()
            .items_start()
            .child(div().pt(px(2.)).flex_none().child(icon(Icon::Info, ICON_SM, if strong { p.amber } else { p.muted })))
            .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(if strong { p.text2 } else { p.muted }).child(words))
    };
    v_flex()
        .w_full()
        .gap_2()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .children(report.filter(|r| r.reading).map(|_| line(t!("plan_use.reading").to_owned(), true)))
        .children(report.filter(|r| r.week.is_none() && r.load > 0).map(|_| line(t!("plan_use.plan_unread").to_owned(), false)))
        .child(line(t!("plan_use.note").to_owned(), false))
        .into_any_element()
}

// ── the notice of a fast burn ──

/// Over every screen while a fast burn is new: what burned, how much of the
/// week in how long, "See usage" and ✕ — then, for a moment, Undo.
pub fn banner(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let ui = &shell.plan_use;
    let strip = |id: &'static str| {
        h_flex()
            .id(id)
            .h(px(38.))
            .flex_none()
            .pl(px(shell.header_inset(16., cx)))
            .pr_3()
            .gap_2p5()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .text_size(px(SMALL))
            .window_control_area(WindowControlArea::Drag)
    };
    if let Some(burn) = ui.notice() {
        let id = burn.id.clone();
        let engine = burn.engine;
        return Some(
            strip("plan-burn-banner")
                .bg(faint(p.amber, if p.is_dark() { 0.13 } else { 0.10 }))
                .child(div().flex_none().child(icon(Icon::Warning, ICON_SM, p.amber)))
                .child(div().flex_1().min_w_0().truncate().text_color(p.text).font_weight(FontWeight::MEDIUM).child(usage::burn_words(burn)))
                .child(div().flex_none().child(ui::button(p, "plan-burn-open", Kind::Secondary, t!("plan_use.see")).on_click(cx.listener(move |this, _, _, cx| {
                    let asks = this.plan_use.pick_engine(engine);
                    this.plan_use_send(asks, cx);
                    this.open_usage(cx);
                }))))
                .child(div().flex_none().child(ui::icon_button(p, "plan-burn-close", Icon::X, t!("plan_use.dismiss"), None).on_click(cx.listener(move |this, _, _, cx| {
                    this.plan_use.dismiss(&id, Instant::now());
                    // Undo goes once its moment is over.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(usage::UNDO_FOR).await;
                        let _ = this.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                    cx.notify();
                }))))
                .into_any_element(),
        );
    }
    ui.undoable(Instant::now())?;
    Some(
        strip("plan-burn-hidden")
            .child(div().flex_1().min_w_0().truncate().text_color(p.muted).child(t!("plan_use.hidden")))
            .child(div().flex_none().child(ui::button(p, "plan-burn-undo", Kind::Ghost, t!("plan_use.undo")).icon(Icon::Withdraw).on_click(cx.listener(|this, _, _, cx| {
                this.plan_use.undo_dismiss();
                cx.notify();
            }))))
            .into_any_element(),
    )
}

impl Shell {
    /// The Usage screen, with a fresh look at what it shows.
    pub fn open_usage(&mut self, cx: &mut Context<Self>) {
        let asks = self.plan_use.ask();
        self.plan_use_send(asks, cx);
        self.show(Screen::Usage, cx);
        cx.notify();
    }

    /// The snapshot tool's `usage[:<what>]` (`snapshot.rs`).
    pub fn snapshot_usage(&mut self, what: &str, cx: &mut Context<Self>) {
        let ui = &mut self.plan_use;
        let asks = match what.split_once(':').map_or((what, ""), |(a, b)| (a, b)) {
            ("day", _) => ui.pick_span(Span::Day),
            ("week", _) => ui.pick_span(Span::Week),
            ("claude", _) => ui.pick_engine(Engine::Claude),
            ("codex", _) => ui.pick_engine(Engine::Codex),
            ("outside", _) => {
                ui.outside_open = true;
                Vec::new()
            }
            ("cut", cut) => {
                ui.cut = Cut::parse(cut).unwrap_or_default();
                Vec::new()
            }
            ("hour", n) => {
                let beats = ui.report().map(|r| r.beats.clone()).unwrap_or_default();
                let at = match n.parse::<usize>() {
                    Ok(n) => beats.get(n).map(|b| b.at),
                    Err(_) => beats.iter().max_by_key(|b| b.load).map(|b| b.at),
                };
                ui.hour = at;
                Vec::new()
            }
            ("dismiss", _) => {
                if let Some(id) = ui.notice().map(|b| b.id.clone()) {
                    ui.dismiss(&id, Instant::now());
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        self.plan_use_send(asks, cx);
        if what != "dismiss" {
            self.open_usage(cx);
        }
    }

    pub fn plan_use_send(&mut self, commands: Vec<Command>, cx: &mut Context<Self>) {
        for command in commands {
            self.send_command(command);
        }
        cx.notify();
    }
}
