//! Motion: three durations and one curve, and the few effects the window is
//! built from — a popover arriving from what opened it, a dialog settling, a
//! card or a message rising into place, a tooltip fading in, a hover fill
//! easing on and off.
//!
//! GPUI has no element transform (no scale), and `opacity` multiplies into each
//! primitive rather than a composited layer — so a shadow is ramped in after
//! its surface (by the cube of the fade), or it shows through mid-fade as a
//! dark slab. Everything here goes through GPUI's animation element or checks
//! `reduce_motion`, so "Reduce motion" (Accessibility → Display) shows each
//! end state at once.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// Hover fills, tooltips, a press.
pub const FAST: Duration = Duration::from_millis(120);
/// Popovers, menus, chips, the jump button.
pub const BASE: Duration = Duration::from_millis(160);
/// Dialogs, the lightbox, cards and messages arriving.
pub const SLOW: Duration = Duration::from_millis(200);

/// The sidebar sliding in or out (⌘\), and the header's content beside it.
pub const SLIDE: Duration = Duration::from_millis(180);

/// The one curve: cubic ease-out — quick to start, soft to land.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1. - (1. - t).powi(3)
}

/// The slide's curve: quintic ease-out, the panel leaving at once and
/// settling softly.
pub fn slide(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1. - (1. - t).powi(5)
}

/// Where in time the slide reaches `v`: a slide turned back midway starts
/// from where the panel stands.
pub fn slide_inverse(v: f32) -> f32 {
    1. - (1. - v.clamp(0., 1.)).powf(0.2)
}

/// How far a popover or a tooltip travels toward its anchor.
const POP_TRAVEL: f32 = 4.;
/// How far a card or a message rises as it arrives.
const RISE_TRAVEL: f32 = 6.;
/// How far a dialog settles.
const DIALOG_TRAVEL: f32 = 8.;

/// The resting shadow of what floats, at `ink` (0–1) of its strength.
pub fn float_shadow(ink: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow { color: hsla(0., 0., 0., 0.22 * ink), offset: point(px(0.), px(8.)), blur_radius: px(24.), spread_radius: px(-4.), inset: false },
        BoxShadow { color: hsla(0., 0., 0., 0.10 * ink), offset: point(px(0.), px(2.)), blur_radius: px(6.), spread_radius: px(0.), inset: false },
    ]
}

/// A popover or a menu: it fades in, travels a few points from its anchor
/// (`above`: it opened above its trigger, so it rises), its shadow last.
pub fn pop<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, above: bool) -> AnimationElement<E> {
    let from = if above { POP_TRAVEL } else { -POP_TRAVEL };
    el.with_animation(id, Animation::new(BASE).with_easing(ease_out), move |el, t| el.top(px(from * (1. - t))).opacity(t).shadow(float_shadow(t * t * t)))
}

/// A dialog: the same, a little slower and from a little further.
pub fn dialog<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>) -> AnimationElement<E> {
    el.with_animation(id, Animation::new(SLOW).with_easing(ease_out), move |el, t| el.top(px(DIALOG_TRAVEL * (1. - t))).opacity(t).shadow(float_shadow(t * t * t)))
}

/// What arrives in a column (a card, a message, an attachment): it fades in
/// and rises into place.
pub fn rise<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>) -> AnimationElement<E> {
    el.with_animation(id, Animation::new(SLOW).with_easing(ease_out), move |el, t| el.top(px(RISE_TRAVEL * (1. - t))).opacity(t))
}

/// A plain fade in.
pub fn fade<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, duration: Duration) -> AnimationElement<E> {
    el.with_animation(id, Animation::new(duration).with_easing(ease_out), move |el, t| el.opacity(t))
}

// ── hover, eased ──

/// Where an eased value stands: from, to, since when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    from: f32,
    to: f32,
    since: Instant,
}

impl Ease {
    pub fn at(value: f32, now: Instant) -> Self {
        Self { from: value, to: value, since: now }
    }

    /// The value at `now`, over `FAST`.
    pub fn value(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.since).as_secs_f32() / FAST.as_secs_f32();
        self.from + (self.to - self.from) * ease_out(t)
    }

    pub fn settled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= FAST
    }

    /// Head for `target` from wherever it is now.
    pub fn toward(&mut self, target: f32, now: Instant) {
        if self.to != target {
            *self = Self { from: self.value(now), to: target, since: now };
        }
    }
}

struct HoverState {
    hovered: Rc<Cell<bool>>,
    ease: Ease,
}

/// An element whose look follows the pointer over it, eased: `style` gets
/// the element and how hovered it is (0–1). `on` holds it at 1 (a row picked
/// with the keyboard, a menu's open trigger).
pub struct Hover<E> {
    id: ElementId,
    element: Option<E>,
    on: bool,
    style: Box<dyn Fn(E, f32) -> E>,
}

