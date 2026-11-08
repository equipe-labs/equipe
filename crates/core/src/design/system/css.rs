//! Tokens written as CSS: custom properties on `:root` (light), on `.dark`,
//! `[data-theme=dark]` or under `@media (prefers-color-scheme: dark)` (dark),
//! and Tailwind v4's `@theme` / `@theme inline` blocks — the shape shadcn's
//! themes, and the exporters of a DTCG file, write. Read as tokens, a value
//! per mode, a `var(--x)` resolved to what `--x` holds.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::design::values;
use crate::protocol::design::Token;

/// Where a custom property was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Light,
    Dark,
    /// Tailwind's `@theme`: the utilities' names (`--color-primary` makes `bg-primary`).
    Theme,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub place: Place,
    pub line: u32,
}

/// What a stylesheet sets: its custom properties, and whether it has a
/// Tailwind `@theme` block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub properties: Vec<Property>,
    pub theme: bool,
}

pub fn read(text: &str) -> Sheet {
    let mut sheet = Sheet::default();
    let mut stack: Vec<String> = Vec::new();
    let mut from = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let line = |at: usize| text[..at].matches('\n').count() as u32 + 1;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = text[i + 2..].find("*/").map_or(text.len(), |e| i + 2 + e + 2);
            from = i;
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'{' => {
                let selector = text[from..i].trim().to_ascii_lowercase();
                if selector.starts_with("@theme") {
                    sheet.theme = true;
                }
                stack.push(selector);
                from = i + 1;
            }
            b';' | b'}' => {
                let piece = text[from..i].trim();
                if let Some((prop, value)) = piece.split_once(':')
                    && let Some(name) = prop.trim().strip_prefix("--")
                    && !stack.is_empty()
                {
                    let place = if stack.iter().any(|s| s.starts_with("@theme")) {
                        Place::Theme
                    } else if stack.iter().any(|s| is_dark(s)) {
                        Place::Dark
                    } else {
                        Place::Light
                    };
                    let lead = text[from..i].len() - text[from..i].trim_start().len();
                    sheet.properties.push(Property { name: name.trim().to_owned(), value: value.trim().trim_end_matches("!important").trim().to_owned(), place, line: line(from + lead) });
                }
                if c == b'}' {
                    stack.pop();
                }
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    sheet
}

fn is_dark(selector: &str) -> bool {
    selector.contains(".dark") || selector.contains("theme=dark") || selector.contains("theme=\"dark\"") || selector.contains("theme='dark'") || (selector.starts_with("@media") && selector.contains("prefers-color-scheme") && selector.contains("dark"))
}

/// What `var(--x)` (or `var(--x, fallback)`) names, when that is the whole value.
fn var_of(value: &str) -> Option<&str> {
    let inner = value.trim().strip_prefix("var(")?.strip_suffix(')')?;
    let name = inner.split(',').next()?.trim().strip_prefix("--")?;
    Some(name)
}

/// The sheet's custom properties as tokens: named as written (`primary`,
/// `color-primary`, `radius`), a colour, size, shadow or family each, with
/// its dark value where one is set.
pub fn tokens<'s>(sheet: &'s Sheet) -> Vec<Token> {
    let pair = |p: &'s Property| (p.name.as_str(), p.value.as_str());
    let light: BTreeMap<&'s str, &'s str> = sheet.properties.iter().filter(|p| p.place != Place::Dark).map(pair).collect();
    let dark: BTreeMap<&'s str, &'s str> = sheet.properties.iter().filter(|p| p.place == Place::Dark).map(pair).collect();
    let resolve = |mut v: &'s str, dark_mode: bool| -> String {
        for _ in 0..8 {
            let Some(name) = var_of(v) else { break };
            let next = if dark_mode { dark.get(name).or_else(|| light.get(name)) } else { light.get(name) };
            match next {
                Some(n) => v = n,
                None => break,
            }
        }
        v.to_owned()
    };
    let mut names: Vec<&str> = Vec::new();
    for p in &sheet.properties {
        if !names.contains(&p.name.as_str()) {
            names.push(&p.name);
        }
    }
    let mut out = Vec::new();
    for name in names {
        let Some(raw) = light.get(name).or_else(|| dark.get(name)) else { continue };
        let value = resolve(raw, false);
        // A dark value: set under dark itself, or a reference that resolves differently there.
        let dark_value = dark.get(name).map(|d| resolve(d, true)).or_else(|| Some(resolve(raw, true)).filter(|d| *d != value));
        let Some(kind) = kind_of(name, &value) else { continue };
        let mut modes = BTreeMap::new();
        if let Some(d) = dark_value {
            modes.insert("light".to_owned(), Value::String(value.clone()));
            modes.insert("dark".to_owned(), Value::String(d));
        }
        out.push(Token { name: name.to_owned(), kind: kind.to_owned(), value: Value::String(value), modes, description: None, alias: var_of(raw).map(|a| format!("{{{a}}}")) });
    }
    out
}

