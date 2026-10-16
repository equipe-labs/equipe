//! Both engines, drawn: two small meters at the sidebar's foot — each
//! engine's five hours and week, the reset on the pointer, a click opens
//! `/usage`; the card above the composer — "엔진 전환 제안" with [옮기기]
//! [이번엔 그대로], what equipe did in `auto` with [되돌리기], the recommended
//! setup once with [적용] [나중에], each with who moves from what to what; and
//! the small badge by a member's name. The rules are `crate::engines`'.

use equipe_core::protocol::Engine;
use equipe_core::protocol::engines::{Proposal, Switching, Why};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

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

/// A window's colour by how low it runs: quiet, then amber, orange, red.
fn tone_color(p: Palette, tone: Tone) -> Rgba {
    match tone {
        Tone::Calm => p.text2,
        Tone::Watch => p.amber,
        Tone::Low => p.orange,
        Tone::Out => p.red,
    }
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

    /// "추천 구성 다시 보기": the daemon proposes the setup again.
    pub fn show_engine_setup(&mut self, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(equipe_core::protocol::Command::ShowEngineSetup { squad });
        cx.notify();
    }
}

// ── the sidebar's meters ──

/// Both engines' meters, pinned above the team: a row each.
pub fn meters(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    if model.members.is_empty() {
        return None;
    }
    let now = now();
    let meters = model.engines.meters(now);
    // Nothing read of either yet: the space stays the team's.
    if meters.iter().all(|m| m.read_at.is_none()) {
        return None;
    }
    let waiting = model.engines.first().is_some();
    let head = h_flex()
        .h(px(22.))
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(t!("engines.usage"))
        .child(div().flex_1())
        .child(div().font_weight(FontWeight::NORMAL).text_size(px(11.)).child(rules::mode_word(model.engines.mode)))
        .children(waiting.then(|| div().size(px(6.)).rounded_full().bg(p.accent)));
    let rows: Vec<AnyElement> = meters.iter().map(|m| meter_row(shell, m, now, cx)).collect();
    Some(v_flex().flex_none().pt(px(6.)).pb_1().border_t_1().border_color(p.hair).child(head).children(rows).into_any_element())
}

/// One engine: its name, then each window — a short label, a thin bar, the percent.
fn meter_row(shell: &Shell, m: &Meter, now: i64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = rules::engine_name(m.engine);
    let read = rules::read_words(m, now);
    let mut row = h_flex()
        .id(SharedString::from(format!("meter-{name}")))
        .h(px(24.))
        .mx_2()
        .px_2()
        .gap(px(10.))
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .on_click(cx.listener(|this, _, window, cx| this.apply_effect(crate::compose::Effect::Usage, window, cx)))
        .child(
            div()
                .id(SharedString::from(format!("meter-name-{name}")))
                .w(px(44.))
                .flex_none()
                .text_size(px(12.))
                .text_color(if m.read_at.is_some() { p.text2 } else { p.muted })
                .tip_at(p, read, None, gpui_kit::base::Placement::Right)
                .child(name),
        );
    if m.gauges.is_empty() {
        row = row.child(div().flex_1().text_size(px(11.5)).text_color(p.muted).child(t!("engines.not_yet")));
    }
    for g in &m.gauges {
        row = row.child(gauge(p, m, g, now));
    }
    row.into_any_element()
}

