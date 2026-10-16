//! The design system screen's state (`views/design.rs` draws it): what the
//! daemon last said the squad's design systems are (`Event::Design`), which
//! one is on screen and which section, what changed since the person last
//! looked. Its pictures are the window's (`crate::visuals`, read through the
//! daemon's `read_image` on this Mac or another, as a decision's are). No UI
//! types here, so the rules can be tested.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use equipe_core::design::contrast;
use equipe_core::design::diff::{self, Change, Seen};
use equipe_core::design::values;
use equipe_core::protocol::design::{AssetKind, DesignSystem, Token};
use gpui_kit::ScrollHandle;

/// The screen's sections, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Overview,
    Colors,
    Typography,
    Spacing,
    Radius,
    Shadow,
    Motion,
    Assets,
    Guidelines,
}

impl Section {
    pub const ALL: [Section; 9] = [Section::Overview, Section::Colors, Section::Typography, Section::Spacing, Section::Radius, Section::Shadow, Section::Motion, Section::Assets, Section::Guidelines];

    /// Its word in the locale files.
    pub fn key(self) -> &'static str {
        match self {
            Section::Overview => "design.section_overview",
            Section::Colors => "design.section_colors",
            Section::Typography => "design.section_typography",
            Section::Spacing => "design.section_spacing",
            Section::Radius => "design.section_radius",
            Section::Shadow => "design.section_shadow",
            Section::Motion => "design.section_motion",
            Section::Assets => "design.section_assets",
            Section::Guidelines => "design.section_guidelines",
        }
    }

    /// Its word, in the person's language.
    pub fn label(self) -> &'static str {
        crate::i18n::catalog().get(self.key())
    }

    /// As a script names it (`snapshot.rs`).
    pub fn parse(word: &str) -> Option<Section> {
        Some(match word {
            "overview" => Section::Overview,
            "colors" | "color" => Section::Colors,
            "type" | "typography" => Section::Typography,
            "spacing" | "space" => Section::Spacing,
            "radius" => Section::Radius,
            "shadow" | "shadows" => Section::Shadow,
            "motion" => Section::Motion,
            "assets" => Section::Assets,
            "guidelines" => Section::Guidelines,
            _ => return None,
        })
    }
}

/// Which section a token is shown in, by its type and name.
pub fn section_of(token: &Token) -> Option<Section> {
    let name = token.name.to_ascii_lowercase();
    let says = |words: &[&str]| name.split(['.', '-', '_']).any(|part| words.iter().any(|w| part.starts_with(w)));
    match token.kind.as_str() {
        "color" => Some(Section::Colors),
        "typography" | "fontFamily" | "fontWeight" => Some(Section::Typography),
        "shadow" => Some(Section::Shadow),
        "duration" | "cubicBezier" | "transition" => Some(Section::Motion),
        "dimension" | "number" if says(&["radius", "radii", "corner", "round"]) => Some(Section::Radius),
        "dimension" if says(&["font", "text", "line", "leading", "tracking", "letter"]) => Some(Section::Typography),
        "dimension" => Some(Section::Spacing),
        _ => None,
    }
}

/// The tokens a section shows, in the file's order.
pub fn tokens_in(system: &DesignSystem, section: Section) -> Vec<&Token> {
    system.tokens.iter().filter(|t| section_of(t) == Some(section)).collect()
}

/// How many things a section shows; `None` for one that has nothing to.
pub fn count(system: &DesignSystem, section: Section) -> Option<usize> {
    let n = match section {
        Section::Overview => return Some(0),
        Section::Assets => system.assets.len(),
        Section::Guidelines => return system.guidelines.as_ref().map(|g| headings(g).len()),
        other => tokens_in(system, other).len(),
    };
    (n > 0).then_some(n)
}

/// The sections there is something to show in.
pub fn sections(system: &DesignSystem) -> Vec<Section> {
    Section::ALL.into_iter().filter(|s| count(system, *s).is_some()).collect()
}

/// The groups of a section's tokens, in order: `color.brand` and its tokens.
pub fn groups<'a>(tokens: &[&'a Token]) -> Vec<(String, Vec<&'a Token>)> {
    let mut out: Vec<(String, Vec<&Token>)> = Vec::new();
    for t in tokens {
        match out.iter_mut().find(|(g, _)| g == t.group()) {
            Some((_, list)) => list.push(t),
            None => out.push((t.group().to_owned(), vec![t])),
        }
    }
    out
}

