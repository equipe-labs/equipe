//! Service marks for the Logins › Connections list: each service's own logo
//! where one is embedded, else a monogram (initials on a colour tile), so a
//! row is recognised at a glance.
//!
//! The logos are single-path SVGs from Simple Icons (CC0, pinned in
//! `THIRD_PARTY_NOTICES.md`), embedded under `assets/brand/` and drawn like
//! any other icon: as a mask tinted in one colour, so they stay crisp at 1x
//! and 2x and take the brand colour in light mode and a readable colour in
//! dark. The marks are their owners' trademarks, shown only to identify the
//! service. A brand Simple Icons does not carry (AWS, Mobbin, 21st.dev) and
//! any server added by address get a monogram: offline and deterministic
//! (the colour comes from the name), nothing is drawn by hand or fetched.

use equipe_core::connections::{CATALOG, Service};
use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::icons::icon_from_asset;
use crate::theme::Palette;

/// What a row's tile shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// The brand's logo (an embedded asset) in its official colour (`0xRRGGBB`).
    Glyph { asset: &'static str, brand: u32 },
    /// Initials on a colour tile.
    Monogram { text: String, brand: u32 },
}

/// The embedded logos: asset path, bytes. Simple Icons' files, unchanged
/// (OpenAI's is the one `Icon::OpenAiMark` draws).
const GLYPHS: [(&str, &[u8]); 8] = [
    ("brand/gmail.svg", include_bytes!("../assets/brand/gmail.svg")),
    ("brand/anthropic.svg", include_bytes!("../assets/brand/anthropic.svg")),
    ("brand/openai.svg", include_bytes!("../assets/brand/openai.svg")),
    ("brand/github.svg", include_bytes!("../assets/brand/github.svg")),
    ("brand/vercel.svg", include_bytes!("../assets/brand/vercel.svg")),
    ("brand/posthog.svg", include_bytes!("../assets/brand/posthog.svg")),
    ("brand/figma.svg", include_bytes!("../assets/brand/figma.svg")),
    ("brand/framer.svg", include_bytes!("../assets/brand/framer.svg")),
];

/// A logo's bytes, for the app's asset source.
pub fn asset(path: &str) -> Option<&'static [u8]> {
    GLYPHS.iter().find(|(p, _)| *p == path).map(|(_, bytes)| *bytes)
}

/// The mailbox row's id: Gmail's mark (the row also reads any IMAP mailbox).
pub const MAILBOX: &str = "mailbox";

/// A catalog service's mark (or `MAILBOX`'s), by id. `None`: not one of ours.
pub fn catalog_mark(id: &str) -> Option<Mark> {
    let glyph = |asset, brand| Some(Mark::Glyph { asset, brand });
    let mono = |text: &str, brand| Some(Mark::Monogram { text: text.to_owned(), brand });
    match id {
        MAILBOX => glyph("brand/gmail.svg", 0xEA4335),
        "anthropic" => glyph("brand/anthropic.svg", 0x191919),
        "openai" => glyph("brand/openai.svg", 0x000000),
        "github" => glyph("brand/github.svg", 0x181717),
        "vercel" => glyph("brand/vercel.svg", 0x000000),
        "posthog" => glyph("brand/posthog.svg", 0x000000),
        "figma" => glyph("brand/figma.svg", 0xF24E1E),
        "framer" => glyph("brand/framer.svg", 0x0055FF),
        // Not in Simple Icons: AWS's orange, and the near-black the other
        // two sites wear.
        "aws" => mono("aws", 0xFF9900),
        "mobbin" => mono("M", 0x1A1A1A),
        "21st" => mono("21", 0x111111),
        _ => None,
    }
}

/// The mark for a row: the catalog's, else a monogram from `name`.
pub fn mark_of(service: Option<&Service>, name: &str) -> Mark {
    service.and_then(|s| catalog_mark(s.id)).unwrap_or_else(|| Mark::Monogram { text: initials(name), brand: hue_of(name) })
}

/// One or two capitals from a name: the first letters of its first two words
/// (`Acme Docs` as `AD`), else its first letter.
pub fn initials(name: &str) -> String {
    let mut words = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    let text: String = first.into_iter().chain(second).flat_map(char::to_uppercase).collect();
    if text.is_empty() { "?".to_owned() } else { text }
}

/// The monogram colours custom names draw from: mid-deep hues that carry
/// white initials.
const HUES: [u32; 8] = [0x2563EB, 0x7C3AED, 0xDB2777, 0x0F766E, 0xB45309, 0x4D7C0F, 0xBE123C, 0x475569];

