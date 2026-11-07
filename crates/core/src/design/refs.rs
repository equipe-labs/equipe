//! A brief's reference board: pages from the galleries and tools a designer
//! studies — landing pages, product screens, palettes on a real layout, type
//! pairings, components — each kept in `design/references/` with where it
//! came from, when, and one line of what to learn from it (a pattern, a
//! layout, a rhythm, type or colour), and drawn as one contact sheet.
//!
//! What is taken from a reference is its patterns and layouts; never another
//! company's brand (its logo, name or distinctive marks) or its words. A
//! "what to learn" line that says to take one is refused ([`learn_problems`]).
//!
//! Pages are read the way a person reads them: public pages only, a few per
//! call ([`MOST_PAGES`]), a pause between two pages of one site
//! ([`PACE_SECS`]), what the site's robots.txt disallows left alone
//! ([`robots_allow`]), and a page that asks whether a person is there is never
//! answered by a tool — it goes to the designer's browser, where the person
//! can. Galleries that need a sign-in or show automated readers a check are
//! browsed in the designer's own browser and their pictures recorded with
//! `refs add` ([`Via::Browser`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The folder, in a design folder, the references are kept in.
pub const DIR: &str = "references";
/// The board's record, in [`DIR`].
pub const BOARD: &str = "board.json";
/// Pages one call reads, at most; and when none is said.
pub const MOST_PAGES: usize = 8;
pub const PAGES: usize = 4;
/// Widths one page is shot at, at most.
pub const MOST_WIDTHS: usize = 2;
/// Seconds between two pages of the same site.
pub const PACE_SECS: u64 = 5;
/// What a board needs before directions are drawn from it.
pub const BOARD_REFS: usize = 5;
pub const BOARD_SOURCES: usize = 2;
/// The longest "what to learn" line.
pub const LEARN_CHARS: usize = 200;

/// How a source's pages are captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// `refs shoot` reads it: public pages, no sign-in, no check in the way.
    Shoot,
    /// In the designer's own browser (the squad's, with the person's
    /// sign-ins), its screenshot recorded with `refs add`.
    Browser,
}

/// A place references come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub id: &'static str,
    pub name: &'static str,
    /// Its sites, by registrable domain.
    pub hosts: &'static [&'static str],
    pub via: Via,
    /// Only with the person's own sign-in (and their plan) in the squad's browser.
    pub needs_sign_in: bool,
    /// Where a query goes: `{q}` is its slug. A source can have several,
    /// tried in order (a font's page, a use's page).
    pub searches: &'static [&'static str],
    /// The aspects it is best for.
    pub aspects: &'static [Aspect],
}

pub const SOURCES: [Source; 6] = [
    Source {
        id: "lapa",
        name: "Lapa Ninja",
        hosts: &["lapa.ninja"],
        via: Via::Browser,
        needs_sign_in: false,
        searches: &["https://www.lapa.ninja/category/{q}/"],
        aspects: &[Aspect::Layout, Aspect::Pattern, Aspect::Rhythm],
    },
    Source {
        id: "mobbin",
        name: "Mobbin",
        hosts: &["mobbin.com"],
        via: Via::Browser,
        needs_sign_in: true,
        searches: &["https://mobbin.com/explore/web/screens/{q}", "https://mobbin.com/explore/mobile/screens/{q}"],
        aspects: &[Aspect::Pattern, Aspect::Layout],
    },
    Source {
        id: "realtime-colors",
        name: "Realtime Colors",
        hosts: &["realtimecolors.com"],
        via: Via::Shoot,
        needs_sign_in: false,
        searches: &[],
        aspects: &[Aspect::Colour],
    },
    Source {
        id: "fontpair",
        name: "Fontpair",
        hosts: &["fontpair.co"],
        via: Via::Shoot,
        needs_sign_in: false,
        searches: &["https://fontpair.co/fonts/google/{q}", "https://fontpair.co/for/{q}", "https://fontpair.co/vibe/{q}"],
        aspects: &[Aspect::Type],
    },
    Source {
        id: "21st",
        name: "21st.dev",
        hosts: &["21st.dev"],
        via: Via::Shoot,
        needs_sign_in: false,
        // The category's markdown twin, which the site offers to agents.
        searches: &["https://21st.dev/community/components/s/{q}.md"],
        aspects: &[Aspect::Pattern, Aspect::Layout],
    },
    Source {
        id: "google-fonts",
        name: "Google Fonts",
        hosts: &["fonts.google.com"],
        via: Via::Shoot,
        needs_sign_in: false,
        searches: &["https://fonts.google.com/specimen/{Q}"],
        aspects: &[Aspect::Type],
    },
];

