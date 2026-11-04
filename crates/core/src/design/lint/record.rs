//! What a screen's check leaves behind, and what a designer's hand-in needs.
//!
//! - `design/checks/<screen>.json`: the screen's latest check — what was run,
//!   found and drawn — and the second look's verdict on those pictures.
//! - `design/checks/pending.json`: the UI files edited since they were last
//!   checked, written as they are edited (the edit-time check).
//!
//! A designer hands in when nothing it edited waits unchecked, and each
//! screen it checked passes the floor and has a second look that passed
//! after the check (for a new screen, a first look at its first screenful
//! too). Said as what to do, not as a rule.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Finding, Report, Summary};

pub const DIR: &str = "checks";
pub const PENDING: &str = "pending.json";

/// One thing the second look says to fix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fix {
    /// Where on the screen: the top bar, the second card.
    #[serde(rename = "where")]
    pub place: String,
    pub what: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// The second look's verdict on a check's pictures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    pub pass: bool,
    pub fixes: Vec<Fix>,
    /// Its impression, in a sentence or two.
    pub summary: String,
    /// When it answered, in unix seconds.
    pub at: u64,
    /// Who looked: the engine run with nothing but the pictures and the
    /// brief (`claude`, `codex`), or the member's own fresh subagent (`subagent`).
    pub by: String,
    /// The widths it saw.
    pub widths: Vec<u32>,
    /// It saw the first screenful alone (a new screen's, before the rest is built).
    pub first: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stamp {
    pub path: String,
    /// When it was last changed when checked, in unix seconds.
    pub modified: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub screen: String,
    /// When it was checked, in unix seconds.
    pub at: u64,
    pub files: Vec<Stamp>,
    /// Drawn in a browser, not only read.
    pub rendered: bool,
    pub summary: Summary,
    pub findings: Vec<Finding>,
    pub pictures: Vec<String>,
    /// A new screen: its first screenful gets a look of its own first.
    pub new: bool,
    /// The look at the first screenful alone.
    pub first: Option<Verdict>,
    /// The look at the whole, at every width.
    pub verdict: Option<Verdict>,
}

/// A screen's name as a path under `checks/`: `onboarding/login` stays two
/// parts; anything else becomes `-`.
pub fn safe(screen: &str) -> String {
    screen
        .split('/')
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(|p| p.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect::<String>())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn path(design: &Path, screen: &str) -> PathBuf {
    design.join(DIR).join(format!("{}.json", safe(screen)))
}

/// The folder a screen's pictures are drawn into.
pub fn pictures_dir(design: &Path, screen: &str) -> PathBuf {
    design.join(DIR).join(safe(screen))
}

pub fn read(design: &Path, screen: &str) -> Option<Record> {
    serde_json::from_str(&std::fs::read_to_string(path(design, screen)).ok()?).ok()
}

pub fn write(design: &Path, record: &Record) -> std::io::Result<PathBuf> {
    let file = path(design, &record.screen);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&file, format!("{}\n", serde_json::to_string_pretty(record).unwrap_or_default()))?;
    Ok(file)
}

