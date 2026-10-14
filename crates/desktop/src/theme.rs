//! The spec's colour tokens (T-54 §2.1) and the type sizes, light and dark.
//! Dark and light follow the system appearance.

use gpui_kit::*;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Rgba,
    pub side: Rgba,
    pub wash: Rgba,
    pub text: Rgba,
    pub text2: Rgba,
    pub muted: Rgba,
    pub hair: Rgba,
    pub accent: Rgba,
    pub accent_bg: Rgba,
    pub code_bg: Rgba,
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
            wash: rgba(0xFFFFFF0E),
            text: rgb(0xEFEDE7),
            text2: rgb(0xCFCCC4),
            muted: rgb(0x7C7E85),
            hair: rgba(0xE9E6DF24),
            accent: rgb(0xB4CDE6),
            accent_bg: rgba(0xB4CDE61A),
            code_bg: rgb(0x111114),
            members: [0xE8BE78, 0xEC96BE, 0x78BEF0, 0x8CD096, 0xB6A2F2, 0x6ED4C8, 0xF0A06E, 0xCDD478].map(rgb),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xF6F5F1),
            side: rgb(0xEFEEE9),
            wash: rgba(0x0000000B),
            text: rgb(0x15161A),
            text2: rgb(0x3A3C42),
            muted: rgb(0x6A6C74),
            hair: rgba(0x14141424),
            accent: rgb(0x2E5C8A),
            accent_bg: rgba(0x2E5C8A17),
            code_bg: rgb(0xECEBE6),
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