/// What a reference is studied for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aspect {
    Pattern,
    Layout,
    Rhythm,
    Type,
    Colour,
}

impl Aspect {
    pub const ALL: [Aspect; 5] = [Aspect::Pattern, Aspect::Layout, Aspect::Rhythm, Aspect::Type, Aspect::Colour];

    pub fn id(self) -> &'static str {
        match self {
            Aspect::Pattern => "pattern",
            Aspect::Layout => "layout",
            Aspect::Rhythm => "rhythm",
            Aspect::Type => "type",
            Aspect::Colour => "colour",
        }
    }

    /// `layout`, `Type`, `color` (either spelling).
    pub fn parse(word: &str) -> Option<Aspect> {
        match word.trim().to_ascii_lowercase().as_str() {
            "pattern" | "patterns" => Some(Aspect::Pattern),
            "layout" | "layouts" | "grid" => Some(Aspect::Layout),
            "rhythm" | "spacing" => Some(Aspect::Rhythm),
            "type" | "typography" | "font" | "fonts" => Some(Aspect::Type),
            "colour" | "color" | "colours" | "colors" | "palette" => Some(Aspect::Colour),
            _ => None,
        }
    }

    /// `layout,type` into aspects; the word it couldn't read, if any.
    pub fn parse_list(text: &str) -> Result<Vec<Aspect>, String> {
        let mut out = Vec::new();
        for word in text.split([',', ' ', '/']).filter(|w| !w.trim().is_empty()) {
            let aspect = Aspect::parse(word).ok_or_else(|| word.trim().to_owned())?;
            if !out.contains(&aspect) {
                out.push(aspect);
            }
        }
        Ok(out)
    }
}

/// One reference on the board.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reference {
    /// `R1`, `R2` …: what a direction cites.
    pub id: String,
    pub url: String,
    /// The site, as a person names it: `fontpair.co`.
    pub site: String,
    /// The source's id ([`SOURCES`]), or `other`.
    pub source: String,
    /// When it was taken, `YYYY-MM-DD`.
    pub date: String,
    /// One line: what to learn from it.
    pub learn: String,
    pub aspects: Vec<Aspect>,
    /// Its pictures, relative to the design folder.
    pub shots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// How it was captured: `shoot` or `browser`.
    pub via: String,
}

/// The source a page is on: by its site, else none.
pub fn source_of(url: &str) -> Option<&'static Source> {
    let site = site(url);
    SOURCES.iter().find(|s| s.hosts.iter().any(|h| site == *h || site.ends_with(&format!(".{h}"))))
}

/// The site of an address, without `www.`: `fontpair.co`.
pub fn site(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default().to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
}

/// A query as a path's slug: `pricing page` → `pricing-page`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Where a query goes on a source: each of its searches, filled in.
pub fn search_urls(source: &Source, query: &str) -> Vec<String> {
    let lower = slug(query);
    // Google Fonts names a family with its capitals and `+` for spaces.
    let named = query.split_whitespace().collect::<Vec<_>>().join("+");
    source.searches.iter().map(|s| s.replace("{q}", &lower).replace("{Q}", &named)).collect()
}

