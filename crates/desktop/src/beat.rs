//! One clock for what moves while nothing else happens: the sidebar's
//! spinners, the live dots, the times that count up, an undo's seconds.
//!
//! Each asks it while it is drawn, for the view it is drawn in; at the next
//! step that changes it, that view — and only that — is drawn again (with
//! the views around it, which GPUI draws again too; the rest of the window
//! is cached: `Pane`). Nothing drawn asks, nothing is drawn: a hidden window
//! draws nothing, so asks nothing, and the clock stops. Every ask falls on one
//! grid of 100 ms steps, so what turns in several views turns in one frame.
//! Before, a 10 Hz ticker the board started drew the whole window for as long
//! as anyone worked, on every screen (`docs/PERF-2026-10.md`, 6).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// A step of the clock, about 30 a second: the sidebar's spinners turn a
/// thirty-sixth each — smooth to the eye (a twelfth at 10 a second read as
/// jerky), still only the spinner's own view drawn again.
pub const STEP: Duration = Duration::from_micros(33_334);
/// Steps in a spinner's turn: 1.2 s, as before.
const TURN: u64 = 36;

/// Who is to be drawn again, and when.
#[derive(Debug, Default)]
pub struct Beats {
    due: HashMap<EntityId, Instant>,
}

impl Beats {
    /// Something drawn in `view` reads differently at `at`: draw it again
    /// then (at the first step from it) — or sooner, if asked sooner.
    pub fn wake(&mut self, view: EntityId, at: Instant, epoch: Instant) {
        let at = on_grid(epoch, at);
        self.due.entry(view).and_modify(|due| *due = (*due).min(at)).or_insert(at);
    }

    /// Something turning in `view`, changing every `every` steps: draw it
    /// again at the next such step after `now`.
    pub fn turn(&mut self, view: EntityId, every: u32, epoch: Instant, now: Instant) {
        let every = u64::from(every.max(1));
        let next = (step_at(epoch, now) / every + 1) * every;
        self.wake(view, epoch + STEP * next as u32, epoch);
    }

    /// When a frame is next wanted; `None` when nothing turns or counts —
    /// then the clock doesn't run.
    pub fn next(&self) -> Option<Instant> {
        self.due.values().min().copied()
    }

    /// The views due by `now`, taken off: drawn again, they ask again.
    pub fn take_due(&mut self, now: Instant) -> Vec<EntityId> {
        let due: Vec<EntityId> = self.due.iter().filter(|(_, at)| **at <= now).map(|(view, _)| *view).collect();
        for view in &due {
            self.due.remove(view);
        }
        due
    }
}

/// The step the clock is at.
pub fn step_at(epoch: Instant, now: Instant) -> u64 {
    (now.saturating_duration_since(epoch).as_nanos() / STEP.as_nanos()) as u64
}

/// `at`, put off to the clock's next step (or kept, on one).
fn on_grid(epoch: Instant, at: Instant) -> Instant {
    let since = at.saturating_duration_since(epoch).as_nanos();
    let steps = since.div_ceil(STEP.as_nanos());
    epoch + Duration::from_nanos((steps * STEP.as_nanos()) as u64)
}

/// How far a spinner has turned at a step: a thirty-sixth a step, 1.2 s a turn.
pub fn spin(step: u64) -> f32 {
    (step % TURN) as f32 * std::f32::consts::TAU / TURN as f32
}

/// How strong a live dot is at a step: a soft pulse, a new strength every
/// other step.
pub fn pulse(step: u64) -> f32 {
    [1.0, 0.75, 0.45, 0.3, 0.45, 0.75][(step / 6 % 6) as usize]
}

/// How long until a time counting up (`45s`, `25m`, `3h`, `2d` —
/// `state::short_span`) reads differently, `elapsed` into it: a second at
/// most, the rest of the minute once it counts minutes, and so on.
pub fn label_changes_in(elapsed: Duration) -> Duration {
    let unit = match elapsed.as_secs() {
        0..=59 => 1,
        60..=3599 => 60,
        3600..=86_399 => 3600,
        _ => 86_400,
    };
    let unit = Duration::from_secs(unit);
    unit - Duration::from_nanos((elapsed.as_nanos() % unit.as_nanos()) as u64)
}