/// `el`, its background `fill` eased in under the pointer (or while `on`).
pub fn hover_fill<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, fill: Rgba, on: bool) -> Hover<E> {
    Hover { id: id.into(), element: Some(el), on, style: Box::new(move |el, t| if t <= 0. { el } else { el.bg(Rgba { a: fill.a * t, ..fill }) }) }
}

/// `el` restyled by `style` as the pointer comes and goes.
pub fn hover_with<E: IntoElement + 'static>(el: E, id: impl Into<ElementId>, on: bool, style: impl Fn(E, f32) -> E + 'static) -> Hover<E> {
    Hover { id: id.into(), element: Some(el), on, style: Box::new(style) }
}

impl<E: IntoElement + 'static> IntoElement for Hover<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: IntoElement + 'static> Element for Hover<E> {
    type RequestLayoutState = (AnyElement, Rc<Cell<bool>>);
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, global_id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
        let now = Instant::now();
        let reduce = cx.reduce_motion();
        let on = self.on;
        let (value, running, hovered) = window.with_element_state(global_id.expect("Hover has an id"), |state: Option<HoverState>, _| {
            let mut state = state.unwrap_or_else(|| HoverState { hovered: Rc::default(), ease: Ease::at(if on { 1. } else { 0. }, now) });
            let target = if state.hovered.get() || on { 1. } else { 0. };
            state.ease.toward(target, now);
            if reduce {
                state.ease = Ease::at(target, now);
            }
            let value = state.ease.value(now);
            let running = !state.ease.settled(now);
            ((value, running, state.hovered.clone()), state)
        });
        if running {
            window.request_animation_frame();
        }
        let element = self.element.take().expect("laid out once");
        let mut element = (self.style)(element, value).into_any_element();
        (element.request_layout(window, cx), (element, hovered))
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, state: &mut Self::RequestLayoutState, window: &mut Window, cx: &mut App) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        state.0.prepaint(window, cx);
        hitbox
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, state: &mut Self::RequestLayoutState, hitbox: &mut Self::PrepaintState, window: &mut Window, cx: &mut App) {
        state.0.paint(window, cx);
        let hovered = state.1.clone();
        hovered.set(hitbox.is_hovered(window));
        let hitbox = hitbox.clone();
        let view = window.current_view();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
            let now = hitbox.is_hovered(window);
            if phase == DispatchPhase::Capture && now != hovered.get() {
                hovered.set(now);
                cx.notify(view);
            }
        });
    }
}

// ── height, eased ──

struct HeightState {
    /// The child's own height, as last laid out.
    natural: Option<Pixels>,
    ease: Ease,
}

/// A box that follows its child's height smoothly: when the child grows or
/// shrinks (a line more in the composer), the box eases to the new height
/// over `FAST`, clipping the child meanwhile.
pub struct SmoothHeight {
    id: ElementId,
    child: AnyElement,
}

impl SmoothHeight {
    pub fn new(id: impl Into<ElementId>, child: AnyElement) -> Self {
        Self { id: id.into(), child }
    }
}

impl IntoElement for SmoothHeight {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SmoothHeight {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, global_id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
        let now = Instant::now();
        let reduce = cx.reduce_motion();
        let (height, running) = window.with_element_state(global_id.expect("SmoothHeight has an id"), |state: Option<HeightState>, _| {
            let mut state = state.unwrap_or(HeightState { natural: None, ease: Ease::at(0., now) });
            let out = match state.natural {
                Some(natural) => {
                    state.ease.toward(f32::from(natural), now);
                    if reduce {
                        state.ease = Ease::at(f32::from(natural), now);
                    }
                    (Some(px(state.ease.value(now))), !state.ease.settled(now))
                }
                // Not measured yet: its own height.
                None => (None, false),
            };
            (out, state)
        });
        if running {
            window.request_animation_frame();
        }
        let child = self.child.request_layout(window, cx);
        // Block layout: the child keeps its own height whatever the box's.
        let mut style = Style { display: Display::Block, ..Style::default() };
        style.overflow.y = Overflow::Hidden;
        if let Some(height) = height {
            style.size.height = height.into();
        }
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(&mut self, global_id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, child: &mut Self::RequestLayoutState, window: &mut Window, cx: &mut App) -> Self::PrepaintState {
        let natural = window.layout_bounds(*child).size.height;
        let changed = window.with_element_state(global_id.expect("SmoothHeight has an id"), |state: Option<HeightState>, _| {
            let mut state = state.unwrap_or(HeightState { natural: None, ease: Ease::at(f32::from(natural), Instant::now()) });
            let first = state.natural.is_none();
            let changed = state.natural != Some(natural);
            if first {
                state.ease = Ease::at(f32::from(natural), Instant::now());
            }
            state.natural = Some(natural);
            (changed && !first, state)
        });
        if changed {
            window.request_animation_frame();
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| self.child.prepaint(window, cx));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut Self::RequestLayoutState, _: &mut Self::PrepaintState, window: &mut Window, cx: &mut App) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| self.child.paint(window, cx));
    }
}