/// The entries of a markdown list page (21st's category twin): each
/// `- [Title](url) — by Author`, as (title, url).
pub fn markdown_links(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("- [")?;
            let (title, rest) = line.split_once("](")?;
            let url = rest.split(')').next()?;
            url.starts_with("http").then(|| (title.to_owned(), url.trim_end_matches(".md").to_owned()))
        })
        .collect()
}

/// Whether the squad's browser holds a sign-in for `source` (from its
/// sign-ins file's sites, [`crate::browser::sites`]).
pub fn signed_in(source: &Source, sites: &[String]) -> bool {
    source.hosts.iter().any(|h| sites.iter().any(|s| s == h))
}

/// A robots.txt group: the readers it names, and its rules (allow?, pattern).
type RobotsGroup = (Vec<String>, Vec<(bool, String)>);

/// What a robots.txt says of `path` for every reader (`User-agent: *`): the
/// longest matching rule wins, `Allow` on a tie; nothing said is allowed.
pub fn robots_allow(robots: &str, path: &str) -> bool {
    let mut groups: Vec<RobotsGroup> = Vec::new();
    let mut in_agents = false;
    for line in robots.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim().to_owned());
        match key.as_str() {
            "user-agent" => {
                if !in_agents {
                    groups.push((Vec::new(), Vec::new()));
                }
                in_agents = true;
                if let Some(g) = groups.last_mut() {
                    g.0.push(value.to_ascii_lowercase());
                }
            }
            "allow" | "disallow" => {
                in_agents = false;
                if let Some(g) = groups.last_mut()
                    && !value.is_empty()
                {
                    g.1.push((key == "allow", value));
                }
            }
            _ => in_agents = false,
        }
    }
    let Some((_, rules)) = groups.iter().find(|(agents, _)| agents.iter().any(|a| a == "*")) else { return true };
    let mut best: Option<(usize, bool)> = None;
    for (allow, pattern) in rules {
        if robots_match(pattern, path) {
            let len = pattern.len();
            if best.is_none_or(|(l, a)| len > l || (len == l && *allow && !a)) {
                best = Some((len, *allow));
            }
        }
    }
    best.is_none_or(|(_, allow)| allow)
}

/// A robots.txt path pattern: a prefix, with `*` for any run and `$` for the end.
fn robots_match(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pattern.split('*').collect();
    let Some(mut rest) = path.strip_prefix(parts[0]) else { return false };
    for (i, part) in parts.iter().enumerate().skip(1) {
        if anchored && i == parts.len() - 1 {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    !anchored || rest.is_empty()
}

/// Why a "what to learn" line can't go on the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearnProblem {
    Empty,
    /// More than one line, or longer than [`LEARN_CHARS`].
    NotOneLine,
    /// It says to take the brand — its logo, name, marks — or its words.
    TakesBrand(String),
}

/// Words that take something.
const TAKING: [&str; 13] = ["use", "using", "copy", "copying", "take", "taking", "reuse", "borrow", "lift", "steal", "clone", "replicate", "keep"];
/// Words that make it theirs.
const THEIRS: [&str; 2] = ["their", "its"];
/// Another company's marks: never taken, however it is said.
const MARKS: [&str; 8] = ["logo", "logos", "logotype", "wordmark", "mascot", "trademark", "tagline", "slogan"];
/// Another company's words and look: theirs only when said to be theirs.
const OWN: [&str; 11] = ["name", "brand", "branding", "headline", "headlines", "copy", "copywriting", "text", "wording", "words", "illustrations"];

/// What keeps `learn` off the board. The person's rule: learn and copy
/// patterns and layouts as they are, never another company's brand (logo,
/// name, distinctive marks) or its words.
pub fn learn_problems(learn: &str) -> Vec<LearnProblem> {
    let line = learn.trim();
    if line.is_empty() {
        return vec![LearnProblem::Empty];
    }
    let mut out = Vec::new();
    if line.contains('\n') || line.chars().count() > LEARN_CHARS {
        out.push(LearnProblem::NotOneLine);
    }
    let lower = line.to_lowercase();
    for phrase in ["word for word", "verbatim", "pixel-perfect copy", "exact copy", "their look", "their brand", "its brand"] {
        if lower.contains(phrase) {
            out.push(LearnProblem::TakesBrand(phrase.to_owned()));
            return out;
        }
    }
    let as_is = lower.contains("as it is") || lower.contains("as-is");
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).collect();
    for (i, word) in words.iter().enumerate() {
        let before = &words[i.saturating_sub(3)..i];
        let near = &words[i.saturating_sub(2)..i];
        let taken = if MARKS.contains(word) {
            before.iter().any(|w| TAKING.contains(w) || THEIRS.contains(w))
        } else if OWN.contains(word) {
            near.iter().any(|w| THEIRS.contains(w)) || (as_is && before.iter().any(|w| TAKING.contains(w) && *w != *word))
        } else {
            false
        };
        if taken {
            out.push(LearnProblem::TakesBrand(format!("{} {word}", before.last().copied().unwrap_or_default()).trim().to_owned()));
            break;
        }
    }
    out
}

