//! The app's controls: one button and one switch.
//!
//! The button comes in four kinds, all 28 tall — or 36 on a field's line —
//! with the same radius and a 1px edge, so they line up whatever their kind:
//! - **Primary**, the one action a card or a dialog is for: a filled slate in
//!   the dark (a step above the cards, the text colour on it), the text colour
//!   inverted in the light. One to a place.
//! - **Secondary**, everything else that is a button: a hairline, nothing
//!   inside. Settings rows use these, as the Mac's System Settings does.
//! - **Ghost**: words alone, a wash under the pointer.
//! - **Danger**: what can't be taken back, in the danger colour with an edge
//!   of it.
//!
//! The fill eases in under the pointer (`motion::FAST`) and deepens while
//! held; a key hint (A, ⇧A, ⌘↵) sits after the words, quieter than them, in
//! every kind; an icon-only button shows its name as a tooltip. Disabled, it
//! is deaf to clicks: the outlined kinds at 40%, the primary a quiet wash.
//!
//! The switch: a neutral track with a light knob on the left when off; the
//! accent track with the knob on the right when on. The knob slides and the
//! track's colour crosses over `motion::BASE`, eased out; Tab reaches it,
//! Space or Return flips it, and a ring shows only when the keys moved there.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::base::h_flex;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, FIELD, ICON, Palette, R_MD, SMALL, mix};

/// The narrowest the words of a card's footer or a banner get beside its
/// buttons; narrower (a panel beside the conversation), the buttons go on a
/// line of their own rather than squeezing the words a letter to a line.
pub const WORDS_AT_LEAST: f32 = 200.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// How a kind of button is drawn: its fill (none: the surface shows), its
/// words, its key hint, its edge, and the fill under the pointer and held.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub fill: Option<Rgba>,
    pub text: Rgba,
    pub key: Rgba,
    pub edge: Rgba,
    pub hover: Rgba,
    pub press: Rgba,
}

