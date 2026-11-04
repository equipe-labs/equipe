//! `equipe design lint`: a product screen held to rules written as code, not
//! as advice — the floor no one waives, the habits generated screens fall
//! into (kept only with a reason in the decision log), and finishing notes.
//!
//! - `rules`: the rules, as data.
//! - `source`: a UI source read loosely (CSS, HTML, JSX, Vue, Svelte).
//! - `scan`: the rules checked in the source.
//! - `render`: the rules checked in a browser at each width — what
//!   `lint.mjs` measured, judged here.
//! - `waivers`: habits kept on purpose, read from `design/decisions/`.
//! - `record`: a screen's latest check kept in `design/checks/`, the edits
//!   not checked yet, and what a designer's hand-in needs.

pub mod hook;
pub mod record;
pub mod render;
pub mod rules;
pub mod scan;
pub mod source;
pub mod waivers;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use rules::{RULES, Rule, Tier};

use super::tokens;
use super::values::{self, Color};

/// The widths a screen is drawn at by default, phone to wide desktop.
pub const WIDTHS: [u32; 7] = [360, 390, 768, 1024, 1280, 1440, 1920];

/// The line a run ends with, for screens that show it: this word, then the
/// summary as JSON.
pub const MARK: &str = "equipe-design-check:";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub tier: Tier,
    /// The file, as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The width it was seen at, when drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// What was found, in a line: the value, the element, the words.
    pub evidence: String,
    /// Kept on purpose: the reason, as the decision log says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waived: Option<String>,
}

impl Finding {
    pub fn new(rule: &'static str, file: &str, line: Option<u32>, evidence: impl Into<String>) -> Self {
        let tier = rules::rule(rule).map_or(Tier::Finish, |r| r.tier);
        Finding { rule: rule.to_owned(), tier, file: Some(file.to_owned()), line, width: None, evidence: evidence.into(), waived: None }
    }

    /// Open: not waived (a floor finding never is).
    pub fn open(&self) -> bool {
        self.waived.is_none()
    }

    pub fn place(&self) -> String {
        let mut at = self.file.clone().unwrap_or_default();
        if let Some(line) = self.line {
            at.push_str(&format!(":{line}"));
        }
        if let Some(width) = self.width {
            if !at.is_empty() {
                at.push(' ');
            }
            at.push_str(&format!("@{width}"));
        }
        at
    }
}

/// What a run looked at and found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub files: Vec<String>,
    /// The widths it was drawn at; none when only the source was read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<u32>,
    /// The rules run, by id.
    pub checked: Vec<String>,
    /// The rules that couldn't be, and why (no spacing tokens to hold to).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<(String, String)>,
    pub findings: Vec<Finding>,
    /// Waivers in the decision log that don't hold: a floor rule's, one with no reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<String>,
    /// The pictures drawn, one per width.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
}

/// The summary a screen shows and a record keeps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Floor rules that passed, of those run.
    pub floor: (usize, usize),
    /// Habits found and not waived.
    pub habits: usize,
    /// Habits kept on purpose: the rule and the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waived: Vec<(String, String)>,
    pub finish: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<u32>,
    /// The floor rules that failed, by id, and their first evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing: Vec<(String, String)>,
    /// The open habits, by id, and their first evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<(String, String)>,
    /// The finishing notes, by id, and their first evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<(String, String)>,
    /// The screen's name, when it was checked as one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    /// Where the record is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    /// Rules that couldn't run, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

impl Summary {
    pub fn passes(&self) -> bool {
        self.floor.0 == self.floor.1
    }
}

