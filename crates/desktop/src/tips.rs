//! Tooltips, as macOS and Claude show them: half a second after the pointer
//! settles on an icon-only control, a small dark chip beside it — its name,
//! and its shortcut in a quieter tone. Moving to the next control shows the
//! next one at once; a click puts it away.
//!
//! GPUI Kit's `TooltipOverlay` owns the timing (the delay, the grace period
//! between controls) and the placement (above or below the control, flipped
//! when there is no room, kept inside the window); this file gives it the
//! window's look, a fade, and a way to ask for it from any of the app's views.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::{ElementExt as _, Placement, h_flex, TooltipOverlay, TooltipRequest, TooltipTransition};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::motion;
use crate::theme::{GAP, Palette, SMALL};

/// The window's tooltip layer (one window, one layer).
struct Tips(Entity<TooltipOverlay>);

impl Global for Tips {}

/// Make the layer; the shell draws it over everything (`layer`).
pub fn init(cx: &mut App) -> Entity<TooltipOverlay> {
    let overlay = cx.new(|_| TooltipOverlay::new().render_with(render));
    cx.set_global(Tips(overlay.clone()));
    overlay
}

/// The layer, for the shell to draw last.
pub fn layer(cx: &App) -> Option<Entity<TooltipOverlay>> {
    cx.try_global::<Tips>().map(|t| t.0.clone())
}

/// The chip, faded in; a tooltip moved to the next control is just there.
fn render(view: AnyView, transition: TooltipTransition, _: &mut Window, _: &mut App) -> AnyElement {
    // The padding is the gap between the control and the chip.
    let chip = div().p(px(GAP)).child(view);
    match transition {
        TooltipTransition::Enter { epoch } => motion::fade(chip, ElementId::NamedInteger("tip-in".into(), epoch as u64), motion::FAST).into_any_element(),
        TooltipTransition::Switch { .. } => chip.into_any_element(),
    }
}

/// What a tooltip says.
struct Tip {
    p: Palette,
    label: SharedString,
    keys: Option<SharedString>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.p;
        // A long label (a ticket's whole title) wraps within a readable width.
        let long = self.label.chars().count() > 48;
        h_flex()
            .min_h(px(24.))
            .when(long, |chip| chip.max_w(px(380.)).py(px(4.)))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(crate::theme::R_SM))
            .bg(p.tip_bg())
            .border_1()
            .border_color(rgba(0xFFFFFF14))
            .shadow(motion::float_shadow(0.6))
            .text_size(px(SMALL - 1.))
            .when(!long, |chip| chip.whitespace_nowrap())
            .text_color(p.tip_text())
            .child(div().when(long, |d| d.flex_1().min_w_0()).child(self.label.clone()))
            .when_some(self.keys.clone(), |chip, keys| chip.child(div().flex_none().whitespace_nowrap().text_color(p.tip_muted()).child(keys)))
    }
}

/// `el` with a tooltip: `label`, and the shortcut that does the same.
/// Placed above the control (below when there's no room above).
pub fn tip<E: StatefulInteractiveElement + ParentElement + 'static>(el: E, p: Palette, label: impl Into<SharedString>, keys: Option<&str>) -> E {
    tip_at(el, p, label, keys, Placement::Top)
}

/// The same, preferring `placement`.
pub fn tip_at<E: StatefulInteractiveElement + ParentElement + 'static>(el: E, p: Palette, label: impl Into<SharedString>, keys: Option<&str>, placement: Placement) -> E {
    let label: SharedString = label.into();
    let keys: Option<SharedString> = keys.map(|k| SharedString::from(k.to_owned()));
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let seen = bounds.clone();
    el.on_prepaint(move |b, _, _| seen.set(b))
        .on_hover(move |hovered, window, cx| {
            let Some(overlay) = cx.try_global::<Tips>().map(|t| t.0.clone()) else { return };
            if *hovered {
                let (label, keys) = (label.clone(), keys.clone());
                let request = TooltipRequest::new(bounds.get(), move |_, cx| cx.new(|_| Tip { p, label: label.clone(), keys: keys.clone() }).into()).placement(placement);
                overlay.update(cx, |o, cx| o.request_show(request, window, cx));
            } else {
                overlay.update(cx, |o, cx| o.request_hide(window, cx));
            }
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| hide(cx))
}

/// The same, as a method: `.tip(p, "Search", Some("⌘K"))`, `.tip_at(…, Placement::Right)`.
pub trait TipExt: StatefulInteractiveElement + ParentElement + Sized + 'static {
    fn tip(self, p: Palette, label: impl Into<SharedString>, keys: Option<&str>) -> Self {
        tip(self, p, label, keys)
    }

    fn tip_at(self, p: Palette, label: impl Into<SharedString>, keys: Option<&str>, placement: Placement) -> Self {
        tip_at(self, p, label, keys, placement)
    }
}

impl<E: StatefulInteractiveElement + ParentElement + 'static> TipExt for E {}

/// Put any tooltip away now (a click, a menu opening).
pub fn hide(cx: &mut App) {
    if let Some(overlay) = cx.try_global::<Tips>().map(|t| t.0.clone()) {
        overlay.update(cx, |o, cx| o.hide(cx));
    }
}