impl Kind {
    pub fn look(self, p: &Palette) -> Look {
        let clear = rgba(0);
        match self {
            Kind::Primary => Look { fill: Some(p.primary), text: p.on_primary, key: p.primary_key(), edge: clear, hover: p.primary_hover(), press: p.primary_press() },
            Kind::Secondary => Look { fill: None, text: p.text, key: p.muted, edge: p.hair, hover: p.wash, press: p.press() },
            Kind::Ghost => Look { fill: None, text: p.text2, key: p.muted, edge: clear, hover: p.wash, press: p.press() },
            Kind::Danger => {
                let red = p.danger();
                Look { fill: None, text: red, key: Rgba { a: 0.7, ..red }, edge: Rgba { a: 0.45, ..red }, hover: p.danger_bg(), press: Rgba { a: (p.danger_bg().a * 1.8).min(1.), ..red } }
            }
        }
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    p: Palette,
    kind: Kind,
    label: Option<SharedString>,
    icon: Option<Icon>,
    key: Option<SharedString>,
    tip: Option<(SharedString, Option<&'static str>)>,
    height: f32,
    disabled: bool,
    selected: bool,
    focus: Option<FocusHandle>,
    on_click: Option<OnClick>,
}

/// A button with words.
pub fn button(p: Palette, id: impl Into<ElementId>, kind: Kind, label: impl Into<SharedString>) -> Button {
    Button { id: id.into(), p, kind, label: Some(label.into()), icon: None, key: None, tip: None, height: CONTROL, disabled: false, selected: false, focus: None, on_click: None }
}

/// A square button with only an icon; its name shows as a tooltip.
pub fn icon_button(p: Palette, id: impl Into<ElementId>, glyph: Icon, name: impl Into<SharedString>, keys: Option<&'static str>) -> Button {
    Button { id: id.into(), p, kind: Kind::Ghost, label: None, icon: Some(glyph), key: None, tip: Some((name.into(), keys)), height: CONTROL, disabled: false, selected: false, focus: None, on_click: None }
}

impl Button {
    pub fn icon(mut self, glyph: Icon) -> Self {
        self.icon = Some(glyph);
        self
    }

    /// The key that does the same, quieter, after the words.
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// On a text field's line: as tall as the field.
    pub fn field(mut self) -> Self {
        self.height = FIELD;
        self
    }

    /// A tooltip saying more than the words do.
    pub fn tip(mut self, text: impl Into<SharedString>, keys: Option<&'static str>) -> Self {
        self.tip = Some((text.into(), keys));
        self
    }

    /// Dimmed, and deaf to clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// One of a few choices, and the one chosen: a tint of the accent with
    /// its edge, the words bolder — whatever its kind.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Reached by Tab through `focus`; Space or Return presses it, and a
    /// ring shows only when the keys moved there.
    pub fn focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = self.p;
        let mut look = self.kind.look(&p);
        let live = !self.disabled;
        // A filled button that can't be pressed gives up its fill for a wash,
        // rather than standing as a grey slab; the others fade to 40%.
        let faded = !live && look.fill.is_none();
        if !live && look.fill.is_some() {
            look = Look { fill: Some(p.wash), text: p.muted, key: p.muted, ..look };
        }
        if self.selected && live {
            let tint = |a: f32| Rgba { a, ..p.accent };
            look = Look { fill: Some(mix(p.raised, p.accent, 0.16)), text: p.text, key: p.text2, edge: tint(0.75), hover: mix(p.raised, p.accent, 0.24), press: mix(p.raised, p.accent, 0.3) };
        }
        let ring = Rgba { a: 0.55, ..p.accent };
        let square = self.label.is_none();
        let b = h_flex()
            .id(self.id.clone())
            .h(px(self.height))
            .flex_none()
            .items_center()
            .justify_center()
            .gap_1p5()
            .rounded(px(R_MD))
            .border_1()
            .border_color(look.edge)
            .text_size(px(SMALL))
            .text_color(look.text)
            .whitespace_nowrap()
            .when(square, |b| b.w(px(self.height)))
            .when(!square, |b| b.px_3())
            .when(self.kind == Kind::Primary || self.selected, |b| b.font_weight(FontWeight::SEMIBOLD))
            .when_some(look.fill, |b, fill| b.bg(fill))
            .when(faded, |b| b.opacity(0.4))
            .when(live, |b| b.cursor_pointer().active(move |s| s.bg(look.press)))
            .children(self.icon.map(|g| icon(g, ICON, look.text)))
            .children(self.label)
            .children(self.key.map(|k| div().text_size(px(12.)).font_weight(FontWeight::NORMAL).text_color(look.key).child(k)))
            .when_some(self.focus.filter(|_| live), |b, focus| {
                b.track_focus(&focus).focus_visible(move |s| s.shadow(vec![BoxShadow { color: ring.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }]))
            })
            .when_some(self.on_click.filter(|_| live), |b, f| b.on_click(move |e, w, cx| f(e, w, cx)));
        let b = match self.tip {
            Some((name, keys)) => crate::tips::tip(b, p, name, keys),
            None => b,
        };
        // Under the pointer the fill eases toward the hover fill (a filled
        // button) or the wash eases in (the others); disabled, nothing.
        motion::hover_with(b, self.id, false, move |b, t| match look.fill {
            _ if !live || t <= 0. => b,
            Some(fill) => b.bg(mix(fill, look.hover, t)),
            None => b.bg(Rgba { a: look.hover.a * t, ..look.hover }),
        })
    }
}

// ── the switch ──

/// The switch's size at 1x: a 36 × 20 track, a 16 knob, 2 in from the edge.
pub const SWITCH_W: f32 = 36.;
pub const SWITCH_H: f32 = 20.;
const KNOB: f32 = 16.;
const INSET: f32 = 2.;

type OnFlip = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Switch {
    id: SharedString,
    p: Palette,
    on: bool,
    disabled: bool,
    on_flip: Option<OnFlip>,
}

/// On or off. `id` must be unique on screen: it keys the slide and the focus.
pub fn switch(p: Palette, id: impl Into<SharedString>, on: bool) -> Switch {
    Switch { id: id.into(), p, on, disabled: false, on_flip: None }
}

impl Switch {
    /// At 40%, deaf to clicks and skipped by Tab.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new state when clicked, or Space/Return while focused.
    pub fn on_flip(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_flip = Some(Rc::new(f));
        self
    }
}

/// Where the knob's left edge stands, `t` (0 off – 1 on) of the way.
pub fn knob_left(t: f32) -> f32 {
    INSET + (SWITCH_W - KNOB - 2. * INSET) * t.clamp(0., 1.)
}

/// The track's colour, `t` of the way from off to on.
pub fn track(p: &Palette, t: f32) -> Rgba {
    mix(p.switch_off(), p.switch_on(), t)
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.p;
        let live = !self.disabled;
        let t = motion::openness(SharedString::from(format!("{}-slide", self.id)), self.on, window, cx);
        let focus = window.use_keyed_state(SharedString::from(format!("{}-focus", self.id)), cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone();
        let shadow = vec![
            BoxShadow { color: hsla(0., 0., 0., 0.28), offset: point(px(0.), px(1.)), blur_radius: px(2.5), spread_radius: px(0.), inset: false },
            BoxShadow { color: hsla(0., 0., 0., 0.08), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
        ];
        let knob = div().absolute().top(px((SWITCH_H - KNOB) / 2.)).left(px(knob_left(t))).size(px(KNOB)).rounded_full().bg(p.knob()).shadow(shadow);
        let ring = Rgba { a: 0.55, ..p.accent };
        let on = self.on;
        let (click, keys) = (self.on_flip.clone(), self.on_flip);
        div()
            .id(ElementId::Name(self.id.clone()))
            .relative()
            .flex_none()
            .w(px(SWITCH_W))
            .h(px(SWITCH_H))
            .rounded_full()
            .bg(track(&p, t))
            .when(!live, |d| d.opacity(0.4))
            .when(live, |d| {
                let clicked = focus.clone();
                d.track_focus(&focus)
                    .cursor_pointer()
                    // The ring only when the keys moved here, as the web's :focus-visible.
                    .focus_visible(move |s| s.shadow(vec![BoxShadow { color: ring.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }]))
                    .on_click(move |_, window, cx| {
                        clicked.focus(window, cx);
                        if let Some(f) = &click {
                            f(!on, window, cx);
                        }
                    })
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let k = &event.keystroke;
                        match k.key.as_str() {
                            "space" | "enter" if !k.modifiers.modified() => {
                                if let Some(f) = &keys {
                                    f(!on, window, cx);
                                }
                            }
                            "tab" if k.modifiers.shift => window.focus_prev(cx),
                            "tab" => window.focus_next(cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    })
            })
            .child(knob)
    }
}

/// Words that grow heavier when their control is the one in use, as wide
/// either way: a hidden copy at the heavy weight holds the room, so picking
/// a tab or a segment moves nothing beside it. `cut`: one line, an ellipsis
/// past the room it has.
pub fn steady(label: impl Into<SharedString>, heavy: FontWeight, on: bool, cut: bool) -> Div {
    let label: SharedString = label.into();
    let line = |d: Div| if cut { d.min_w_0().truncate() } else { d.whitespace_nowrap() };
    div()
        .flex()
        .flex_col()
        .when(cut, |d| d.min_w_0())
        .child(line(div()).when(on, |d| d.font_weight(heavy)).child(label.clone()))
        .child(line(div()).h_0().overflow_hidden().invisible().font_weight(heavy).child(label))
}

#[cfg(test)]
mod tests {
    use super::{Kind, SWITCH_W, knob_left, track};
    use crate::theme::{Palette, contrast, mix};
    use gpui_kit::WindowAppearance;

    /// What a button's words and key sit on: its own fill, or else each
    /// surface a button is drawn on, with the hover wash over it.
    fn grounds(p: &Palette, fill: Option<gpui_kit::Rgba>, hover: gpui_kit::Rgba) -> Vec<(&'static str, gpui_kit::Rgba)> {
        match fill {
            Some(fill) => vec![("fill", fill), ("hover", hover)],
            None => [("bg", p.bg), ("raised", p.raised), ("popover", p.popover()), ("code_bg", p.code_bg)]
                .into_iter()
                .flat_map(|(name, s)| [(name, s), ("hovered", mix(s, gpui_kit::Rgba { a: 1., ..hover }, hover.a))])
                .collect(),
        }
    }

    /// Every button's words read at 4.5:1 on what they sit on, hovered too; its
    /// key hint is quieter than the words yet still 3:1, in both appearances.
    #[test]
    fn button_labels_pass_aa_and_keys_stay_quieter() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            for kind in [Kind::Primary, Kind::Secondary, Kind::Ghost, Kind::Danger] {
                let look = kind.look(&p);
                // A key hint drawn with alpha lands on the ground; flatten it.
                for (ground, bg) in grounds(&p, look.fill, look.hover) {
                    let text = contrast(look.text, bg);
                    let key = contrast(mix(bg, gpui_kit::Rgba { a: 1., ..look.key }, look.key.a), bg);
                    assert!(text >= 4.5, "{kind:?} words on {ground} ({appearance:?}): {text:.2}");
                    assert!(key >= 3., "{kind:?} key on {ground} ({appearance:?}): {key:.2}");
                    assert!(key < text, "{kind:?} key as loud as its words on {ground} ({appearance:?})");
                }
            }
        }
    }

