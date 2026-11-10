//! Candidate palettes tried on a real layout before one is chosen: each as
//! five roles — text, background, primary, secondary, accent — light and
//! dark (dark derived when not given), drawn on the page being designed
//! when there is one (its own colour variables set to the palette's,
//! [`page_css`]) and on a page of our own otherwise, checked for WCAG AA on
//! the pairs text is set in, and written as tokens in the design system's
//! format (`docs/DESIGN-SYSTEM.md`).

use serde_json::{Value, json};

use super::contrast::{Grade, grade, ratio};
use super::values::Color;
use crate::i18n::Lang;

/// The five roles, in the order a palette is written: text, background,
/// primary, secondary, accent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roles {
    pub text: Color,
    pub bg: Color,
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub name: String,
    pub light: Roles,
    pub dark: Roles,
    /// The dark roles were derived here, not given: worth a look.
    pub derived_dark: bool,
}

pub const MOST: usize = 4;

fn hex(text: &str) -> Option<Color> {
    let t = text.trim().trim_start_matches('#');
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(Color::rgb)
}

/// Five colours, `050315-fbfbfe-2f27ce-dedcff-433bff` (`#` optional), in
/// the roles' order.
fn roles(text: &str) -> Result<Roles, String> {
    let parts: Vec<&str> = text.split(['-', ',', ' ']).filter(|p| !p.is_empty()).collect();
    let colours: Vec<Color> = parts.iter().map(|p| hex(p).ok_or_else(|| (*p).to_owned())).collect::<Result<_, _>>()?;
    match colours[..] {
        [text, bg, primary, secondary, accent] => Ok(Roles { text, bg, primary, secondary, accent }),
        _ => Err(text.to_owned()),
    }
}

/// A candidate: `Name=<text>-<bg>-<primary>-<secondary>-<accent>[/<the same, dark>]`.
/// What it couldn't read, if not.
pub fn parse(spec: &str) -> Result<Palette, String> {
    let (name, colours) = spec.split_once('=').ok_or_else(|| spec.to_owned())?;
    let name = name.trim();
    if name.is_empty() {
        return Err(spec.to_owned());
    }
    let colours = colours.trim();
    let (light, dark) = match colours.split_once('/') {
        Some((l, d)) => (roles(l)?, Some(roles(d)?)),
        None => (roles(colours)?, None),
    };
    Ok(Palette { name: name.to_owned(), light, derived_dark: dark.is_none(), dark: dark.unwrap_or_else(|| derive_dark(&light)) })
}

/// Hue (0–360), saturation and lightness (0–1).
fn hsl(c: Color) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(c.r) / 255., f64::from(c.g) / 255., f64::from(c.b) / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if (max - min).abs() < 1e-9 {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2. - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Color {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f64| ((v + m).clamp(0., 1.) * 255.).round() as u8;
    Color { r: byte(r), g: byte(g), b: byte(b), a: 1. }
}

fn with_lightness(c: Color, f: impl Fn(f64) -> f64) -> Color {
    let (h, s, l) = hsl(c);
    from_hsl(h, s, f(l).clamp(0., 1.))
}

/// Lighten (or darken) `c` in steps until it reaches `need` on `bg`, its hue kept.
fn reach(c: Color, bg: Color, need: f64) -> Color {
    let up = super::contrast::luminance(bg) < 0.18;
    let mut out = c;
    for _ in 0..50 {
        if ratio(out, bg) >= need {
            break;
        }
        out = with_lightness(out, |l| if up { l + 0.02 } else { l - 0.02 });
    }
    out
}

/// A dark version of light roles: text and background swap ends of the
/// scale, the secondary surface sits a step above the background, and the
/// primary and accent keep their hue, lightened until they read on it.
pub fn derive_dark(light: &Roles) -> Roles {
    let bg = with_lightness(light.text, |_| 0.07);
    let text = with_lightness(light.bg, |_| 0.94);
    let secondary = with_lightness(light.secondary, |l| (1. - l).clamp(0.14, 0.24));
    Roles { text, bg, primary: reach(light.primary, bg, 4.5), secondary, accent: reach(light.accent, bg, 3.) }
}

/// What a button's label is set in on the primary: the palette's own text
/// or background, whichever reads better.
pub fn on_primary(r: &Roles) -> Color {
    if ratio(r.bg, r.primary) >= ratio(r.text, r.primary) { r.bg } else { r.text }
}

/// One pair of colours text is set in, and what it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// What the pair is: `text_bg`, `text_secondary`, `primary_bg`, `on_primary`, `accent_bg`.
    pub pair: &'static str,
    pub fg: Color,
    pub bg: Color,
    pub ratio: f64,
    /// 4.5 for text (AA), 3 for large text and graphics.
    pub need: f64,
}

