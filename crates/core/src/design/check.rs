//! `equipe design check`: what a designer runs before handing a design
//! system in — whether equipe's screen reads every token, whether the system
//! has its layers (values, roles, components), dark values, text that passes
//! WCAG AA on what it sits on, a Korean-capable type family, a brand file, a
//! logo — and whether it wears the stock look of AI-made screens. Findings,
//! not taste: each says what is wrong and what to do.

use std::path::Path;

use super::contrast;
use super::tokens;
use super::values::{self, Color};
use crate::protocol::design::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Wrong: fix before handing in.
    Error,
    /// Missing or weak: fix unless there is a reason, and say it.
    Warn,
    /// Fine, said so the report reads whole.
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Finding {
    pub level: Level,
    /// What it is about: `tokens`, `layers`, `dark`, `contrast`, `type`, `brand`, `assets`, `look`.
    pub area: &'static str,
    pub message: String,
}

impl Finding {
    fn new(level: Level, area: &'static str, message: impl Into<String>) -> Self {
        Self { level, area, message: message.into() }
    }
}

/// Families that set Korean well — a Korean-first face, or one with Hangul.
const KOREAN: [&str; 16] = [
    "pretendard", "noto sans kr", "noto sans cjk", "noto serif kr", "apple sd gothic", "suit", "spoqa han sans", "nanum", "ibm plex sans kr", "wanted sans", "gmarket", "s-core dream", "nanumsquare", "kopubworld", "gothic a1", "malgun",
];

/// Roles a semantic layer names its colours by.
const ROLES: [&str; 16] = ["text", "fg", "foreground", "bg", "background", "surface", "border", "action", "accent", "primary", "success", "warning", "danger", "error", "info", "muted"];

/// Groups that hold one component's tokens.
const COMPONENTS: [&str; 10] = ["component", "components", "button", "input", "card", "badge", "chip", "nav", "table", "dialog"];

/// Check the design folder at `dir` (the one holding `tokens.json`).
pub fn check(dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let file = dir.join(super::folder::TOKENS);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(_) => {
            out.push(Finding::new(Level::Error, "tokens", format!("No {} in {} — run `equipe design init` and fill it in.", super::folder::TOKENS, dir.display())));
            files(dir, &mut out);
            return out;
        }
    };
    let parsed = tokens::parse(&text);
    for problem in &parsed.problems {
        out.push(Finding::new(Level::Error, "tokens", format!("equipe's design screen can't read it: {problem:?}")));
    }
    if parsed.problems.is_empty() {
        out.push(Finding::new(Level::Ok, "tokens", format!("{} tokens, all read.", parsed.tokens.len())));
    }
    let tokens = &parsed.tokens;
    kinds(tokens, &mut out);
    layers(tokens, &mut out);
    let dark = parsed.modes.iter().find(|m| m.to_ascii_lowercase().contains("dark")).cloned();
    match &dark {
        Some(_) => out.push(Finding::new(Level::Ok, "dark", "Colours have dark values.")),
        None => out.push(Finding::new(Level::Warn, "dark", "No dark values: give each colour one in `$extensions.mode.dark` (or `light`/`dark` groups).")),
    }
    contrast_of(tokens, None, false, &mut out);
    if let Some(dark) = &dark {
        contrast_of(tokens, Some(dark), true, &mut out);
    }
    typefaces(tokens, &mut out);
    look(tokens, &locked(dir), &mut out);
    files(dir, &mut out);
    out.sort_by_key(|f| f.level);
    out
}