    /// The main button is calm in the dark: no lighter than the text it
    /// carries is dark, and nowhere near the white slab it was.
    #[test]
    fn the_dark_primary_is_a_fill_not_a_slab() {
        let p = Palette::new(WindowAppearance::Dark);
        assert!(contrast(p.primary, p.bg) < 3., "the main button glares: {:.2}", contrast(p.primary, p.bg));
        assert!(contrast(p.primary, p.raised) > 1.3, "the main button vanishes into the card");
    }

    /// The knob reads on the track when on (3:1, a control's part), and the
    /// on track reads against the surfaces the switch sits on.
    #[test]
    fn a_switch_reads_on_and_off() {
        for p in [Palette::new(WindowAppearance::Dark), Palette::new(WindowAppearance::Light)] {
            assert!(contrast(p.knob(), p.switch_on()) >= 3., "knob on the on track: {:.2}", contrast(p.knob(), p.switch_on()));
            for surface in [p.bg, p.raised] {
                assert!(contrast(p.switch_on(), surface) >= 3., "on track on its card: {:.2}", contrast(p.switch_on(), surface));
            }
        }
        assert_eq!(knob_left(0.), 2.);
        assert_eq!(knob_left(1.), SWITCH_W - 16. - 2.);
        let p = Palette::new(WindowAppearance::Dark);
        assert_eq!(track(&p, 0.), p.switch_off());
        assert_eq!(track(&p, 1.), p.switch_on());
    }
}