/// The clock: the app's one, for every window.
pub struct Beat {
    beats: Beats,
    epoch: Instant,
    /// When the running timer fires.
    at: Option<Instant>,
    _timer: Option<Task<()>>,
}

struct TheBeat(Entity<Beat>);

impl gpui_kit::Global for TheBeat {}

impl Beat {
    fn get(cx: &mut App) -> Entity<Beat> {
        if let Some(clock) = cx.try_global::<TheBeat>() {
            return clock.0.clone();
        }
        let clock = cx.new(|_| Beat { beats: Beats::default(), epoch: Instant::now(), at: None, _timer: None });
        cx.set_global(TheBeat(clock.clone()));
        clock
    }

    /// Something turning drawn in `view`, changing every `every` steps: the
    /// step it is at now. Under reduce motion it stands still (step 0).
    pub fn turning(view: EntityId, every: u32, cx: &mut App) -> u64 {
        if cx.reduce_motion() {
            return 0;
        }
        Self::get(cx).update(cx, |clock, cx| {
            let now = Instant::now();
            clock.beats.turn(view, every, clock.epoch, now);
            clock.arm(cx);
            step_at(clock.epoch, now)
        })
    }

    /// Something drawn in `view` that reads differently at `at` (a time
    /// counting up, an undo's seconds).
    pub fn wake(view: EntityId, at: Instant, cx: &mut App) {
        Self::get(cx).update(cx, |clock, cx| {
            clock.beats.wake(view, at, clock.epoch);
            clock.arm(cx);
        });
    }

    /// A timer for the next frame wanted, unless one fires by then already.
    fn arm(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.beats.next() else { return };
        if self.at.is_some_and(|at| at <= next) {
            return;
        }
        self.at = Some(next);
        self._timer = Some(cx.spawn(async move |this, cx| {
            let mut next = next;
            loop {
                cx.background_executor().timer(next.saturating_duration_since(Instant::now())).await;
                let again = this.update(cx, |clock, cx| {
                    for view in clock.beats.take_due(Instant::now()) {
                        let app: &mut App = cx;
                        app.notify(view);
                    }
                    clock.at = clock.beats.next();
                    clock.at
                });
                match again {
                    Ok(Some(at)) => next = at,
                    _ => break,
                }
            }
        }));
    }
}

impl crate::Shell {
    /// The view the shell's state is being drawn into now: a pane, or the shell.
    fn drawing(&self, cx: &Context<Self>) -> EntityId {
        self.panes.drawing.get().unwrap_or_else(|| cx.entity_id())
    }

    /// A spinner drawn now: how far it has turned. At its next step the part
    /// of the window it is in is drawn again.
    pub fn spin(&self, cx: &mut Context<Self>) -> f32 {
        spin(Beat::turning(self.drawing(cx), 1, cx))
    }

    /// A live dot drawn now: how strong it is, the same way.
    pub fn pulse(&self, cx: &mut Context<Self>) -> f32 {
        pulse(Beat::turning(self.drawing(cx), 6, cx))
    }

    /// A time counting up drawn now, `elapsed` into it, as `short_span` says
    /// it: drawn again when it reads differently.
    pub fn counting(&self, elapsed: Duration, cx: &mut Context<Self>) -> String {
        Beat::wake(self.drawing(cx), Instant::now() + label_changes_in(elapsed), cx);
        crate::state::short_span(elapsed.as_secs())
    }

