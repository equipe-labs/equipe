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
    /// Facts the screen needs that neither the brief, the project's files nor
    /// the design system give: asked, never a fix — each with neutral wording
    /// the screen can use until someone answers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    /// Which engine looked and why that one (`design::reviewer`); `None` for
    /// a subagent's answer, or a verdict kept before it was said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<equipe_protocol::engines::Reviewed>,
    /// Against the references it was given: `same`, `below` or `no references`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tier: String,
}

/// A fact the second look found missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Question {
    /// What is missing: the save control's name, the launch date.
    pub what: String,
    /// What the screen can say meanwhile, without the fact.
    pub wording: String,
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
    /// Looks in a row — the first screenful's or the whole's — that asked
    /// for fixes; a pass starts again. At [`LOOKS_BEFORE_THE_PERSON`] what is
    /// left goes to the person with the hand-in instead of round again.
    pub fixes_in_a_row: u32,
}

/// How many looks in a row may ask for fixes before what is left goes to
/// the person with the hand-in, marked, rather than round and round.
pub const LOOKS_BEFORE_THE_PERSON: u32 = 3;

/// Keep a second look's `verdict` on `record`: as its first-screenful look
/// or its whole one, and counted in the row of looks that asked for fixes.
pub fn note_verdict(record: &mut Record, verdict: Verdict) {
    record.fixes_in_a_row = if verdict.pass { 0 } else { record.fixes_in_a_row + 1 };
    if verdict.first {
        record.first = Some(verdict);
    } else {
        record.verdict = Some(verdict);
    }
}

/// Whether `record`'s looks asked for fixes often enough in a row for what
/// is left to go to the person.
fn to_the_person(record: &Record) -> bool {
    record.fixes_in_a_row >= LOOKS_BEFORE_THE_PERSON
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
    // Checked again, its row of looks asking for fixes goes on: only a pass starts it again.
    let fixes_in_a_row = before.as_ref().map_or(0, |b| b.fixes_in_a_row);
    let (first, verdict) = match before {
        Some(b) if unchanged => (b.first, b.verdict),
        Some(b) => (b.first, None),
        None => (None, None),
    };
    let mut summary = report.summary();
    summary.screen = Some(screen.to_owned());
    Record { screen: screen.to_owned(), at: now(), files: stamps, rendered: !report.widths.is_empty(), summary, findings: report.findings.clone(), pictures: report.pictures.clone(), new, first, verdict, fixes_in_a_row }
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

/// Where checks are kept and the hand-in looks for them: the design folder
/// the edit-time check writes edits down in — the one nearest the files
/// checked (or, with none, the one `design` sits in) — whichever design
/// system they were checked against (`--design`). Only a folder with no
/// design folder above it is its own.
pub fn ledger_home(design: &Path, files: &[PathBuf]) -> PathBuf {
    files.iter().find_map(|f| super::design_dir_of(f)).or_else(|| design.parent().and_then(super::design_dir_of).filter(|home| home != design && design.starts_with(home))).unwrap_or_else(|| design.to_path_buf())
}

/// Where `screen`'s check record is: in `design` when it is there, else
/// where the hand-in looks ([`ledger_home`]).
pub fn record_home(design: &Path, screen: &str) -> PathBuf {
    if path(design, screen).is_file() {
        return design.to_path_buf();
    }
    ledger_home(design, &[])
}

/// Not the design's own bookkeeping, nor its generated sheets, nor the
/// directions explored before one was chosen (`design/directions/`): those
/// are kept by the directions' log, and what ships is checked on its own.
fn tracked(name: &str) -> bool {
    let skip = ["design/checks/", "design/audit/", "design/critiques/", "design/qa/", "design/build/", "design/references/", "design/directions/", "node_modules/"];
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
    let root = design.parent().unwrap_or(design);
    for e in &pending.edited {
        let file = root.join(&e.path);
        let elsewhere = checked_elsewhere(design, &file);
        let screen = elsewhere.as_ref().map(|(_, s)| s.clone()).or_else(|| screen_of(design, &file)).unwrap_or_else(|| {
            let stem = Path::new(&e.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            safe(if stem.is_empty() { "page" } else { &stem })
        });
        let mut line = format!(
            "{} was edited and not checked since (every UI file a hand-in ships is drawn and checked first): run `equipe design lint {} --render --screen {screen}`, then `equipe design second-look {screen}`.",
            e.path, e.path
        );
        if let Some((other, _)) = elsewhere {
            let shown = |d: &Path| d.strip_prefix(root).map_or_else(|_| d.display().to_string(), |p| p.display().to_string());
            line.push_str(&format!(" A check of it was kept in `{}/{DIR}/`, not in `{}/{DIR}/` where the hand-in looks: run it again without `--design`.", shown(&other), shown(design)));
        }
        out.push(line);
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
        let latest = [&record.verdict, &record.first].into_iter().flatten().map(|v| v.at).max();
        if to_the_person(&record) && latest.is_some_and(|at| at >= record.at) {
            // Looked at, asked for fixes again and again: [`open_fixes`] takes it to the person.
            continue;
        }
        if record.new && record.first.as_ref().is_none_or(|v| !v.pass) {
            out.push(format!("{screen} is new: its first screenful gets a second look of its own first — `equipe design second-look {screen} --first`."));
        }
        match &record.verdict {
            None => out.push(format!("{screen} has had no second look: `equipe design second-look {screen}`.")),
            Some(v) if v.at < record.at => out.push(format!("{screen} changed after its second look: `equipe design second-look {screen}` again.")),
            Some(v) if !v.pass => out.push(format!(
                "{screen}: the second look asked for {} fix(es) — make them, check again, and ask it again ({} of {LOOKS_BEFORE_THE_PERSON} looks in a row asking for fixes; at {LOOKS_BEFORE_THE_PERSON}, what is left goes to the person with your hand-in).",
                v.fixes.len(),
                record.fixes_in_a_row
            )),
            Some(_) => {}
        }
    }
    out
}

/// The records under `design/checks/`, each with its screen.
fn records_in(design: &Path) -> Vec<Record> {
    let mut out = Vec::new();
    let mut stack = vec![design.join(DIR)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json")
                && p.file_name().is_some_and(|n| n != PENDING)
                && let Some(r) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Record>(&t).ok()).filter(|r| !r.screen.is_empty())
            {
                out.push(r);
            }
        }
    }
    out
}