impl Report {
    pub fn summary(&self) -> Summary {
        let floor_run: Vec<&String> = self.checked.iter().filter(|id| rules::rule(id).is_some_and(|r| r.tier == Tier::Floor)).collect();
        let first = |tier: Tier, open: bool| -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = Vec::new();
            for f in self.findings.iter().filter(|f| f.tier == tier && f.open() == open) {
                if !out.iter().any(|(id, _)| *id == f.rule) {
                    out.push((f.rule.clone(), f.evidence.clone()));
                }
            }
            out
        };
        let failing = first(Tier::Floor, true);
        let open = first(Tier::Habit, true);
        let waived = {
            let mut out: Vec<(String, String)> = Vec::new();
            for f in self.findings.iter().filter(|f| f.tier == Tier::Habit) {
                if let Some(why) = &f.waived
                    && !out.iter().any(|(id, _)| *id == f.rule)
                {
                    out.push((f.rule.clone(), why.clone()));
                }
            }
            out
        };
        let notes = first(Tier::Finish, true);
        Summary {
            floor: (floor_run.len() - failing.len().min(floor_run.len()), floor_run.len()),
            habits: open.len(),
            waived,
            finish: notes.len(),
            widths: self.widths.clone(),
            failing,
            open,
            notes,
            screen: None,
            record: None,
            skipped: self.skipped.iter().map(|(id, _)| id.clone()).collect(),
        }
    }

    /// The findings of a tier and above: `floor` alone, or all.
    pub fn only(&mut self, tier: Tier) {
        self.findings.retain(|f| f.tier <= tier);
        self.checked.retain(|id| rules::rule(id).is_some_and(|r| r.tier <= tier));
    }

    /// As a person reads it: each tier's findings, worst first, then the count.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for tier in [Tier::Floor, Tier::Habit, Tier::Finish] {
            let mut found: Vec<&Finding> = self.findings.iter().filter(|f| f.tier == tier).collect();
            found.sort_by_key(|f| RULES.iter().position(|r| r.id == f.rule));
            if found.is_empty() {
                continue;
            }
            out.push_str(&format!("{}\n", match tier {
                Tier::Floor => "Floor — never waived; fix before handing in:",
                Tier::Habit => "Habits — fix, or keep with a reason in design/decisions/ (`equipe design lint --waive <id> --why \"…\"`):",
                Tier::Finish => "Finish — worth doing, never blocking:",
            }));
            let mut last_rule = "";
            for f in found {
                if f.rule != last_rule {
                    let rule = rules::rule(&f.rule);
                    out.push_str(&format!("  {} {} — {}\n", if f.waived.is_some() { "~" } else { mark(tier) }, f.rule, rule.map_or("", |r| r.checks)));
                    if let Some(rule) = rule
                        && f.waived.is_none()
                    {
                        out.push_str(&format!("      fix: {}\n", rule.fix));
                    }
                    last_rule = &f.rule;
                }
                match &f.waived {
                    Some(why) => out.push_str(&format!("      {} {} (kept: {why})\n", f.place(), f.evidence)),
                    None => out.push_str(&format!("      {} {}\n", f.place(), f.evidence)),
                }
            }
        }
        for refused in &self.refused {
            out.push_str(&format!("! {refused}\n"));
        }
        for (id, why) in &self.skipped {
            out.push_str(&format!("- {id} not checked: {why}\n"));
        }
        let s = self.summary();
        out.push_str(&format!(
            "\nFloor {}/{} · {} habit(s) to fix · {} kept · {} finish note(s){}\n",
            s.floor.0,
            s.floor.1,
            s.habits,
            s.waived.len(),
            s.finish,
            if self.widths.is_empty() { " · source only (add --render to draw it)".to_owned() } else { format!(" · drawn at {}", self.widths.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")) }
        ));
        out
    }
}

fn mark(tier: Tier) -> &'static str {
    match tier {
        Tier::Floor => "✗",
        Tier::Habit => "!",
        Tier::Finish => "·",
    }
}

/// The summary line screens read: [`MARK`] and the JSON.
pub fn mark_line(summary: &Summary) -> String {
    format!("{MARK} {}", serde_json::to_string(summary).unwrap_or_default())
}

/// The summary in a run's output, if it ends with one.
pub fn summary_in(output: &str) -> Option<Summary> {
    let line = output.lines().rev().find(|l| l.starts_with(MARK))?;
    serde_json::from_str(line[MARK.len()..].trim()).ok()
}

/// What the design system lets a screen use, read from its `tokens.json`.
#[derive(Debug, Clone, Default)]
pub struct System {
    /// The design folder.
    pub dir: PathBuf,
    /// Every colour, each mode's: the token and its value.
    pub colours: Vec<(String, Color)>,
    /// The spacing scale, in px.
    pub spacing: Vec<(String, f64)>,
    /// The type scale, in px.
    pub sizes: Vec<(String, f64)>,
    /// The families it names, lower-cased.
    pub families: Vec<String>,
    /// The palette's groups (`color.blue.600` → `blue`), lower-cased: the
    /// utility colours it has.
    pub palettes: Vec<String>,
    /// Whether there was a `tokens.json` to read.
    pub found: bool,
}

const SPACE_WORDS: [&str; 8] = ["space", "spacing", "gap", "inset", "padding", "margin", "gutter", "layout"];
const SIZE_WORDS: [&str; 5] = ["font", "text", "type", "typography", "fontsize"];

