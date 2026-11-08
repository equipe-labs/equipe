//! Candidate type pairs set side by side before one is chosen: each a
//! heading and a body family from Google Fonts (as Fontpair pairs them),
//! set as a real specimen — headline, lede, body, UI labels, figures — at
//! several sizes and in the product's languages; with what each family
//! covers of those languages (from the `unicode-range`s Google Fonts serves
//! it in), its licence (from the family's folder in Google Fonts' own
//! repository: `ofl/`, `apache/` or `ufl/`) and what it weighs to load.

use crate::i18n::Lang;

/// A heading family and a body family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub heading: String,
    pub body: String,
}

pub const MOST: usize = 4;
/// The weights a specimen loads of each family.
pub const WEIGHTS: &str = "400;500;600;700";

impl Pair {
    /// `Fraunces/Inter`, `Fraunces + Inter`; one family alone sets both.
    pub fn parse(text: &str) -> Option<Pair> {
        let mut parts = text.split(['/', '+']).map(str::trim).filter(|p| !p.is_empty());
        let heading = clean(parts.next()?)?;
        let body = match parts.next() {
            Some(b) => clean(b)?,
            None => heading.clone(),
        };
        parts.next().is_none().then_some(Pair { heading, body })
    }

    pub fn families(&self) -> Vec<&str> {
        if self.heading == self.body { vec![&self.heading] } else { vec![&self.heading, &self.body] }
    }

    pub fn label(&self) -> String {
        if self.heading == self.body { self.heading.clone() } else { format!("{} / {}", self.heading, self.body) }
    }
}

/// A family's name as Google Fonts writes it: letters, digits and single spaces.
fn clean(name: &str) -> Option<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')).then_some(name)
}

/// Google Fonts' CSS for `families`, every weight a specimen sets.
pub fn css_url(families: &[&str]) -> String {
    let q: Vec<String> = families.iter().map(|f| format!("family={}:wght@{WEIGHTS}", f.replace(' ', "+"))).collect();
    format!("https://fonts.googleapis.com/css2?{}&display=swap", q.join("&"))
}