pub fn modified(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A record of `report`, checked now, keeping the verdicts it had when the
/// files haven't changed since (a re-run that only reads again).
pub fn of(design: &Path, screen: &str, report: &Report, files: &[PathBuf], before: Option<Record>, new: bool) -> Record {
    let stamps: Vec<Stamp> = files.iter().map(|f| Stamp { path: ledger_name(design, f), modified: modified(f) }).collect();
    let unchanged = before.as_ref().is_some_and(|b| b.files == stamps && b.rendered == !report.widths.is_empty());
    let (first, verdict) = match before {
        Some(b) if unchanged => (b.first, b.verdict),
        Some(b) => (b.first, None),
        None => (None, None),
    };
    let mut summary = report.summary();
    summary.screen = Some(screen.to_owned());
    Record { screen: screen.to_owned(), at: now(), files: stamps, rendered: !report.widths.is_empty(), summary, findings: report.findings.clone(), pictures: report.pictures.clone(), new, first, verdict }
}

// ── the edits not checked yet ──

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edited {
    /// Relative to the folder the design folder is in.
    pub path: String,
    pub at: u64,
    /// Made by this edit, not changed.
    pub created: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pending {
    /// Edited and not checked since.
    pub edited: Vec<Edited>,
    /// The screens checked since the hand-in before, by name.
    pub checked: Vec<String>,
}

pub fn pending_path(design: &Path) -> PathBuf {
    design.join(DIR).join(PENDING)
}

pub fn read_pending(design: &Path) -> Pending {
    std::fs::read_to_string(pending_path(design)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn write_pending(design: &Path, pending: &Pending) -> std::io::Result<()> {
    let file = pending_path(design);
    if pending.edited.is_empty() && pending.checked.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, format!("{}\n", serde_json::to_string_pretty(pending).unwrap_or_default()))
}

/// `file` as the ledger names it: relative to the design folder's parent.
pub fn ledger_name(design: &Path, file: &Path) -> String {
    let root = design.parent().unwrap_or(design);
    let real = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    real.strip_prefix(&root).map_or_else(|_| real.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned())
}

/// Not the design's own bookkeeping, nor its generated sheets.
fn tracked(name: &str) -> bool {
    let skip = ["design/checks/", "design/audit/", "design/critiques/", "design/qa/", "design/build/", "design/references/", "node_modules/"];
    !skip.iter().any(|s| name.starts_with(s) || name.contains(&format!("/{s}"))) && !name.contains("specimen") && !super::is_token_file(Path::new(name))
}

/// An edit to `file`: waiting for its check.
pub fn note_edit(design: &Path, file: &Path, created: bool) -> std::io::Result<()> {
    let name = ledger_name(design, file);
    if !tracked(&name) {
        return Ok(());
    }
    let mut pending = read_pending(design);
    match pending.edited.iter_mut().find(|e| e.path == name) {
        Some(e) => {
            e.at = now();
            e.created |= created;
        }
        None => pending.edited.push(Edited { path: name, at: now(), created }),
    }
    write_pending(design, &pending)
}

/// A drawn check of `screen` over `files`: their edits are checked.
/// Returns whether one of them was new.
pub fn note_check(design: &Path, screen: &str, files: &[PathBuf]) -> std::io::Result<bool> {
    let mut pending = read_pending(design);
    let names: Vec<String> = files.iter().map(|f| ledger_name(design, f)).collect();
    let new = pending.edited.iter().any(|e| e.created && names.contains(&e.path));
    let before = pending.edited.len();
    pending.edited.retain(|e| !names.contains(&e.path));
    let cleared = pending.edited.len() != before;
    if cleared && !pending.checked.iter().any(|s| s == screen) {
        pending.checked.push(screen.to_owned());
    }
    write_pending(design, &pending)?;
    Ok(new)
}

/// What stands between a designer and its hand-in, a line each; empty when
/// nothing does.
pub fn hand_in(design: &Path) -> Vec<String> {
    let pending = read_pending(design);
    let mut out = Vec::new();
    for e in &pending.edited {
        out.push(format!("{} was edited and not checked: `equipe design lint {} --render --screen <flow/screen>`, then `equipe design second-look <flow/screen>`.", e.path, e.path));
    }
    for screen in &pending.checked {
        let Some(record) = read(design, screen) else {
            out.push(format!("{screen}: its check record is gone — run `equipe design lint … --render --screen {screen}` again."));
            continue;
        };
        if !record.summary.passes() {
            let failing: Vec<&str> = record.summary.failing.iter().map(|(id, _)| id.as_str()).collect();
            out.push(format!("{screen} fails the floor ({}): fix it and check again — floor rules are never waived.", failing.join(", ")));
            continue;
        }
        if !record.rendered {
            out.push(format!("{screen} was only read, not drawn: check it again with --render."));
            continue;
        }
        let root = design.parent().unwrap_or(design);
        let stale = record.files.iter().any(|f| modified(&root.join(&f.path)) > f.modified);
        if stale {
            out.push(format!("{screen} changed after its check: check it again."));
            continue;
        }
        if record.new && record.first.as_ref().is_none_or(|v| !v.pass) {
            out.push(format!("{screen} is new: its first screenful gets a second look of its own first — `equipe design second-look {screen} --first`."));
        }
        match &record.verdict {
            None => out.push(format!("{screen} has had no second look: `equipe design second-look {screen}`.")),
            Some(v) if v.at < record.at => out.push(format!("{screen} changed after its second look: `equipe design second-look {screen}` again.")),
            Some(v) if !v.pass => out.push(format!("{screen}: the second look asked for {} fix(es) — make them, check again, and ask it again.", v.fixes.len())),
            Some(_) => {}
        }
    }
    out
}

/// After a hand-in: the next ticket starts with nothing pending.
pub fn handed_in(design: &Path) {
    let _ = write_pending(design, &Pending::default());
}