/// The kinds a system needs, each at least once.
fn kinds(tokens: &[Token], out: &mut Vec<Finding>) {
    let has = |kinds: &[&str]| tokens.iter().any(|t| kinds.contains(&t.kind.as_str()));
    let named = |words: &[&str]| tokens.iter().any(|t| t.name.split('.').any(|p| words.contains(&p.to_ascii_lowercase().as_str())));
    let need: [(&str, bool); 6] = [
        ("colour", has(&["color"])),
        ("typography (styles or families)", has(&["typography", "fontFamily"])),
        ("spacing", named(&["space", "spacing", "gap", "size"]) && has(&["dimension", "number"])),
        ("radius", named(&["radius", "radii", "rounded", "corner"])),
        ("shadow", has(&["shadow"])),
        ("motion (duration, easing)", has(&["duration", "cubicBezier", "transition"])),
    ];
    let missing: Vec<&str> = need.iter().filter(|(_, ok)| !ok).map(|(n, _)| *n).collect();
    if missing.is_empty() {
        out.push(Finding::new(Level::Ok, "tokens", "Colour, type, spacing, radius, shadow and motion are all there."));
    } else {
        out.push(Finding::new(Level::Warn, "tokens", format!("Missing: {}.", missing.join(", "))));
    }
}

/// Values, then roles, then components — a system an engineer builds from
/// names, not hex codes.
fn layers(tokens: &[Token], out: &mut Vec<Finding>) {
    let colours: Vec<&Token> = tokens.iter().filter(|t| t.kind == "color").collect();
    let roles = colours.iter().filter(|t| t.name.split('.').any(|p| ROLES.contains(&p.to_ascii_lowercase().as_str()))).count();
    let aliased = colours.iter().filter(|t| t.alias.is_some()).count();
    let components = tokens.iter().filter(|t| t.name.split('.').next().is_some_and(|root| COMPONENTS.contains(&root.to_ascii_lowercase().as_str())) || t.name.split('.').nth(1).is_some_and(|p| COMPONENTS.contains(&p.to_ascii_lowercase().as_str()))).count();
    if roles == 0 {
        out.push(Finding::new(Level::Error, "layers", "No colour is named by its role (text, background, surface, border, action, danger, …): add a semantic layer that aliases the palette, e.g. `color.action.primary = {color.blue.600}`."));
    } else if aliased == 0 && colours.len() > 8 {
        out.push(Finding::new(Level::Warn, "layers", "Roles hold raw values: alias them to the palette (`{color.blue.600}`) so one change moves every role that uses it."));
    } else {
        out.push(Finding::new(Level::Ok, "layers", format!("{roles} colours named by role, {aliased} aliased to the palette.")));
    }
    if components == 0 {
        out.push(Finding::new(Level::Warn, "layers", "No component tokens (button, input, card, …): add the key components' tokens, aliasing the roles."));
    }
}

/// Text and `on…` colours that don't reach AA (4.5:1) on what they sit on;
/// borders and icons under 3:1.
fn contrast_of(tokens: &[Token], mode: Option<&str>, dark: bool, out: &mut Vec<Finding>) {
    let which = if dark { "dark" } else { "light" };
    let mut failing = Vec::new();
    let mut weak = Vec::new();
    for t in tokens.iter().filter(|t| t.kind == "color") {
        let lower = t.name.to_ascii_lowercase();
        let words = lower.split(['.', '-', '_']).collect::<Vec<_>>();
        let is_text = words.iter().any(|w| ["text", "fg", "foreground", "ink", "body", "heading", "label", "link"].contains(w)) || contrast::drawn_on(tokens, t).is_some();
        // A component's own fill is a surface, measured through its text.
        if contrast::sibling_fill(tokens, t).is_none() && ["bg", "background", "fill"].contains(&t.leaf().to_ascii_lowercase().as_str()) {
            continue;
        }
        let is_line = words.iter().any(|w| ["border", "icon", "outline", "focus", "ring"].contains(w));
        if contrast::is_surface(t) || !(is_text || is_line) || words.iter().any(|w| ["disabled", "placeholder", "subtle", "faint"].contains(w)) {
            continue;
        }
        let Some(ratio) = contrast::measure(tokens, t, mode, dark) else { continue };
        if is_text && ratio < 4.5 {
            failing.push(format!("{} {ratio:.2}:1", t.name));
        } else if is_line && !is_text && ratio < 3. && words.iter().any(|w| ["focus", "ring", "icon"].contains(w)) {
            weak.push(format!("{} {ratio:.2}:1", t.name));
        }
    }
    if failing.is_empty() {
        out.push(Finding::new(Level::Ok, "contrast", format!("Text passes AA in {which}.")));
    } else {
        out.push(Finding::new(Level::Error, "contrast", format!("Text under 4.5:1 in {which}: {}.", failing.join(", "))));
    }
    if !weak.is_empty() {
        out.push(Finding::new(Level::Warn, "contrast", format!("Icons or focus rings under 3:1 in {which}: {}.", weak.join(", "))));
    }
}