impl Check {
    pub fn passes(&self) -> bool {
        let g = grade(self.ratio);
        if self.need >= 4.5 { g >= Grade::Aa } else { g >= Grade::Large }
    }
}

/// The pairs a page sets text in, measured: body text on the background
/// and on the secondary surface, links in the primary, a button's label on
/// it, and the accent as a graphic.
pub fn checks(r: &Roles) -> Vec<Check> {
    let on = on_primary(r);
    [("text_bg", r.text, r.bg, 4.5), ("text_secondary", r.text, r.secondary, 4.5), ("primary_bg", r.primary, r.bg, 4.5), ("on_primary", on, r.primary, 4.5), ("accent_bg", r.accent, r.bg, 3.)]
        .into_iter()
        .map(|(pair, fg, bg, need)| Check { pair, fg, bg, ratio: ratio(fg, bg), need })
        .collect()
}

/// The palette as design tokens (DTCG, light in `$value`, dark in
/// `$extensions.mode.dark`), the roles under `color.` as the design
/// system names them, with the button label as `onPrimary`.
pub fn tokens(p: &Palette) -> Value {
    let token = |l: Color, d: Color| json!({ "$value": l.hex(), "$extensions": { "mode": { "dark": d.hex() } } });
    let (l, d) = (&p.light, &p.dark);
    json!({
        "$description": format!("Palette “{}”, tried with `equipe design palette`", p.name),
        "color": {
            "$type": "color",
            "bg": token(l.bg, d.bg),
            "text": token(l.text, d.text),
            "primary": token(l.primary, d.primary),
            "onPrimary": token(on_primary(l), on_primary(d)),
            "secondary": token(l.secondary, d.secondary),
            "accent": token(l.accent, d.accent),
        }
    })
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The sample words a page is drawn with, in `lang`: a headline, a lede,
/// body text and a row of UI labels — neutral words of equipe's own, never
/// another company's.
pub struct Sample {
    pub headline: String,
    pub lede: String,
    pub body: String,
    pub labels: Vec<String>,
}

pub fn sample(lang: Lang) -> Sample {
    let w = crate::i18n::core(lang);
    Sample {
        headline: w.get("design.sample.headline").to_owned(),
        lede: w.get("design.sample.lede").to_owned(),
        body: w.get("design.sample.body").to_owned(),
        labels: w.get("design.sample.labels").split(" · ").map(str::to_owned).collect(),
    }
}

/// The CSS that loads `families` from Google Fonts, if any.
pub fn font_link(families: &[&str]) -> String {
    if families.is_empty() {
        return String::new();
    }
    let q: Vec<String> = families.iter().map(|f| format!("family={}:wght@400;500;600;700", f.split_whitespace().collect::<Vec<_>>().join("+"))).collect();
    format!(r#"<link rel="stylesheet" href="https://fonts.googleapis.com/css2?{}&display=swap">"#, q.join("&"))
}

/// One page of our own drawn in `r`: a hero with its two actions, three
/// feature cards on the secondary surface, and a panel of controls — a
/// field, a link, a badge in the accent, figures.
fn template(r: &Roles, s: &Sample, fonts: Option<(&str, &str)>) -> String {
    let label = |i: usize| esc(s.labels.get(i).map(String::as_str).unwrap_or("·"));
    let (head, body) = fonts.unwrap_or(("system-ui", "system-ui"));
    let card = format!(r#"<div class="card"><i></i><b>{}</b><p>{}</p></div>"#, esc(&s.headline), esc(&s.body));
    format!(
        r#"<div class="tpl" style="--bg:{bg};--text:{text};--primary:{primary};--on:{on};--secondary:{secondary};--accent:{accent};--head:'{head}',system-ui;--body:'{body}',system-ui">
<nav><span class="mark"></span><span class="links"><a>{l2}</a><a>{l3}</a></span></nav>
<h3>{headline}</h3><p class="lede">{lede}</p>
<p class="actions"><button class="primary">{l0}</button><button class="ghost">{l1}</button></p>
<div class="cards">{card}{card}{card}</div>
<div class="panel"><label>{l3}<input value="{l4}"></label><p><a class="link">{l2}</a> <span class="badge">{l4}</span> <span class="num">1,284 · 98.6%</span></p></div>
</div>"#,
        bg = r.bg.hex(),
        text = r.text.hex(),
        primary = r.primary.hex(),
        on = on_primary(r).hex(),
        secondary = r.secondary.hex(),
        accent = r.accent.hex(),
        headline = esc(&s.headline),
        lede = esc(&s.lede),
        l0 = label(0),
        l1 = label(1),
        l2 = label(2),
        l3 = label(3),
        l4 = label(4),
    )
}

/// A picture of the page being designed in one palette: the palette's
/// name, light or dark, and its path relative to the sheet.
pub struct Picture<'a> {
    pub palette: &'a str,
    pub dark: bool,
    pub path: String,
}

/// Which role a page's colour variable plays, by its name: `--color-bg`,
/// `--surface`, `--ink`, `--brand-primary`, `--accent` …; `None` for one it
/// can't tell (a border, a shadow, a state).
pub fn role_of(name: &str) -> Option<&'static str> {
    let n = name.trim_start_matches('-').to_ascii_lowercase();
    let words: Vec<&str> = n.split(['-', '_']).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["border", "shadow", "line", "divider", "outline", "ring", "focus", "hover", "active", "disabled", "error", "danger", "success", "warning", "info"]) {
        return None;
    }
    if words.windows(2).any(|w| w[0] == "on" && ["primary", "accent", "action", "brand"].contains(&w[1])) || has(&["onprimary", "onaccent"]) {
        return Some("on");
    }
    if has(&["muted", "subtle"]) && has(&["text", "ink", "fg", "foreground"]) {
        return None;
    }
    if has(&["text", "ink", "fg", "foreground", "body", "heading"]) {
        return Some("text");
    }
    if has(&["bg", "background", "canvas", "page", "paper", "base"]) && !has(&["subtle", "alt", "muted"]) {
        return Some("bg");
    }
    if has(&["surface", "card", "panel", "subtle", "secondary", "muted", "raised", "sunken"]) {
        return Some("secondary");
    }
    if has(&["primary", "action", "brand", "cta", "link"]) {
        return Some("primary");
    }
    if has(&["accent", "highlight"]) {
        return Some("accent");
    }
    None
}

