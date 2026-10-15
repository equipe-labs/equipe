//! The app's one button: primary (filled), secondary (a hairline) or ghost
//! (nothing until the pointer comes), 28 tall — or 36 on a field's line — with
//! the same radius everywhere, an optional icon and key, a fill that eases in
//! under the pointer, and a tooltip when it has only an icon.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::base::h_flex;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, FIELD, ICON, Palette, R_MD, SMALL};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
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
    on_click: Option<OnClick>,
}

/// A button with words.
pub fn button(p: Palette, id: impl Into<ElementId>, kind: Kind, label: impl Into<SharedString>) -> Button {
    Button { id: id.into(), p, kind, label: Some(label.into()), icon: None, key: None, tip: None, height: CONTROL, disabled: false, on_click: None }
}

/// A square button with only an icon; its name shows as a tooltip.
pub fn icon_button(p: Palette, id: impl Into<ElementId>, glyph: Icon, name: impl Into<SharedString>, keys: Option<&'static str>) -> Button {
    Button { id: id.into(), p, kind: Kind::Ghost, label: None, icon: Some(glyph), key: None, tip: Some((name.into(), keys)), height: CONTROL, disabled: false, on_click: None }
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

    /// Dimmed, and deaf to clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = self.p;
        let (bg, fg, border) = match self.kind {
            Kind::Primary => (Some(p.primary), p.on_primary, None),
            Kind::Secondary => (None, p.text, Some(p.hair)),
            Kind::Ghost => (None, p.text2, None),
        };
        let (fg, bg) = if self.disabled { (p.muted, bg.map(|_| p.wash)) } else { (fg, bg) };
        let square = self.label.is_none();
        let b = h_flex()
            .id(self.id.clone())
            .h(px(self.height))
            .flex_none()
            .items_center()
            .justify_center()
            .gap_1p5()
            .rounded(px(R_MD))
            .text_size(px(SMALL))
            .text_color(fg)
            .whitespace_nowrap()
            .when(square, |b| b.w(px(self.height)))
            .when(!square, |b| b.px_3())
            .when(self.kind == Kind::Primary, |b| b.font_weight(FontWeight::SEMIBOLD))
            .when_some(bg, |b, bg| b.bg(bg))
            .when_some(border, |b, c| b.border_1().border_color(c))
            .when(!self.disabled, |b| b.cursor_pointer())
            .children(self.icon.map(|g| icon(g, ICON, fg)))
            .children(self.label)
            .children(self.key.map(|k| div().text_size(px(12.)).opacity(0.6).child(k)))
            .when_some(self.on_click.filter(|_| !self.disabled), |b, f| b.on_click(move |e, w, cx| f(e, w, cx)));
        let b = match self.tip {
            Some((name, keys)) => crate::tips::tip(b, p, name, keys),
            None => b,
        };
        let id = self.id;
        match self.kind {
            // A filled button eases a little lighter instead of washing over.
            Kind::Primary if !self.disabled => motion::hover_with(b, id, false, |b, t| b.opacity(1. - 0.14 * t)),
            Kind::Primary => motion::hover_with(b, id, false, |b, _| b),
            _ => motion::hover_fill(b, id, if self.disabled { rgba(0) } else { p.wash }, false),
        }
    }
}