// ── opening and closing in place ──

/// How far a disclosure keyed `id` is open (0–1), easing over `BASE` toward
/// `open` from wherever it was; frames are asked for until it settles. The
/// first time it is asked it is where it should be, so rows scrolled back
/// into view don't play it again.
pub fn openness(id: impl Into<ElementId>, open: bool, window: &mut Window, cx: &mut App) -> f32 {
    let now = Instant::now();
    let target = if open { 1. } else { 0. };
    let state = window.use_keyed_state(id, cx, |_, _| Slide { from: target, to: target, since: now });
    let reduce = cx.reduce_motion();
    let (value, settled) = state.update(cx, |slide, _| {
        if slide.to != target {
            *slide = Slide { from: slide.value(now), to: target, since: now };
        }
        if reduce {
            *slide = Slide { from: target, to: target, since: now };
        }
        (slide.value(now), slide.settled(now))
    });
    if !settled {
        window.request_animation_frame();
    }
    value
}

/// An eased value over `BASE`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    from: f32,
    to: f32,
    since: Instant,
}

impl Slide {
    pub fn value(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.since).as_secs_f32() / BASE.as_secs_f32();
        self.from + (self.to - self.from) * ease_out(t)
    }

    pub fn settled(&self, now: Instant) -> bool {
        self.from == self.to || now.saturating_duration_since(self.since) >= BASE
    }
}

struct RevealState {
    natural: Option<Pixels>,
}

/// What a disclosure shows, at `progress` (0–1) of its own height, clipped:
/// it grows open and shrinks shut. The caller fades it with the same value.
pub struct Reveal {
    id: ElementId,
    child: AnyElement,
    progress: f32,
}

impl Reveal {
    pub fn new(id: impl Into<ElementId>, progress: f32, child: AnyElement) -> Self {
        Self { id: id.into(), child, progress: progress.clamp(0., 1.) }
    }
}

impl IntoElement for Reveal {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Reveal {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, global_id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState) {
        let natural = window.with_element_state(global_id.expect("Reveal has an id"), |state: Option<RevealState>, _| {
            let state = state.unwrap_or(RevealState { natural: None });
            (state.natural, state)
        });
        let child = self.child.request_layout(window, cx);
        let mut style = Style { display: Display::Block, ..Style::default() };
        style.overflow.y = Overflow::Hidden;
        if self.progress < 1. {
            style.size.height = px(f32::from(natural.unwrap_or(px(0.))) * self.progress).into();
        }
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(&mut self, global_id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, child: &mut Self::RequestLayoutState, window: &mut Window, cx: &mut App) -> Self::PrepaintState {
        let natural = window.layout_bounds(*child).size.height;
        let unknown = window.with_element_state(global_id.expect("Reveal has an id"), |state: Option<RevealState>, _| {
            let unknown = state.as_ref().is_none_or(|s| s.natural.is_none());
            (unknown, RevealState { natural: Some(natural) })
        });
        // Opening from nothing measured: the next frame knows how tall.
        if unknown && self.progress > 0. && self.progress < 1. {
            window.request_animation_frame();
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| self.child.prepaint(window, cx));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut Self::RequestLayoutState, _: &mut Self::PrepaintState, window: &mut Window, cx: &mut App) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| self.child.paint(window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::{Ease, FAST, ease_out, slide, slide_inverse};
    use std::time::{Duration, Instant};

    #[test]
    fn a_slide_turned_back_starts_where_it_stands() {
        assert_eq!(slide(0.), 0.);
        assert_eq!(slide(1.), 1.);
        for v in [0., 0.1, 0.5, 0.9, 1.] {
            assert!((slide(slide_inverse(v)) - v).abs() < 1e-4, "{v}");
        }
    }

    #[test]
    fn the_curve_eases_out_and_stays_in_range() {
        assert_eq!(ease_out(0.), 0.);
        assert_eq!(ease_out(1.), 1.);
        assert!(ease_out(0.25) > 0.5, "most of the way in the first quarter");
        assert_eq!(ease_out(-1.), 0.);
        assert_eq!(ease_out(2.), 1.);
    }

    #[test]
    fn a_hover_fill_turns_back_from_where_it_is() {
        let t0 = Instant::now();
        let mut ease = Ease::at(0., t0);
        ease.toward(1., t0);
        let half = t0 + FAST / 2;
        let midway = ease.value(half);
        assert!(midway > 0.5 && midway < 1.);
        // The pointer leaves halfway: it fades from there, not from 1.
        ease.toward(0., half);
        assert!((ease.value(half) - midway).abs() < 1e-6);
        assert!(!ease.settled(half + FAST / 2));
        assert_eq!(ease.value(half + FAST + Duration::from_millis(1)), 0.);
        assert!(ease.settled(half + FAST));
    }
}