/// What the board still needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardGap {
    /// Fewer references than [`BOARD_REFS`].
    FewRefs(usize),
    /// From fewer sources than [`BOARD_SOURCES`].
    FewSources(usize),
    NoAspect(String),
    NoShot(String),
    Learn(String, LearnProblem),
}

/// The board's gaps: enough references, from enough sources, each with its
/// picture, aspects and a line to learn that takes no brand.
pub fn board_gaps(design: &Path, refs: &[Reference]) -> Vec<BoardGap> {
    let mut out = Vec::new();
    if refs.len() < BOARD_REFS {
        out.push(BoardGap::FewRefs(refs.len()));
    }
    let sources = source_count(refs);
    if sources < BOARD_SOURCES {
        out.push(BoardGap::FewSources(sources));
    }
    for r in refs {
        if r.aspects.is_empty() {
            out.push(BoardGap::NoAspect(r.id.clone()));
        }
        if r.shots.is_empty() || r.shots.iter().any(|s| !design.join(s).is_file()) {
            out.push(BoardGap::NoShot(r.id.clone()));
        }
        out.extend(learn_problems(&r.learn).into_iter().map(|p| BoardGap::Learn(r.id.clone(), p)));
    }
    out
}

/// How many sources the references come from: each named source once, each
/// other site once.
pub fn source_count(refs: &[Reference]) -> usize {
    let mut seen: Vec<&str> = refs.iter().map(|r| if r.source == "other" { r.site.as_str() } else { r.source.as_str() }).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

pub fn board_path(design: &Path) -> PathBuf {
    design.join(DIR).join(BOARD)
}

pub fn read(design: &Path) -> Vec<Reference> {
    std::fs::read_to_string(board_path(design)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn write(design: &Path, refs: &[Reference]) -> std::io::Result<()> {
    let file = board_path(design);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, format!("{}\n", serde_json::to_string_pretty(refs).unwrap_or_default()))
}

/// The next reference's id: one past the highest.
pub fn next_id(refs: &[Reference]) -> String {
    let n = refs.iter().filter_map(|r| r.id.strip_prefix('R')?.parse::<u32>().ok()).max().unwrap_or(0);
    format!("R{}", n + 1)
}

/// A new reference to `url`, numbered after `refs`; its pictures go in
/// `references/<id>/`.
pub fn new_reference(refs: &[Reference], url: &str, learn: &str, aspects: Vec<Aspect>, date: &str, via: Via) -> Reference {
    let site = site(url);
    Reference {
        id: next_id(refs),
        url: url.to_owned(),
        source: source_of(url).map_or("other", |s| s.id).to_owned(),
        site,
        date: date.to_owned(),
        learn: learn.trim().to_owned(),
        aspects,
        shots: Vec::new(),
        ticket: None,
        via: match via {
            Via::Shoot => "shoot",
            Via::Browser => "browser",
        }
        .to_owned(),
    }
}

/// Where a reference's picture goes, relative to the design folder.
pub fn shot_path(id: &str, width: u32) -> String {
    format!("{DIR}/{id}/{width}.png")
}

/// `refs` with `reference` added, or put over the one of its id.
pub fn upsert(refs: &mut Vec<Reference>, reference: Reference) {
    match refs.iter_mut().find(|r| r.id == reference.id) {
        Some(r) => *r = reference,
        None => refs.push(reference),
    }
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The contact sheet: every reference's first picture, its site and date,
/// its aspects and what to learn — one page, in `references/board.html`
/// (pictures by relative path), drawn to PNG beside it.
pub fn board_html(refs: &[Reference], brief: Option<&str>, lang: crate::i18n::Lang) -> String {
    let words = crate::i18n::core(lang);
    let w = |key: &str| esc(words.get(key));
    let summary = esc(&words.fill("design.refs.board_summary", &[("count", refs.len().to_string()), ("sources", source_count(refs).to_string())]));
    let mut cards = String::new();
    for r in refs {
        let picture = r.shots.first().map(|s| s.strip_prefix(&format!("{DIR}/")).unwrap_or(s)).map_or_else(String::new, |s| format!(r#"<img src="{}" alt="">"#, esc(s)));
        let chips: String = r.aspects.iter().map(|a| format!(r#"<span class="chip">{}</span>"#, w(&format!("design.refs.aspect_{}", a.id())))).collect();
        cards.push_str(&format!(
            r#"<figure><div class="shot">{picture}</div><figcaption><p class="meta"><b>{id}</b> · {site} · {date}</p><p class="chips">{chips}</p><p class="learn">{learn}</p><p class="url">{url}</p></figcaption></figure>"#,
            id = esc(&r.id),
            site = esc(&r.site),
            date = esc(&r.date),
            learn = esc(&r.learn),
            url = esc(&r.url),
        ));
    }
    let brief = brief.map(|b| format!("<p class=\"brief\">{}</p>", esc(b))).unwrap_or_default();
    format!(
        r#"<!doctype html><html lang="{code}"><meta charset="utf-8"><title>{title}</title>
<style>
:root{{--bg:#F4F3EF;--card:#FFFFFF;--ink:#16171A;--muted:#5C5E66;--line:#DAD8D2;--chip:#E9E7E1}}
*{{box-sizing:border-box}}body{{margin:0;padding:40px;background:var(--bg);color:var(--ink);font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}}
h1{{font-size:24px;margin:0 0 4px}}.summary,.brief{{color:var(--muted);margin:0 0 4px}}.rule{{color:var(--muted);font-size:12px;margin:0 0 24px}}
main{{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px}}
figure{{margin:0;background:var(--card);border:1px solid var(--line);border-radius:8px;overflow:hidden}}
.shot{{aspect-ratio:16/10;overflow:hidden;background:var(--chip);border-bottom:1px solid var(--line)}}.shot img{{width:100%;display:block}}
figcaption{{padding:12px 14px}}figcaption p{{margin:0 0 6px}}.meta{{color:var(--muted)}}.meta b{{color:var(--ink)}}
.chip{{display:inline-block;background:var(--chip);border-radius:4px;padding:1px 8px;margin-right:4px;font-size:12px}}
.learn{{font-size:15px}}.url{{color:var(--muted);font-size:11px;word-break:break-all}}
</style>
<h1>{title}</h1><p class="summary">{summary}</p>{brief}<p class="rule">{rule}</p><main>{cards}</main></html>
"#,
        code = lang.code(),
        title = w("design.refs.board_title"),
        rule = w("design.refs.board_rule"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_known_by_its_site_and_a_query_goes_where_the_source_searches() {
        assert_eq!(source_of("https://www.lapa.ninja/post/june/").map(|s| s.id), Some("lapa"));
        assert_eq!(source_of("https://fontpair.co/fonts/google/inter").map(|s| s.id), Some("fontpair"));
        assert_eq!(source_of("https://21st.dev/@a/components/b").map(|s| s.id), Some("21st"));
        assert_eq!(source_of("https://www.realtimecolors.com/?colors=000000-ffffff-111111-222222-333333").map(|s| s.id), Some("realtime-colors"));
        assert!(source_of("https://example.com/").is_none());
        assert!(source_of("https://notmobbin.com/").is_none(), "a site is matched whole, not by its ending");
        assert_eq!(site("https://user@WWW.Example.com:8080/a?b"), "example.com");
        let lapa = source_of("https://lapa.ninja").unwrap();
        assert_eq!(search_urls(lapa, "SaaS"), ["https://www.lapa.ninja/category/saas/"]);
        assert_eq!(search_urls(&SOURCES[5], "Space Grotesk"), ["https://fonts.google.com/specimen/Space+Grotesk"]);
        assert_eq!(slug("  Pricing page / Plans "), "pricing-page-plans");
        assert_eq!(lapa.via, Via::Browser, "its pages ask automated readers to prove they're human: browsed by the designer");
        assert!(SOURCES.iter().find(|s| s.id == "mobbin").is_some_and(|s| s.needs_sign_in && s.via == Via::Browser));
    }

    #[test]
    fn mobbin_counts_only_with_a_sign_in_in_the_squads_browser() {
        let mobbin = SOURCES.iter().find(|s| s.id == "mobbin").unwrap();
        assert!(!signed_in(mobbin, &["github.com".into(), "vercel.com".into()]));
        assert!(signed_in(mobbin, &["mobbin.com".into()]));
        let state = r#"{"cookies":[{"domain":".mobbin.com","name":"s"}],"origins":[]}"#;
        assert!(signed_in(mobbin, &crate::browser::sites(state)));
    }

    #[test]
    fn robots_rules_are_read_for_every_reader_longest_first() {
        let robots = "User-agent: GPTBot\nDisallow: /\n\nUser-agent: *\nAllow: /\nDisallow: /login\nDisallow: /brand$\nDisallow: /private/*.json\nAllow: /login/help\n";
        assert!(robots_allow(robots, "/fonts/google/inter"));
        assert!(!robots_allow(robots, "/login"));
        assert!(robots_allow(robots, "/login/help"), "the longer allow wins");
        assert!(!robots_allow(robots, "/brand"));
        assert!(robots_allow(robots, "/brand/kit"), "`$` ends the pattern");
        assert!(!robots_allow(robots, "/private/a/b.json"));
        assert!(robots_allow(robots, "/private/a/b.html"));
        assert!(robots_allow("", "/anything"), "nothing said: allowed");
        assert!(robots_allow("User-agent: GPTBot\nDisallow: /\n", "/"), "a rule for another reader isn't ours");
        assert!(!robots_allow("User-agent: *\nDisallow: /\n", "/x"));
        // Two agents sharing one group.
        assert!(!robots_allow("User-agent: a\nUser-agent: *\nDisallow: /x\n", "/x/y"));
    }

    #[test]
    fn a_line_to_learn_takes_patterns_and_layouts_never_a_brand_or_its_words() {
        for fine in [
            "Copy the layout as-is: a three-column bento under a left-aligned hero",
            "A strip of customer logos under the hero, as social proof",
            "Section rhythm: 120px between bands, 24px inside cards",
            "Serif headline over a grotesque body, tight leading",
            "Pricing as two cards with the recommended one raised",
            "Keep the text left-aligned and use short headlines",
            "Use the same headline size on every section",
        ] {
            assert!(learn_problems(fine).is_empty(), "{fine}: {:?}", learn_problems(fine));
        }
        for (taken, word) in [
            ("Use their logo in our header", "logo"),
            ("Take the headline as it is", "headline"),
            ("Reuse the same tagline", "tagline"),
            ("Keep their name on the hero", "name"),
            ("Copy their brand look", "their brand"),
            ("Hero text verbatim", "verbatim"),
            ("Use their copy for the features", "copy"),
        ] {
            let said = learn_problems(taken);
            assert!(matches!(said.as_slice(), [LearnProblem::TakesBrand(w)] if w.contains(word)), "{taken}: {said:?}");
        }
        assert_eq!(learn_problems("  "), vec![LearnProblem::Empty]);
        assert_eq!(learn_problems(&"a".repeat(LEARN_CHARS + 1)), vec![LearnProblem::NotOneLine]);
        assert_eq!(learn_problems("layout\nand more"), vec![LearnProblem::NotOneLine]);
    }

    #[test]
    fn aspects_are_read_in_either_spelling() {
        assert_eq!(Aspect::parse_list("layout, Color type").unwrap(), vec![Aspect::Layout, Aspect::Colour, Aspect::Type]);
        assert_eq!(Aspect::parse_list("rhythm,rhythm").unwrap(), vec![Aspect::Rhythm]);
        assert_eq!(Aspect::parse_list("layout,vibes").unwrap_err(), "vibes");
    }

    #[test]
    fn a_board_needs_five_references_from_two_sources_each_pictured_and_said() {
        let dir = std::env::temp_dir().join(format!("equipe-refs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut refs = Vec::new();
        for (i, url) in ["https://fontpair.co/fonts/google/inter", "https://fontpair.co/for/saas", "https://21st.dev/@a/components/hero", "https://example.com/", "https://example.org/"].iter().enumerate() {
            let mut r = new_reference(&refs, url, "Two-column hero with the form on the right", vec![Aspect::Layout], "2026-11-07", Via::Shoot);
            assert_eq!(r.id, format!("R{}", i + 1));
            let shot = shot_path(&r.id, 1280);
            std::fs::create_dir_all(dir.join(&shot).parent().unwrap()).unwrap();
            std::fs::write(dir.join(&shot), b"png").unwrap();
            r.shots.push(shot);
            upsert(&mut refs, r);
        }
        assert_eq!(source_count(&refs), 4, "fontpair once, 21st, and each other site");
        assert!(board_gaps(&dir, &refs).is_empty(), "{:?}", board_gaps(&dir, &refs));
        write(&dir, &refs).unwrap();
        assert_eq!(read(&dir), refs);
        assert_eq!(next_id(&refs), "R6");

        let few = &refs[..2];
        let gaps = board_gaps(&dir, few);
        assert!(gaps.contains(&BoardGap::FewRefs(2)) && gaps.contains(&BoardGap::FewSources(1)), "{gaps:?}");
        let mut bad = refs.clone();
        bad[0].learn = "Use their logo".into();
        bad[1].shots = vec!["references/R2/missing.png".into()];
        bad[2].aspects.clear();
        let gaps = board_gaps(&dir, &bad);
        assert!(gaps.iter().any(|g| matches!(g, BoardGap::Learn(id, LearnProblem::TakesBrand(_)) if id == "R1")));
        assert!(gaps.contains(&BoardGap::NoShot("R2".into())) && gaps.contains(&BoardGap::NoAspect("R3".into())));

        let html = board_html(&refs, Some("A calm page for <teams>"), crate::i18n::Lang::En);
        assert!(html.contains(r#"<img src="R1/1280.png""#), "pictures by path relative to the sheet");
        assert!(html.contains("&lt;teams&gt;") && html.contains("References: 5 · Sources: 4"), "{html}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_category_twin_lists_its_components() {
        let md = "# Hero components\n\n- [Hero Section](https://21st.dev/@x/components/hero-section.md) — by x\n- [Aurora](https://21st.dev/@k/components/aurora.md) — by k\n- not a link\n";
        assert_eq!(
            markdown_links(md),
            [("Hero Section".to_owned(), "https://21st.dev/@x/components/hero-section".to_owned()), ("Aurora".to_owned(), "https://21st.dev/@k/components/aurora".to_owned())]
        );
    }
}