    /// Something drawn now that reads differently at `at` (an undo's seconds).
    pub fn wake_at(&self, at: Instant, cx: &mut Context<Self>) {
        Beat::wake(self.drawing(cx), at, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{Beats, STEP, label_changes_in, pulse, spin, step_at};
    use gpui_kit::EntityId;
    use std::time::{Duration, Instant};

    fn view(n: u64) -> EntityId {
        EntityId::from(n)
    }

    #[test]
    fn nothing_turning_wants_no_frame() {
        let epoch = Instant::now();
        let mut beats = Beats::default();
        assert_eq!(beats.next(), None, "nothing drawn asked");
        beats.turn(view(1), 1, epoch, epoch + STEP / 3);
        assert_eq!(beats.next(), Some(epoch + STEP), "a spinner drawn: the next step");
        assert_eq!(beats.take_due(epoch + STEP - Duration::from_millis(1)), [], "not yet");
        assert_eq!(beats.take_due(epoch + STEP), [view(1)]);
        assert_eq!(beats.next(), None, "it didn't ask again (it wasn't drawn): the clock stops");
    }

    #[test]
    fn what_turns_in_several_views_turns_in_one_frame() {
        let epoch = Instant::now();
        let mut beats = Beats::default();
        beats.turn(view(1), 1, epoch, epoch + STEP * 3 + STEP / 10);
        beats.turn(view(2), 1, epoch, epoch + STEP * 3 + STEP * 7 / 10);
        let at = epoch + STEP * 4;
        assert_eq!(beats.next(), Some(at));
        let mut due = beats.take_due(at);
        due.sort();
        assert_eq!(due, [view(1), view(2)]);
    }

    #[test]
    fn a_pulse_every_sixth_step_asks_for_the_next_sixth() {
        let epoch = Instant::now();
        let mut beats = Beats::default();
        beats.turn(view(1), 6, epoch, epoch + STEP * 7);
        assert_eq!(beats.next(), Some(epoch + STEP * 12));
        assert_ne!(pulse(5), pulse(6));
        assert_eq!(pulse(6), pulse(11), "the dot holds for six steps (~200 ms), as it did");
        // A spinner in the same view wants it sooner.
        beats.turn(view(1), 1, epoch, epoch + STEP * 7);
        assert_eq!(beats.next(), Some(epoch + STEP * 8));
    }

    #[test]
    fn a_wake_falls_on_a_step() {
        let epoch = Instant::now();
        let mut beats = Beats::default();
        beats.wake(view(1), epoch + STEP * 37 + STEP / 2, epoch);
        assert_eq!(beats.next(), Some(epoch + STEP * 38));
        beats.wake(view(1), epoch + Duration::from_secs(60), epoch);
        assert_eq!(beats.next(), Some(epoch + STEP * 38), "the sooner one stands");
    }

    #[test]
    fn the_spinner_turns_smoothly_once_in_1_2_seconds() {
        let epoch = Instant::now();
        let turn = step_at(epoch, epoch + Duration::from_millis(1205));
        assert_eq!(turn, 36, "36 steps a turn, ~30 a second");
        assert_eq!(spin(turn), 0.);
        assert!((spin(9) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(spin(1) < std::f32::consts::TAU / 30., "small steps: no visible jump");
    }

    #[test]
    fn a_time_counting_up_changes_once_a_second_at_most() {
        let s = Duration::from_secs;
        let ms = Duration::from_millis;
        assert_eq!(label_changes_in(ms(12_300)), ms(700), "45s: at the next whole second");
        assert_eq!(label_changes_in(s(59) + ms(500)), ms(500), "59s → 1m");
        assert_eq!(label_changes_in(s(60)), s(60), "1m: in a minute");
        assert_eq!(label_changes_in(s(25 * 60 + 10)), s(50), "25m: at the next whole minute");
        assert_eq!(label_changes_in(s(3 * 3600 + 120)), s(3600 - 120), "3h: at the next whole hour");
        assert_eq!(label_changes_in(s(2 * 86_400)), s(86_400), "2d: in a day");
        for at in (0..7200).map(s) {
            assert!(label_changes_in(at) >= s(1), "never more than once a second");
            let next = at + label_changes_in(at);
            assert_ne!(crate::state::short_span(next.as_secs()), crate::state::short_span(at.as_secs()), "{at:?}: it does change then");
            if next > at + s(1) {
                let before = next - s(1);
                assert_eq!(crate::state::short_span(before.as_secs()), crate::state::short_span(at.as_secs()), "{at:?}: and not before");
            }
        }
    }
}