/// The CSS that puts `r` on the page being designed: each of its own colour
/// variables (`vars`, as its `:root` declares them) set to the role it
/// plays; where it names no background or text, its page, links and buttons
/// coloured directly.
pub fn page_css(vars: &[String], r: &Roles) -> String {
    let value = |role: &str| match role {
        "text" => r.text,
        "bg" => r.bg,
        "secondary" => r.secondary,
        "primary" => r.primary,
        "accent" => r.accent,
        _ => on_primary(r),
    };
    let mapped: Vec<(&String, &str)> = vars.iter().filter_map(|v| role_of(v).map(|role| (v, role))).collect();
    let mut css = String::from(":root{");
    for (var, role) in &mapped {
        css.push_str(&format!("{var}:{} !important;", value(role).hex()));
    }
    css.push('}');
    let named = |role: &str| mapped.iter().any(|(_, r)| *r == role);
    if !named("bg") || !named("text") {
        css.push_str(&format!(
            "html,body{{background:{bg} !important;color:{text} !important}}a{{color:{primary} !important}}button,[type=submit],.button,.btn{{background:{primary} !important;color:{on} !important;border-color:{primary} !important}}",
            bg = r.bg.hex(),
            text = r.text.hex(),
            primary = r.primary.hex(),
            on = on_primary(r).hex(),
        ));
    }
    css
}

