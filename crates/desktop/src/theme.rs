//! The spec's colour tokens (T-54 §2.1) and the type sizes, light and dark.
//! Dark and light follow the system appearance, unless the person picked one
//! in settings.

use gpui_kit::*;

/// Light or dark, as the person picked in settings (kept with the window's
/// other choices, in `target.rs`); the Mac's by default.
pub use crate::target::Appearance;

impl Appearance {
    /// What the window draws in, given what the Mac says.
    pub fn resolve(self, system: WindowAppearance) -> WindowAppearance {
        match self {
            Appearance::System => system,
            Appearance::Light => WindowAppearance::Light,
            Appearance::Dark => WindowAppearance::Dark,
        }
    }

    /// Make the app itself light or dark — the traffic lights, menus and
    /// Sparkle's windows too — or hand it back to the Mac (`System`).
    pub fn apply(self) {
        #[cfg(target_os = "macos")]
        {
            use objc2::msg_send;
            use objc2::runtime::{AnyClass, AnyObject};
            use objc2_foundation::NSString;
            let (Some(app_class), Some(appearance_class)) = (AnyClass::get(c"NSApplication"), AnyClass::get(c"NSAppearance")) else { return };
            // SAFETY: AppKit's `+[NSApplication sharedApplication]`,
            // `+[NSAppearance appearanceNamed:]` and `-[NSApplication setAppearance:]`
            // (nil follows the system), on the main thread, where the app runs.
            unsafe {
                let app: *mut AnyObject = msg_send![app_class, sharedApplication];
                if app.is_null() {
                    return;
                }
                let look: *mut AnyObject = match self {
                    Appearance::System => std::ptr::null_mut(),
                    Appearance::Light => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameAqua")],
                    Appearance::Dark => msg_send![appearance_class, appearanceNamed: &*NSString::from_str("NSAppearanceNameDarkAqua")],
                };
                let _: () = msg_send![app, setAppearance: look];
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Rgba,
    pub side: Rgba,
    /// A sidebar row under the pointer, and the one open: solid, so a title
    /// can fade into them.
    pub side_hover: Rgba,
    pub side_sel: Rgba,
    pub wash: Rgba,
    pub text: Rgba,
    /// Reading text — replies, your messages: a shade softer than `text`,
    /// which stays for titles and controls. Pure white on near-black glares
    /// and makes long Korean text look heavy.
    pub prose: Rgba,
    pub text2: Rgba,
    pub muted: Rgba,
    pub hair: Rgba,
    pub accent: Rgba,
    pub accent_bg: Rgba,
    pub code_bg: Rgba,
    pub raised: Rgba,
    /// The main action's fill (a button, the composer's send): in the dark a
    /// deep slate of the accent's hue, a step above the cards, with the text
    /// colour on it — emphasis that belongs to the window rather than a white
    /// slab; in the light, the text colour, inverted.
    pub primary: Rgba,
    pub on_primary: Rgba,
    /// Words or a mark on an `accent` fill (a count badge, a tick).
    pub on_accent: Rgba,
    /// Work under way.
    pub amber: Rgba,
    /// Blocked: waits for other work.
    pub orange: Rgba,
    /// Something went wrong.
    pub red: Rgba,
    members: [Rgba; 8],
}

impl Palette {
    /// Give the kit's text fields this palette's caret and selection, and its
    /// light or dark: left alone they drew the kit's default caret, which on
    /// our dark fields hardly showed.
    pub fn apply_to_kit(&self, appearance: WindowAppearance, cx: &mut gpui_kit::App) {
        use gpui_kit::component::{Theme, ThemeMode};
        let dark = matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark);
        let caret: gpui_kit::Hsla = self.text.into();
        let mut selection: gpui_kit::Hsla = self.accent.into();
        selection.a = if dark { 0.38 } else { 0.28 };
        Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
        Theme::update(cx, |theme| {
            theme.colors.caret = caret;
            theme.colors.selection = selection;
        });
    }

    pub fn new(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    fn dark() -> Self {
        Self {
            bg: rgb(0x0A0A0C),
            side: rgb(0x0E0E11),
            side_hover: rgb(0x19191D),
            side_sel: rgb(0x222227),
            wash: rgba(0xFFFFFF0E),
            text: rgb(0xEFEDE7),
            prose: rgb(0xE4E2DC),
            text2: rgb(0xCFCCC4),
            // At least 4.5:1 (WCAG AA, small text) on every surface it sits
            // on, the selected sidebar row and the popover included.
            muted: rgb(0x8A8C93),
            hair: rgba(0xE9E6DF24),
            accent: rgb(0xB4CDE6),
            accent_bg: rgba(0xB4CDE61A),
            code_bg: rgb(0x111114),
            raised: rgb(0x141417),
            primary: rgb(0x2F4256),
            on_primary: rgb(0xEFEDE7),
            on_accent: rgb(0x0A0A0C),
            amber: rgb(0xE2B865),
            orange: rgb(0xEC9A62),
            red: rgb(0xEE7B74),
            members: [0xE8BE78, 0xEC96BE, 0x78BEF0, 0x8CD096, 0xB6A2F2, 0x6ED4C8, 0xF0A06E, 0xCDD478].map(rgb),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xF6F5F1),
            side: rgb(0xEFEEE9),
            side_hover: rgb(0xE6E5DF),
            side_sel: rgb(0xDDDCD5),
            wash: rgba(0x0000000B),
            text: rgb(0x15161A),
            prose: rgb(0x1F2024),
            text2: rgb(0x3A3C42),
            muted: rgb(0x5E6068),
            hair: rgba(0x14141424),
            accent: rgb(0x2E5C8A),
            accent_bg: rgba(0x2E5C8A17),
            code_bg: rgb(0xECEBE6),
            raised: rgb(0xFFFFFF),
            primary: rgb(0x15161A),
            on_primary: rgb(0xF6F5F1),
            on_accent: rgb(0xF6F5F1),
            // Darker than the dark appearance's, to be read as words (a
            // status in the sidebar) at 4.5:1 on any surface.
            amber: rgb(0x825600),
            orange: rgb(0xA04610),
            red: rgb(0xAD2E27),
            members: [0xA06814, 0xB0326E, 0x1E68AA, 0x28803C, 0x6446BE, 0x14807A, 0xB4501E, 0x6E7814].map(rgb),
        }
    }

    pub fn member(&self, hue: usize) -> Rgba {
        self.members[hue % 8]
    }
}

/// A member's mark and colour index, the same as the terminal console's.
pub fn look(role: &str, name: &str) -> (&'static str, usize) {
    match role {
        "pm" => ("◆", 0),
        "designer" => ("✦", 1),
        "frontend" => ("●", 2),
        "backend" => ("■", 3),
        "qa" => ("◎", 4),
        "data" => ("▲", 5),
        "mobile" => ("◈", 6),
        _ => {
            const MARKS: [&str; 4] = ["◇", "★", "▼", "✚"];
            let h = name.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
            (MARKS[h % MARKS.len()], 6 + h % 2)
        }
    }
}

pub const SANS: &str = "Helvetica Neue";

// ── type ──
//
// Three sizes carry the app: reading text 15, the interface 14, secondary 13;
// meta (times, counts, "thought for 11s") 12. Korean is never below 12, and
// wants more leading than Latin: reading text is 15 on 25 (1.65).

/// Reading text: replies, your messages, the composer.
pub const BODY: f32 = 15.;
pub const BODY_LINE: f32 = 25.;
/// The interface: rows, buttons, menus.
pub const UI: f32 = 14.;
/// Text at the interface size read as text (a ticket's description, an ask).
pub const UI_PROSE_LINE: f32 = 23.;
/// Secondary text: notes, tool rows, thoughts, hints.
pub const SMALL: f32 = 13.;
pub const SMALL_LINE: f32 = 20.;
/// Meta: how long, when, how many.
pub const META: f32 = 12.;
pub const META_LINE: f32 = 18.;

/// The conversation's column, the composer's with it.
pub const COLUMN: f32 = 720.;
/// The measure: a reply's text is no wider than this — about 40 Korean
/// characters, or 75 Latin, at 15. Cards, the composer and the column are wider.
pub const MEASURE: f32 = 600.;
/// Between paragraphs: 0.7 of the body size.
pub const PARAGRAPH_GAP: f32 = 11.;
/// Between a list's items: tighter than paragraphs, so a list reads as one.
pub const LIST_GAP: f32 = 4.;
/// A heading's room: more above (beyond the paragraph gap) than below, so it
/// belongs to what follows.
pub const HEADING_ABOVE: f32 = 8.;
pub const HEADING_BELOW: f32 = 4.;
/// A link's underline: a hairline (one device pixel on a Retina screen), in
/// the link's colour at this opacity.
pub const LINK_UNDERLINE: f32 = 0.5;
pub const LINK_UNDERLINE_ALPHA: f32 = 0.5;
/// The room under the conversation's last line, over the composer.
pub const TAIL_ROOM: f32 = 20.;

/// Project chips take the project's order and these colours (T-54 §2.1) —
/// blue, violet, teal, pink, green: never the amber of work under way or the
/// orange of what waits on the person, which a project's dot would be read as.
pub const PROJECT_HUES: [usize; 5] = [2, 4, 5, 1, 3];
pub const MONO: &str = "Menlo";

// ── one scale for everything ──
//
// Space: 4 · 8 · 12 · 16 · 24 (GPUI's `_1` `_2` `_3` `_4` `_6`; S3 and S6 named where a
// padding has to be computed). Radii: 6 for chips and small buttons, 8 for
// controls, menus and rows, 12 for cards, 16 for the composer and dialogs.
// Controls are 28 tall with 16-point icons; a text field is 36; a menu or list
// row is 32. Motion is short and eases out (`crate::motion`).

pub const S3: f32 = 12.;
pub const S6: f32 = 24.;

pub const R_SM: f32 = 6.;
pub const R_MD: f32 = 8.;
pub const R_LG: f32 = 12.;
pub const R_XL: f32 = 16.;

/// Buttons, pickers and icon buttons.
pub const CONTROL: f32 = 28.;
/// A one-line text field, and the buttons on its line.
pub const FIELD: f32 = 36.;
/// A row of a menu, a list or a chooser.
pub const ROW: f32 = 32.;
pub const ICON: f32 = 16.;
pub const ICON_SM: f32 = 14.;
/// Popovers and tooltips sit this far from what opened them.
pub const GAP: f32 = 6.;

impl Palette {
    pub fn is_dark(&self) -> bool {
        self.bg.r < 0.5
    }

    /// What floats over the window: menus, popovers, the jump button.
    pub fn popover(&self) -> Rgba {
        if self.is_dark() { rgb(0x1B1B1F) } else { rgb(0xFFFFFF) }
    }

    /// A tooltip: a small dark chip in either appearance.
    pub fn tip_bg(&self) -> Rgba {
        if self.is_dark() { rgb(0x2E2E33) } else { rgb(0x232428) }
    }

    pub fn tip_text(&self) -> Rgba {
        rgb(0xF3F2EE)
    }

    pub fn tip_muted(&self) -> Rgba {
        rgb(0xA3A4AB)
    }

    /// What went wrong.
    pub fn danger(&self) -> Rgba {
        if self.is_dark() { rgb(0xF0918A) } else { rgb(0xB3261E) }
    }

    pub fn danger_bg(&self) -> Rgba {
        if self.is_dark() { rgba(0xF0918A1F) } else { rgba(0xB3261E14) }
    }

    // ── controls ──

    /// The main button under the pointer: a step lighter (dark) or softer (light).
    pub fn primary_hover(&self) -> Rgba {
        if self.is_dark() { rgb(0x384D63) } else { rgb(0x2C2E34) }
    }

    /// The main button held down: a step deeper.
    pub fn primary_press(&self) -> Rgba {
        if self.is_dark() { rgb(0x283849) } else { rgb(0x050506) }
    }

    /// A key hint inside the main button (⌘↵): the label's colour, held back.
    pub fn primary_key(&self) -> Rgba {
        mix(self.on_primary, self.primary, 0.3)
    }

    /// A secondary or ghost button held down: the hover wash, doubled.
    pub fn press(&self) -> Rgba {
        Rgba { a: (self.wash.a * 2.).min(1.), ..self.wash }
    }

    /// A switch that is on: the accent, deep enough to carry a white knob.
    pub fn switch_on(&self) -> Rgba {
        if self.is_dark() { rgb(0x4A76A0) } else { self.accent }
    }

    /// A switch that is off: a neutral track, a step off the card.
    pub fn switch_off(&self) -> Rgba {
        if self.is_dark() { rgb(0x3A3A40) } else { rgb(0xD9D8D2) }
    }

    /// The switch's knob, in either state.
    pub fn knob(&self) -> Rgba {
        if self.is_dark() { rgb(0xF3F2EE) } else { rgb(0xFFFFFF) }
    }
}

/// `a` moved `t` (0–1) of the way to `b`, alpha too.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0., 1.);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a) }
}

/// WCAG's contrast ratio of two colours (1 to 21), alpha ignored.
#[cfg(test)]
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let lum = |c: Rgba| {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::{Appearance, Palette, contrast};
    use gpui_kit::WindowAppearance;

    /// Every colour text is drawn in, on every surface it is drawn on, reads
    /// at WCAG AA for small text (4.5:1), in both appearances.
    #[test]
    fn text_colours_pass_aa_on_every_surface() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::Light] {
            let p = Palette::new(appearance);
            let surfaces = [("bg", p.bg), ("side", p.side), ("side_hover", p.side_hover), ("side_sel", p.side_sel), ("raised", p.raised), ("code_bg", p.code_bg), ("popover", p.popover())];
            let inks = [("text", p.text), ("prose", p.prose), ("text2", p.text2), ("muted", p.muted), ("accent", p.accent), ("danger", p.danger()), ("amber", p.amber), ("orange", p.orange), ("red", p.red)];
            for (surface, bg) in surfaces {
                for (ink, fg) in inks {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{ink} on {surface} ({appearance:?}): {ratio:.2}");
                }
            }
            assert!(contrast(p.tip_muted(), p.tip_bg()) >= 4.5 && contrast(p.tip_text(), p.tip_bg()) >= 4.5);
            assert!(contrast(p.on_primary, p.primary) >= 4.5);
            // Reading text is softer than titles, and still far past AAA (7:1).
            assert!(contrast(p.prose, p.bg) < contrast(p.text, p.bg) && contrast(p.prose, p.bg) >= 7.);
        }
    }

    #[test]
    fn a_picked_appearance_wins_over_the_macs_and_system_follows_it() {
        assert_eq!(Appearance::System.resolve(WindowAppearance::Dark), WindowAppearance::Dark);
        assert_eq!(Appearance::Light.resolve(WindowAppearance::VibrantDark), WindowAppearance::Light);
        assert!(Palette::new(Appearance::Dark.resolve(WindowAppearance::Light)).is_dark());
        assert!(Appearance::ALL.iter().all(|a| Appearance::parse(a.code()) == Some(*a)));
        assert_eq!(Appearance::parse("sepia"), None);
    }

    #[test]
    fn no_projects_colour_reads_as_a_status() {
        let hue = |c: gpui_kit::Rgba| gpui_kit::Hsla::from(c).h * 360.;
        let apart = |a: f32, b: f32| (a - b).abs().min(360. - (a - b).abs());
        for p in [Palette::new(WindowAppearance::Dark), Palette::new(WindowAppearance::Light)] {
            for at in 0..12 {
                let c = hue(p.member(super::PROJECT_HUES[at % super::PROJECT_HUES.len()]));
                for status in [p.orange, p.amber] {
                    assert!(apart(c, hue(status)) >= 40., "project {at}'s hue {c} is too near a status's {}", hue(status));
                }
            }
        }
    }
}