/// A heading of the guidelines: its level, its words, and the section it
/// starts (the guidelines are drawn a section at a time, for the contents to jump to).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: usize,
    pub title: String,
    pub section: usize,
}

/// The guidelines cut into sections at each `##` and `###` (outside code
/// fences), and the contents.
pub fn split_guidelines(text: &str) -> (Vec<String>, Vec<Heading>) {
    let mut sections: Vec<String> = vec![String::new()];
    let mut headings = Vec::new();
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(f) = fence {
            if trimmed.starts_with(f) {
                fence = None;
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
        } else if let Some(level) = heading_level(trimmed).filter(|l| (2..=3).contains(l)) {
            if !sections.last().is_some_and(|s| s.trim().is_empty()) {
                sections.push(String::new());
            }
            let title = trimmed[level..].trim().trim_end_matches('#').trim().replace("**", "").replace('`', "");
            headings.push(Heading { level, title, section: sections.len() - 1 });
        }
        let last = sections.last_mut().expect("one at least");
        last.push_str(line);
        last.push('\n');
    }
    if sections.len() > 1 && sections[0].trim().is_empty() {
        sections.remove(0);
        for h in &mut headings {
            h.section -= 1;
        }
    }
    (sections, headings)
}

fn heading_level(line: &str) -> Option<usize> {
    let level = line.chars().take_while(|c| *c == '#').count();
    ((1..=6).contains(&level) && line[level..].starts_with(' ')).then_some(level)
}

/// The contents of the guidelines.
pub fn headings(text: &str) -> Vec<Heading> {
    split_guidelines(text).1
}

/// The mode that is dark, if the system has one; and the one that is light.
pub fn dark_mode(system: &DesignSystem) -> Option<&str> {
    system.modes.iter().find(|m| m.to_ascii_lowercase().contains("dark")).map(String::as_str)
}

pub fn light_mode(system: &DesignSystem) -> Option<&str> {
    system.modes.iter().find(|m| !m.to_ascii_lowercase().contains("dark")).map(String::as_str)
}

/// The system's own background in light and in dark: what its logos sit on.
pub fn backgrounds(system: &DesignSystem) -> (values::Color, values::Color) {
    let light = contrast::background(&system.tokens, light_mode(system), values::Color::rgb(0xF6F5F1));
    let dark = match dark_mode(system) {
        Some(mode) => contrast::background(&system.tokens, Some(mode), values::Color::rgb(0x0A0A0C)),
        None => values::Color::rgb(0x0A0A0C),
    };
    (light, dark)
}

/// Whether a colour reads as dark (to measure on, and to pick the words on it).
pub fn is_dark(c: values::Color) -> bool {
    contrast::luminance(c) < 0.18
}

/// The colours to lead with: the brand's and the accent, then the surfaces
/// and the text — at most `n`.
pub fn primary_colors(system: &DesignSystem, n: usize) -> Vec<&Token> {
    const ORDER: [&str; 9] = ["brand", "primary", "accent", "dot", "bg", "background", "surface", "text", "fg"];
    let colors: Vec<&Token> = system.tokens.iter().filter(|t| t.kind == "color").collect();
    // By its own name (`color.accent`), or a group's plain one (`brand.primary`).
    let rank = |t: &Token| {
        let leaf = t.leaf().to_ascii_lowercase();
        let group = t.group().rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        ORDER.iter().position(|w| leaf == *w || (group == *w && ["default", "base", "main", "500"].contains(&leaf.as_str()))).unwrap_or(ORDER.len())
    };
    let mut ranked: Vec<(usize, usize, &Token)> = colors.iter().enumerate().map(|(i, t)| (rank(t), i, *t)).filter(|(r, _, _)| *r < ORDER.len()).collect();
    ranked.sort_by_key(|(r, i, _)| (*r, *i));
    let mut out: Vec<&Token> = ranked.into_iter().map(|(_, _, t)| t).collect();
    if out.len() < n {
        let more: Vec<&Token> = colors.into_iter().filter(|t| !out.iter().any(|o| o.name == t.name)).take(n - out.len()).collect();
        out.extend(more);
    }
    out.truncate(n);
    out
}