/// Whether `record` (kept by `design`) checked `file`, by its real path.
fn covers(design: &Path, record: &Record, file: &Path) -> bool {
    let root = design.parent().unwrap_or(design);
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let file = real(file);
    record.files.iter().any(|s| real(&root.join(&s.path)) == file)
}

/// The screen a check in `design` last drew `file` as.
fn screen_of(design: &Path, file: &Path) -> Option<String> {
    records_in(design).into_iter().filter(|r| covers(design, r, file)).max_by_key(|r| r.at).map(|r| r.screen)
}

/// A check of `file` kept by another design folder in the same work — one
/// `--design` pointed at — and its screen.
fn checked_elsewhere(design: &Path, file: &Path) -> Option<(PathBuf, String)> {
    let root = design.parent().unwrap_or(design);
    let mut found = None;
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            if !p.is_dir() || name == ".git" || name == "node_modules" || depth > 6 {
                continue;
            }
            if name == DIR && p.parent() != Some(design) {
                let other = p.parent().unwrap_or(root).to_path_buf();
                if let Some(r) = records_in(&other).into_iter().filter(|r| covers(&other, r, file)).max_by_key(|r| r.at) {
                    found = Some((other, r.screen));
                }
                continue;
            }
            stack.push((p, depth + 1));
        }
    }
    found
}

/// What a hand-in takes to the person instead of refusing: each checked
/// screen whose looks asked for fixes [`LOOKS_BEFORE_THE_PERSON`] times in a
/// row, with the fixes its last look asked for.
pub fn open_fixes(design: &Path) -> Vec<String> {
    let pending = read_pending(design);
    let mut out = Vec::new();
    for screen in &pending.checked {
        let Some(record) = read(design, screen).filter(to_the_person) else { continue };
        let last = [&record.verdict, &record.first].into_iter().flatten().filter(|v| !v.pass).max_by_key(|v| v.at);
        let fixes = last.map(|v| v.fixes.iter().map(|f| format!("{}: {}", f.place, f.what)).collect::<Vec<_>>().join("; ")).unwrap_or_default();
        out.push(format!("{screen}: the second look still asked for fixes after {} looks in a row — shown to the person: {fixes}", record.fixes_in_a_row));
    }
    out
}

/// After a hand-in: the next ticket starts with nothing pending, and its
/// screens' rows of looks start again.
pub fn handed_in(design: &Path) {
    for screen in read_pending(design).checked {
        if let Some(mut record) = read(design, &screen).filter(|r| r.fixes_in_a_row > 0) {
            record.fixes_in_a_row = 0;
            let _ = write(design, &record);
        }
    }
    let _ = write_pending(design, &Pending::default());
}
