//! The design system screen's state (`views/design.rs` draws it): what the
//! daemon last said the squad's design systems are (`Event::Design`), which
//! one is on screen and which section, what changed since the person last
//! looked, and what a ticket still at work on one has made so far. Its pictures are the window's (`crate::visuals`, read through the
//! daemon's `read_image` on this Mac or another, as a decision's are). No UI
//! types here, so the rules can be tested.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use equipe_core::design::contrast;
use equipe_core::design::diff::{self, Change, Seen};
use equipe_core::design::values;
use equipe_core::protocol::design::{AssetKind, DesignSystem, DesignVersion, FlowMap, Taste, TasteFrom, TasteNote, Token};
use equipe_core::t;

use crate::compare::{self, Side};
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
    /// Taste & decisions: the person's taste notes and the decisions made.
    Taste,
}

impl Section {
    pub const ALL: [Section; 10] = [Section::Overview, Section::Colors, Section::Typography, Section::Spacing, Section::Radius, Section::Shadow, Section::Motion, Section::Assets, Section::Guidelines, Section::Taste];

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
            Section::Taste => "design.taste_tab",
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
            "taste" => Section::Taste,
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
        // Always there, to invite the first note.
        Section::Overview | Section::Taste => return Some(0),
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
    // One of each word: `card.bg` and `input.bg` after `color.bg` say nothing more.
    let mut words: Vec<String> = Vec::new();
    let mut out: Vec<&Token> = Vec::new();
    for (_, _, t) in ranked {
        let word = t.leaf().to_ascii_lowercase();
        if !words.contains(&word) || ["default", "base", "main", "500"].contains(&word.as_str()) {
            words.push(word);
            out.push(t);
        }
    }
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
    if crate::readonly::blocked(&seen_path(home).display().to_string()) {
        return;
    }
    let _ = std::fs::create_dir_all(home);
    if let Ok(text) = serde_json::to_string(seen) {
        let _ = std::fs::write(seen_path(home), text);
    }
}

// ── the screen ──

pub struct DesignUi {
    /// As shown: each map with the pictures of the data on show
    /// (`flows::with_data`).
    pub systems: Vec<DesignSystem>,
    /// As the daemon sent them: every map's real-data pictures and examples.
    pub raw: Vec<DesignSystem>,
    /// Real data or Example data, as the person picked; `None`: real where
    /// there is some and the daemon is this Mac's.
    pub data: Option<crate::flows::Data>,
    /// The daemon is this Mac's.
    pub local: bool,
    /// The person's taste and the decisions made, by source.
    pub taste: Vec<Taste>,
    /// The notes whose evidence is open, by their words.
    pub evidence_open: HashSet<String>,
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
    /// Looking at the work a ticket is still making of it, as far as it has
    /// come: read-only, not in review.
    pub working: bool,
    pub section: Section,
    /// What was seen when this look began, by system — the "Changed since you last looked" are since then.
    pub baseline: HashMap<String, Seen>,
    /// What each system was last seen as, kept on disk.
    pub seen: BTreeMap<String, Seen>,
    pub seen_loaded: bool,
    pub changes_open: bool,
    /// What couldn't be read, listed rather than said in one line.
    pub problems_open: bool,
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
    /// UI map: where the person is in the system's map.
    pub map: crate::flows::MapUi,
    /// Talk while looking: the conversation docked beside it (`views/design_talk.rs`).
    pub talk: crate::design_talk::TalkUi,
    /// Compare and review under way: the draft held against the accepted system,
    /// the side on show being `draft`'s.
    pub compare: bool,
    /// Its box for what to change, open.
    pub redo_open: bool,
    /// The window is wide enough for Now and After side by side.
    pub wide: bool,
    /// The design pane's width at the last frame: the header picks its switch by it.
    pub pane: f32,
    /// Version history open.
    pub versions_open: bool,
    /// Each source's accepted versions, newest first, as the daemon last said.
    pub versions: HashMap<String, Vec<DesignVersion>>,
    /// The source asked for and not answered yet.
    pub versions_asked: Option<String>,
}

impl Default for DesignUi {
    fn default() -> Self {
        Self {
            systems: Vec::new(),
            raw: Vec::new(),
            data: None,
            local: true,
            taste: Vec::new(),
            evidence_open: HashSet::new(),
            folder: String::new(),
            loaded: false,
            asked: None,
            source: "squad".into(),
            draft: false,
            working: false,
            section: Section::Overview,
            baseline: HashMap::new(),
            seen: BTreeMap::new(),
            seen_loaded: false,
            changes_open: false,
            problems_open: false,
            sources_open: false,
            lightbox: None,
            copied: None,
            playing: None,
            plays: 0,
            fonts: None,
            scroll: ScrollHandle::new(),
            toc_at: 0,
            on_screen: false,
            map: Default::default(),
            talk: Default::default(),
            compare: false,
            redo_open: false,
            wide: false,
            pane: f32::MAX,
            versions_open: false,
            versions: HashMap::new(),
            versions_asked: None,
        }
    }
}