/// A family that sets Korean, and body text at a line height Korean reads at.
fn typefaces(tokens: &[Token], out: &mut Vec<Finding>) {
    let mut families: Vec<String> = Vec::new();
    // The body style's line height over its size.
    let mut body_line: Option<f64> = None;
    for t in tokens {
        match t.kind.as_str() {
            "fontFamily" => families.extend(values::font_family(&t.value).unwrap_or_default()),
            "typography" => {
                if let Some(style) = values::typography(&t.value) {
                    families.extend(style.family.clone());
                    let lower = t.name.to_ascii_lowercase();
                    if body_line.is_none() && (lower.contains("body") || lower.contains("paragraph")) {
                        body_line = style.line.map(|line| line / style.size.unwrap_or(16.));
                    }
                }
            }
            _ => {}
        }
    }
    if families.is_empty() {
        out.push(Finding::new(Level::Warn, "type", "No type family: name the families (a Korean-capable one first for Korean text)."));
    } else if !families.iter().any(|f| KOREAN.iter().any(|k| f.to_ascii_lowercase().contains(k))) {
        out.push(Finding::new(Level::Warn, "type", format!("None of {} sets Korean well: put a Korean-first family (Pretendard, SUIT, Noto Sans KR, …) first for Korean text.", dedup(&families).join(", "))));
    } else {
        out.push(Finding::new(Level::Ok, "type", "A Korean-capable family is there."));
    }
    if let Some(multiple) = body_line
        && !(1.4..=1.8).contains(&multiple)
    {
        out.push(Finding::new(Level::Warn, "type", format!("Body line height {multiple:.2}× reads tight or loose for Korean: 1.5–1.7 of the size.")));
    }
}

fn dedup(list: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for f in list {
        if !seen.contains(f) {
            seen.push(f.clone());
        }
    }
    seen
}

/// What `brand.md` lists under `## Locked` (or `## 고정`): the brand's
/// own values, lower-cased — hex colours and face names — which are the
/// brand, not a cliché.
pub fn locked(dir: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("brand.md")).unwrap_or_default();
    let mut on = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let lower = trimmed.to_lowercase();
            on = lower.contains("locked") || lower.contains("고정");
            continue;
        }
        if on {
            out.push(line.to_lowercase());
        }
    }
    out
}

/// The stock looks of AI-made screens: a warm cream page with serif
/// headings, or Inter with an indigo-violet accent — unless the brand locked them.
fn look(tokens: &[Token], locked: &[String], out: &mut Vec<Finding>) {
    let is_locked = |what: &str| !what.is_empty() && locked.iter().any(|l| l.contains(&what.to_lowercase()));
    let bg = contrast::background(tokens, None, Color::WHITE);
    let families: Vec<String> = tokens.iter().flat_map(|t| match t.kind.as_str() {
        "fontFamily" => values::font_family(&t.value).unwrap_or_default(),
        "typography" => values::typography(&t.value).map(|s| s.family).unwrap_or_default(),
        _ => Vec::new(),
    }).map(|f| f.to_ascii_lowercase()).collect();
    let serif = families.iter().any(|f| (f.contains("serif") && !f.contains("sans") || ["playfair", "garamond", "fraunces", "instrument serif", "dm serif", "lora", "cormorant"].iter().any(|s| f.contains(s))) && !is_locked(f));
    let inter = families.first().is_some_and(|f| f.contains("inter") && !is_locked(f));
    let cream = is_cream(bg) && !is_locked(&bg.hex());
    let indigo = tokens.iter().filter(|t| t.kind == "color" && t.name.to_ascii_lowercase().split('.').any(|p| ["primary", "accent", "action", "brand"].contains(&p))).filter_map(|t| values::color(&t.value)).any(|c| is_indigo(c) && !is_locked(&c.hex()));
    if cream && serif {
        out.push(Finding::new(Level::Warn, "look", "A cream page with serif headings is the stock look of AI-made screens: keep it only if the brand is that, and say why."));
    } else if inter && indigo {
        out.push(Finding::new(Level::Warn, "look", "Inter with an indigo accent is the stock look of AI-made screens: keep it only if the brand is that, and say why."));
    }
}

