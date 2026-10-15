//! Zoom, as a browser (or Claude) zooms a page: ⌘+ and ⌘- step through a few
//! sizes, ⌘0 is actual size. Everything grows together — text, icons,
//! spacing, pictures, the QR code — because GPUI draws the whole window at a
//! larger scale (`Window::set_content_zoom`, an `equipe:` patch in the vendored
//! gpui-pre): text is laid out again and rasterized at the new size, not a
//! bitmap stretched, and clicks land where they look. The size is remembered
//! (`target.rs`) and shown for a moment in a small chip near the window's top.
//!
//! The traffic lights are AppKit's and keep their size; they move down to stay
//! centred on the (taller or shorter) title bar, and the strip's own buttons
//! start after them whatever the size (`views::sidebar::lights_at`).

use std::time::{Duration, Instant};

use gpui_kit::*;

use crate::target::Zoom;
use crate::theme::{R_MD, UI};
use crate::{Shell, motion};

/// The sizes, in percent, as browsers offer them.
pub const LEVELS: [u16; 10] = Zoom::LEVELS;
/// Actual size.
pub const ACTUAL: u16 = Zoom::ACTUAL.0;

/// What a key (or the menu, or settings) asks of the size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    In,
    Out,
    Actual,
}

/// The size a step lands on from `now`: the next size up or down, staying in
/// the range; from a size that isn't one of the steps, the nearest step that
/// way.
pub fn step(now: u16, step: Step) -> u16 {
    match step {
        Step::Actual => ACTUAL,
        Step::In => LEVELS.iter().copied().find(|&l| l > now).unwrap_or(LEVELS[LEVELS.len() - 1]),
        Step::Out => LEVELS.iter().rev().copied().find(|&l| l < now).unwrap_or(LEVELS[0]),
    }
}

/// `pct` as one of the steps: the nearest (a hand-edited file, an old size).
pub fn clamp(pct: u16) -> u16 {
    Zoom::nearest(pct).0
}

/// How large each point is drawn.
pub fn factor(pct: u16) -> f32 {
    f32::from(clamp(pct)) / 100.
}

/// What the chip and settings show: `110%`.
pub fn label(pct: u16) -> String {
    format!("{pct}%")
}

/// Draw the window at `pct`, and put the traffic lights where the zoomed
/// title bar has its centre line.
pub fn apply(pct: u16, window: &mut Window) {
    let zoom = factor(pct);
    window.set_content_zoom(zoom);
    let (x, y) = crate::views::sidebar::lights_at(zoom);
    window.set_traffic_light_position(point(px(x), px(y)));
}

// ── the chip ──

/// The chip fades in over `FAST`, stays this long after the last change,
/// then fades out over `BASE`.
const HOLD: Duration = Duration::from_millis(900);

/// The chip on screen: since when, and the last change (presses in a row
/// keep it up without fading it in again).
#[derive(Debug, Clone, Copy)]
pub struct Hud {
    pub pct: u16,
    since: Instant,
    last: Instant,
}

impl Hud {
    /// How visible the chip is at `now` (0–1), and whether it is gone.
    pub fn opacity(&self, now: Instant, reduce: bool) -> Option<f32> {
        let held = now.saturating_duration_since(self.last);
        if held >= HOLD + motion::BASE {
            return None;
        }
        if reduce {
            return Some(if held < HOLD { 1. } else { 0. });
        }
        let fade_in = motion::ease_out(now.saturating_duration_since(self.since).as_secs_f32() / motion::FAST.as_secs_f32());
        let fade_out = motion::ease_out(held.saturating_sub(HOLD).as_secs_f32() / motion::BASE.as_secs_f32());
        Some(fade_in * (1. - fade_out))
    }
}

impl Shell {
    /// ⌘+, ⌘-, ⌘0, the menu's and settings' stepper.
    pub fn zoom(&mut self, how: Step, window: &mut Window, cx: &mut Context<Self>) {
        let next = step(self.remembered.zoom.0, how);
        self.set_zoom(next, window, cx);
    }