/// What a custom property holds: a colour, a size, a shadow, a family.
fn kind_of(name: &str, value: &str) -> Option<&'static str> {
    let v = Value::String(value.to_owned());
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("font") && !lower.contains("size") && !lower.contains("weight") && values::dimension(&v).is_none() {
        return values::font_family(&v).map(|_| "fontFamily");
    }
    if values::color(&v).is_some() {
        return Some("color");
    }
    if lower.contains("shadow") && value.split_whitespace().count() >= 3 {
        return Some("shadow");
    }
    if values::dimension(&v).is_some() && value.trim() != "0" || value.trim() == "0" && lower.contains("radius") {
        return Some("dimension");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADCN: &str = r#"@import "tailwindcss";
@custom-variant dark (&:is(.dark *));
:root {
  --radius: 0.625rem;
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  --primary: oklch(0.205 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --ring: oklch(0.708 0 0);
}
.dark {
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
  --primary: oklch(0.922 0 0);
}
@theme inline {
  --color-background: var(--background);
  --color-primary: var(--primary);
  --radius-lg: var(--radius);
  --shadow-card: 0 1px 2px rgb(0 0 0 / 0.05);
  --font-sans: "Inter", sans-serif;
}
"#;

    #[test]
    fn a_shadcn_theme_reads_as_tokens_light_and_dark() {
        let sheet = read(SHADCN);
        assert!(sheet.theme);
        let tokens = tokens(&sheet);
        let t = |n: &str| tokens.iter().find(|t| t.name == n).unwrap_or_else(|| panic!("{n}: {tokens:#?}"));
        assert_eq!(t("background").kind, "color");
        assert_eq!(values::color(&t("background").value).unwrap().hex(), "#FFFFFF");
        assert_eq!(values::color(&t("primary").modes["dark"]).unwrap().hex(), "#E5E5E5");
        // The theme's names resolve to the colours and keep their dark ones.
        assert_eq!(values::color(&t("color-primary").value).unwrap().hex(), "#171717");
        assert_eq!(values::color(&t("color-primary").modes["dark"]).unwrap().hex(), "#E5E5E5");
        assert_eq!(t("color-primary").alias.as_deref(), Some("{primary}"));
        assert_eq!(t("radius").kind, "dimension");
        assert_eq!(values::dimension(&t("radius-lg").value), Some(10.));
        assert_eq!(t("shadow-card").kind, "shadow");
        assert_eq!(t("font-sans").kind, "fontFamily");
        assert!(t("ring").modes.is_empty(), "no dark value set");
    }

    #[test]
    fn dark_values_are_found_under_each_way_of_writing_dark() {
        for css in ["[data-theme=dark] { --bg: #000; } :root { --bg: #fff; }", "@media (prefers-color-scheme: dark) { :root { --bg: #000; } } :root { --bg: #fff; }"] {
            let tokens = tokens(&read(css));
            assert_eq!(tokens.len(), 1, "{css}");
            assert_eq!(tokens[0].value, Value::String("#fff".into()), "{css}");
            assert_eq!(tokens[0].modes["dark"], Value::String("#000".into()), "{css}");
        }
    }
}