/// The comparison sheet: each palette in a row — its swatches, the page
/// being designed in it light and dark (`pictures`), or a page of our own
/// where there is none, and the contrast of each pair with its grade.
pub fn sheet_html(palettes: &[Palette], fonts: Option<(&str, &str)>, pictures: &[Picture<'_>], lang: Lang) -> String {
    let words = crate::i18n::core(lang);
    let w = |key: &str| esc(words.get(key));
    let s = sample(lang);
    let mut rows = String::new();
    for p in palettes {
        let swatch = |c: Color, key: &str| format!(r#"<span class="sw"><i style="background:{0}"></i>{1}<code>{0}</code></span>"#, c.hex(), w(&format!("design.palette.role_{key}")));
        let swatches: String = [(p.light.text, "text"), (p.light.bg, "bg"), (p.light.primary, "primary"), (p.light.secondary, "secondary"), (p.light.accent, "accent")].iter().map(|(c, k)| swatch(*c, k)).collect();
        let (lc, dc) = (checks(&p.light), checks(&p.dark));
        let mut table = format!("<table><tr><th></th><th>{}</th><th>{}</th></tr>", w("design.palette.light"), w("design.palette.dark"));
        for (l, d) in lc.iter().zip(&dc) {
            let cell = |c: &Check| format!(r#"<td class="{}">{:.2}:1 {}</td>"#, if c.passes() { "ok" } else { "no" }, (c.ratio * 100.).floor() / 100., if c.passes() { "✓" } else { "✗" });
            table.push_str(&format!("<tr><th>{}</th>{}{}</tr>", w(&format!("design.palette.pair_{}", l.pair)), cell(l), cell(d)));
        }
        table.push_str("</table>");
        let shown = |dark: bool| pictures.iter().find(|x| x.palette == p.name && x.dark == dark).map(|x| format!(r#"<img class="page" src="{}" alt="">"#, esc(&x.path)));
        let derived = if p.derived_dark { format!(r#"<p class="note">{}</p>"#, w("design.palette.derived_dark")) } else { String::new() };
        rows.push_str(&format!(
            r#"<section><h2>{name}</h2><p class="sws">{swatches}</p>{derived}<div class="pair"><figure>{light}<figcaption>{lw}</figcaption></figure><figure>{dark}<figcaption>{dw}</figcaption></figure><div>{table}</div></div></section>"#,
            name = esc(&p.name),
            light = shown(false).unwrap_or_else(|| template(&p.light, &s, fonts)),
            dark = shown(true).unwrap_or_else(|| template(&p.dark, &s, fonts)),
            lw = w("design.palette.light"),
            dw = w("design.palette.dark"),
        ));
    }
    let families: Vec<&str> = fonts.map(|(h, b)| if h == b { vec![h] } else { vec![h, b] }).unwrap_or_default();
    format!(
        r#"<!doctype html><html lang="{code}"><meta charset="utf-8"><title>{title}</title>{link}
<style>
*{{box-sizing:border-box}}body{{margin:0;padding:32px;background:#F4F3EF;color:#16171A;font:13px/1.45 system-ui,-apple-system,sans-serif}}
h1{{font-size:22px;margin:0 0 20px}}h2{{font-size:18px;margin:0 0 8px}}section{{margin:0 0 36px;padding-bottom:28px;border-bottom:1px solid #DAD8D2}}
.sws{{display:flex;gap:14px;flex-wrap:wrap;margin:0 0 12px}}.sw{{display:flex;align-items:center;gap:6px}}.sw i{{width:22px;height:22px;border-radius:4px;border:1px solid #0002;display:inline-block}}.sw code{{color:#5C5E66}}
.note{{color:#5C5E66;margin:0 0 10px}}.pair{{display:grid;grid-template-columns:1fr 1fr 300px;gap:16px;align-items:start}}figure{{margin:0}}figcaption{{color:#5C5E66;margin-top:4px}}
table{{border-collapse:collapse;width:100%}}th,td{{text-align:left;padding:5px 6px;border-bottom:1px solid #DAD8D2;font-weight:400}}td{{white-space:nowrap}}td.ok{{color:#1B6B3A}}td.no{{color:#B3261E;font-weight:600}}
.page{{width:100%;border:1px solid #DAD8D2;border-radius:6px;display:block}}
.tpl{{background:var(--bg);color:var(--text);font-family:var(--body);padding:18px 20px;border-radius:8px;border:1px solid #0001}}
.tpl nav{{display:flex;justify-content:space-between;align-items:center;margin-bottom:18px}}.tpl .mark{{width:22px;height:22px;border-radius:6px;background:var(--text)}}.tpl .links a{{margin-left:14px}}
.tpl h3{{font-family:var(--head);font-size:26px;line-height:1.15;margin:0 0 8px;font-weight:650}}.tpl .lede{{margin:0 0 14px;font-size:14px;opacity:.85}}
.tpl button{{font:inherit;font-weight:600;border-radius:6px;padding:7px 14px;margin-right:8px;border:1px solid var(--primary)}}.tpl .primary{{background:var(--primary);color:var(--on)}}.tpl .ghost{{background:transparent;color:var(--primary)}}
.tpl .cards{{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin:14px 0}}.tpl .card{{background:var(--secondary);border-radius:6px;padding:10px}}.tpl .card i{{display:block;width:14px;height:14px;border-radius:50%;background:var(--accent);margin-bottom:6px}}.tpl .card b{{font-family:var(--head);font-size:12px}}.tpl .card p{{margin:4px 0 0;font-size:11px;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}}
.tpl .panel label{{display:block;font-size:12px}}.tpl input{{display:block;width:100%;margin-top:4px;font:inherit;padding:6px 8px;border-radius:6px;border:1px solid color-mix(in srgb,var(--text) 30%,transparent);background:var(--bg);color:var(--text)}}
.tpl .link{{color:var(--primary);text-decoration:underline}}.tpl .badge{{background:var(--accent);color:var(--bg);border-radius:10px;padding:1px 8px;font-size:11px}}.tpl .num{{font-variant-numeric:tabular-nums}}
</style>
<h1>{title}</h1>{rows}</html>
"#,
        code = lang.code(),
        title = w("design.palette.sheet_title"),
        link = font_link(&families),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_candidate_is_read_from_five_colours() {
        let p = parse("Ink=050315-fbfbfe-2f27ce-dedcff-433bff").unwrap();
        assert_eq!(p.name, "Ink");
        assert_eq!(p.light.text, Color::rgb(0x050315));
        assert_eq!(p.light.accent, Color::rgb(0x433BFF));
        assert!(p.derived_dark);
        let both = parse("Two=#111111,#ffffff,#1d4ed8,#eeeeee,#ff6600/eeeeee-0a0a0c-8fb0ff-1c1c22-ff9955").unwrap();
        assert!(!both.derived_dark && both.dark.bg == Color::rgb(0x0A0A0C));
        assert_eq!(parse("Bad=050315-fbfbfe").unwrap_err(), "050315-fbfbfe");
        assert_eq!(parse("Bad=050315-fbfbfe-2f27ce-dedcff-zzzzzz").unwrap_err(), "zzzzzz");
        assert!(parse("no name here").is_err() && parse("=050315-fbfbfe-2f27ce-dedcff-433bff").is_err());
    }

    #[test]
    fn the_page_being_designed_wears_each_palette_through_its_own_variables() {
        assert_eq!(role_of("--color-bg"), Some("bg"));
        assert_eq!(role_of("--color-ink"), Some("text"));
        assert_eq!(role_of("--surface-subtle"), Some("secondary"));
        assert_eq!(role_of("--brand-primary"), Some("primary"));
        assert_eq!(role_of("--color-on-primary"), Some("on"));
        assert_eq!(role_of("--accent"), Some("accent"));
        assert_eq!(role_of("--color-border"), None, "a border keeps its own");
        assert_eq!(role_of("--text-muted"), None, "a muted text isn't the text");
        assert_eq!(role_of("--color-rust-hover"), None, "a state keeps its own");
        let p = parse("Ink=050315-fbfbfe-2f27ce-dedcff-433bff").unwrap();
        let vars: Vec<String> = ["--color-bg", "--color-ink", "--color-surface", "--color-accent", "--color-border"].map(String::from).into();
        let css = page_css(&vars, &p.light);
        assert!(css.contains("--color-bg:#FBFBFE !important;") && css.contains("--color-ink:#050315 !important;") && css.contains("--color-accent:#433BFF"), "{css}");
        assert!(!css.contains("--color-border") && !css.contains("html,body"), "its own variables carry it: {css}");
        let plain = page_css(&[], &p.dark);
        assert!(plain.contains(&format!("html,body{{background:{} !important", p.dark.bg.hex())) && plain.contains("button"), "a page without variables is coloured directly: {plain}");
    }

    #[test]
    fn contrast_is_checked_on_every_pair_text_is_set_in() {
        let p = parse("Ink=050315-fbfbfe-2f27ce-dedcff-433bff").unwrap();
        let c = checks(&p.light);
        assert_eq!(c.iter().map(|c| c.pair).collect::<Vec<_>>(), ["text_bg", "text_secondary", "primary_bg", "on_primary", "accent_bg"]);
        assert!(c.iter().all(Check::passes), "{c:?}");
        assert_eq!(on_primary(&p.light), p.light.bg, "the light background reads best on a deep primary");
        // A pale primary fails as a link, and its label goes dark.
        let pale = parse("Pale=222222-ffffff-9fc5ff-f0f0f0-ffd400").unwrap();
        let c = checks(&pale.light);
        let link = c.iter().find(|c| c.pair == "primary_bg").unwrap();
        assert!(!link.passes() && link.ratio < 3.);
        assert_eq!(on_primary(&pale.light), pale.light.text);
        let accent = c.iter().find(|c| c.pair == "accent_bg").unwrap();
        assert!(!accent.passes(), "yellow on white is under 3:1 even for graphics: {}", accent.ratio);
        // 4.5 exactly passes; just under doesn't.
        let at = Check { pair: "text_bg", fg: Color::BLACK, bg: Color::WHITE, ratio: 4.5, need: 4.5 };
        assert!(at.passes() && !Check { ratio: 4.4999, ..at.clone() }.passes());
        assert!(Check { ratio: 3.0, need: 3., ..at }.passes());
    }

    #[test]
    fn a_derived_dark_reads_as_well_as_the_light() {
        for spec in ["Ink=050315-fbfbfe-2f27ce-dedcff-433bff", "Warm=2b1d14-fbf7f2-b4532a-f1e4d6-2a7f62", "Pale=222222-ffffff-9fc5ff-f0f0f0-ffd400"] {
            let p = parse(spec).unwrap();
            let d = &p.dark;
            assert!(super::super::contrast::luminance(d.bg) < 0.03, "{spec}: a dark background, {}", d.bg.hex());
            for c in checks(d) {
                assert!(c.passes(), "{spec} dark: {} {:.2} ({} on {})", c.pair, c.ratio, c.fg.hex(), c.bg.hex());
            }
        }
    }

    #[test]
    fn tokens_are_written_in_the_design_systems_format() {
        let p = parse("Ink=050315-fbfbfe-2f27ce-dedcff-433bff").unwrap();
        let t = tokens(&p);
        assert_eq!(t["color"]["$type"], "color");
        assert_eq!(t["color"]["bg"]["$value"], "#FBFBFE");
        assert_eq!(t["color"]["bg"]["$extensions"]["mode"]["dark"], p.dark.bg.hex());
        assert_eq!(t["color"]["onPrimary"]["$value"], "#FBFBFE");
        // equipe reads it as a system: every role a colour, light and dark.
        let parsed = crate::design::tokens::parse(&t.to_string());
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        assert_eq!(parsed.tokens.iter().filter(|t| t.kind == "color").count(), 6);
        assert_eq!(parsed.modes, ["light", "dark"]);
    }

    #[test]
    fn the_sheet_shows_each_palette_light_and_dark_with_its_grades() {
        let ps = [parse("Ink=050315-fbfbfe-2f27ce-dedcff-433bff").unwrap(), parse("Pale=222222-ffffff-9fc5ff-f0f0f0-ffd400").unwrap()];
        let page = [Picture { palette: "Ink", dark: false, path: "page/ink-light.png".into() }];
        let html = sheet_html(&ps, Some(("Fraunces", "Inter")), &page, Lang::En);
        assert_eq!(html.matches(r#"class="tpl""#).count(), 3, "the page in its place where it was drawn");
        assert!(html.contains("--primary:#9FC5FF") && !html.contains("--primary:#2F27CE") && html.contains(r#"src="page/ink-light.png""#));
        assert!(html.contains(r#"class="no""#), "the pale palette's failures are marked");
        assert!(html.contains("family=Fraunces:wght") && html.contains("family=Inter:wght"));
    }
}