/// The family's folder name in Google Fonts' repository: `Noto Sans KR` → `notosanskr`.
pub fn repo_name(family: &str) -> String {
    family.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

/// Where the family's metadata would be under each licence Google Fonts
/// publishes under — whichever answers is its licence.
pub fn licence_urls(family: &str) -> [(&'static str, String); 3] {
    let name = repo_name(family);
    [("OFL-1.1", "ofl"), ("Apache-2.0", "apache"), ("UFL-1.0", "ufl")].map(|(licence, dir)| (licence, format!("https://raw.githubusercontent.com/google/fonts/main/{dir}/{name}/METADATA.pb")))
}

/// One `@font-face` of Google Fonts' CSS: its family, weight, the file and
/// the code points it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub weight: String,
    pub url: String,
    pub ranges: Vec<(u32, u32)>,
}

/// The faces in Google Fonts' CSS. A face without `unicode-range` covers everything.
pub fn faces(css: &str) -> Vec<Face> {
    let mut out = Vec::new();
    for block in css.split("@font-face").skip(1) {
        let body = block.split_once('{').map_or(block, |(_, b)| b).split('}').next().unwrap_or_default();
        let prop = |name: &str| {
            body.split(';').find_map(|d| {
                let (k, v) = d.split_once(':')?;
                (k.trim() == name).then(|| v.trim().to_owned())
            })
        };
        let family = prop("font-family").unwrap_or_default().trim_matches(['\'', '"']).to_owned();
        let url = prop("src").and_then(|s| s.split_once("url(").map(|(_, r)| r.split(')').next().unwrap_or_default().trim_matches(['\'', '"']).to_owned())).unwrap_or_default();
        let ranges = prop("unicode-range").map(|r| r.split(',').filter_map(range).collect()).unwrap_or_else(|| vec![(0, 0x10FFFF)]);
        out.push(Face { family, weight: prop("font-weight").unwrap_or_default(), url, ranges });
    }
    out
}

/// `U+0000-00FF`, `U+0131`, `U+4??` as a span of code points.
fn range(text: &str) -> Option<(u32, u32)> {
    let t = text.trim().strip_prefix("U+").or_else(|| text.trim().strip_prefix("u+"))?;
    if let Some((a, b)) = t.split_once('-') {
        return Some((u32::from_str_radix(a, 16).ok()?, u32::from_str_radix(b, 16).ok()?));
    }
    if t.contains('?') {
        return Some((u32::from_str_radix(&t.replace('?', "0"), 16).ok()?, u32::from_str_radix(&t.replace('?', "F"), 16).ok()?));
    }
    let n = u32::from_str_radix(t, 16).ok()?;
    Some((n, n))
}

/// The characters of `text` none of `family`'s faces covers (spaces aside),
/// each once, in order.
pub fn missing(faces: &[Face], family: &str, text: &str) -> Vec<char> {
    let mine: Vec<&Face> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
    let mut out: Vec<char> = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let n = c as u32;
        if !mine.iter().any(|f| f.ranges.iter().any(|(a, b)| (*a..=*b).contains(&n))) && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// What a family sets of one language's sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub family: String,
    pub lang: Lang,
    /// Characters of the sample it doesn't have: the browser sets them in
    /// another font.
    pub missing: Vec<char>,
}

/// Every language's sample words, as the specimen sets them.
pub fn sample_text(lang: Lang) -> String {
    let s = super::palette::sample(lang);
    [s.headline, s.lede, s.body, s.labels.join(" ")].join(" ")
}

/// What each family of `pairs` covers of each of `langs`.
pub fn coverage(css: &str, pairs: &[Pair], langs: &[Lang]) -> Vec<Coverage> {
    let faces = faces(css);
    let mut families: Vec<&str> = pairs.iter().flat_map(Pair::families).collect();
    families.sort_unstable();
    families.dedup();
    let mut out = Vec::new();
    for family in families {
        for &lang in langs {
            out.push(Coverage { family: family.to_owned(), lang, missing: missing(&faces, family, &sample_text(lang)) });
        }
    }
    out
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// What is known of a family besides its faces: its licence and what the
/// specimen loaded of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub licence: Option<String>,
    pub bytes: Option<u64>,
}

/// The specimen: a column for each pair — the headline at three sizes, a
/// lede, body text, UI labels and figures, in the person's language — then
/// each other language's headline and body; under each family what it
/// lacks of a language, its licence and what it weighs.
pub fn specimen_html(pairs: &[Pair], langs: &[Lang], covered: &[Coverage], facts: &dyn Fn(&str) -> Facts, lang: Lang) -> String {
    let words = crate::i18n::core(lang);
    let w = |key: &str| esc(words.get(key));
    let mut families: Vec<&str> = pairs.iter().flat_map(Pair::families).collect();
    families.sort_unstable();
    families.dedup();
    let own = super::palette::sample(lang);
    let mut columns = String::new();
    for p in pairs {
        let (h, b) = (esc(&p.heading), esc(&p.body));
        let mut col = format!(
            r#"<section style="--h:'{h}',system-ui;--b:'{b}',system-ui"><h2>{label}</h2>
<p class="k">56</p><p class="h" style="font-size:56px">{hl}</p><p class="k">36</p><p class="h" style="font-size:36px">{hl}</p><p class="k">24</p><p class="h" style="font-size:24px">{hl}</p>
<p class="k">{lede_w} 20</p><p class="lede">{lede}</p><p class="k">{body_w} 16/26</p><p class="body">{body}</p>
<p class="k">{ui_w} 14 · 12</p><p class="ui">{labels}</p><p class="ui small">{labels}</p>
<p class="k">{num_w}</p><p class="num">0123456789 · 1,284.50 · 98.6% · 03:45</p><p class="num big">$12,480</p>"#,
            label = esc(&p.label()),
            hl = esc(&own.headline),
            lede = esc(&own.lede),
            body = esc(&own.body),
            labels = own.labels.iter().map(|l| format!("<span>{}</span>", esc(l))).collect::<String>(),
            lede_w = w("design.type.lede"),
            body_w = w("design.type.body"),
            ui_w = w("design.type.ui"),
            num_w = w("design.type.figures"),
        );
        for &other in langs.iter().filter(|l| **l != lang) {
            let s = super::palette::sample(other);
            col.push_str(&format!(r#"<div lang="{code}" class="other"><p class="k">{code}</p><p class="h" style="font-size:28px">{}</p><p class="body">{}</p></div>"#, esc(&s.headline), esc(&s.body), code = other.code()));
        }
        for family in p.families() {
            let f = facts(family);
            let gaps: Vec<String> = covered.iter().filter(|c| c.family == family && !c.missing.is_empty()).map(|c| words.fill("design.type.falls_back", &[("lang", c.lang.code().to_owned()), ("count", c.missing.len().to_string())])).collect();
            let weight = f.bytes.map(|b| format!("{} KB", b.div_ceil(1024))).unwrap_or_else(|| "?".into());
            col.push_str(&format!(
                r#"<p class="fact"><b>{name}</b> · {licence} · {weight}{gaps}</p>"#,
                name = esc(family),
                licence = esc(f.licence.as_deref().unwrap_or("?")),
                gaps = if gaps.is_empty() { format!(" · {}", w("design.type.covers_all")) } else { format!(r#" · <span class="gap">{}</span>"#, esc(&gaps.join(", "))) },
            ));
        }
        col.push_str("</section>");
        columns.push_str(&col);
    }
    let link = format!(r#"<link rel="stylesheet" href="{}">"#, esc(&css_url(&families)));
    format!(
        r#"<!doctype html><html lang="{code}"><meta charset="utf-8"><title>{title}</title>{link}
<style>
*{{box-sizing:border-box}}body{{margin:0;padding:32px;background:#FBFAF7;color:#16171A;font:13px/1.45 system-ui,-apple-system,sans-serif}}
h1{{font-size:22px;margin:0 0 20px}}main{{display:grid;grid-template-columns:repeat({n},minmax(0,1fr));gap:28px}}
section{{border-top:2px solid #16171A;padding-top:10px}}h2{{font-size:15px;margin:0 0 10px}}p{{margin:0 0 10px}}.k{{color:#76787F;font-size:11px;margin:12px 0 2px}}
.h{{font-family:var(--h);font-weight:650;line-height:1.08;letter-spacing:-.01em}}.lede{{font-family:var(--b);font-size:20px;line-height:1.4}}.body{{font-family:var(--b);font-size:16px;line-height:26px}}
.ui{{font-family:var(--b);font-size:14px;font-weight:500;display:flex;gap:14px;flex-wrap:wrap}}.ui.small{{font-size:12px}}.num{{font-family:var(--b);font-variant-numeric:tabular-nums;font-size:16px}}.num.big{{font-family:var(--h);font-size:32px;font-weight:650}}
.other{{border-top:1px solid #E2E0DA;margin-top:12px}}.fact{{color:#4A4C53;border-top:1px solid #E2E0DA;padding-top:8px;margin-top:12px}}.gap{{color:#B3261E}}
</style>
<h1>{title}</h1><main>{columns}</main></html>
"#,
        code = lang.code(),
        title = w("design.type.sheet_title"),
        n = pairs.len().max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Google Fonts' CSS as it serves a Latin family and a Korean one, trimmed.
    const CSS: &str = "/* latin-ext */\n@font-face {\n  font-family: 'Inter';\n  font-style: normal;\n  font-weight: 400;\n  font-display: swap;\n  src: url(https://fonts.gstatic.com/s/inter/v20/ext.woff2) format('woff2');\n  unicode-range: U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, U+0304, U+0308, U+0329, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF;\n}\n/* latin */\n@font-face {\n  font-family: 'Inter';\n  font-style: normal;\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/s/inter/v20/latin.woff2) format('woff2');\n  unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD;\n}\n/* [119] */\n@font-face {\n  font-family: 'Noto Sans KR';\n  font-weight: 400;\n  src: url(https://fonts.gstatic.com/s/notosanskr/v1/119.woff2) format('woff2');\n  unicode-range: U+AC00-D7A3, U+3131-318E, U+0020-007E, U+00B7;\n}\n@font-face {\n  font-family: 'Whole';\n  src: url(whole.woff2);\n}\n";

    #[test]
    fn a_pair_is_read_as_a_person_writes_it() {
        assert_eq!(Pair::parse("Fraunces/Inter"), Some(Pair { heading: "Fraunces".into(), body: "Inter".into() }));
        assert_eq!(Pair::parse(" Space  Grotesk + IBM Plex Sans "), Some(Pair { heading: "Space Grotesk".into(), body: "IBM Plex Sans".into() }));
        assert_eq!(Pair::parse("Inter").map(|p| p.families().len()), Some(1));
        assert_eq!(Pair::parse("A/B/C"), None);
        assert_eq!(Pair::parse("Inter</style>"), None, "a family name is letters, digits and spaces");
        assert_eq!(css_url(&["Space Grotesk", "Inter"]), "https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;600;700&family=Inter:wght@400;500;600;700&display=swap");
        assert_eq!(repo_name("Noto Sans KR"), "notosanskr");
        assert_eq!(licence_urls("DM Sans")[0].1, "https://raw.githubusercontent.com/google/fonts/main/ofl/dmsans/METADATA.pb");
    }

    #[test]
    fn faces_and_their_ranges_are_read_from_google_fonts_css() {
        let f = faces(CSS);
        assert_eq!(f.len(), 4);
        assert_eq!((f[1].family.as_str(), f[1].weight.as_str(), f[1].url.as_str()), ("Inter", "400", "https://fonts.gstatic.com/s/inter/v20/latin.woff2"));
        assert_eq!(f[1].ranges[0], (0, 0xFF));
        assert_eq!(f[3].ranges, vec![(0, 0x10FFFF)], "no range: everything");
        assert_eq!(range("U+4??"), Some((0x400, 0x4FF)));
        assert_eq!(range("U+0131"), Some((0x131, 0x131)));
        assert_eq!(range("nonsense"), None);
    }

    #[test]
    fn glyph_coverage_says_which_characters_fall_back() {
        let f = faces(CSS);
        assert!(missing(&f, "Inter", "Grüße, ça coûte 12 € — œuvre").is_empty(), "German and French are in Latin");
        assert_eq!(missing(&f, "Inter", "Ελλάδα"), "Ελάδα".chars().collect::<Vec<_>>(), "no Greek served here, each letter once");
        let hangul: String = [0xD55C, 0xAE00].iter().filter_map(|&n| char::from_u32(n)).collect();
        assert_eq!(missing(&f, "Inter", &format!("A {hangul} {hangul}")).len(), 2, "each missing character once");
        assert!(missing(&f, "Noto Sans KR", &hangul).is_empty());
        assert!(missing(&f, "Nowhere", "a") == vec!['a'], "a family not served covers nothing");
        let pairs = [Pair::parse("Inter/Noto Sans KR").unwrap()];
        let c = coverage(CSS, &pairs, &[Lang::En, Lang::Ko, Lang::Ja]);
        let gap = |family: &str, lang: Lang| c.iter().find(|c| c.family == family && c.lang == lang).map(|c| c.missing.len()).unwrap();
        assert_eq!(gap("Inter", Lang::En), 0);
        assert!(gap("Inter", Lang::Ko) > 0 && gap("Noto Sans KR", Lang::Ko) == 0, "the Korean sample is set by the Korean family");
        assert!(gap("Noto Sans KR", Lang::Ja) > 0, "kana aren't in the Hangul ranges");
    }

    #[test]
    fn the_specimen_sets_each_pair_in_every_language_with_its_facts() {
        let pairs = [Pair::parse("Fraunces/Inter").unwrap(), Pair::parse("Space Grotesk").unwrap()];
        let covered = coverage(CSS, &pairs, &[Lang::En, Lang::Ja]);
        let facts = |f: &str| Facts { licence: Some("OFL-1.1".into()), bytes: (f == "Inter").then_some(48_000) };
        let html = specimen_html(&pairs, &[Lang::En, Lang::Ja], &covered, &facts, Lang::En);
        assert_eq!(html.matches("<section").count(), 2);
        assert!(html.contains("--h:'Fraunces',system-ui;--b:'Inter',system-ui"));
        assert_eq!(html.matches(r#"<div lang="ja""#).count(), 2, "the other language under each pair");
        assert!(html.contains("<b>Inter</b> · OFL-1.1 · 47 KB"), "{html}");
        assert!(html.contains("family=Space+Grotesk:wght") && html.contains("tabular-nums"));
        assert!(html.contains(r#"class="gap""#), "a family that lacks Japanese says so");
    }
}