/// A warm light page: light, a little yellow-red, not white.
fn is_cream(c: Color) -> bool {
    let (r, g, b) = (f64::from(c.r), f64::from(c.g), f64::from(c.b));
    let light = (r + g + b) / 3.;
    light > 225. && light < 252. && r >= g && g > b && r - b > 6.
}

/// The indigo-violet band: blue leaning red, saturated.
fn is_indigo(c: Color) -> bool {
    let (r, g, b) = (f64::from(c.r), f64::from(c.g), f64::from(c.b));
    // Tailwind's indigo-500 #6366F1 and violet-600 #7C3AED in, a plain blue #2563EB out.
    b > 180. && r > 60. && r < 170. && g < 130. && b - g > 100. && r + 15. > g
}

/// The files around the tokens: a brand file, guidelines, a logo, component pictures.
fn files(dir: &Path, out: &mut Vec<Finding>) {
    let brand = std::fs::read_to_string(dir.join("brand.md")).unwrap_or_default();
    if brand.trim().is_empty() {
        out.push(Finding::new(Level::Warn, "brand", "No brand.md: who it is for, the brand in three to five words (this, not that), and its voice — with Korean lines for buttons, errors and empty states."));
    } else {
        let lower = brand.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
        if !has(&["voice", "tone", "말투", "보이스", "톤"]) {
            out.push(Finding::new(Level::Warn, "brand", "brand.md says nothing about voice: how it speaks (해요체 or 합니다체, words it uses and avoids) with sample lines."));
        } else {
            out.push(Finding::new(Level::Ok, "brand", "brand.md has the brand and its voice."));
        }
    }
    if !dir.join(super::folder::GUIDELINES).is_file() {
        out.push(Finding::new(Level::Warn, "brand", "No guidelines.md: the rules in prose — how each role, type style and component is used."));
    }
    let logo = dir.join("assets").join("logo");
    let logos: Vec<String> = std::fs::read_dir(&logo).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_lowercase()).collect()).unwrap_or_default();
    if logos.is_empty() {
        out.push(Finding::new(Level::Warn, "assets", "No logo in assets/logo/ (SVG where there is one)."));
    } else if !logos.iter().any(|n| n.contains("dark") || n.contains("white") || n.contains("inverse")) {
        out.push(Finding::new(Level::Warn, "assets", "No logo for dark backgrounds (`-on-dark`)."));
    }
    let previews = std::fs::read_dir(dir.join("components")).map(|d| d.count()).unwrap_or(0);
    if previews == 0 {
        out.push(Finding::new(Level::Warn, "assets", "No component previews in components/: run `equipe design specimen --png`, or render each key component."));
    }
}

