//! Contrast, as WCAG 2.x reckons it: relative luminance, the ratio of two
//! colours (a translucent one laid over what it is on first), and the grade
//! — AAA at 7:1, AA at 4.5:1, and 3:1 for large text and graphics. And which
//! of a design system's colours is its background, and which its text, to
//! measure the others against.

use equipe_protocol::design::Token;

use super::values::{Color, color};

/// WCAG 2.x relative luminance.
pub fn luminance(c: Color) -> f64 {
    let channel = |v: u8| {
        let s = f64::from(v) / 255.;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// `fg` laid over `bg`: what shows where a translucent colour is drawn.
pub fn over(fg: Color, bg: Color) -> Color {
    let a = f64::from(fg.a.clamp(0., 1.));
    let mix = |f: u8, b: u8| (f64::from(f) * a + f64::from(b) * (1. - a)).round() as u8;
    Color { r: mix(fg.r, bg.r), g: mix(fg.g, bg.g), b: mix(fg.b, bg.b), a: 1. }
}

/// The contrast ratio of `fg` on `bg`, 1–21.
pub fn ratio(fg: Color, bg: Color) -> f64 {
    let bg = over(bg, Color::WHITE);
    let (a, b) = (luminance(over(fg, bg)), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grade {
    /// Under 3:1.
    Fail,
    /// 3:1: large text and graphics only.
    Large,
    /// 4.5:1.
    Aa,
    /// 7:1.
    Aaa,
}

/// The grade of a ratio. The ratio is rounded down to two places first, as
/// a checker shows it, so 4.499 is never called AA.
pub fn grade(ratio: f64) -> Grade {
    let r = (ratio * 100.).floor() / 100.;
    if r >= 7. {
        Grade::Aaa
    } else if r >= 4.5 {
        Grade::Aa
    } else if r >= 3. {
        Grade::Large
    } else {
        Grade::Fail
    }
}

const BACKGROUNDS: [&str; 6] = ["bg", "background", "canvas", "page", "base", "surface"];
const TEXTS: [&str; 6] = ["text", "fg", "foreground", "ink", "body", "content"];
/// Names of colours that are surfaces — compared with the text, not the background.
const SURFACES: [&str; 12] = ["bg", "background", "canvas", "page", "base", "surface", "sunken", "raised", "card", "panel", "overlay", "elevated"];

/// How much a colour token's name says it is one of `names`: its own name
/// (`color.bg`), or its group with a plain name (`color.background.default`).
fn score(token: &Token, names: &[&str]) -> u8 {
    let parts: Vec<&str> = token.name.split('.').collect();
    let leaf = parts.last().copied().unwrap_or_default().to_ascii_lowercase();
    if names.iter().position(|n| *n == leaf).is_some_and(|at| at == 0) {
        3
    } else if names.contains(&leaf.as_str()) {
        2
    } else if parts[..parts.len().saturating_sub(1)].iter().any(|p| names.contains(&p.to_ascii_lowercase().as_str())) && ["default", "primary", "base", "main", "page"].contains(&leaf.as_str()) {
        1
    } else {
        0
    }
}

fn best<'a>(tokens: &'a [Token], names: &[&str]) -> Option<&'a Token> {
    let mut found: Option<(&Token, u8)> = None;
    for t in tokens.iter().filter(|t| t.kind == "color") {
        let s = score(t, names);
        if s > 0 && found.is_none_or(|(_, b)| s > b) {
            found = Some((t, s));
        }
    }
    found.map(|(t, _)| t)
}

/// The design system's background in `mode`: a colour named `bg`,
/// `background`, `canvas`, … — else `fallback`.
pub fn background(tokens: &[Token], mode: Option<&str>, fallback: Color) -> Color {
    best(tokens, &BACKGROUNDS).and_then(|t| color(t.value_in(mode))).map_or(fallback, |c| over(c, fallback))
}

/// Its text colour in `mode`: `text`, `fg`, `foreground`, … — else `fallback`.
pub fn text(tokens: &[Token], mode: Option<&str>, fallback: Color) -> Color {
    best(tokens, &TEXTS).and_then(|t| color(t.value_in(mode))).unwrap_or(fallback)
}

/// Whether a colour is a surface (measured with the text on it) rather
/// than something drawn on the background.
pub fn is_surface(token: &Token) -> bool {
    score(token, &SURFACES) > 0
}

/// The colour an `on…` colour is drawn on: `onAccent`, `on-primary`,
/// `on_accent` are words on `accent`, `primary`.
pub fn drawn_on<'a>(tokens: &'a [Token], token: &Token) -> Option<&'a Token> {
    let leaf = token.leaf();
    let rest = leaf.strip_prefix("on").filter(|r| r.starts_with(|c: char| c.is_ascii_uppercase() || c == '-' || c == '_'))?;
    let rest = rest.trim_start_matches(['-', '_']).to_ascii_lowercase();
    tokens.iter().filter(|t| t.kind == "color" && t.leaf().to_ascii_lowercase() == rest).min_by_key(|t| usize::from(t.group() != token.group()))
}

