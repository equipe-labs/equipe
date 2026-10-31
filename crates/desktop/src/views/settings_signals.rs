//! Settings' signals, a card of their own under Team: each project's checks
//! of what the squad measures and ships (`protocol::signals`) — how it stands
//! now, its latest value and when it was read, a small trend, what it reads —
//! and a switch to pause one or start it again. The PM proposes a signal and
//! the person turns it on on a card; here they only watch and pause. A child
//! of `views/settings.rs`, drawn with its rows and controls.

use equipe_core::protocol::Command;
use equipe_core::protocol::signals::{SignalReading, SignalView};
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use super::{Line, Section, switch};
use crate::Shell;
use crate::theme::{MONO, Palette, SMALL, UI};
use crate::views::board::dot;

/// The sparkline's box.
const SPARK_W: f32 = 64.;
const SPARK_H: f32 = 20.;
/// Readings a pass/fail strip shows, newest last.
const STRIP: usize = 12;
/// The trend's column, one width for every row so the switches line up.
const TREND_W: f32 = SPARK_W + 8. + 48.;

/// "신호": one row a signal, or a line saying where they come from.
pub(super) fn section(shell: &Shell, connected: bool, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let now = crate::state::now_unix();
    let mut lines: Vec<Line> = Vec::new();
    for s in &shell.model.signals {
        let (project, name) = (s.project.clone(), s.name.clone());
        let flip = switch(shell, &format!("signal-{}-{}", s.project, s.name), s.on, connected, move |this, on, _, _| {
            let squad = this.model.squad.clone();
            this.send_command(Command::SetSignalOn { squad, project: project.clone(), name: name.clone(), on });
        }, cx);
        let control = h_flex().gap_3().items_center().child(h_flex().w(px(TREND_W)).flex_none().justify_end().items_center().children(trend(p, s))).child(flip);
        lines.push(Line::new(s.name.clone(), "").label_el(label(p, s, now)).words(format!("{} {} {} signal metric", s.project, s.what, s.expect)).control(control));
    }
    if lines.is_empty() {
        lines.push(Line::new(t!("settings.no_signals"), t!("settings.no_signals_desc")).words("signal metric posthog app store"));
    }
    Section::new(t!("settings.section_signals"), lines).foot(t!("settings.signals_foot"))
}

/// Name and project; how it stands, its value and when; what it reads.
fn label(p: Palette, s: &SignalView, now: u64) -> AnyElement {
    let last = s.last();
    let (ink, word) = match last {
        _ if !s.on => (p.muted, t!("settings.signal_paused")),
        None => (p.muted, t!("settings.signal_unread")),
        Some(r) if !r.ok => (p.danger(), t!("settings.signal_failing")),
        Some(_) => (p.member(3), t!("settings.signal_passing")),
    };
    let word_ink = if s.failing() && s.on { p.danger() } else { p.text2 };
    let sep = || div().flex_none().text_color(p.muted).child("·");
    let note = last.filter(|r| !r.ok && s.on).and_then(|r| r.note.clone());
    let status = h_flex()
        .w_full()
        .min_w_0()
        .gap(px(6.))
        .items_center()
        .text_size(px(12.5))
        .line_height(px(18.))
        .child(dot(ink, 6.))
        .child(div().flex_none().text_color(word_ink).child(word))
        .children(last.and_then(|r| r.value.clone()).map(|v| h_flex().flex_none().gap(px(6.)).child(sep()).child(div().max_w(px(160.)).truncate().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(v))))
        .children(last.map(|r| h_flex().flex_none().gap(px(6.)).child(sep()).child(div().text_color(p.muted).child(crate::activity::ago(now.saturating_sub(r.at))))));
    // Why it fails, under how it stands: what the person reads first.
    let note = note.map(|n| div().w_full().pl(px(12.)).text_size(px(12.5)).line_height(px(18.)).text_color(p.danger()).line_clamp(2).text_ellipsis().child(n));
    let expect = format!("{} · {}", s.expect, t!("settings.signal_every", span = crate::state::short_span(s.every)));
    let reads = h_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(p.muted)
        .child(div().flex_1().min_w_0().truncate().font_family(MONO).child(s.what.clone()))
        .child(div().flex_none().child(expect));
    v_flex()
        .w_full()
        .min_w_0()
        .gap(px(2.))
        .child(h_flex().w_full().min_w_0().gap_2().items_baseline().child(div().flex_none().text_size(px(UI)).text_color(if s.on { p.text } else { p.text2 }).child(s.name.clone())).child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(s.project.clone())))
        .child(status)
        .children(note)
        .child(reads)
        .into_any_element()
}

/// How it moved: a sparkline of its numbers and the change in percent, or —
/// a signal that reads no number — its latest passes and failures.
fn trend(p: Palette, s: &SignalView) -> Option<AnyElement> {
    let numbers: Vec<f64> = s.readings.iter().filter_map(SignalReading::number).collect();
    let ink = if !s.on { p.muted } else if s.failing() { p.danger() } else { p.text2 };
    if numbers.len() >= 2 {
        let pct = s.trend_pct().map(percent);
        return Some(
            h_flex()
                .gap_2()
                .items_center()
                .child(sparkline(ink, numbers))
                .children(pct.map(|t| div().w(px(48.)).flex_none().text_right().text_size(px(12.)).text_color(p.muted).child(t)))
                .into_any_element(),
        );
    }
    if s.readings.is_empty() {
        return None;
    }
    let from = s.readings.len().saturating_sub(STRIP);
    let off = Rgba { a: 0.5, ..p.muted };
    Some(
        h_flex()
            .gap(px(2.))
            .items_center()
            .children(s.readings[from..].iter().map(|r| div().w(px(3.)).h(px(12.)).rounded(px(1.5)).bg(if r.ok { off } else { p.danger() })))
            .into_any_element(),
    )
}

/// `+98%`, `+0.4%`, `-12%`, `0%`.
fn percent(t: f64) -> String {
    // `+ 0.` turns a rounded -0 into 0.
    let t = if t.abs() < 10. { (t * 10.).round() / 10. } else { t.round() } + 0.;
    let sign = if t > 0. { "+" } else { "" };
    if t.abs() < 10. { format!("{sign}{t}%") } else { format!("{sign}{t:.0}%") }
}

/// The numbers as a line, lowest at the foot and highest at the top.
fn sparkline(ink: Rgba, values: Vec<f64>) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let span = hi - lo;
            let o = bounds.origin;
            let n = values.len().max(2) - 1;
            let mut b = PathBuilder::stroke(px(1.5));
            for (i, v) in values.iter().enumerate() {
                let x = i as f32 / n as f32 * SPARK_W;
                let t = if span > 0. { ((v - lo) / span) as f32 } else { 0.5 };
                let pt = point(o.x + px(x), o.y + px(SPARK_H - 2. - t * (SPARK_H - 4.)));
                if i == 0 { b.move_to(pt) } else { b.line_to(pt) }
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, ink);
            }
        },
    )
    .flex_none()
    .w(px(SPARK_W))
    .h(px(SPARK_H))
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn a_trend_reads_short_with_its_sign() {
        assert_eq!(percent(97.6), "+98%");
        assert_eq!(percent(0.42), "+0.4%");
        assert_eq!(percent(-12.3), "-12%");
        assert_eq!(percent(0.), "0%");
        assert_eq!(percent(-0.01), "0%");
    }
}
