//! Both engines, drawn: small meters at the sidebar's foot, once either plan
//! was read — how much of each engine's five hours and week is used, a bar
//! that fills as it is, the weekly one with its even-pace mark, the reset on
//! the pointer, a click opens `/usage`, foldable to the window used most —
//! and that window in the title bar while the sidebar is away; the
//! card above the composer — "Engine switch" with [Move] [Not this time],
//! what equipe did in `auto` with [Undo], the recommended
//! setup once with [Apply] [Later], each with who moves from what to what; and
//! the small badge by a member's name. The rules are `crate::switching`'.

use equipe_core::protocol::Engine;
use equipe_core::protocol::engines::{Proposal, Switching, Why};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use equipe_core::protocol::engines::Window;

use crate::switching::{self as rules, Answer, Gauge, Meter, Tone};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{Palette, R_LG, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A window's colour by how used it is: `calm`, then amber from 80%, red
/// from 95% — the same everywhere a plan's use shows.
pub fn tone_color(p: Palette, tone: Tone, calm: Rgba) -> Rgba {
    match tone {
        Tone::Calm => calm,
        Tone::Near => p.amber,
        Tone::Critical | Tone::Out => p.red,
    }
}

/// A meter, not a progress bar: a track that fills as the window is used,
/// in its tone's colour, and on a weekly window the even-pace mark — where
/// its use would be by now if spread evenly (`rules::even_pace`). The mark
/// stands out of the track, so it reads over the fill and the track alike.
pub fn meter_bar(p: Palette, used: u8, color: Rgba, pace: Option<f32>, height: f32) -> Div {
    let fill = (f32::from(used.min(100)) / 100.).clamp(0., 1.);
    let round = px(height / 2.);
    div()
        .relative()
        .h(px(height))
        .rounded(round)
        .bg(p.hair)
        .child(div().h_full().w(relative(fill)).rounded(round).bg(color))
        .children(pace.map(|at| {
            div()
                .absolute()
                .top(px(-2.))
                .bottom(px(-2.))
                .left(relative(at.clamp(0., 1.)))
                .ml(px(-1.))
                .w(px(2.))
                .rounded(px(1.))
                .bg(p.text)
        }))
}

impl Shell {
    /// The person's answer to card `id`, as their command.
    pub fn engines_decide(&mut self, id: u64, answer: Answer, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        if let Some(command) = self.model.engines.decide(&squad, id, answer) {
            self.send_command(command);
            cx.notify();
        }
    }

    /// The squad's choice, from settings or `/engines`.
    pub fn set_switching(&mut self, mode: Switching, cx: &mut Context<Self>) {
        self.model.engines.mode = mode;
        let squad = self.model.squad.clone();
        self.send_command(equipe_core::protocol::Command::SetEngineSwitching { squad, mode });
        cx.notify();
    }

    /// "See the recommended setup again": the daemon proposes the setup again.
    pub fn show_engine_setup(&mut self, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(equipe_core::protocol::Command::ShowEngineSetup { squad });
        cx.notify();
    }
}

// ── the sidebar's meters ──

/// Both engines' plans, pinned above the team, as soon as either was read:
/// each engine's windows a row apiece — how much is used, the reset on the
/// pointer, how old a reading is once it isn't sure — a click opens
/// `/usage`. The heading folds them (kept per Mac) to the window used most.
pub fn meters(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    if model.members.is_empty() {
        return None;
    }
    let now = now();
    let meters = model.engines.meters(now);
    let waiting = model.engines.first().is_some();
    if !rules::worth_showing(&meters, waiting) {
        return None;
    }
    let folded = shell.remembered.usage_folded;
    // Folded, the heading keeps the window used most in sight.
    let glance = rules::tightest(&meters).filter(|_| folded).map(|(engine, g)| {
        let stale = meters.iter().any(|m| m.engine == engine && m.stale);
        let color = if stale { p.muted } else { tone_color(p, g.tone, p.muted) };
        div().font_weight(FontWeight::NORMAL).text_size(px(11.)).text_color(color).child(rules::glance(engine, &g))
    });
    let head = h_flex()
        .id("usage-heading")
        .h(px(22.))
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .aria_expanded(!folded)
        .aria_label(t!("engines.usage_title"))
        .on_click(cx.listener(|this, _, _, cx| {
            this.remembered.usage_folded = !this.remembered.usage_folded;
            this.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(
            div()
                .w(px(crate::views::sidebar::MARK_BOX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(crate::views::sidebar::icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 12., p.muted)),
        )
        .child(div().min_w_0().truncate().child(t!("engines.usage_title")))
        .child(div().flex_1())
        .children(glance)
        .when(!folded, |h| h.child(div().flex_none().font_weight(FontWeight::NORMAL).text_size(px(11.)).child(rules::mode_word(model.engines.mode))))
        .children(waiting.then(|| div().flex_none().size(px(6.)).rounded_full().bg(p.accent)));
    let rows: Vec<AnyElement> = if folded { Vec::new() } else { meters.iter().map(|m| meter_rows(shell, m, now, cx)).collect() };
    // Its hairline above is the sidebar's, to drag (`sidebar::split_area`).
    Some(v_flex().flex_none().pt(px(6.)).pb_1().child(head).children(rows).into_any_element())
}

/// One engine: its name, then a row per window — a short label, a bar that
/// fills as it is used, "42% used" — and under them, once a reading is old,
/// how old. Never read: "not read yet" and how it gets read.
fn meter_rows(shell: &Shell, m: &Meter, now: i64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = rules::engine_name(m.engine);
    let name_cell = |first: bool| {
        div()
            .id(SharedString::from(format!("meter-name-{name}-{first}")))
            .w(px(NAME_W))
            .flex_none()
            .text_size(px(12.))
            .text_color(if m.read_at.is_some() { p.text2 } else { p.muted })
            .when(first, |d| d.tip_at(p, rules::read_words(m, now), None, gpui_kit::base::Placement::Right).child(name))
    };
    let row = || h_flex().h(px(ROW_H)).gap(px(8.)).items_center();
    let mut lines: Vec<AnyElement> = Vec::new();
    if m.read_at.is_none() {
        let (words, how) = rules::unread_words(m.engine);
        lines.push(
            row()
                .h(px(24.))
                .child(name_cell(true))
                .child(
                    div()
                        .id(SharedString::from(format!("meter-unread-{name}")))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .tip_at(p, how, None, gpui_kit::base::Placement::Right)
                        .child(words),
                )
                .into_any_element(),
        );
    } else if m.gauges.is_empty() {
        // Read, but a plan with no five-hour or weekly window (a free one): its kind.
        let words = match shell.model.engines.plan(m.engine).and_then(|p| p.plan_type.clone()) {
            Some(kind) => t!("engines.plan_no_windows", plan = crate::engines::title_case(&kind)),
            None => t!("engines.not_yet").to_owned(),
        };
        lines.push(
            row()
                .h(px(24.))
                .child(name_cell(true))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.5)).text_color(p.muted).child(words))
                .into_any_element(),
        );
    } else {
        for (i, g) in m.gauges.iter().enumerate() {
            lines.push(row().child(name_cell(i == 0)).child(gauge(p, m, g, now)).into_any_element());
        }
    }
    // An old reading says how old, a caption under the engine's name.
    let age = rules::age_words(m, now).map(|words| div().h(px(14.)).mb(px(2.)).text_size(px(10.5)).text_color(p.muted).truncate().child(words));
    v_flex()
        .id(SharedString::from(format!("meter-{name}")))
        .mx_2()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .on_click(cx.listener(|this, _, window, cx| this.apply_effect(crate::compose::Effect::Usage, window, cx)))
        .children(lines)
        .children(age)
        .into_any_element()
}

/// The engine's name's column.
const NAME_W: f32 = 42.;
/// A window's row, under the engine's name.
const ROW_H: f32 = 20.;
const LABEL_W: f32 = 16.;
/// The least the words beside a bar get: "100% used" in English.
const USED_W: f32 = 52.;
/// The least a bar is drawn at: narrower, the words alone.
const BAR_MIN: f32 = 18.;

/// A window: its label, a bar that fills as it is used — the weekly one
/// with its even-pace mark — and "42% used"; out, "Limit reached · resets
/// in 2h" in the bar's place. Quiet while calm, amber from 80%, red from
/// 95%; grey when the reading is old. A meter to a screen reader, its words
/// and the mark's on the pointer.
fn gauge(p: Palette, m: &Meter, g: &Gauge, now: i64) -> AnyElement {
    let color = if m.stale { p.muted } else { tone_color(p, g.tone, p.text2) };
    let out = g.tone == Tone::Out;
    let words_color = if g.tone >= Tone::Near && !m.stale { color } else { p.muted };
    h_flex()
        .id(SharedString::from(format!("gauge-{}-{:?}", rules::engine_name(m.engine), g.window)))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .gap(px(4.))
        .items_center()
        .role(Role::Meter)
        .aria_numeric_value(f64::from(g.percent))
        .aria_min_numeric_value(0.)
        .aria_max_numeric_value(100.)
        .tip_at(p, rules::used_words(m.engine, g, now, crate::usage::local_time), None, gpui_kit::base::Placement::Right)
        // After the tip: its first line would name it without the engine.
        .aria_label(rules::gauge_aria(m.engine, g))
        .child(div().flex_none().w(px(LABEL_W)).text_size(px(10.5)).text_color(p.muted).child(rules::window_short(g.window)))
        .when(!out, |row| row.child(meter_bar(p, g.percent, color, g.pace(now), 4.).flex_1().min_w(px(BAR_MIN))))
        .child(
            div()
                .when(out, |d| d.flex_1())
                .when(!out, |d| d.flex_none().min_w(px(USED_W)).text_right())
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(10.5))
                .text_color(words_color)
                .when(g.tone >= Tone::Critical && !m.stale, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(rules::used_short(g, now)),
        )
        .into_any_element()
}

/// The title bar's glance, while the sidebar is away: the engine of the
/// window used most and how much ("Claude 58% used"), its words on the
/// pointer, a click opens `/usage`.
pub fn glance(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let now = now();
    let meters = shell.model.engines.meters(now);
    let (engine, g) = rules::header_glance(&meters, shell.board.sidebar_hidden)?;
    let stale = meters.iter().any(|m| m.engine == engine && m.stale);
    let color = if stale { p.muted } else { tone_color(p, g.tone, p.muted) };
    let words = rules::used_words(engine, &g, now, crate::usage::local_time);
    Some(
        div()
            .id("header-usage")
            .flex_none()
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(R_MD))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .text_size(px(SMALL))
            .text_color(color)
            .when(g.tone >= Tone::Critical && !stale, |d| d.font_weight(FontWeight::SEMIBOLD))
            .tip(p, words, None)
            .aria_label(rules::gauge_aria(engine, &g))
            .on_click(cx.listener(|this, _, window, cx| this.apply_effect(crate::compose::Effect::Usage, window, cx)))
            .child(rules::glance(engine, &g))
            .into_any_element(),
    )
}

/// The engine's mark alone, for a row with no room for its name (the team
/// list): its name on the pointer.
pub fn mark(p: Palette, id: impl Into<ElementId>, engine: Engine) -> AnyElement {
    let (glyph, tint) = if engine == Engine::Codex { (crate::icons::Icon::OpenAiMark, p.text2) } else { (crate::icons::Icon::ClaudeMark, rgb(0xD97757)) };
    div()
        .id(id.into())
        .flex_none()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .tip_at(p, rules::engine_name(engine).to_owned(), None, gpui_kit::base::Placement::Right)
        .child(crate::icons::icon(glyph, 11., tint))
        .into_any_element()
}

/// The engine as words with its mark, unboxed: inside a control that has an
/// edge of its own (the header's title).
pub fn label(p: Palette, engine: Engine) -> AnyElement {
    h_flex().flex_none().gap(px(4.)).items_center().text_size(px(12.)).text_color(p.muted).child(glyph(p, engine)).child(rules::engine_name(engine)).into_any_element()
}

/// An engine's mark alone, in its colour.
pub fn glyph(p: Palette, engine: Engine) -> AnyElement {
    let (glyph, tint) = if engine == Engine::Codex { (crate::icons::Icon::OpenAiMark, p.text2) } else { (crate::icons::Icon::ClaudeMark, rgb(0xD97757)) };
    crate::icons::icon(glyph, 11., tint).into_any_element()
}

// ── the card ──

/// The card above the composer: the oldest one showing, in any conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let card = shell.model.engines.first()?.clone();
    let id = card.id;
    let body = card_element(shell, &card, cx);
    Some(motion::rise(div().w_full().mb_3().child(body), ("engines-in", id)).into_any_element())
}

