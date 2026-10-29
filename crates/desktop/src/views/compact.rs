//! The band over the composer while a member's conversation is summarized
//! (`design/specs/compaction.md`, T-164): what it is doing, how long it has
//! gone on — never a percentage, the engine gives none — what waits behind
//! it, and on a failure a way to try again. Its words and look are decided
//! here, away from the window, so they can be tested.

use std::time::{Duration, Instant};

use equipe_core::protocol::Command;
use equipe_core::t;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::convo::{COMPACT_DONE_SHOWN, COMPACT_LONG, Compact, elapsed};
use crate::icons::{Icon, icon};
use crate::theme::{META, META_LINE, Palette, R_LG, SMALL, SMALL_LINE};
use crate::{Shell, motion, ui};

/// A flowing piece of the bar: this fraction of the track wide.
const PIECE: f32 = 0.32;
/// Where the piece stands when motion is reduced: this far from the left.
const PIECE_AT_REST: f32 = 0.2;
/// One flow of the bar (`motion.duration.loop`).
const LOOP: Duration = Duration::from_millis(1600);
/// The band comes in over this long (`motion.duration.fast`), or this when
/// motion is reduced (`motion.duration.reduced`).
const IN: Duration = Duration::from_millis(120);
const IN_REDUCED: Duration = Duration::from_millis(100);

/// What the band says in one state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    pub title: &'static str,
    pub body: &'static str,
}

/// The title and the line under it for a state, `ran` into a summary.
pub fn words(compact: &Compact, ran: Duration) -> Words {
    match compact {
        Compact::Running { .. } if ran >= COMPACT_LONG => Words { title: t!("compact.title.long"), body: t!("compact.body.long") },
        Compact::Running { manual: true, .. } => Words { title: t!("compact.title.manual"), body: t!("compact.body.manual") },
        Compact::Running { manual: false, .. } => Words { title: t!("compact.title.auto"), body: t!("compact.body.auto") },
        Compact::Done { .. } => Words { title: t!("compact.done.title"), body: t!("compact.done.body") },
        Compact::Failed => Words { title: t!("compact.fail.title"), body: t!("compact.fail.body") },
    }
}

/// "메시지 2개 대기 중 · 요약이 끝나면 보낼게요".
pub fn queued(n: usize) -> String {
    t!("compact.queued", count = n)
}

/// Whether the band is up at `now`: while it runs, until it failed is closed,
/// and a few seconds after it is done.
pub fn shown(compact: &Compact, now: Instant) -> bool {
    match compact {
        Compact::Done { at } => now.saturating_duration_since(*at) < COMPACT_DONE_SHOWN,
        _ => true,
    }
}

/// The band for the conversation on screen, if its summary is up.
pub fn band(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let session = shell.model.viewed()?;
    let convo = shell.model.convos.get(&session)?;
    let compact = convo.compact?;
    let now = Instant::now();
    if !shown(&compact, now) {
        return None;
    }
    let p = shell.palette;
    let tones = crate::render::tones(&p);
    let ran = convo.compacting_for().unwrap_or_default();
    let said = words(&compact, ran);
    let reduce = cx.reduce_motion();
    let (glyph, tone) = match compact {
        Compact::Running { .. } => (Icon::Compact, p.amber),
        Compact::Done { .. } => (Icon::Check, tones.added),
        Compact::Failed => (Icon::XCircle, p.red),
    };
    match compact {
        // The seconds count up; the band goes when the few seconds are over.
        Compact::Running { .. } => shell.wake_at(now + Duration::from_secs(1), cx),
        Compact::Done { at } => shell.wake_at(at + COMPACT_DONE_SHOWN, cx),
        Compact::Failed => {}
    }
    let waiting = if matches!(compact, Compact::Running { .. }) { convo.queue.items().len() } else { 0 };
    let head = h_flex()
        .gap_3()
        .items_baseline()
        .child(div().min_w_0().flex_1().text_size(px(SMALL)).line_height(px(SMALL_LINE)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(said.title))
        .when(matches!(compact, Compact::Running { .. }), |h| h.child(div().flex_none().min_w(px(48.)).text_right().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.muted).child(elapsed(ran))));
    let text = v_flex()
        .min_w_0()
        .flex_1()
        .gap_1()
        .child(head)
        .child(div().text_size(px(META)).line_height(px(META_LINE)).text_color(p.muted).child(said.body))
        .when(matches!(compact, Compact::Running { .. }), |v| v.child(bar(p, tone, reduce)))
        .when(waiting > 0, |v| v.child(div().pt_1().child(chip(p, queued(waiting)))));
    let actions = matches!(compact, Compact::Failed).then(|| {
        h_flex()
            .flex_none()
            .gap_1()
            .items_center()
            .child(ui::button(p, "compact-retry", ui::Kind::Secondary, t!("compact.retry")).on_click(cx.listener(move |shell, _, _, cx| {
                shell.send_command(Command::Refresh { session, summary: true });
                shell.close_compact(session, cx);
            })))
            .child(ui::icon_button(p, "compact-close", Icon::X, t!("compact.close"), None).on_click(cx.listener(move |shell, _, _, cx| shell.close_compact(session, cx))))
    });
    let card = h_flex()
        .id("compaction")
        .w_full()
        .mb_3()
        .px_4()
        .py_3()
        .gap_3()
        .items_start()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .aria_label(said.title)
        .aria_description(said.body)
        .child(div().flex_none().pt(px(1.)).child(icon(glyph, 20., tone)))
        .child(text)
        .children(actions);
    // It comes in with a fade and a small rise; motion reduced, a fade alone.
    let id = ("compaction-in", matches!(compact, Compact::Running { .. }) as u64 | (matches!(compact, Compact::Failed) as u64) << 1);
    Some(
        card.with_animation(id, Animation::new(if reduce { IN_REDUCED } else { IN }).with_easing(motion::ease_out), move |el, t| {
            let el = el.opacity(t);
            if reduce { el } else { el.top(px(6. * (1. - t))) }
        })
        .into_any_element(),
    )
}