impl DesignUi {
    /// The accepted systems: the squad's, then each project's.
    pub fn accepted(&self) -> Vec<&DesignSystem> {
        self.systems.iter().filter(|s| s.draft.is_none()).collect()
    }

    /// The draft handed in for the system on screen, if any: one in review,
    /// never work still under way.
    pub fn draft_of(&self, source: &str) -> Option<&DesignSystem> {
        self.systems.iter().find(|s| in_review(s) && base_key(s) == source)
    }

    /// The work a ticket is still making of `source`, as far as it has come
    /// — none while a draft of it is in review, which comes first.
    pub fn working_of(&self, source: &str) -> Option<&DesignSystem> {
        if self.draft_of(source).is_some() {
            return None;
        }
        self.systems.iter().find(|s| is_working(s) && base_key(s) == source)
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
    /// asked, or when there is only a draft; the work under way when asked).
    /// Work under way is never shown unasked: alone, the source is still empty.
    pub fn current(&self) -> Option<&DesignSystem> {
        if self.working
            && let Some(work) = self.working_of(&self.source)
        {
            return Some(work);
        }
        let accepted = self.systems.iter().find(|s| s.draft.is_none() && base_key(s) == self.source);
        let draft = self.draft_of(&self.source);
        match (accepted, draft) {
            (Some(_), Some(d)) if self.draft => Some(d),
            (Some(a), _) => Some(a),
            (None, Some(d)) => Some(d),
            (None, None) if self.working_of(&self.source).is_some() => None,
            (None, None) => self.systems.iter().find(|s| !is_working(s)),
        }
    }

    /// The work under way on the source on screen viewed (`on`) or left:
    /// whether it is on show now.
    pub fn view_working(&mut self, on: bool) -> bool {
        self.working = on && self.working_of(&self.source).is_some();
        if self.working {
            self.draft = false;
            self.end_compare();
        }
        self.lightbox = None;
        let map = self.current().and_then(|s| s.flows.clone());
        self.map.arrived(map.as_ref());
        if let Some(system) = self.current()
            && count(system, self.section).is_none()
        {
            self.section = Section::Overview;
        }
        self.working
    }

    /// The work under way on the source on screen, as the screen tells it:
    /// its ticket, who is at it and what they are doing now (as the team
    /// says), what has landed. `None` with none — or a draft in review.
    pub fn making(&self, model: &crate::state::Model) -> Option<Making> {
        let work = self.working_of(&self.source)?;
        let of = work.draft.as_ref()?;
        let member = model.member_on(of.ticket);
        Some(Making {
            ticket: of.ticket,
            title: of.title.clone(),
            who: member.map(|m| (m.name.clone(), m.role.clone())),
            doing: member.and_then(|m| crate::board::step_words(m.estimate.as_ref().and_then(|e| e.what.as_deref()), m.target.as_deref())),
            busy: member.is_some_and(|m| m.presence == crate::state::Presence::Working),
            landed: landed(work),
        })
    }

    /// `systems` as shown: each map with the data on show's pictures.
    fn shown(&self, systems: &[DesignSystem]) -> Vec<DesignSystem> {
        let mut out = systems.to_vec();
        for system in &mut out {
            if let Some(map) = system.flows.as_mut() {
                let data = if self.compare { crate::flows::Data::Example } else { crate::flows::data_shown(self.data, map, self.local) };
                *map = crate::flows::with_data(map, data);
            }
        }
        out
    }

    /// The data the system on show's map shows; `None`: it has none of the
    /// person's real data, so there is nothing to pick between.
    pub fn data_on_show(&self) -> Option<crate::flows::Data> {
        let map = self.raw_current()?.flows.as_ref()?;
        (crate::flows::has_live(map) && !self.compare).then(|| crate::flows::data_shown(self.data, map, self.local))
    }

    /// The system on show, as the daemon sent it.
    fn raw_current(&self) -> Option<&DesignSystem> {
        let key = self.current()?.key();
        self.raw.iter().find(|s| s.key() == key)
    }

    /// Real data or Example data: every map shows it from now on.
    pub fn pick_data(&mut self, data: crate::flows::Data) {
        self.data = Some(data);
        self.systems = self.shown(&self.raw);
        let map = self.current().and_then(|s| s.flows.clone());
        self.map.arrived(map.as_ref());
    }

    /// The daemon's word: keep it, keep the person's place where it still is,
    /// and note what each system was when first seen this look.
    pub fn arrived(&mut self, systems: Vec<DesignSystem>, folder: String, taste: Vec<Taste>, seen_key: &dyn Fn(&DesignSystem) -> String) {
        self.loaded = true;
        self.folder = folder;
        self.systems = self.shown(&systems);
        self.raw = systems;
        self.taste = taste;
        if !self.sources().contains(&self.source) {
            self.source = self.sources().into_iter().next().unwrap_or_else(|| "squad".into());
            self.draft = false;
        }
        if self.draft_of(&self.source).is_none() {
            self.draft = false;
        }
        // The work handed in (now in review), or given up: nothing under way to show.
        if self.working_of(&self.source).is_none() {
            self.working = false;
        }
        // The draft accepted, sent back or gone: nothing to compare.
        if self.compare && !self.can_compare() {
            self.end_compare();
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
        let map = self.current().and_then(|s| s.flows.clone());
        self.map.arrived(map.as_ref());
        // A section that has nothing now: back to the overview.
        if let Some(system) = self.current()
            && count(system, self.section).is_none()
        {
            self.section = Section::Overview;
        }
    }

    /// The taste of the source on screen: the squad's when it has no system yet.
    pub fn current_taste(&self) -> Option<&Taste> {
        let key = self.current().map_or_else(|| "squad".to_owned(), base_key);
        self.taste.iter().find(|t| t.key() == key)
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
        let (draft, working) = (self.draft, self.working);
        for system in self.systems.iter().filter(|s| s.draft.is_none() || (draft && in_review(s)) || (working && is_working(s))) {
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

    // ── Compare and review ──

    /// The source's accepted system.
    pub fn accepted_of(&self, source: &str) -> Option<&DesignSystem> {
        self.systems.iter().find(|s| s.draft.is_none() && base_key(s) == source)
    }

    /// Whether the system on screen has a draft to compare: both are here.
    pub fn can_compare(&self) -> bool {
        self.accepted_of(&self.source).is_some() && self.draft_of(&self.source).is_some()
    }

    /// The side on show.
    pub fn side(&self) -> Side {
        if self.draft { Side::After } else { Side::Now }
    }

    /// Comparing begins on After, the draft.
    pub fn begin_compare(&mut self) -> bool {
        if !self.can_compare() {
            return false;
        }
        self.compare = true;
        // Both sides as the repository has them: the draft's are examples.
        self.systems = self.shown(&self.raw);
        self.show_side(Side::After);
        true
    }

    pub fn end_compare(&mut self) {
        self.compare = false;
        self.redo_open = false;
        self.systems = self.shown(&self.raw);
    }

    /// Now or After on show: the whole screen flips, the place in the map kept where it is.
    pub fn show_side(&mut self, side: Side) {
        self.draft = side == Side::After && self.draft_of(&self.source).is_some();
        self.working = false;
        self.lightbox = None;
        let map = self.current().and_then(|s| s.flows.clone());
        self.map.arrived(map.as_ref());
        if let Some(system) = self.current()
            && count(system, self.section).is_none()
        {
            self.section = Section::Overview;
        }
    }

    /// The UI maps compared: the accepted one's and the draft's (an empty one
    /// for a side that has none).
    /// Both as examples: a draft is the repository's, so the person's real
    /// data is never held against it.
    pub fn compared_maps(&self) -> Option<(FlowMap, FlowMap)> {
        let now = self.raw.iter().find(|s| s.draft.is_none() && base_key(s) == self.source)?;
        let after = self.raw.iter().find(|s| in_review(s) && base_key(s) == self.source)?;
        let example = |s: &DesignSystem| s.flows.as_ref().map(|m| crate::flows::with_data(m, crate::flows::Data::Example)).unwrap_or_default();
        Some((example(now), example(after)))
    }

    /// What the draft changed in the UI map, while comparing.
    pub fn compare_changes(&self) -> Option<compare::Changes> {
        if !self.compare {
            return None;
        }
        let (now, after) = self.compared_maps()?;
        Some(compare::changes(&now, &after))
    }

    /// ←/→ while comparing: the changed screen after (or before) the one open,
    /// in the map on show — across its flows. Whether one was.
    pub fn step_changed(&mut self, forward: bool) -> bool {
        let Some(changes) = self.compare_changes() else { return false };
        let Some(map) = self.current().and_then(|s| s.flows.clone()) else { return false };
        let flow = self.map.flow(&map).map(|f| f.id.clone());
        let at = flow.as_deref().zip(self.map.screen.as_deref());
        let Some((flow, screen)) = changes.step(&map, at, forward) else { return false };
        self.map.pick_flow(&flow);
        self.map.screen = Some(screen);
        self.map.chip = None;
        true
    }

    // ── Version history ──

    /// The project the source is (`None`: the squad's own).
    pub fn source_project(&self) -> Option<String> {
        self.source.strip_prefix("project:").map(str::to_owned)
    }

    /// The daemon's list for a system: kept by its source.
    pub fn versions_arrived(&mut self, project: Option<String>, versions: Vec<DesignVersion>) {
        let source = project.map_or_else(|| "squad".to_owned(), |p| format!("project:{p}"));
        if self.versions_asked.as_ref() == Some(&source) {
            self.versions_asked = None;
        }
        self.versions.insert(source, versions);
    }

    /// The system on screen's versions; `None`: not said yet.
    pub fn shown_versions(&self) -> Option<&[DesignVersion]> {
        self.versions.get(&self.source).map(Vec::as_slice)
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

/// Who wrote a note, and where it came from: the person's own, or the
/// designer's from what it saw — each its words' key and what fills them.
pub fn note_origin(note: &TasteNote) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    if !note.by_designer() {
        return vec![note_from(note)];
    }
    vec![("design.taste_by_designer", Vec::new()), note_from(note)]
}

/// Where a taste note came from, in a line: its words' key and what fills them.
pub fn note_from(note: &TasteNote) -> (&'static str, Vec<(&'static str, String)>) {
    let mut args = Vec::new();
    let key = match note.from {
        TasteFrom::Person => "design.taste_from_person",
        TasteFrom::SentBack { ticket } => {
            args.push(("id", ticket.to_string()));
            "design.taste_from_sent_back"
        }
        TasteFrom::Answered { ticket } => {
            args.push(("id", ticket.to_string()));
            "design.taste_from_answered"
        }
        TasteFrom::Noticed { ticket: Some(ticket) } => {
            args.push(("id", ticket.to_string()));
            "design.taste_from_noticed_on"
        }
        TasteFrom::Noticed { ticket: None } => "design.taste_from_noticed",
    };
    (key, args)
}

/// A file's date, `2026-11-03`, as year, month and day.
pub fn ymd(date: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let day: u32 = parts.next()?.get(..2)?.parse().ok().filter(|d| (1..=31).contains(d))?;
    Some((year, month, day))
}

/// The decisions grouped by month, newest first: `(2026, 11)` and its
/// entries' places in `decisions`.
pub fn by_month(decisions: &[equipe_core::protocol::design::Decision]) -> Vec<((i32, u32), Vec<usize>)> {
    let mut out: Vec<((i32, u32), Vec<usize>)> = Vec::new();
    for (i, d) in decisions.iter().enumerate() {
        let month = ymd(&d.date).map_or((0, 0), |(y, m, _)| (y, m));
        match out.iter_mut().find(|(m, _)| *m == month) {
            Some((_, list)) => list.push(i),
            None => out.push((month, vec![i])),
        }
    }
    out
}

/// A draft handed in and waiting for the person: offered for accepting and comparing.
pub fn in_review(system: &DesignSystem) -> bool {
    system.draft.as_ref().is_some_and(|d| !d.working)
}

/// Work a ticket is still making in its worktree: shown as it lands, never
/// accepted, compared or called in review.
pub fn is_working(system: &DesignSystem) -> bool {
    system.draft.as_ref().is_some_and(|d| d.working)
}

/// What work under way has made so far: the flows in its map, their
/// screens, the pictures shot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Landed {
    pub flows: usize,
    pub screens: usize,
    pub pictures: usize,
}

impl Landed {
    pub fn is_empty(self) -> bool {
        self.flows == 0 && self.screens == 0 && self.pictures == 0
    }
}

pub fn landed(system: &DesignSystem) -> Landed {
    let flows = system.flows.as_ref().map_or(&[][..], |m| m.flows.as_slice());
    Landed {
        flows: flows.len(),
        screens: flows.iter().map(|f| f.screens.len()).sum(),
        pictures: flows.iter().flat_map(|f| &f.screens).flat_map(|s| &s.states).map(|s| s.pictures.len()).sum(),
    }
}

/// Work under way on the source on screen (`DesignUi::making`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Making {
    pub ticket: equipe_core::protocol::TicketId,
    pub title: String,
    /// Who is at it — name and role — when the team says.
    pub who: Option<(String, String)>,
    /// What it is doing now, in a few words: its own, else what its last tool touched.
    pub doing: Option<String>,
    /// In the middle of a turn.
    pub busy: bool,
    pub landed: Landed,
}

impl Making {
    /// The header's chip: "Creating · T-378", and what it is doing now.
    pub fn chip(&self) -> (String, Option<String>) {
        (t!("design.working_chip", id = self.ticket), self.doing.clone())
    }

    /// Who is making it, and on which ticket: "designer · T-378 “UI map”".
    pub fn whose(&self) -> String {
        match &self.who {
            Some((name, _)) => t!("design.working_who", name = name, id = self.ticket, title = self.title),
            None => t!("design.working_ticket", id = self.ticket, title = self.title),
        }
    }

    /// What it is doing now, in a line — `None` when the team doesn't say.
    pub fn now(&self) -> Option<String> {
        self.doing.as_ref().map(|what| t!("design.working_now", what = what))
    }

    /// What has landed so far, in a line.
    pub fn so_far(&self) -> String {
        let Landed { flows, screens, pictures } = self.landed;
        if self.landed.is_empty() {
            return t!("design.working_nothing_yet").to_owned();
        }
        t!("design.working_landed", flows = flows, screens = screens, pictures = pictures)
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
        DesignSystem { project: project.map(str::to_owned), draft: draft.map(|t| DraftOf { ticket: t, title: "브랜드".into(), working: false }), dir: "/d".into(), tokens, ..Default::default() }
    }

    /// T-378 at work on the source's UI map: `shots` pictures taken of each screen named.
    fn making(project: Option<&str>, screens: &[(&str, usize)]) -> DesignSystem {
        use equipe_core::protocol::design::{Flow, Picture, Screen, ScreenState};
        let screen = |(id, shots): &(&str, usize)| Screen {
            id: (*id).into(),
            title: (*id).into(),
            states: vec![ScreenState { id: "default".into(), label: "기본".into(), kind: "default".into(), note: None, pictures: (0..*shots).map(|n| Picture { path: format!("/w/{id}.{n}.png"), mode: "light".into(), width: 390, taken: 1, ..Default::default() }).collect() }],
            ..Default::default()
        };
        let (first, rest) = screens.split_at(screens.len().min(2));
        let flows = [("onboarding", first), ("checkout", rest)].into_iter().filter(|(_, s)| !s.is_empty()).map(|(id, s)| Flow { id: id.into(), title: id.into(), screens: s.iter().map(screen).collect(), ..Default::default() }).collect();
        DesignSystem {
            draft: Some(DraftOf { ticket: 378, title: "화면 지도".into(), working: true }),
            dir: "/w/t378/design".into(),
            flows: Some(FlowMap { flows, ..Default::default() }),
            ..system(project, None, vec![])
        }
    }

    #[test]
    fn work_under_way_shows_only_when_asked_and_is_never_in_review() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        ui.arrived(vec![system(None, None, vec![]), making(None, &[("login", 2), ("signup", 0), ("plans", 1)])], "/f".into(), vec![], &key);
        assert_eq!(ui.sources(), ["squad"]);
        assert!(ui.draft_of("squad").is_none(), "not a draft in review");
        assert_eq!(ui.working_of("squad").and_then(|s| s.draft.as_ref()).map(|d| d.ticket), Some(378));
        assert!(!ui.can_compare() && !ui.begin_compare(), "nothing to compare it with");
        assert!(ui.current().unwrap().draft.is_none(), "the accepted one, until asked");
        // Viewed: on show, read-only; the draft's switch leaves it.
        assert!(ui.view_working(true));
        assert!(is_working(ui.current().unwrap()) && !in_review(ui.current().unwrap()));
        let map = ui.current().unwrap().flows.as_ref().unwrap();
        assert_eq!(ui.map.flow(map).map(|f| f.id.as_str()), Some("onboarding"), "its map, from its first flow");
        ui.show_side(Side::Now);
        assert!(!ui.working && ui.current().unwrap().draft.is_none());
        // Alone: the source is still empty, its work not shown unasked.
        ui.arrived(vec![making(None, &[("login", 1)])], "/f".into(), vec![], &key);
        assert_eq!(ui.sources(), ["squad"]);
        assert!(ui.current().is_none());
        assert!(ui.view_working(true) && ui.current().is_some());
        // Handed in: in review now, and the view goes back to what is accepted.
        let mut handed = making(None, &[("login", 1)]);
        handed.draft.as_mut().unwrap().working = false;
        ui.arrived(vec![system(None, None, vec![]), handed.clone()], "/f".into(), vec![], &key);
        assert!(!ui.working && ui.working_of("squad").is_none());
        assert_eq!(ui.draft_of("squad").map(|s| s.dir.as_str()), Some("/w/t378/design"));
        // In review and under way at once: the one in review comes first.
        ui.arrived(vec![system(None, None, vec![]), handed, making(None, &[("plans", 1)])], "/f".into(), vec![], &key);
        assert!(ui.working_of("squad").is_none() && !ui.view_working(true));
        assert!(ui.can_compare());
        // A project's work is its own: not the squad's.
        ui.arrived(vec![system(None, None, vec![]), making(Some("web"), &[("login", 1)])], "/f".into(), vec![], &key);
        assert!(ui.working_of("squad").is_none() && ui.working_of("project:web").is_some());
    }

    #[test]
    fn the_chip_and_the_empty_map_say_who_is_making_it_what_it_is_doing_and_what_has_landed() {
        use crate::state::{Member, Model, Presence, Verb};
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        let mut model = Model::new("acme");
        assert!(ui.making(&model).is_none(), "nothing under way");
        ui.arrived(vec![system(None, None, vec![]), making(None, &[("login", 2), ("signup", 0), ("plans", 3)])], "/f".into(), vec![], &key);
        // The team not heard from yet: the ticket alone.
        let m = ui.making(&model).unwrap();
        assert_eq!((m.ticket, m.who.clone(), m.doing.clone(), m.busy), (378, None, None, false));
        assert_eq!(m.landed, Landed { flows: 2, screens: 3, pictures: 5 });
        assert_eq!(m.chip(), ("만드는 중 · T-378".to_owned(), None));
        assert_eq!(m.whose(), "T-378 「화면 지도」");
        assert_eq!(m.now(), None);
        assert_eq!(m.so_far(), "지금까지 흐름 2 · 화면 3 · 그림 5");
        // The designer at it, mid-turn, saying what it is at.
        let designer = |presence, what: Option<&str>, target: Option<&str>| Member {
            session: 4, name: "designer".into(), role: "designer".into(), presence, verb: Verb::Editing, ticket: Some(378), memos: 0, context: None, model: None, engine: None,
            estimate: what.map(|w| equipe_core::protocol::Estimate { what: Some(w.into()), ..Default::default() }), target: target.map(Into::into), last_seen: 0, failed: false,
        };
        model.members = vec![designer(Presence::Working, Some("결제 흐름 상태 찍는 중"), Some("/w/t378/design/flows/checkout/plans/default.light.390.png"))];
        let m = ui.making(&model).unwrap();
        assert!(m.busy);
        assert_eq!(m.chip(), ("만드는 중 · T-378".to_owned(), Some("결제 흐름 상태 찍는 중".to_owned())));
        assert_eq!(m.whose(), "designer · T-378 「화면 지도」");
        assert_eq!(m.now().as_deref(), Some("지금: 결제 흐름 상태 찍는 중"));
        // No words of its own: the file its last tool touched, by name.
        model.members = vec![designer(Presence::Ready, None, Some("/w/t378/design/flows/flows.json"))];
        let m = ui.making(&model).unwrap();
        assert!(!m.busy);
        assert_eq!(m.now().as_deref(), Some("지금: flows.json"));
        // Nothing landed yet.
        ui.arrived(vec![system(None, None, vec![]), DesignSystem { flows: None, ..making(None, &[]) }], "/f".into(), vec![], &key);
        assert_eq!(ui.making(&model).unwrap().so_far(), "아직 들어온 화면이 없어요");
        // Every word, in every language, filled.
        let m = Making { ticket: 378, title: "t".into(), who: Some(("designer".into(), "designer".into())), doing: Some("x".into()), busy: true, landed: Landed { flows: 1, screens: 2, pictures: 3 } };
        for said in [m.chip().0, m.whose(), m.now().unwrap(), m.so_far()] {
            assert!(!said.contains('{') && !said.starts_with("design."), "{said}");
        }
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
        assert_eq!(sections(&s), [Section::Overview, Section::Colors, Section::Spacing, Section::Guidelines, Section::Taste]);
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
        ui.arrived(vec![system(None, None, vec![]), system(Some("web"), None, vec![]), system(None, Some(144), vec![])], "/f".into(), vec![], &key);
        assert_eq!(ui.sources(), ["squad", "project:web"]);
        assert!(ui.current().unwrap().draft.is_none());
        assert_eq!(ui.draft_of("squad").unwrap().draft.as_ref().unwrap().ticket, 144);
        ui.draft = true;
        assert_eq!(ui.current().unwrap().draft.as_ref().unwrap().ticket, 144);
        // The draft is accepted: it is the system now, and the screen follows.
        ui.arrived(vec![system(None, None, vec![])], "/f".into(), vec![], &key);
        assert!(!ui.draft && ui.current().unwrap().draft.is_none());
        // Only a draft: it shows.
        ui.arrived(vec![system(None, Some(150), vec![])], "/f".into(), vec![], &key);
        assert_eq!(ui.current().unwrap().draft.as_ref().unwrap().ticket, 150);
        // The project picked goes: back to the first.
        ui.source = "project:web".into();
        ui.arrived(vec![system(None, None, vec![])], "/f".into(), vec![], &key);
        assert_eq!(ui.source, "squad");
    }

    #[test]
    fn what_changed_is_since_the_person_last_looked_and_their_look_is_kept() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        let v1 = vec![token("color.accent", "color", json!("#2E5C8A"))];
        ui.arrived(vec![system(None, None, v1)], "/f".into(), vec![], &key);
        assert!(ui.changes("squad").is_empty(), "never looked: nothing called new");
        assert!(ui.looked(&key));
        ui.arrived(vec![system(None, None, vec![token("color.accent", "color", json!("#1D4ED8")), token("color.dot", "color", json!("#3F6FF5"))])], "/f".into(), vec![], &key);
        let changes = ui.changes("squad");
        assert_eq!(changes.len(), 2, "{changes:?}");
        // Saved again live: still against what was seen when this look began.
        ui.arrived(vec![system(None, None, vec![token("color.accent", "color", json!("#1D4ED8"))])], "/f".into(), vec![], &key);
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
        let s = system(None, None, vec![token("color.accent", "color", json!("#1D4ED8")), token("color.bg", "color", json!("#FFF")), token("card.bg", "color", json!("#FFF")), token("color.text", "color", json!("#111"))]);
        let names: Vec<&str> = primary_colors(&s, 3).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["color.accent", "color.bg", "color.text"], "a component's own bg doesn't lead");
        let mut with_logos = system(None, None, vec![]);
        with_logos.assets = ["assets/logo/mark-mono-on-dark.svg", "assets/logo/mark.svg", "assets/icons/x.svg", "assets/logo/wordmark.svg"]
            .iter()
            .map(|n| equipe_core::protocol::design::Asset { path: format!("/d/{n}"), name: (*n).into(), kind: if n.contains("logo") { AssetKind::Logo } else { AssetKind::Icon }, size: 1 })
            .collect();
        assert_eq!(DesignUi::logos(&with_logos), [1, 3, 0], "plain ones first");
    }

    fn mapped(project: Option<&str>, draft: Option<u32>, shots: &[(&str, &str)]) -> DesignSystem {
        use equipe_core::protocol::design::{Flow, Picture, Screen, ScreenState};
        let screens = shots
            .iter()
            .map(|(id, hash)| Screen {
                id: (*id).into(),
                title: (*id).into(),
                states: vec![ScreenState { id: "default".into(), label: "기본".into(), kind: "default".into(), note: None, pictures: vec![Picture { path: format!("/d/{id}.png"), mode: "light".into(), width: 390, taken: 1, hash: Some((*hash).into()), ..Default::default() }] }],
                ..Default::default()
            })
            .collect();
        DesignSystem { flows: Some(FlowMap { flows: vec![Flow { id: "app".into(), title: "App".into(), screens, ..Default::default() }], ..Default::default() }), ..system(project, draft, vec![]) }
    }

    #[test]
    fn comparing_flips_the_whole_screen_between_now_and_after_and_steps_through_what_changed() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        ui.arrived(vec![mapped(None, None, &[("home", "a"), ("list", "b"), ("pay", "c")])], "/f".into(), vec![], &key);
        assert!(!ui.can_compare() && !ui.begin_compare(), "no draft: nothing to compare");
        assert!(ui.compare_changes().is_none());
        ui.arrived(vec![mapped(None, None, &[("home", "a"), ("list", "b"), ("pay", "c")]), mapped(None, Some(170), &[("home", "A"), ("list", "b"), ("pay", "C")])], "/f".into(), vec![], &key);
        assert!(ui.can_compare() && ui.begin_compare());
        assert_eq!(ui.side(), Side::After, "begins on the draft");
        assert_eq!(ui.current().unwrap().draft.as_ref().unwrap().ticket, 170);
        assert_eq!(ui.compare_changes().unwrap().count(), 2);
        assert!(ui.step_changed(true));
        assert_eq!(ui.map.screen.as_deref(), Some("home"));
        assert!(ui.step_changed(true));
        assert_eq!(ui.map.screen.as_deref(), Some("pay"), "list didn't change: passed by");
        assert!(!ui.step_changed(true), "the last changed one");
        ui.show_side(Side::Now);
        assert!(ui.current().unwrap().draft.is_none());
        assert_eq!(ui.map.screen.as_deref(), Some("pay"), "the same screen, as it is now");
        // Accepted: the draft goes, and so does the comparing.
        ui.redo_open = true;
        ui.arrived(vec![mapped(None, None, &[("home", "A"), ("list", "b"), ("pay", "C")])], "/f".into(), vec![], &key);
        assert!(!ui.compare && !ui.redo_open && !ui.draft);
    }

    #[test]
    fn versions_are_kept_by_the_system_they_are_of() {
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        ui.arrived(vec![system(None, None, vec![]), system(Some("web"), None, vec![])], "/f".into(), vec![], &key);
        assert_eq!(ui.source_project(), None);
        assert!(ui.shown_versions().is_none(), "not said yet");
        let v = |c: &str| DesignVersion { commit: c.repeat(40), at: 1, subject: c.into(), ticket: None, parts: vec![] };
        ui.versions_asked = Some("project:web".into());
        ui.versions_arrived(Some("web".into()), vec![v("a"), v("b")]);
        assert!(ui.versions_asked.is_none());
        assert!(ui.shown_versions().is_none(), "the squad's still unsaid");
        ui.versions_arrived(None, vec![]);
        assert_eq!(ui.shown_versions().map(<[_]>::len), Some(0));
        ui.source = "project:web".into();
        assert_eq!((ui.source_project().as_deref(), ui.shown_versions().map(<[_]>::len)), (Some("web"), Some(2)));
    }

    #[test]
    fn the_taste_tab_is_always_there_and_shows_the_taste_of_the_source_on_screen() {
        use equipe_core::protocol::design::Decision;
        let s = system(None, None, vec![]);
        assert_eq!(sections(&s), [Section::Overview, Section::Taste], "there with nothing in it, to invite the first note");
        assert_eq!(Section::parse("taste"), Some(Section::Taste));
        let taste = |project: Option<&str>, note: &str| Taste { project: project.map(str::to_owned), notes: vec![TasteNote { text: note.into(), ..Default::default() }], ..Default::default() };
        let mut ui = DesignUi::default();
        let key = |s: &DesignSystem| s.key();
        ui.arrived(vec![], "/f".into(), vec![taste(None, "둥글게")], &key);
        assert_eq!(ui.current_taste().unwrap().notes[0].text, "둥글게", "the squad's, before it has a system");
        ui.arrived(vec![system(None, None, vec![]), system(Some("web"), None, vec![])], "/f".into(), vec![taste(None, "둥글게"), taste(Some("web"), "선은 얇게")], &key);
        ui.source = "project:web".into();
        assert_eq!(ui.current_taste().unwrap().notes[0].text, "선은 얇게");
        // Who wrote it: the person's own says so once; the designer's says who, then where from.
        let theirs = TasteNote { text: "x".into(), from: TasteFrom::SentBack { ticket: 336 }, ..Default::default() };
        assert_eq!(note_origin(&theirs).iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["design.taste_by_designer", "design.taste_from_sent_back"]);
        assert_eq!(note_origin(&TasteNote::default()).iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["design.taste_from_person"]);
        // Where each came from has words in every language.
        for from in [TasteFrom::Person, TasteFrom::SentBack { ticket: 336 }, TasteFrom::Answered { ticket: 272 }, TasteFrom::Noticed { ticket: Some(9) }, TasteFrom::Noticed { ticket: None }] {
            let (key, args) = note_from(&TasteNote { text: "x".into(), from, ..Default::default() });
            let said = crate::i18n::catalog().fill(key, &args);
            assert!(!said.is_empty() && !said.contains('{') && !said.starts_with("design."), "{key}: {said}");
            assert!(args.iter().all(|(_, v)| said.contains(v.as_str())), "{said}");
        }
        let d = |date: &str| Decision { date: date.into(), question: "q".into(), chosen: "c".into(), ..Default::default() };
        assert_eq!(by_month(&[d("2026-11-03"), d("2026-11-01"), d("2026-10-30"), d("someday")]), [((2026, 11), vec![0, 1]), ((2026, 10), vec![2]), ((0, 0), vec![3])]);
        assert_eq!(ymd("2026-11-03"), Some((2026, 11, 3)));
        assert_eq!(ymd("2026-13-03"), None);
    }
}