/// The type style to show off: the biggest.
pub fn display_style(system: &DesignSystem) -> Option<&Token> {
    let styles = system.tokens.iter().filter(|t| t.kind == "typography");
    styles.max_by(|a, b| {
        let size = |t: &Token| values::typography(&t.value).and_then(|s| s.size).unwrap_or(0.);
        size(a).total_cmp(&size(b))
    })
}

/// Where a logo goes: on light, on dark, or both — by its name (`mark-on-dark.svg`).
pub fn logo_sides(name: &str) -> (bool, bool) {
    let file = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    match (file.contains("dark"), file.contains("light")) {
        (true, false) => (false, true),
        (false, true) => (true, false),
        _ => (true, true),
    }
}

// ── fonts ──

/// CSS's generic families, as the Mac draws them.
fn generic(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "system-ui" | "-apple-system" | "blinkmacsystemfont" | "sans-serif" | "ui-sans-serif" => crate::theme::SANS,
        "ui-monospace" | "monospace" | "sfmono-regular" => crate::theme::MONO,
        "serif" | "ui-serif" => "Times New Roman",
        _ => return None,
    })
}

/// What a font stack is drawn in here: the first family installed, and
/// whether that is the one it asks for first. Generic names count as
/// installed (the Mac's own).
pub fn resolve_font(stack: &[String], installed: &HashSet<String>) -> (String, bool) {
    for (i, name) in stack.iter().enumerate() {
        if installed.contains(&name.to_ascii_lowercase()) {
            return (name.clone(), i == 0);
        }
        if let Some(mac) = generic(name) {
            return (mac.to_owned(), i == 0);
        }
    }
    (crate::theme::SANS.to_owned(), stack.is_empty())
}

// ── what the person has seen ──

/// Where what the person last saw is kept: by daemon, squad and system.
pub fn seen_path(home: &Path) -> PathBuf {
    home.join("desktop-design-seen.json")
}