/// The flowing bar: a piece a third of the track wide runs along it,
/// left to right and again. Motion reduced, the piece stands.
fn bar(p: Palette, tone: Rgba, reduce: bool) -> impl IntoElement {
    let piece = div().absolute().top_0().h_full().w(relative(PIECE)).rounded_full().bg(tone);
    let piece = if reduce {
        piece.left(relative(PIECE_AT_REST)).into_any_element()
    } else {
        // Redrawn no more than 20 times a second, as the other loops are.
        piece.with_animation("compaction-flow", Animation::new(LOOP).repeat().with_max_fps(20.), |el, t| el.left(relative(t * (1. - PIECE)))).into_any_element()
    };
    div().relative().w_full().h(px(4.)).mt_1().overflow_hidden().rounded_full().bg(p.wash).child(piece)
}

/// The note of what waits behind the summary.
fn chip(p: Palette, words: String) -> impl IntoElement {
    div().px_2().py(px(2.)).rounded_full().bg(p.wash).text_size(px(META)).line_height(px(META_LINE)).text_color(p.text2).child(words)
}

/// The divider the conversation keeps where a summary finished.
pub fn divider(p: Palette, tone: Rgba) -> impl IntoElement {
    h_flex()
        .w_full()
        .h(px(24.))
        .gap_2()
        .items_center()
        .text_size(px(META))
        .text_color(p.muted)
        .child(div().h(px(1.)).flex_1().bg(p.hair))
        .child(icon(Icon::Check, 12., tone))
        .child(t!("compact.divider"))
        .child(div().h(px(1.)).flex_1().bg(p.hair))
}

impl Shell {
    /// The band closed: a failure read, or about to be tried again.
    pub fn close_compact(&mut self, session: equipe_core::protocol::SessionId, cx: &mut Context<Self>) {
        if let Some(convo) = self.model.convos.get_mut(&session) {
            convo.close_compact();
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{Compact, queued, shown, words};
    use std::time::{Duration, Instant};

    fn running(manual: bool) -> Compact {
        Compact::Running { manual, since: Instant::now() }
    }

    #[test]
    fn the_band_says_what_the_summary_is_and_that_it_takes_long_after_a_minute() {
        assert_eq!(words(&running(false), Duration::from_secs(3)).title, "대화를 요약하고 있어요");
        assert_eq!(words(&running(true), Duration::from_secs(3)).title, "요약해서 새로 시작하고 있어요");
        assert_eq!(words(&running(false), Duration::from_secs(59)).title, "대화를 요약하고 있어요");
        assert_eq!(words(&running(true), Duration::from_secs(60)).title, "요약이 오래 걸리고 있어요");
        assert_eq!(words(&Compact::Done { at: Instant::now() }, Duration::ZERO).body, "이어서 대화할 수 있어요.");
        assert_eq!(words(&Compact::Failed, Duration::ZERO).body, "대화는 그대로예요. 다시 시도할까요?");
    }

    #[test]
    fn done_goes_after_four_seconds_a_failure_stays_and_nothing_gives_up_on_a_long_one() {
        let at = Instant::now();
        assert!(shown(&Compact::Done { at }, at + Duration::from_secs(3)));
        assert!(!shown(&Compact::Done { at }, at + Duration::from_secs(4)));
        assert!(shown(&Compact::Failed, at + Duration::from_secs(3600)));
        assert!(shown(&Compact::Running { manual: false, since: at }, at + Duration::from_secs(3600)));
    }

    #[test]
    fn what_waits_is_counted() {
        assert_eq!(queued(1), "메시지 1개 대기 중 · 요약이 끝나면 보낼게요");
        assert_eq!(queued(3), "메시지 3개 대기 중 · 요약이 끝나면 보낼게요");
    }
}
