//! The spec's colour tokens (T-54 §2.1) and the type sizes, light and dark.
//! Dark and light follow the system appearance.

use gpui_kit::*;

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
    pub text2: Rgba,
    pub muted: Rgba,
    pub hair: Rgba,
    pub accent: Rgba,
    pub accent_bg: Rgba,
    pub code_bg: Rgba,
    pub raised: Rgba,
    /// The main button: the text colour on the background's, inverted.
    pub primary: Rgba,
    pub on_primary: Rgba,
    /// Work under way.
    pub amber: Rgba,
    /// Blocked: waits for other work.
    pub orange: Rgba,
    /// Something went wrong.
    pub red: Rgba,
    members: [Rgba; 8],
}

impl Palette {
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
            text2: rgb(0xCFCCC4),
            muted: rgb(0x7C7E85),
            hair: rgba(0xE9E6DF24),
            accent: rgb(0xB4CDE6),
            accent_bg: rgba(0xB4CDE61A),
            code_bg: rgb(0x111114),
            raised: rgb(0x141417),
            primary: rgb(0xEFEDE7),
            on_primary: rgb(0x0A0A0C),
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
            text2: rgb(0x3A3C42),
            muted: rgb(0x6A6C74),
            hair: rgba(0x14141424),
            accent: rgb(0x2E5C8A),
            accent_bg: rgba(0x2E5C8A17),
            code_bg: rgb(0xECEBE6),
            raised: rgb(0xFFFFFF),
            primary: rgb(0x15161A),
            on_primary: rgb(0xF6F5F1),
            amber: rgb(0xA8700F),
            orange: rgb(0xB8551C),
            red: rgb(0xC0392F),
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
/// Korean body text is never below 13; reading text is 15/24.
pub const BODY: f32 = 15.;
pub const UI: f32 = 14.;
pub const SMALL: f32 = 13.;

/// Project chips take the project's order and these colours (T-54 §2.1).
pub const PROJECT_HUES: [usize; 8] = [2, 6, 4, 7, 5, 1, 3, 0];
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
}