    /// Draw the window at `pct`, remember it, and show it for a moment.
    pub fn set_zoom(&mut self, pct: u16, window: &mut Window, cx: &mut Context<Self>) {
        let pct = clamp(pct);
        let now = Instant::now();
        // The chip shows even when the size can't go further, so the press is seen.
        self.zoom_hud = Some(match self.zoom_hud {
            Some(hud) if hud.opacity(now, false).is_some_and(|o| o > 0.) => Hud { pct, since: hud.since, last: now },
            _ => Hud { pct, since: now, last: now },
        });
        if pct != self.remembered.zoom.0 {
            self.remembered.zoom = Zoom(pct);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            apply(pct, window);
            self.sync_browser(window, cx);
        }
        crate::tips::hide(cx);
        cx.notify();
    }

    /// The chip, centred near the window's top, while it shows.
    pub fn zoom_chip(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hud = self.zoom_hud?;
        let Some(opacity) = hud.opacity(Instant::now(), cx.reduce_motion()) else {
            self.zoom_hud = None;
            return None;
        };
        // Frames until it has faded out (and one more to take it away).
        window.request_animation_frame();
        let p = self.palette;
        let chip = div()
            .px(px(14.))
            .py(px(6.))
            .rounded(px(R_MD))
            .bg(p.tip_bg())
            .border_1()
            .border_color(rgba(0xFFFFFF14))
            .shadow(motion::float_shadow(0.6 * opacity * opacity * opacity))
            .text_size(px(UI))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.tip_text())
            .child(label(hud.pct));
        Some(
            div()
                .absolute()
                .top(px(crate::views::sidebar::STRIP + 16.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .opacity(opacity)
                .child(chip)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{HOLD, Hud, LEVELS, Step, clamp, factor, label, motion, step};
    use std::time::Instant;

    #[test]
    fn steps_walk_the_levels_and_stop_at_the_ends() {
        assert_eq!(step(100, Step::In), 110);
        assert_eq!(step(100, Step::Out), 90);
        assert_eq!(step(67, Step::Out), 67, "the smallest stays");
        assert_eq!(step(200, Step::In), 200, "the largest stays");
        assert_eq!(step(175, Step::Actual), 100);
        let mut up = vec![67];
        while *up.last().unwrap() != 200 {
            up.push(step(*up.last().unwrap(), Step::In));
        }
        assert_eq!(up, LEVELS.to_vec(), "⌘+ visits every size once, in order");
        // A size between steps goes to the next one that way.
        assert_eq!((step(105, Step::In), step(105, Step::Out)), (110, 100));
    }

    #[test]
    fn a_size_off_the_steps_is_taken_to_the_nearest() {
        assert_eq!(clamp(100), 100);
        assert_eq!(clamp(0), 67);
        assert_eq!(clamp(500), 200);
        assert_eq!(clamp(112), 110);
        assert_eq!(factor(150), 1.5);
        assert_eq!(label(110), "110%");
    }

    #[test]
    fn the_chip_fades_in_holds_and_goes() {
        let t0 = Instant::now();
        let hud = Hud { pct: 110, since: t0, last: t0 };
        assert_eq!(hud.opacity(t0, false), Some(0.));
        assert_eq!(hud.opacity(t0 + motion::FAST, false), Some(1.));
        assert_eq!(hud.opacity(t0 + HOLD, false), Some(1.));
        assert!(hud.opacity(t0 + HOLD + motion::BASE / 2, false).is_some_and(|o| o > 0. && o < 1.));
        assert_eq!(hud.opacity(t0 + HOLD + motion::BASE, false), None);
        // Reduce motion: there, then not.
        assert_eq!(hud.opacity(t0, true), Some(1.));
        assert_eq!(hud.opacity(t0 + HOLD + motion::BASE / 2, true), Some(0.));
    }
}