impl System {
    pub fn read(dir: &Path) -> System {
        let mut system = System { dir: dir.to_path_buf(), ..System::default() };
        let Ok(text) = std::fs::read_to_string(dir.join(super::folder::TOKENS)) else { return system };
        system.found = true;
        let parsed = tokens::parse(&text);
        for t in &parsed.tokens {
            let parts: Vec<String> = t.name.split('.').map(str::to_ascii_lowercase).collect();
            let has = |words: &[&str]| parts.iter().any(|p| words.iter().any(|w| p == w || p.starts_with(w)));
            match t.kind.as_str() {
                "color" => {
                    for v in std::iter::once(&t.value).chain(t.modes.values()) {
                        if let Some(c) = values::color(v) {
                            system.colours.push((t.name.clone(), c));
                        }
                    }
                    if parts.len() >= 3 && ["color", "colors", "colour", "palette", "raw"].contains(&parts[0].as_str()) {
                        let group = if parts[1] == "raw" || parts[1] == "palette" { &parts[2] } else { &parts[1] };
                        if !system.palettes.contains(group) {
                            system.palettes.push(group.clone());
                        }
                    }
                }
                "dimension" | "number" => {
                    if let Some(px) = values::dimension(&t.value) {
                        if has(&SPACE_WORDS) {
                            system.spacing.push((t.name.clone(), px));
                        } else if has(&SIZE_WORDS) && !parts.iter().any(|p| p.contains("weight") || p.contains("line") || p.contains("radius")) {
                            system.sizes.push((t.name.clone(), px));
                        }
                    }
                }
                // A shadow's colour is the system's too.
                "shadow" => {
                    for v in std::iter::once(&t.value).chain(t.modes.values()) {
                        system.colours.extend(values::shadows(v).unwrap_or_default().into_iter().map(|s| (t.name.clone(), s.color)));
                    }
                }
                "fontFamily" => system.families.extend(values::font_family(&t.value).unwrap_or_default().into_iter().map(|f| f.to_ascii_lowercase())),
                "typography" => {
                    if let Some(style) = values::typography(&t.value) {
                        system.families.extend(style.family.iter().map(|f| f.to_ascii_lowercase()));
                        if let Some(size) = style.size {
                            system.sizes.push((t.name.clone(), size));
                        }
                    }
                }
                _ => {}
            }
        }
        system
    }

    /// The token holding colour `c`, if one does.
    pub fn colour_token(&self, c: Color) -> Option<&str> {
        self.colours.iter().find(|(_, t)| t.r == c.r && t.g == c.g && t.b == c.b && (t.a - c.a).abs() < 0.02).map(|(n, _)| n.as_str())
    }

    pub fn space_token(&self, px: f64) -> Option<&str> {
        self.spacing.iter().find(|(_, v)| (v - px).abs() < 0.5).map(|(n, _)| n.as_str())
    }

    pub fn size_token(&self, px: f64) -> Option<&str> {
        self.sizes.iter().find(|(_, v)| (v - px).abs() < 0.5).map(|(n, _)| n.as_str())
    }
}

/// The design folder a file belongs to: the nearest `design/` beside or
/// above it that holds a `tokens.json` — the file itself may be in it.
pub fn design_dir_of(file: &Path) -> Option<PathBuf> {
    let start = if file.is_dir() { file.to_path_buf() } else { file.parent()?.to_path_buf() };
    let start = std::fs::canonicalize(&start).unwrap_or(start);
    for dir in start.ancestors() {
        if dir.file_name().is_some_and(|n| n == super::folder::DIR) && dir.join(super::folder::TOKENS).is_file() {
            return Some(dir.to_path_buf());
        }
        let inside = dir.join(super::folder::DIR);
        if inside.join(super::folder::TOKENS).is_file() {
            return Some(inside);
        }
    }
    None
}

/// The UI files at `path`: the file, or a folder's (not its dependencies' or builds').
pub fn ui_files(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if path.is_file() {
        if source::Kind::of(path).is_some() {
            out.push(path.to_path_buf());
        }
        return out;
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        if depth > 8 || out.len() > 400 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p.is_dir() {
                if !name.starts_with('.') && !["node_modules", "dist", "build", "target", "out", ".next", "vendor", "checks", "audit", "critiques"].contains(&name.as_str()) {
                    walk(&p, out, depth + 1);
                }
            } else if source::Kind::of(&p).is_some() {
                out.push(p);
            }
        }
    }
    walk(path, &mut out, 0);
    out
}

/// A token file: its literals are the system's own values, not a screen's.
pub fn is_token_file(path: &Path) -> bool {
    let p = path.to_string_lossy().to_ascii_lowercase();
    p.contains("token") || p.contains("/design/build/") || p.contains("/theme.") || p.contains("variables.")
}

/// Lint the source of `files` against `system`, then the decision log's waivers.
pub fn lint_sources(files: &[PathBuf], system: &System) -> Report {
    let mut report = Report::default();
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        let Some(source) = source::Source::read(file, text) else { continue };
        report.files.push(source.path.clone());
        report.findings.extend(scan::scan(&source, system, is_token_file(file)));
    }
    for r in RULES.iter().filter(|r| r.how.in_source()) {
        match scan::skipped(r.id, system) {
            Some(why) => report.skipped.push((r.id.to_owned(), why)),
            None => report.checked.push(r.id.to_owned()),
        }
    }
    report
}

/// The waivers in the design folder's decision log, applied.
pub fn apply_waivers(report: &mut Report, design: &Path, screen: Option<&str>) {
    let log = waivers::read(design);
    report.refused.extend(log.refused.iter().cloned());
    for f in &mut report.findings {
        if let Some(w) = log.waivers.iter().find(|w| w.covers(f, screen)) {
            f.waived = Some(w.why.clone());
        }
    }
}

#[cfg(test)]
mod tests;