fn gauge(p: Palette, m: &Meter, g: &Gauge, now: i64) -> AnyElement {
    let color = if m.stale { p.muted } else { tone_color(p, g.tone) };
    let words = rules::gauge_words(g, now, crate::usage::local_time);
    let fill = (f32::from(g.percent) / 100.).clamp(0., 1.);
    h_flex()
        .id(SharedString::from(format!("gauge-{}-{:?}", rules::engine_name(m.engine), g.window)))
        .flex_1()
        .min_w_0()
        .gap(px(5.))
        .items_center()
        .tip_at(p, words, None, gpui_kit::base::Placement::Right)
        .child(div().flex_none().text_size(px(10.5)).text_color(p.muted).child(rules::window_short(g.window)))
        .child(
            div()
                .flex_1()
                .min_w(px(14.))
                .h(px(4.))
                .rounded(px(2.))
                .bg(p.hair)
                .overflow_hidden()
                .child(div().h_full().w(relative(fill)).rounded(px(2.)).bg(color)),
        )
        .child(
            div()
                .flex_none()
                .min_w(px(26.))
                .text_right()
                .text_size(px(11.))
                .text_color(if g.tone >= Tone::Low && !m.stale { color } else { p.muted })
                .when(g.tone >= Tone::Low, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(format!("{}%", g.percent)),
        )
        .into_any_element()
}

/// The small badge by a member's name: the engine it runs on.
pub fn badge(p: Palette, engine: Engine) -> AnyElement {
    div()
        .flex_none()
        .h(px(16.))
        .px(px(5.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(p.hair)
        .text_size(px(10.))
        .line_height(px(14.))
        .text_color(p.muted)
        .child(rules::engine_name(engine))
        .into_any_element()
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

    // The limit that set it off, as a bar: how used, when it starts over.
    let limit = (!setup && card.why != Why::Return).then(|| {
        let tone = rules::tone(card.used_percent);
        let color = tone_color(p, tone);
        let engine = card.engine.map(rules::engine_name).unwrap_or_default();
        let fill = (f32::from(card.used_percent.min(100)) / 100.).clamp(0., 1.);
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .text_size(px(12.))
            .child(div().flex_none().text_color(p.text2).child(format!("{engine} · {}", rules::window_word(card.window))))
            .child(div().flex_1().h(px(5.)).rounded(px(3.)).bg(p.hair).overflow_hidden().child(div().h_full().w(relative(fill)).rounded(px(3.)).bg(color)))
            .child(div().flex_none().font_weight(FontWeight::SEMIBOLD).text_color(color).child(format!("{}%", card.used_percent)))
            .child(div().flex_none().text_color(p.muted).child(t!("engines.resets_in", span = rules::span(card.resets_at, now))))
    });

    // Who moves: now → after.
    let rows: Vec<AnyElement> = rules::rows(card)
        .into_iter()
        .map(|r| {
            let (_, hue) = crate::theme::look(&r.role, &r.member);
            h_flex()
                .w_full()
                .h(px(28.))
                .gap_3()
                .items_center()
                .text_size(px(12.5))
                .child(h_flex().w(px(112.)).flex_none().gap(px(6.)).items_center().child(div().size(px(6.)).rounded_full().bg(p.member(hue))).child(div().truncate().text_color(p.text).child(r.member)))
                .child(div().w(px(168.)).flex_none().truncate().text_color(p.muted).child(r.now))
                .child(icon(Icon::ArrowRight, 12., if r.moves_engine { p.accent } else { p.muted }))
                .child(div().flex_1().min_w_0().truncate().text_color(if r.done { p.muted } else { p.text }).font_weight(if r.moves_engine { FontWeight::MEDIUM } else { FontWeight::NORMAL }).child(r.after))
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
    let yes_answer = if card.applied { Answer::Undo } else { Answer::Apply };
    let yes = {
        let handle = handle.clone();
        let kind = if card.applied { Kind::Secondary } else { Kind::Primary };
        let button = ui::button(p, ("engines-yes", id), kind, yes_words).disabled(!shell.can_act());
        let button = if card.applied { button.icon(Icon::Withdraw) } else { button };
        button.on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.engines_decide(id, yes_answer, cx)).ok();
        })
    };
    let no = {
        let handle = handle.clone();
        ui::button(p, ("engines-no", id), if card.applied { Kind::Primary } else { Kind::Secondary }, no_words).disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.engines_decide(id, Answer::Leave, cx)).ok();
        })
    };
    let mode = div().text_size(px(SMALL)).text_color(p.muted).child(t!("engines.mode_line", mode = rules::mode_word(shell.model.engines.mode)));
    let foot = h_flex().w_full().pt_3().gap_2().items_center().border_t_1().border_color(p.hair).child(div().flex_1().min_w_0().child(mode)).child(yes).child(no);

    let inner = v_flex().w_full().gap_2p5().child(head).child(v_flex().gap_1().child(title).child(why)).children(limit).child(table).child(note).child(foot);
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