pub fn load_seen(home: &Path) -> BTreeMap<String, Seen> {
    std::fs::read_to_string(seen_path(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_seen(home: &Path, seen: &BTreeMap<String, Seen>) {
    let _ = std::fs::create_dir_all(home);
    if let Ok(text) = serde_json::to_string(seen) {
        let _ = std::fs::write(seen_path(home), text);
    }
}

// ── the screen ──

pub struct DesignUi {
    pub systems: Vec<DesignSystem>,
    /// Where the squad's own would be.
    pub folder: String,
    /// The daemon answered since the screen asked.
    pub loaded: bool,
    /// When it was asked, this connection; `None`: not yet.
    pub asked: Option<Instant>,
    /// The system on screen, by `DesignSystem::key` without a draft's part.
    pub source: String,
    /// Looking at that system's draft rather than the accepted one.
    pub draft: bool,
    pub section: Section,
    /// What was seen when this look began, by system — the "바뀐 것" are since then.
    pub baseline: HashMap<String, Seen>,
    /// What each system was last seen as, kept on disk.
    pub seen: BTreeMap<String, Seen>,
    pub seen_loaded: bool,
    pub changes_open: bool,
    pub sources_open: bool,
    /// The asset shown big, by its place in the system's assets.
    pub lightbox: Option<usize>,
    /// What was copied last, and when — its swatch says so for a moment.
    pub copied: Option<(String, Instant)>,
    /// The motion token under the pointer, and how many times one was: a
    /// new hover plays it again.
    pub playing: Option<(String, u64)>,
    pub plays: u64,
    /// Installed font families, lowercase, read once.
    pub fonts: Option<HashSet<String>>,
    pub scroll: ScrollHandle,
    pub toc_at: usize,
    /// The screen is on show: a look is under way.
    pub on_screen: bool,
}

impl Default for DesignUi {
    fn default() -> Self {
        Self {
            systems: Vec::new(),
            folder: String::new(),
            loaded: false,
            asked: None,
            source: "squad".into(),
            draft: false,
            section: Section::Overview,
            baseline: HashMap::new(),
            seen: BTreeMap::new(),
            seen_loaded: false,
            changes_open: false,
            sources_open: false,
            lightbox: None,
            copied: None,
            playing: None,
            plays: 0,
            fonts: None,
            scroll: ScrollHandle::new(),
            toc_at: 0,
            on_screen: false,
        }
    }
}

impl DesignUi {
    /// The accepted systems: the squad's, then each project's.
    pub fn accepted(&self) -> Vec<&DesignSystem> {
        self.systems.iter().filter(|s| s.draft.is_none()).collect()
    }

    /// The draft handed in for the system on screen, if any.
    pub fn draft_of(&self, source: &str) -> Option<&DesignSystem> {
        self.systems.iter().find(|s| s.draft.is_some() && base_key(s) == source)
    }

    /// The sources to switch between: every system's, accepted or only drafted.
    pub fn sources(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.systems {
            let key = base_key(s);
            if !out.contains(&key) {
                out.push(key);
            }
        }
        out
    }

    /// The system on screen: the accepted one of the source (its draft when
    /// asked, or when there is only a draft).
    pub fn current(&self) -> Option<&DesignSystem> {
        let accepted = self.systems.iter().find(|s| s.draft.is_none() && base_key(s) == self.source);
        let draft = self.draft_of(&self.source);
        match (accepted, draft) {
            (Some(_), Some(d)) if self.draft => Some(d),
            (Some(a), _) => Some(a),
            (None, Some(d)) => Some(d),
            (None, None) => self.systems.first(),
        }
    }

    /// The daemon's word: keep it, keep the person's place where it still is,
    /// and note what each system was when first seen this look.
    pub fn arrived(&mut self, systems: Vec<DesignSystem>, folder: String, seen_key: &dyn Fn(&DesignSystem) -> String) {
        self.loaded = true;
        self.folder = folder;
        self.systems = systems;
        if !self.sources().contains(&self.source) {
            self.source = self.sources().into_iter().next().unwrap_or_else(|| "squad".into());
            self.draft = false;
        }
        if self.draft_of(&self.source).is_none() {
            self.draft = false;
        }
        for system in &self.systems {
            let key = seen_key(system);
            if !self.baseline.contains_key(&key)
                && let Some(seen) = self.seen.get(&key)
            {
                self.baseline.insert(key, seen.clone());
            }
        }
        if let Some(at) = self.lightbox
            && self.current().is_none_or(|s| at >= s.assets.len())
        {
            self.lightbox = None;
        }
        // A section that has nothing now: back to the overview.
        if let Some(system) = self.current()
            && count(system, self.section).is_none()
        {
            self.section = Section::Overview;
        }
    }

    /// A look begins: what changed is counted from what each system was last seen as.
    pub fn begin_look(&mut self, seen_key: &dyn Fn(&DesignSystem) -> String) {
        self.baseline.clear();
        for system in &self.systems {
            let key = seen_key(system);
            if let Some(seen) = self.seen.get(&key) {
                self.baseline.insert(key, seen.clone());
            }
        }
    }

    /// What changed in the system on screen since the person last looked.
    pub fn changes(&self, key: &str) -> Vec<Change> {
        match (self.baseline.get(key), self.current()) {
            (Some(before), Some(system)) => diff::diff(before, &system.tokens),
            _ => Vec::new(),
        }
    }

    /// The look ends (the screen left, another system picked): what is on
    /// screen now is what the person has seen.
    pub fn looked(&mut self, seen_key: &dyn Fn(&DesignSystem) -> String) -> bool {
        let mut changed = false;
        let draft = self.draft;
        for system in self.systems.iter().filter(|s| s.draft.is_none() || draft) {
            let key = seen_key(system);
            let now = diff::seen(&system.tokens);
            if self.seen.get(&key) != Some(&now) {
                self.seen.insert(key.clone(), now);
                changed = true;
            }
        }
        self.baseline.clear();
        changed
    }

    /// Another connection: nothing of the last one's holds.
    pub fn reset(&mut self) {
        let seen = std::mem::take(&mut self.seen);
        let seen_loaded = self.seen_loaded;
        let fonts = self.fonts.take();
        *self = Self { seen, seen_loaded, fonts, ..Self::default() };
    }

    /// The asset kinds' order in the grid: logos, then the rest as they come.
    pub fn assets_sorted(system: &DesignSystem) -> Vec<usize> {
        let mut ix: Vec<usize> = (0..system.assets.len()).collect();
        ix.sort_by_key(|i| (system.assets[*i].kind != AssetKind::Logo, *i));
        ix
    }

    /// The logos to lead with: the plain ones (`mark.svg`) before their variants.
    pub fn logos(system: &DesignSystem) -> Vec<usize> {
        let mut ix: Vec<usize> = (0..system.assets.len()).filter(|i| system.assets[*i].kind == AssetKind::Logo).collect();
        ix.sort_by_key(|i| {
            let name = system.assets[*i].name.rsplit('/').next().unwrap_or_default().to_owned();
            (name.matches('-').count(), name.len(), name)
        });
        ix
    }
}

/// A system's key without a draft's part: the source it belongs to.
pub fn base_key(system: &DesignSystem) -> String {
    system.project.as_ref().map_or_else(|| "squad".to_owned(), |p| format!("project:{p}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::design::DraftOf;
    use serde_json::json;

    fn token(name: &str, kind: &str, value: serde_json::Value) -> Token {
        Token { name: name.into(), kind: kind.into(), value, modes: Default::default(), description: None, alias: None }
    }

    fn system(project: Option<&str>, draft: Option<u32>, tokens: Vec<Token>) -> DesignSystem {
        DesignSystem { project: project.map(str::to_owned), draft: draft.map(|t| DraftOf { ticket: t, title: "브랜드".into() }), dir: "/d".into(), tokens, ..Default::default() }
    }

    #[test]
    fn tokens_go_to_their_section_by_type_and_name() {
        let cases = [
            ("color.bg", "color", Section::Colors),
            ("radius.md", "dimension", Section::Radius),
            ("corner-lg", "dimension", Section::Radius),
            ("space.4", "dimension", Section::Spacing),
            ("size.icon", "dimension", Section::Spacing),
            ("font.size.body", "dimension", Section::Typography),
            ("type.body", "typography", Section::Typography),
            ("elevation.card", "shadow", Section::Shadow),
            ("motion.fast", "duration", Section::Motion),
            ("ease.out", "cubicBezier", Section::Motion),
        ];
        for (name, kind, want) in cases {
            assert_eq!(section_of(&token(name, kind, json!(1))), Some(want), "{name}");
        }
        assert_eq!(section_of(&token("z.index", "number", json!(10))), None);
    }

    #[test]
    fn only_sections_with_something_show_and_colours_keep_their_groups() {
        let mut s = system(None, None, vec![token("color.brand.dot", "color", json!("#3F6FF5")), token("color.bg", "color", json!("#fff")), token("color.brand.ink", "color", json!("#000")), token("space.2", "dimension", json!("8px"))]);
        s.guidelines = Some("# Title\n\n## Logo\n\ntext\n\n```md\n## not a heading\n```\n\n### Colour\n".into());
        assert_eq!(sections(&s), [Section::Overview, Section::Colors, Section::Spacing, Section::Guidelines]);
        let colors = tokens_in(&s, Section::Colors);
        let g = groups(&colors);
        assert_eq!(g.iter().map(|(n, l)| (n.as_str(), l.len())).collect::<Vec<_>>(), [("color.brand", 2), ("color", 1)]);
        assert_eq!(count(&s, Section::Guidelines), Some(2));
    }

    #[test]
    fn the_guidelines_are_cut_at_their_headings_for_the_contents() {
        let (sections, heads) = split_guidelines("# Brand\n\nintro\n\n## 1. 로고\n\n**점**\n\n### 1.1 구성\n\nx\n\n## 2. `색`\n\ny\n");
        assert_eq!(sections.len(), 4);
        assert!(sections[0].starts_with("# Brand"));
        assert_eq!(heads.iter().map(|h| (h.level, h.title.as_str(), h.section)).collect::<Vec<_>>(), [(2, "1. 로고", 1), (3, "1.1 구성", 2), (2, "2. 색", 3)]);
        let (sections, heads) = split_guidelines("## First\n\nbody\n");
        assert_eq!((sections.len(), heads[0].section), (1, 0), "nothing before the first heading");
    }

    #[test]
    fn the_accepted_system_is_shown_and_a_draft_on_asking_or_when_alone() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        ui.arrived(vec![system(None, None, vec![]), system(Some("web"), None, vec![]), system(None, Some(144), vec![])], "/f".into(), &key);
        assert_eq!(ui.sources(), ["squad", "project:web"]);
        assert!(ui.current().unwrap().draft.is_none());
        assert_eq!(ui.draft_of("squad").unwrap().draft.as_ref().unwrap().ticket, 144);
        ui.draft = true;
        assert_eq!(ui.current().unwrap().draft.as_ref().unwrap().ticket, 144);
        // The draft is accepted: it is the system now, and the screen follows.
        ui.arrived(vec![system(None, None, vec![])], "/f".into(), &key);
        assert!(!ui.draft && ui.current().unwrap().draft.is_none());
        // Only a draft: it shows.
        ui.arrived(vec![system(None, Some(150), vec![])], "/f".into(), &key);
        assert_eq!(ui.current().unwrap().draft.as_ref().unwrap().ticket, 150);
        // The project picked goes: back to the first.
        ui.source = "project:web".into();
        ui.arrived(vec![system(None, None, vec![])], "/f".into(), &key);
        assert_eq!(ui.source, "squad");
    }

    #[test]
    fn what_changed_is_since_the_person_last_looked_and_their_look_is_kept() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        let v1 = vec![token("color.accent", "color", json!("#2E5C8A"))];
        ui.arrived(vec![system(None, None, v1)], "/f".into(), &key);
        assert!(ui.changes("squad").is_empty(), "never looked: nothing called new");
        assert!(ui.looked(&key));
        ui.arrived(vec![system(None, None, vec![token("color.accent", "color", json!("#1D4ED8")), token("color.dot", "color", json!("#3F6FF5"))])], "/f".into(), &key);
        let changes = ui.changes("squad");
        assert_eq!(changes.len(), 2, "{changes:?}");
        // Saved again live: still against what was seen when this look began.
        ui.arrived(vec![system(None, None, vec![token("color.accent", "color", json!("#1D4ED8"))])], "/f".into(), &key);
        assert_eq!(ui.changes("squad"), [Change::Changed { name: "color.accent".into(), was: "#2E5C8A".into(), now: "#1D4ED8".into() }]);
        assert!(ui.looked(&key));
        assert!(ui.changes("squad").is_empty());
        let home = std::env::temp_dir().join(format!("equipe-design-seen-{}", std::process::id()));
        save_seen(&home, &ui.seen);
        assert_eq!(load_seen(&home), ui.seen);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_font_stack_is_drawn_in_the_first_one_installed_and_says_when_it_isnt_the_first() {
        let installed: HashSet<String> = ["helvetica neue", "pretendard variable", "menlo"].iter().map(|s| (*s).to_owned()).collect();
        let stack = |s: &[&str]| s.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(resolve_font(&stack(&["Pretendard Variable", "Inter"]), &installed), ("Pretendard Variable".into(), true));
        assert_eq!(resolve_font(&stack(&["Inter", "Pretendard Variable"]), &installed), ("Pretendard Variable".into(), false));
        assert_eq!(resolve_font(&stack(&["Inter", "system-ui"]), &installed), (crate::theme::SANS.into(), false));
        assert_eq!(resolve_font(&stack(&["JetBrains Mono", "ui-monospace"]), &installed), (crate::theme::MONO.into(), false));
        assert_eq!(resolve_font(&stack(&["Nowhere"]), &installed), (crate::theme::SANS.into(), false));
    }

    #[test]
    fn logos_sit_where_their_names_say_and_the_brand_leads_the_colours() {
        assert_eq!(logo_sides("assets/logo/mark-on-dark.svg"), (false, true));
        assert_eq!(logo_sides("assets/logo/wordmark-light.png"), (true, false));
        assert_eq!(logo_sides("assets/logo/mark.svg"), (true, true));
        let s = system(None, None, vec![token("color.text", "color", json!("#111")), token("color.ok", "color", json!("#0A0")), token("color.accent", "color", json!("#1D4ED8")), token("brand.dot", "color", json!("#3F6FF5"))]);
        let names: Vec<&str> = primary_colors(&s, 3).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["color.accent", "brand.dot", "color.text"]);
        let mut with_logos = system(None, None, vec![]);
        with_logos.assets = ["assets/logo/mark-mono-on-dark.svg", "assets/logo/mark.svg", "assets/icons/x.svg", "assets/logo/wordmark.svg"]
            .iter()
            .map(|n| equipe_core::protocol::design::Asset { path: format!("/d/{n}"), name: (*n).into(), kind: if n.contains("logo") { AssetKind::Logo } else { AssetKind::Icon }, size: 1 })
            .collect();
        assert_eq!(DesignUi::logos(&with_logos), [1, 3, 0], "plain ones first");
    }
}