/// What `token` is measured against in `mode`, and the ratio: an `on…`
/// colour on what it names, a surface with the text on it, anything else on
/// the background.
pub fn measure(tokens: &[Token], token: &Token, mode: Option<&str>, dark: bool) -> Option<f64> {
    let own = color(token.value_in(mode))?;
    let (base, ink) = if dark { (Color::rgb(0x0A0A0C), Color::rgb(0xEFEDE7)) } else { (Color::WHITE, Color::rgb(0x15161A)) };
    let bg = background(tokens, mode, base);
    if let Some(under) = drawn_on(tokens, token).and_then(|t| color(t.value_in(mode))) {
        return Some(ratio(own, over(under, bg)));
    }
    if is_surface(token) {
        Some(ratio(text(tokens, mode, ink), over(own, bg)))
    } else {
        Some(ratio(own, bg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(hex: u32) -> Color {
        Color::rgb(hex)
    }

    /// What T-144's guidelines measured (§2.4), to two places.
    #[test]
    fn ratios_agree_with_the_designers_table() {
        let cases = [
            (0x15161A, 0xF6F5F1, 16.57),
            (0x4F5159, 0xF6F5F1, 7.25),
            (0x1D4ED8, 0xFFFFFF, 6.70),
            (0x3F6FF5, 0xF6F5F1, 4.01),
            (0x3F6FF5, 0x0A0A0C, 4.52),
            (0xEFEDE7, 0x0A0A0C, 16.90),
            (0x0B1226, 0x8FB0FF, 8.70),
        ];
        for (fg, bg, want) in cases {
            let got = ratio(c(fg), c(bg));
            assert!((got - want).abs() < 0.006, "#{fg:06X} on #{bg:06X}: {got:.3}, not {want}");
            assert!((ratio(c(bg), c(fg)) - got).abs() < 1e-12, "the same either way round");
        }
        assert!((ratio(Color::BLACK, Color::WHITE) - 21.).abs() < 1e-9);
        assert_eq!(ratio(c(0x777777), c(0x777777)), 1.);
    }

    #[test]
    fn grades_round_down_at_their_thresholds() {
        assert_eq!(grade(16.57), Grade::Aaa);
        assert_eq!(grade(7.0), Grade::Aaa);
        assert_eq!(grade(6.99), Grade::Aa);
        assert_eq!(grade(4.5), Grade::Aa);
        assert_eq!(grade(4.4999), Grade::Large, "never rounded up into AA");
        assert_eq!(grade(3.0), Grade::Large);
        assert_eq!(grade(2.99), Grade::Fail);
    }

    #[test]
    fn a_translucent_colour_is_measured_as_it_shows() {
        let line = Color { a: 0.14, ..c(0x0A0A0C) };
        let shown = over(line, c(0xF6F5F1));
        assert_eq!(shown.hex(), "#D5D4D1");
        assert!((ratio(line, c(0xF6F5F1)) - ratio(shown, c(0xF6F5F1))).abs() < 1e-12);
        assert_eq!(over(Color { a: 0., ..Color::BLACK }, Color::WHITE), Color::WHITE);
    }

    #[test]
    fn the_background_and_text_are_found_by_name_per_mode() {
        let t = |name: &str, light: &str, dark: &str| Token {
            name: name.into(),
            kind: "color".into(),
            value: json!(light),
            modes: [("light".to_owned(), json!(light)), ("dark".to_owned(), json!(dark))].into(),
            description: None,
            alias: None,
        };
        let tokens = vec![t("color.surface", "#FFFFFF", "#16171A"), t("color.bg", "#F6F5F1", "#0A0A0C"), t("color.text", "#15161A", "#EFEDE7"), t("color.accent", "#1D4ED8", "#8FB0FF")];
        assert_eq!(background(&tokens, Some("dark"), Color::WHITE), c(0x0A0A0C), "bg beats surface");
        assert_eq!(text(&tokens, Some("light"), Color::BLACK), c(0x15161A));
        let accent = measure(&tokens, &tokens[3], Some("light"), false).unwrap();
        assert!((accent - 6.14).abs() < 0.01, "on the background: {accent}");
        let surface = measure(&tokens, &tokens[0], Some("dark"), true).unwrap();
        assert!((surface - 15.31).abs() < 0.01, "a surface, with the text on it: {surface}");
        let on_accent = t("color.onAccent", "#FFFFFF", "#0B1226");
        let with_on: Vec<Token> = tokens.iter().cloned().chain([on_accent.clone()]).collect();
        assert_eq!(drawn_on(&with_on, &on_accent).map(|t| t.name.as_str()), Some("color.accent"));
        let on = measure(&with_on, &on_accent, Some("dark"), true).unwrap();
        assert!((on - 8.70).abs() < 0.01, "on the accent, as the designer measured it: {on}");
        assert!(drawn_on(&with_on, &tokens[0]).is_none() && drawn_on(&with_on, &t("color.ontology", "#000", "#000")).is_none());
        let nested = Token { name: "color.background.default".into(), ..t("x", "#101010", "#101010") };
        assert_eq!(background(std::slice::from_ref(&nested), None, Color::WHITE), c(0x101010));
        assert_eq!(background(&[], Some("dark"), c(0x0A0A0C)), c(0x0A0A0C), "none named: the fallback");
    }
}