/// A colour for a name: the same name, the same colour, on every machine.
pub fn hue_of(name: &str) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for byte in name.trim().to_lowercase().bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    HUES[(hash as usize) % HUES.len()]
}

// ───────────────────────── colour ─────────────────────────

fn channels(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xFF) as f32 / 255., ((hex >> 8) & 0xFF) as f32 / 255., (hex & 0xFF) as f32 / 255.]
}

fn luminance(c: [f32; 3]) -> f32 {
    let lin = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio of two colours (`0xRRGGBB`), 1 to 21.
pub fn contrast(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(channels(a)), luminance(channels(b)));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The least contrast a logo needs against its tile (WCAG's bar for graphics).
pub const GLYPH_CONTRAST: f32 = 3.0;

const WHITE: u32 = 0xFFFFFF;
const BLACK: u32 = 0x000000;

/// Whichever of white and black reads better on `surface`.
pub fn ink_on(surface: u32) -> u32 {
    if contrast(WHITE, surface) >= contrast(BLACK, surface) { WHITE } else { BLACK }
}

/// The colour a logo is drawn in on `surface`: its brand colour when that
/// stands apart from the tile, else white or black (a black brand in dark
/// mode turns white; a pale one in light mode turns black).
pub fn glyph_color(brand: u32, surface: u32) -> u32 {
    if contrast(brand, surface) >= GLYPH_CONTRAST { brand } else { ink_on(surface) }
}

/// A monogram's tile fill and initials colour on `surface`: the brand fill
/// with the clearer of white and black on it — unless the fill nearly
/// vanishes into the surface (black in dark mode), then the surface's own
/// ink as the fill and the other ink on it.
pub fn monogram_colors(brand: u32, surface: u32) -> (u32, u32) {
    if contrast(brand, surface) < 1.6 {
        let fill = ink_on(surface);
        (fill, if fill == WHITE { BLACK } else { WHITE })
    } else {
        (brand, ink_on(brand))
    }
}

fn hex_of(c: Rgba) -> u32 {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    (byte(c.r) << 16) | (byte(c.g) << 8) | byte(c.b)
}

/// The colour a tile's wash makes over the card (`raised`), as a hex.
fn surface(p: Palette) -> u32 {
    let (under, over) = (p.raised, p.wash);
    let mix = |u: f32, o: f32| u * (1. - over.a) + o * over.a;
    hex_of(Rgba { r: mix(under.r, over.r), g: mix(under.g, over.g), b: mix(under.b, over.b), a: 1. })
}

// ───────────────────────── drawing ─────────────────────────

/// The tile's side, in points.
pub const TILE: f32 = 32.;

/// A row's tile: the logo on the subtle wash, or the monogram.
pub fn tile(p: Palette, mark: &Mark) -> Div {
    let base = div().flex_none().size(px(TILE)).flex().items_center().justify_center().rounded(px(8.));
    let on = surface(p);
    match mark {
        Mark::Glyph { asset, brand } => base.bg(p.wash).child(icon_from_asset(asset, 18., rgb(glyph_color(*brand, on)))),
        Mark::Monogram { text, brand } => {
            let (fill, ink) = monogram_colors(*brand, on);
            // One or two capitals, a little smaller for the longer text.
            let size = if text.chars().count() > 2 { 10. } else if text.chars().count() == 2 { 12.5 } else { 14. };
            base.bg(rgb(fill)).child(h_flex().child(div().text_size(px(size)).font_weight(FontWeight::BOLD).text_color(rgb(ink)).child(text.clone())))
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob brings its own `test` macro over `#[test]`.
    use super::{BLACK, CATALOG, GLYPHS, GLYPH_CONTRAST, HUES, MAILBOX, Mark, Service, WHITE};
    use super::{catalog_mark, contrast, glyph_color, hue_of, initials, mark_of, monogram_colors, surface};
    use crate::icons::AppAssets;
    use crate::theme::Palette;
    use gpui_kit::AssetSource as _;
    use gpui_kit::WindowAppearance;

    #[test]
    fn every_catalog_service_and_the_mailbox_has_a_logo_or_a_monogram() {
        for id in CATALOG.iter().map(|s| s.id).chain([MAILBOX]) {
            assert!(catalog_mark(id).is_some(), "{id} has no mark");
        }
        // Simple Icons carries no AWS, Mobbin or 21st.dev: a monogram, not a hand-drawn logo.
        for id in ["aws", "mobbin", "21st"] {
            assert!(matches!(catalog_mark(id), Some(Mark::Monogram { .. })), "{id}");
        }
        for id in ["anthropic", "openai", "github", "vercel", "posthog", "figma", "framer", MAILBOX] {
            assert!(matches!(catalog_mark(id), Some(Mark::Glyph { .. })), "{id}");
        }
    }

    #[test]
    fn every_logo_is_embedded_as_one_path_and_loads_through_the_app_assets() {
        for (path, bytes) in GLYPHS {
            let text = std::str::from_utf8(bytes).expect("utf-8");
            assert!(text.starts_with("<svg") && text.matches("<path").count() == 1, "{path}");
            assert!(AppAssets.load(path).expect("loads").is_some(), "{path}");
        }
        for service in CATALOG {
            if let Some(Mark::Glyph { asset, .. }) = catalog_mark(service.id) {
                assert!(super::asset(asset).is_some(), "{asset}");
            }
        }
    }

    #[test]
    fn a_server_added_by_address_gets_a_steady_monogram_from_its_name() {
        assert_eq!(initials("docs"), "D");
        assert_eq!(initials("acme-docs"), "AD");
        assert_eq!(initials("  my.cool_server x"), "MC");
        assert_eq!(initials("---"), "?");
        assert_eq!(hue_of("docs"), hue_of(" Docs "), "case and edges do not matter");
        assert!(HUES.contains(&hue_of("anything")));
        let mark = mark_of(None, "Acme Docs");
        assert_eq!(mark, Mark::Monogram { text: "AD".into(), brand: hue_of("Acme Docs") });
        assert!(matches!(mark_of(Service::of("github"), "ignored"), Mark::Glyph { .. }));
    }

    #[test]
    fn contrast_follows_wcag_and_picks_a_readable_logo_colour() {
        assert!((contrast(WHITE, BLACK) - 21.).abs() < 0.01);
        assert!((contrast(0x777777, WHITE) - 4.48).abs() < 0.05);
        let (light, dark) = (surface(Palette::new(WindowAppearance::Light)), surface(Palette::new(WindowAppearance::Dark)));
        // Black brands turn white in dark mode and stay themselves in light.
        for black in [0x000000, 0x181717, 0x191919] {
            assert_eq!(glyph_color(black, light), black);
            assert_eq!(glyph_color(black, dark), WHITE, "{black:06X}");
        }
        // A brand colour that already reads keeps its colour in both modes.
        for colour in [0xEA4335, 0xF24E1E] {
            assert_eq!(glyph_color(colour, light), colour);
            assert_eq!(glyph_color(colour, dark), colour);
        }
        // Framer's blue is too dim on the dark tile: white there.
        assert_eq!(glyph_color(0x0055FF, light), 0x0055FF);
        assert_eq!(glyph_color(0x0055FF, dark), WHITE);
        // Whatever the colour, what is drawn clears the bar on each tile (or is the best ink).
        for (_, brand) in CATALOG.iter().filter_map(|s| match catalog_mark(s.id)? {
            Mark::Glyph { asset, brand } => Some((asset, brand)),
            Mark::Monogram { .. } => None,
        }) {
            for surface in [light, dark] {
                assert!(contrast(glyph_color(brand, surface), surface) >= GLYPH_CONTRAST, "{brand:06X} on {surface:06X}");
            }
        }
    }

    #[test]
    fn a_monogram_stays_readable_and_visible_in_both_modes() {
        let (light, dark) = (surface(Palette::new(WindowAppearance::Light)), surface(Palette::new(WindowAppearance::Dark)));
        let mut brands: Vec<u32> = HUES.to_vec();
        brands.extend([0xFF9900, 0x1A1A1A, 0x111111]);
        for brand in brands {
            for on in [light, dark] {
                let (fill, ink) = monogram_colors(brand, on);
                assert!(contrast(fill, ink) >= 4.5, "{brand:06X} on {on:06X}: initials {ink:06X} on {fill:06X}");
                assert!(contrast(fill, on) >= 1.6, "{brand:06X} on {on:06X}: the tile vanishes");
            }
        }
        // Black in dark mode turns into a light tile with dark initials.
        assert_eq!(monogram_colors(0x111111, dark), (WHITE, BLACK));
        assert_eq!(monogram_colors(0xFF9900, dark), (0xFF9900, BLACK));
    }
}