/// The report as a person reads it: errors, then warnings, then what is fine.
pub fn report(findings: &[Finding]) -> String {
    let mut text = String::new();
    for f in findings {
        let mark = match f.level {
            Level::Error => "✗",
            Level::Warn => "!",
            Level::Ok => "✓",
        };
        text.push_str(&format!("{mark} {:<9} {}\n", f.area, f.message));
    }
    let errors = findings.iter().filter(|f| f.level == Level::Error).count();
    let warnings = findings.iter().filter(|f| f.level == Level::Warn).count();
    text.push_str(&format!("\n{errors} to fix, {warnings} to look at.\n"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tokens: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("equipe-check-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("tokens.json"), tokens).unwrap();
        for (name, body) in files {
            let path = d.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        d
    }

    fn levels(findings: &[Finding], area: &str) -> Vec<Level> {
        findings.iter().filter(|f| f.area == area).map(|f| f.level).collect()
    }

    #[test]
    fn a_thin_system_hears_what_it_lacks() {
        let d = dir(r##"{ "color": { "$type": "color", "blue": { "$value": "#2563EB" }, "gray": { "$value": "#9CA3AF" } } }"##, &[]);
        let f = check(&d);
        assert!(levels(&f, "layers").contains(&Level::Error), "no roles: {f:#?}");
        assert!(levels(&f, "dark").contains(&Level::Warn));
        assert!(f.iter().any(|x| x.area == "tokens" && x.message.starts_with("Missing: typography")), "{f:#?}");
        assert!(levels(&f, "brand").contains(&Level::Warn));
        assert_eq!(f.first().map(|x| x.level), Some(Level::Error), "errors first");
        assert!(report(&f).contains("to fix"));
    }

    #[test]
    fn text_that_fails_aa_is_named_with_its_ratio() {
        let tokens = r##"{
          "color": { "$type": "color",
            "gray": { "300": { "$value": "#D1D5DB" }, "900": { "$value": "#111827" } },
            "bg": { "$value": "#FFFFFF", "$extensions": { "mode": { "dark": "#0B0B0D" } } },
            "text": { "primary": { "$value": "{color.gray.900}", "$extensions": { "mode": { "dark": "#F3F4F6" } } },
                      "secondary": { "$value": "{color.gray.300}", "$extensions": { "mode": { "dark": "#9CA3AF" } } } }
          },
          "font": { "body": { "$type": "fontFamily", "$value": ["Pretendard", "sans-serif"] } }
        }"##;
        let d = dir(tokens, &[("brand.md", "# 브랜드\n## Voice\n해요체"), ("guidelines.md", "x"), ("assets/logo/logo.svg", "<svg/>"), ("assets/logo/logo-on-dark.svg", "<svg/>"), ("components/button.png", "x")]);
        let f = check(&d);
        let light = f.iter().find(|x| x.area == "contrast" && x.message.contains("light")).unwrap();
        assert_eq!(light.level, Level::Error);
        assert!(light.message.contains("color.text.secondary 1."), "{}", light.message);
        assert!(!light.message.contains("text.primary"), "{}", light.message);
        assert!(f.iter().any(|x| x.area == "contrast" && x.message.contains("dark") && x.level == Level::Ok), "{f:#?}");
        assert!(levels(&f, "type").contains(&Level::Ok), "Pretendard sets Korean");
        assert!(levels(&f, "brand").iter().all(|l| *l == Level::Ok), "{f:#?}");
        assert!(levels(&f, "assets").is_empty(), "{f:#?}");
    }

    #[test]
    fn the_stock_ai_looks_are_called_out() {
        let cream = r##"{ "color": { "$type": "color", "bg": { "$value": "#F5F0E8" }, "text": { "$value": "#1A1A1A" } },
          "font": { "heading": { "$type": "fontFamily", "$value": ["Playfair Display", "serif"] } } }"##;
        assert!(levels(&check(&dir(cream, &[])), "look").contains(&Level::Warn));
        let indigo = r##"{ "color": { "$type": "color", "bg": { "$value": "#FFFFFF" }, "primary": { "$value": "#6366F1" } },
          "font": { "body": { "$type": "fontFamily", "$value": ["Inter", "sans-serif"] } } }"##;
        assert!(levels(&check(&dir(indigo, &[])), "look").contains(&Level::Warn));
        assert!(is_indigo(Color::rgb(0x7C3AED)) && !is_indigo(Color::rgb(0x2563EB)));
        // The brand's own: locked in brand.md, not a cliché.
        let d = dir(cream, &[("brand.md", "# Brand\n## Locked\n- page #F5F0E8\n- Playfair Display for headings\n## Voice\n해요체")]);
        assert!(levels(&check(&d), "look").is_empty(), "{:#?}", check(&d));
        let own = r##"{ "color": { "$type": "color", "bg": { "$value": "#FFFFFF" }, "primary": { "$value": "#C8102E" } },
          "font": { "body": { "$type": "fontFamily", "$value": ["Pretendard"] } } }"##;
        assert!(levels(&check(&dir(own, &[])), "look").is_empty());
    }
}