fn card_element(shell: &Shell, card: &Proposal, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let now = now();
    let id = card.id;
    let handle = cx.entity().downgrade();
    let setup = card.why == Why::Setup;
    let glyph = match (card.why, card.applied) {
        (Why::Setup, _) => Icon::Lightbulb,
        (_, true) => Icon::Done,
        _ => Icon::Retry,
    };
    let head = h_flex()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(glyph, 13., p.accent))
        .child(div().text_color(p.accent).child(rules::kind(card)))
        .child(div().flex_1())
        .children(rules::progress(card).map(|words| div().text_size(px(11.5)).font_weight(FontWeight::NORMAL).text_color(p.muted).child(words)));
    let title = div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(rules::headline(card, now));
    let why = div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(rules::subline(card));
    // The forecast behind it: when the plan runs out at this pace, and who used it.
    let forecast = rules::forecast_words(card, now).map(|words| {
        let warn = card.forecast.is_some_and(|f| f.runs_out_at.is_some());
        h_flex()
            .id(("engines-forecast", id))
            .gap_1p5()
            .items_start()
            .text_size(px(12.5))
            .line_height(px(18.))
            .text_color(if warn { p.amber } else { p.text2 })
            .child(div().pt(px(2.)).flex_none().child(icon(Icon::Clock, 12., if warn { p.amber } else { p.muted })))
            .child(div().flex_1().min_w_0().child(words))
    });
    let stands = |name: &str| shell.model.members.iter().find(|m| m.name == name).map_or_else(rules::Stands::default, |m| rules::Stands { working: m.presence == crate::state::Presence::Working, on_ticket: m.ticket.is_some() });

    // The limit that set it off, as a meter: how used, when it starts over.
    let limit = (!setup && card.why != Why::Return).then(|| {
        let tone = rules::tone(card.used_percent);
        let color = tone_color(p, tone, p.text2);
        let engine = card.engine.map(rules::engine_name).unwrap_or_default();
        let window = card.window.unwrap_or(Window::FiveHour);
        let pace = (window == Window::Weekly).then(|| rules::even_pace(window, card.resets_at, now)).flatten();
        let used = if tone == Tone::Out { t!("usage.limit_reached").to_owned() } else { t!("engines.used_short", used = card.used_percent) };
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .text_size(px(12.))
            .child(div().flex_none().text_color(p.text2).child(format!("{engine} · {}", rules::window_word(card.window))))
            .child(meter_bar(p, card.used_percent, color, pace, 5.).flex_1())
            .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(color).child(used))
            .child(div().flex_none().text_color(p.muted).child(t!("engines.resets_in", span = rules::span(card.resets_at, now))))
    });

    // Who moves: now → after, and when.
    let rows: Vec<AnyElement> = rules::rows(card, stands)
        .into_iter()
        .map(|r| {
            h_flex()
                .w_full()
                .min_h(px(28.))
                .gap_3()
                .items_center()
                .flex_wrap()
                .text_size(px(12.5))
                .child(h_flex().w(px(112.)).flex_none().gap(px(6.)).items_center().child(crate::theme::avatar(p, &r.role, &r.member, 15.)).child(div().truncate().text_color(p.text).child(r.member)))
                .child(div().w(px(150.)).flex_none().truncate().text_color(p.muted).child(r.now))
                .child(icon(Icon::ArrowRight, 12., if r.moves_engine { p.accent } else { p.muted }))
                .child(div().flex_1().min_w(px(96.)).truncate().text_color(if r.done { p.muted } else { p.text }).font_weight(if r.moves_engine { FontWeight::MEDIUM } else { FontWeight::NORMAL }).child(r.after))
                .child(div().flex_none().text_size(px(11.5)).text_color(p.muted).child(r.when))
                .child(div().w(px(16.)).flex_none().children(r.done.then(|| icon(Icon::Check, 13., p.accent))))
                .into_any_element()
        })
        .collect();
    let table = v_flex().w_full().px_3().py_1().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).children(rows);

    let note = h_flex()
        .gap_1p5()
        .items_start()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(p.muted)
        .child(div().pt(px(2.)).child(icon(Icon::Info, 12., p.muted)))
        .child(div().flex_1().min_w_0().child(if setup { t!("engines.setup_note") } else { t!("engines.note") }));

    let (yes_words, no_words) = rules::buttons(card);
    // A member mid-ticket: "Move now" hands its ticket over at its turn's end;
    // the gentle way waits for the ticket to end.
    let now_offered = rules::offers_now(card, stands);
    let yes_answer = if card.applied { Answer::Undo } else { Answer::Apply };
    let yes = {
        let handle = handle.clone();
        let kind = if card.applied || now_offered { Kind::Secondary } else { Kind::Primary };
        let words = if now_offered && !card.applied { t!("engines.move_after") } else { yes_words };
        let button = ui::button(p, ("engines-yes", id), kind, words).disabled(!shell.can_act());
        let button = if card.applied { button.icon(Icon::Withdraw) } else { button };
        button.on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.engines_decide(id, yes_answer, cx)).ok();
        })
    };
    let now_button = now_offered.then(|| {
        let handle = handle.clone();
        ui::button(p, ("engines-now", id), if card.applied { Kind::Secondary } else { Kind::Primary }, t!("engines.move_now")).disabled(!shell.can_act()).tip(t!("engines.move_now_tip"), None).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.engines_decide(id, Answer::Now, cx)).ok();
        })
    });
    // In suggest: one click lets equipe make moves like this itself.
    let auto = rules::offers_auto(card, shell.model.engines.mode).then(|| {
        ui::button(p, ("engines-auto", id), Kind::Ghost, t!("engines.auto_offer")).disabled(!shell.can_act()).tip(t!("engines.auto_offer_tip"), None).on_click(cx.listener(|this, _, _, cx| this.set_switching(Switching::Auto, cx)))
    });
    let no = {
        let handle = handle.clone();
        ui::button(p, ("engines-no", id), if card.applied { Kind::Primary } else { Kind::Secondary }, no_words).disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.engines_decide(id, Answer::Leave, cx)).ok();
        })
    };
    let mode = div().text_size(px(SMALL)).text_color(p.muted).child(t!("engines.mode_line", mode = rules::mode_word(shell.model.engines.mode)));
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .flex_wrap()
        .child(h_flex().flex_1().min_w(px(crate::ui::WORDS_AT_LEAST)).gap_2().items_center().flex_wrap().child(mode).children(auto))
        .child(h_flex().flex_none().ml_auto().gap_2().flex_wrap().children(now_button).child(yes).child(no));

    let inner = v_flex().w_full().gap_2p5().child(head).child(v_flex().gap_1().child(title).child(why).children(forecast)).children(limit).child(table).child(note).child(foot);
    div()
        .id(("engines-card", id))
        .w_full()
        .p(px(14.))
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .shadow_sm()
        .child(inner)
        .into_any_element()
}
