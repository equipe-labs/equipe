//! What a design ticket made, found where it left it (`design/` of the
//! project, of the squad's folder and of the ticket's worktree) and put on
//! one card for the design screen's Designs tab: its renders — the screen as
//! it was and each direction, at each width —, the change notes of
//! `CHANGES.md`, the preview links its words and comments give, the decision
//! record that names the direction chosen, and the ticket's own state.
//!
//! Nothing is asked of the designer beyond what it already does. A render is
//! known by its name — `before-1440.png`, `mock-a-1440.png`, `mock-a-390.png`,
//! `direction-calm-ledger-1440.png` — under `design/directions/<t<id>-…>/`
//! (any folder there whose name starts with the ticket's number), and a
//! picture a hand-in embeds or names (`Hero:` and `Before:` lines) counts too,
//! as long as it is a picture inside `design/` ([`visuals::check`]: no `..`,
//! no link out of the folder, an image of a kind taken) or one the ticket
//! itself showed. Read only; every walk, file and list is bounded, so a folder
//! that grew big costs a screen one answer. Pictures are served through
//! `read_image` (`visuals`' references gate), by the paths named here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use equipe_protocol::design::{DesignChange, DesignDecision, DesignDirection, DesignLink, DesignShot, DesignState, DesignWork};
use equipe_protocol::title::TicketLabel;
use equipe_protocol::{Ticket, TicketEntry, TicketId, TicketStatus};

use crate::visuals;

/// Cards sent at most, and tickets looked at to find them.
pub const CARDS: usize = 24;
const CANDIDATES: usize = 80;
/// What a walk of a ticket's folders looks at: entries, depth.
const WALK_ENTRIES: usize = 1500;
const WALK_DEPTH: usize = 3;
/// How much of `CHANGES.md` and of a decision record is read.
const CHANGES_MAX: u64 = 64 << 10;
const DECISION_MAX: u64 = 32 << 10;
/// What a card carries at most.
const SECTIONS: usize = 12;
const SECTION_CUT: usize = 3000;
const DECISION_CUT: usize = 4000;
const SUMMARY_CUT: usize = 600;
const LINKS: usize = 12;
const PICTURES: usize = 12;
const SHOTS_PER_DIRECTION: usize = 6;
const DIRECTIONS: usize = 8;
const DECISIONS_LOOKED_AT: usize = 200;
/// How many entries of a ticket's record are read, and how much of each.
const ENTRIES_READ: usize = 120;
const ENTRY_CUT: usize = 8000;

/// Words that mark a render as one of the directions' (`mock-a-1440`), and
/// the ones that mark it as the screen as it was.
const DIRECTION_WORDS: [&str; 6] = ["mock", "mockup", "direction", "option", "proposal", "after"];
const BEFORE_WORDS: [&str; 3] = ["before", "current", "baseline"];
/// Words in front of a direction's key in prose ("Direction A").
const MARKERS: [&str; 7] = ["direction", "option", "mock", "mockup", "variant", "concept", "proposal"];

// ── what a render's name says ──

/// What a render is, by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The screen as it was.
    Before,
    /// A direction's, by its key (`a`, `calm-ledger`; empty for a lone design).
    Direction(String),
    /// A render of something, named only by its width (`home-1440.png`).
    Other,
}

/// A render's name, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub kind: Kind,
    /// The width it was drawn at; 0 when the name doesn't say.
    pub width: u32,
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_lowercase).collect()
}

/// The width a name gives: a number of 200 to 4000, as one of its words.
fn width_in(tokens: &[String]) -> Option<(usize, u32)> {
    tokens.iter().enumerate().rev().find_map(|(i, t)| t.parse::<u32>().ok().filter(|w| (200..=4000).contains(w)).map(|w| (i, w)))
}

/// What a picture's file name says it is — `before-1440.png`, `mock-a-390.png`,
/// `direction-calm-ledger-1440.png`, `home-1440.png` — or `None` when it says
/// nothing (a logo in the app's folder, a dark variant: only the light renders
/// are paired).
pub fn classify(file: &str) -> Option<Named> {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut words = tokens(stem);
    if words.iter().any(|w| w == "dark") {
        return None;
    }
    words.retain(|w| w != "light" && w != "desktop" && w != "mobile" && w != "2x" && w != "1x");
    let width = match width_in(&words) {
        Some((at, w)) => {
            words.remove(at);
            w
        }
        None if tokens(stem).iter().any(|w| w == "mobile") => 390,
        None if tokens(stem).iter().any(|w| w == "desktop") => 1440,
        None => 0,
    };
    if words.iter().any(|w| BEFORE_WORDS.contains(&w.as_str())) {
        return Some(Named { kind: Kind::Before, width });
    }
    if let Some(at) = words.iter().position(|w| DIRECTION_WORDS.contains(&w.as_str())) {
        let key = words[at + 1..].join("-");
        return Some(Named { kind: Kind::Direction(key), width });
    }
    (width > 0).then_some(Named { kind: Kind::Other, width })
}

/// Whether `text` speaks of direction `key`: its words in a row; a single
/// letter only after a word like "direction", or at the very start followed by
/// a dash or colon — never the article in "a calmer ledger".
pub fn mentions(text: &str, key: &str) -> bool {
    let key_words = tokens(key);
    if key_words.is_empty() {
        return false;
    }
    let words = tokens(text);
    let at = (0..words.len()).filter(|i| words[*i..].starts_with(&key_words));
    let lone = key_words.len() == 1 && key_words[0].chars().count() == 1;
    // "(B)" is a letter in brackets, never an article.
    if lone && text.to_lowercase().contains(&format!("({})", key_words[0])) {
        return true;
    }
    for i in at {
        if !lone || (i > 0 && MARKERS.contains(&words[i - 1].as_str())) {
            return true;
        }
        if lone && i == 0 {
            let lower = text.trim_start().to_lowercase();
            let rest = lower.trim_start_matches(|c: char| !c.is_alphanumeric()).strip_prefix(key_words[0].as_str()).unwrap_or_default();
            if rest.is_empty() || rest.starts_with(['—', '–', '-', ':', '.', ')']) || rest.starts_with(" —") || rest.starts_with(" -") || rest.starts_with(" –") || rest.starts_with(" :") {
                return true;
            }
        }
    }
    false
}

// ── scanning a design folder ──

fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// Whether a folder's name is a ticket's: `t2562-dashboard`, `T-2562`, `t2562`.
pub fn names_ticket(name: &str, id: TicketId) -> bool {
    let lower = name.to_lowercase();
    let rest = lower.strip_prefix("t-").or_else(|| lower.strip_prefix('t'));
    let Some(rest) = rest else { return false };
    let digits = id.to_string();
    rest.strip_prefix(digits.as_str()).is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_digit()) && (after.is_empty() || after.starts_with(|c: char| !c.is_alphanumeric())))
}

/// What one design folder holds of one ticket.
#[derive(Debug, Default)]
struct Scan {
    /// Each render, by its path from the design folder, with what its name says.
    shots: Vec<(String, Named, DesignShot)>,
    /// `CHANGES.md` files, by path from the design folder, as text.
    changes: Vec<(String, String)>,
    newest: u64,
}

/// Skipped in a walk: what an app keeps that is no design.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build" | "out" | "coverage")
}

fn scan(design: &Path, id: TicketId) -> Scan {
    let mut out = Scan::default();
    let directions = design.join("directions");
    if !real_dir(design) || !real_dir(&directions) {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(&directions) else { return out };
    let mut dirs: Vec<(String, PathBuf)> = entries.flatten().filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path()))).filter(|(n, p)| names_ticket(n, id) && real_dir(p)).collect();
    dirs.sort();
    let roots = vec![design.to_path_buf()];
    let mut seen = 0usize;
    for (_, dir) in dirs {
        let mut stack = vec![(dir, 0usize)];
        while let Some((at, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&at) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                seen += 1;
                if seen > WALK_ENTRIES {
                    return out;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
                if skipped(&name) {
                    continue;
                }
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                if meta.is_dir() {
                    if depth < WALK_DEPTH {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                let Ok(rel) = path.strip_prefix(design).map(|r| r.to_string_lossy().into_owned()) else { continue };
                if name.eq_ignore_ascii_case("changes.md") {
                    if let Some(bytes) = super::folder::inside(design, &rel, CHANGES_MAX).filter(|b| b.len() as u64 <= CHANGES_MAX) {
                        out.newest = out.newest.max(secs(&meta));
                        out.changes.push((rel, String::from_utf8_lossy(&bytes).into_owned()));
                    }
                    continue;
                }
                if !visuals::is_image_name(&name) {
                    continue;
                }
                let Some(named) = classify(&name) else { continue };
                // A render is a regular file or a link that stays inside `design/`.
                let Ok(real) = visuals::check(&path, &roots) else { continue };
                out.newest = out.newest.max(std::fs::metadata(&real).map_or(secs(&meta), |m| secs(&m)));
                out.shots.push((rel, named.clone(), DesignShot { path: path.to_string_lossy().into_owned(), name, width: named.width }));
            }
        }
    }
    // The walk's order is the disk's; the card's is the names'.
    out.shots.sort_by(|a, b| a.0.cmp(&b.0));
    out.changes.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ── what a hand-in says ──

/// What follows `Key:` at the start of `line`, for the first of `keys` it
/// starts with, in any case and with any bold around the key.
fn after_key<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    let line = line.trim_start_matches(['*', '_']);
    keys.iter().find_map(|key| {
        let head = line.get(..key.len())?;
        if !head.eq_ignore_ascii_case(key) {
            return None;
        }
        let rest = line[key.len()..].trim_start_matches(['*', '_', ' ']);
        let value = rest.strip_prefix([':', '：'])?;
        let value = value.trim_start_matches(['*', '_']).trim();
        (!value.is_empty()).then_some(value)
    })
}

/// The path a value gives: in backticks, the target of a Markdown image or
/// link, in `<…>`, or plain — only when it names a picture, never an address.
fn picture_path(value: &str) -> Option<String> {
    let value = value.trim();
    let path = if let Some(open) = value.find('`') {
        let rest = &value[open + 1..];
        &rest[..rest.find('`')?]
    } else if let Some(open) = value.find("](") {
        let rest = &value[open + 2..];
        rest[..rest.find(')')?].trim()
    } else {
        value.split_whitespace().next()?.trim_matches(['<', '>', '"', '\'', '*', '_']).trim_end_matches(['.', ',', ';'])
    };
    let path = path.trim().trim_matches(['<', '>']);
    let bad = path.is_empty() || path.contains("://") || path.chars().any(char::is_control) || path.len() > 400 || !visuals::is_image_name(path);
    (!bad).then(|| path.to_owned())
}

/// The hero render and the before render a hand-in names, as written: a line
/// `Hero: <path>` and `Before: <path>` (also `Hero render:`, bold, bulleted).
pub fn named_pictures(text: &str) -> (Option<String>, Option<String>) {
    let (mut hero, mut before) = (None, None);
    for line in text.lines() {
        let trimmed = line.trim_start().trim_start_matches(['-', '*', '>', '#', ' ', '•']).trim_start_matches(['*', '_']);
        for (slot, words) in [(&mut hero, ["hero render", "hero"]), (&mut before, ["before render", "before"])] {
            if slot.is_some() {
                continue;
            }
            let Some(value) = after_key(trimmed, &words) else { continue };
            *slot = picture_path(value);
        }
    }
    (hero, before)
}

/// Every https address in `text` that passes [`crate::connections::clean_url`],
/// each once, in order — what the one check every card's link goes through lets by.
pub fn urls_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("https://") {
        let start = from + i;
        let end = text[start..].find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '`' | '|' | '<' | '}')).map_or(text.len(), |e| start + e);
        let raw = text[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_']);
        from = end.max(start + 1);
        if let Ok(url) = crate::connections::clean_url(raw)
            && !out.contains(&url)
        {
            out.push(url);
        }
    }
    out
}

/// The direction an address shows: its path's first part when it is a key
/// (`…vercel.app/b`), else the one its line speaks of before it.
fn link_direction(text: &str, url: &str, keys: &[String]) -> String {
    let path_part = url.strip_prefix("https://").and_then(|r| r.split_once('/')).map(|(_, p)| p.split(['/', '?', '#']).next().unwrap_or_default().to_lowercase()).unwrap_or_default();
    if let Some(key) = keys.iter().find(|k| !k.is_empty() && **k == path_part) {
        return key.clone();
    }
    let line = text.lines().find(|l| l.contains(url)).unwrap_or_default();
    let before = line.split(url).next().unwrap_or_default();
    keys.iter().filter(|k| !k.is_empty()).find(|k| mentions(before, k)).cloned().unwrap_or_default()
}

// ── CHANGES.md ──

fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    (1..=4).contains(&level).then(|| t[level..].strip_prefix(' ').map(|rest| (level, rest.trim()))).flatten()
}

fn cut(text: &str, most: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let cut: String = text.chars().take(most).collect();
    format!("{}…", cut.trim_end())
}

/// The key a heading names: one of `keys` it speaks of, else the word after
/// "Direction" (`## Direction C — Dense` with no render of C), else none.
fn key_of_heading(title: &str, keys: &[String]) -> String {
    let mut by_length: Vec<&String> = keys.iter().filter(|k| !k.is_empty()).collect();
    by_length.sort_by_key(|k| std::cmp::Reverse(k.len()));
    if let Some(key) = by_length.into_iter().find(|k| mentions(title, k)) {
        return key.clone();
    }
    let words = tokens(title);
    match words.iter().position(|w| MARKERS.contains(&w.as_str())) {
        Some(at) if at + 1 < words.len() => words[at + 1].clone(),
        _ => String::new(),
    }
}

/// `CHANGES.md` as sections: split at its second-level headings (else its
/// first-level ones, else its third), each tied to the direction its heading
/// names; what comes before the first is for all. A `# Title` alone is not a
/// section.
pub fn parse_changes(text: &str, keys: &[String]) -> Vec<DesignChange> {
    let heads: Vec<(usize, usize)> = text.lines().enumerate().filter_map(|(i, l)| heading(l).map(|(level, _)| (i, level))).collect();
    let count = |level: usize| heads.iter().filter(|(_, l)| *l == level).count();
    // `##` when there are any; `#` only when there are several (one is the title); else `###`.
    let split = if count(2) >= 1 {
        2
    } else if count(1) >= 2 {
        1
    } else if count(3) >= 1 {
        3
    } else {
        0
    };
    let lines: Vec<&str> = text.lines().collect();
    let starts: Vec<usize> = heads.iter().filter(|(_, l)| *l == split).map(|(i, _)| *i).collect();
    let mut out = Vec::new();
    let mut push = |title: &str, body: &[&str]| {
        let body = body.join("\n");
        if body.trim().is_empty() || out.len() >= SECTIONS {
            return;
        }
        let direction = key_of_heading(title, keys);
        out.push(DesignChange { direction, title: title.to_owned(), text: cut(&body, SECTION_CUT) });
    };
    if split == 0 {
        let body: Vec<&str> = lines.iter().copied().filter(|l| heading(l).is_none_or(|(level, _)| level > 1)).collect();
        push("", &body);
        return out;
    }
    let first = starts[0];
    // What comes before the first section: for all, without a lone title.
    let preface: Vec<&str> = lines[..first].iter().copied().filter(|l| heading(l).is_none_or(|(level, _)| level > 1)).collect();
    push("", &preface);
    for (n, start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(lines.len());
        let title = heading(lines[*start]).map(|(_, t)| t.to_owned()).unwrap_or_default();
        push(&title, &lines[start + 1..end]);
    }
    out
}

// ── decision records ──

fn mentions_ticket(text: &str, id: TicketId) -> bool {
    let lower = text.to_lowercase();
    let digits = id.to_string();
    for marker in ["t-", "t"] {
        let needle = format!("{marker}{digits}");
        for (i, _) in lower.match_indices(needle.as_str()) {
            let before_ok = i == 0 || !lower[..i].chars().next_back().is_some_and(char::is_alphanumeric);
            let after_ok = !lower[i + needle.len()..].starts_with(|c: char| c.is_ascii_digit());
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// The value of a `Key:` line of a record (any bold around the key).
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| after_key(line.trim_start().trim_start_matches(['-', '*', '>', ' ', '•']), &[key]).map(str::to_owned))
}

/// A decision record that is about ticket `id`: its name or its words say so.
struct Record {
    file: String,
    text: String,
}

fn decisions(design: &Path, id: TicketId) -> Option<Record> {
    let dir = design.join("decisions");
    if !real_dir(design) || !real_dir(&dir) {
        return None;
    }
    let mut files: Vec<String> = std::fs::read_dir(&dir).ok()?.flatten().filter_map(|e| e.file_name().to_str().map(str::to_owned)).filter(|n| n.to_lowercase().ends_with(".md") && !n.starts_with('.')).collect();
    files.sort();
    // The latest first.
    files.reverse();
    files.truncate(DECISIONS_LOOKED_AT);
    files.into_iter().find_map(|file| {
        let bytes = super::folder::inside(design, &format!("decisions/{file}"), DECISION_MAX).filter(|b| b.len() as u64 <= DECISION_MAX)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (mentions_ticket(&file, id) || mentions_ticket(&text, id)).then_some(Record { file, text })
    })
}

fn decision_of(record: &Record) -> DesignDecision {
    let title = record.text.lines().find_map(|l| l.trim().strip_prefix("# ").map(|t| t.trim().to_owned())).unwrap_or_else(|| record.file.trim_end_matches(".md").to_owned());
    DesignDecision { file: record.file.clone(), title, chose: field(&record.text, "chose").map(|c| cut(&c, 200)).unwrap_or_default(), text: cut(&record.text, DECISION_CUT) }
}

// ── one ticket's card ──

/// Where things are for a read: the squad's folder and its projects.
pub struct Where<'a> {
    pub squad_home: PathBuf,
    pub projects: &'a [PathBuf],
}

/// The `design/` folders a ticket's work may be in: its worktree's, then its
/// project's (the squad's own for a task).
fn design_roots(t: &Ticket, at: &Where) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(worktree) = t.worktree.as_ref().filter(|w| real_dir(w)) {
        out.push(worktree.join(super::folder::DIR));
    }
    let base = if t.kind.in_home() { Some(at.squad_home.clone()) } else { t.project.clone().or_else(|| at.projects.first().cloned()) };
    if let Some(base) = base {
        let dir = base.join(super::folder::DIR);
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// Whether a ticket is design work: labelled so, or a designer's.
pub fn is_design(t: &Ticket) -> bool {
    let designer = |who: &Option<String>| who.as_deref().is_some_and(|w| w == "designer" || w.starts_with("designer-"));
    t.label == Some(TicketLabel::Design) || designer(&t.owner) || designer(&t.assignee)
}

fn state_of(status: TicketStatus) -> Option<DesignState> {
    match status {
        TicketStatus::InProgress => Some(DesignState::Working),
        TicketStatus::Waiting => Some(DesignState::WaitingOnYou),
        TicketStatus::Review => Some(DesignState::HandedIn),
        TicketStatus::Done => Some(DesignState::Accepted),
        _ => None,
    }
}

fn widest(shots: &[DesignShot]) -> Option<&DesignShot> {
    // A desktop render stands for a direction: the widest that is no wider than a big screen.
    shots.iter().filter(|s| s.width <= 2000).max_by_key(|s| s.width).or_else(|| shots.first())
}

/// The words a record of the ticket holds: its latest word, then each entry.
fn texts<'a>(t: &'a Ticket, entries: &'a [TicketEntry]) -> Vec<(&'static str, &'a str)> {
    let mut out: Vec<(&'static str, &str)> = Vec::new();
    if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push(("hand-in", note));
    }
    out.extend(entries.iter().map(|e| (if e.kind == "comment" { "comment" } else { "hand-in" }, e.text.as_str())));
    out
}

/// A ticket's card, from `entries` (its record) and `followups` (the records
/// of tickets that wait on it: a preview link is often posted by one). `None`
/// when it is no design work or has made nothing to show yet.
pub fn build(t: &Ticket, entries: &[TicketEntry], followups: &[Vec<TicketEntry>], at: &Where) -> Option<DesignWork> {
    let state = state_of(t.status)?;
    let roots = design_roots(t, at);
    // Each root's own, the worktree's over the project's.
    let scans: Vec<(&PathBuf, Scan)> = roots.iter().map(|r| (r, scan(r, t.id))).collect();
    let mut shots: Vec<(String, Named, DesignShot)> = Vec::new();
    let mut changes_files: Vec<(String, String)> = Vec::new();
    for (_, found) in &scans {
        for s in &found.shots {
            if !shots.iter().any(|(rel, ..)| *rel == s.0) {
                shots.push(s.clone());
            }
        }
        for c in &found.changes {
            if !changes_files.iter().any(|(rel, _)| *rel == c.0) {
                changes_files.push(c.clone());
            }
        }
    }
    let entries: Vec<TicketEntry> = entries.iter().rev().take(ENTRIES_READ).rev().map(|e| TicketEntry { text: cut(&e.text, ENTRY_CUT), ..e.clone() }).collect();
    let words = texts(t, &entries);

    // The pictures a hand-in names, embeds or showed, each only when it is a
    // picture inside a `design/` folder, or one the ticket itself showed.
    let design_roots_only: Vec<PathBuf> = roots.clone();
    let from_ticket: HashSet<&str> = t.images.iter().map(String::as_str).collect();
    let resolve = |given: &str| -> Option<String> {
        let given = given.trim().strip_prefix("file://").unwrap_or(given.trim());
        let path = Path::new(given);
        let candidates: Vec<PathBuf> = if path.is_absolute() {
            vec![path.to_owned()]
        } else {
            roots.iter().flat_map(|r| [r.parent().map(|p| p.join(path)), Some(r.join(path))]).flatten().collect()
        };
        candidates.into_iter().find_map(|p| {
            let clean: PathBuf = p.components().filter(|c| !matches!(c, std::path::Component::CurDir)).collect();
            let inside = visuals::check(&clean, &design_roots_only).is_ok();
            let showed = from_ticket.contains(clean.to_string_lossy().as_ref()) && visuals::check(&clean, &[clean.parent().map(Path::to_path_buf).unwrap_or_default()]).is_ok();
            (inside || showed).then(|| clean.to_string_lossy().into_owned())
        })
    };
    let (hero_named, before_named) = words.iter().map(|(_, text)| named_pictures(text)).fold((None, None), |(h, b), (h2, b2)| (h.or(h2), b.or(b2)));
    let hero_named = hero_named.and_then(|p| resolve(&p));
    let before_named = before_named.and_then(|p| resolve(&p));
    let mut loose: Vec<String> = Vec::new();
    let embedded: Vec<String> = words.iter().flat_map(|(_, text)| visuals::markdown::local_images(text)).map(|i| i.path).collect();
    for image in t.images.iter().chain(embedded.iter()).filter_map(|i| resolve(i)) {
        if !loose.contains(&image) && !shots.iter().any(|(_, _, s)| s.path == image) {
            loose.push(image);
        }
    }

    // Directions, by key, in the order their files come.
    let mut directions: Vec<DesignDirection> = Vec::new();
    let mut before: Vec<DesignShot> = Vec::new();
    let mut pictures: Vec<DesignShot> = Vec::new();
    for (_, named, shot) in &shots {
        match &named.kind {
            Kind::Before => {
                if !before.iter().any(|b| b.width == shot.width && b.width != 0) {
                    before.push(shot.clone());
                }
            }
            Kind::Direction(key) => match directions.iter().position(|d| d.key == *key) {
                Some(at) => directions[at].shots.push(shot.clone()),
                None if directions.len() < DIRECTIONS => directions.push(DesignDirection { key: key.clone(), chosen: false, shots: vec![shot.clone()] }),
                None => {}
            },
            Kind::Other => pictures.push(shot.clone()),
        }
    }
    for d in &mut directions {
        d.shots.sort_by_key(|s| std::cmp::Reverse(s.width));
        d.shots.truncate(SHOTS_PER_DIRECTION);
    }
    before.sort_by_key(|s| std::cmp::Reverse(s.width));
    // A picture a hand-in showed that is no render of the folder's: by its name still.
    for path in &loose {
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let named = classify(&name);
        let shot = DesignShot { path: path.clone(), name, width: named.as_ref().map_or(0, |n| n.width) };
        match named.map(|n| n.kind) {
            Some(Kind::Before) if before_named.is_none() && !before.iter().any(|b| b.width == shot.width) => before.push(shot),
            _ => {
                if pictures.len() < PICTURES {
                    pictures.push(shot);
                }
            }
        }
    }
    pictures.truncate(PICTURES);
    // The screen as it was that the hand-in named.
    if let Some(path) = &before_named {
        let shot = match before.iter().position(|b| &b.path == path) {
            Some(at) => before.remove(at),
            None => {
                let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                let width = classify(&name).map_or(0, |n| n.width);
                DesignShot { path: path.clone(), name, width }
            }
        };
        before.insert(0, shot);
    }

    // The decision, and the direction it names.
    let record = roots.iter().find_map(|r| decisions(r, t.id));
    let decision = record.as_ref().map(decision_of);
    let keys: Vec<String> = directions.iter().map(|d| d.key.clone()).collect();
    let chosen = record.as_ref().and_then(|r| {
        let picture = field(&r.text, "picture").and_then(|v| picture_path(&v));
        let by_picture = picture.and_then(|p| {
            let file = p.rsplit('/').next().unwrap_or(&p).to_lowercase();
            directions.iter().position(|d| d.shots.iter().any(|s| s.name.to_lowercase() == file))
        });
        let chose = field(&r.text, "chose").unwrap_or_default();
        let mut by_length: Vec<(usize, &String)> = keys.iter().enumerate().filter(|(_, k)| !k.is_empty()).collect();
        by_length.sort_by_key(|(_, k)| std::cmp::Reverse(k.len()));
        by_picture.or_else(|| by_length.into_iter().find(|(_, k)| mentions(&chose, k)).map(|(i, _)| i))
    });
    // The log the designer's tool keeps of what was chosen, for a ticket with no record.
    let chosen = chosen.or_else(|| {
        let mark = format!("T-{}", t.id);
        roots.iter().find_map(|r| {
            super::directions::read(r).into_iter().filter(|d| d.chosen && d.ticket.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(&mark))).find_map(|d| {
                let name = super::studio::slug(&d.name);
                keys.iter().position(|k| !k.is_empty() && (*k == name || mentions(&d.name, k)))
            })
        })
    });
    if let Some(i) = chosen {
        directions[i].chosen = true;
    }

    // The hero: what the hand-in named, else the chosen direction's, else the first.
    let hero = hero_named
        .or_else(|| chosen.and_then(|i| widest(&directions[i].shots)).map(|s| s.path.clone()))
        .or_else(|| directions.iter().find_map(|d| widest(&d.shots)).map(|s| s.path.clone()))
        .or_else(|| pictures.first().map(|s| s.path.clone()));

    let mut changes: Vec<DesignChange> = Vec::new();
    for (_, text) in &changes_files {
        for section in parse_changes(text, &keys) {
            if changes.len() < SECTIONS {
                changes.push(section);
            }
        }
    }

    // Preview links: the hand-in's words, the comments of this ticket and of the tickets after it, `CHANGES.md`.
    let mut sources: Vec<(&str, String)> = words.iter().map(|(from, text)| (*from, (*text).to_owned())).collect();
    for record in followups {
        sources.extend(record.iter().rev().take(ENTRIES_READ).map(|e| ("comment", cut(&e.text, ENTRY_CUT))));
    }
    sources.extend(changes_files.iter().map(|(_, text)| ("changes", text.clone())));
    let mut links: Vec<DesignLink> = Vec::new();
    for (from, text) in &sources {
        for url in urls_in(text) {
            if links.len() < LINKS && !links.iter().any(|l| l.url == url) {
                links.push(DesignLink { direction: link_direction(text, &url, &keys), url, from: (*from).to_owned() });
            }
        }
    }

    let summary = t.note.as_deref().map(|n| n.lines().filter(|l| !l.trim_start().starts_with("![")).collect::<Vec<_>>().join("\n")).map(|n| cut(&n, SUMMARY_CUT)).unwrap_or_default();
    let work = DesignWork {
        ticket: t.id,
        title: t.title.clone(),
        state,
        project: (!t.kind.in_home()).then(|| t.project.as_deref().map(crate::board::project_name)).flatten(),
        updated: t.updated_at.unwrap_or_else(|| scans.iter().map(|(_, s)| s.newest).max().unwrap_or(0)),
        hero,
        before,
        directions,
        pictures,
        changes,
        links,
        decision,
        summary,
    };
    let made = work.hero.is_some() || !work.before.is_empty() || !work.directions.is_empty() || !work.pictures.is_empty() || !work.changes.is_empty() || !work.links.is_empty() || work.decision.is_some();
    made.then_some(work)
}

/// The cards of a squad's design tickets, newest first: the work at hand, handed in or accepted.
pub fn read(home: &Path, squad: &str) -> Vec<DesignWork> {
    read_in(home, squad, None)
}

/// One ticket's card, when it has one.
pub fn for_ticket(home: &Path, squad: &str, id: TicketId) -> Option<DesignWork> {
    read_in(home, squad, Some(id)).into_iter().next()
}

fn read_in(home: &Path, squad: &str, only: Option<TicketId>) -> Vec<DesignWork> {
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !crate::squads::valid_name(squad) || !db.exists() {
        return Vec::new();
    }
    let Ok(journal) = crate::journal::Journal::open(&db) else { return Vec::new() };
    let projects = journal.projects().unwrap_or_default();
    let at = Where { squad_home: crate::squad_home::path(home, squad), projects: &projects };
    let tickets = journal.tickets().unwrap_or_default();
    let mut candidates: Vec<&Ticket> = tickets.iter().filter(|t| only.is_none_or(|id| t.id == id) && is_design(t) && state_of(t.status).is_some()).collect();
    candidates.sort_by_key(|t| std::cmp::Reverse((t.updated_at.unwrap_or(0), t.id)));
    candidates.truncate(CANDIDATES);
    let mut out = Vec::new();
    for t in candidates {
        let entries = journal.ticket_entries(t.id).unwrap_or_default();
        let followups: Vec<Vec<TicketEntry>> = tickets.iter().filter(|f| f.id != t.id && f.depends_on.contains(&t.id)).take(6).map(|f| journal.ticket_entries(f.id).unwrap_or_default()).collect();
        if let Some(work) = build(t, &entries, &followups, &at) {
            out.push(work);
            if out.len() >= CARDS {
                break;
            }
        }
    }
    out
}

/// What to put in a message so the person sees the work in it: the hero and
/// the screen as it was, each as a Markdown image on a line of its own.
pub fn embed(work: &DesignWork) -> String {
    work.headline()
        .into_iter()
        .map(|(word, path)| {
            let dest = if path.contains(char::is_whitespace) { format!("<{path}>") } else { path.to_owned() };
            format!("![T-{} {word}]({dest})", work.ticket)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_is_known_by_its_name() {
        let n = |file: &str| classify(file);
        assert_eq!(n("before-1440.png"), Some(Named { kind: Kind::Before, width: 1440 }));
        assert_eq!(n("Before-390.PNG"), Some(Named { kind: Kind::Before, width: 390 }));
        assert_eq!(n("before.png"), Some(Named { kind: Kind::Before, width: 0 }));
        assert_eq!(n("mock-a-1440.png"), Some(Named { kind: Kind::Direction("a".into()), width: 1440 }));
        assert_eq!(n("mock-b-390.png"), Some(Named { kind: Kind::Direction("b".into()), width: 390 }));
        assert_eq!(n("direction-calm-ledger-1440.png"), Some(Named { kind: Kind::Direction("calm-ledger".into()), width: 1440 }));
        assert_eq!(n("dashboard-mock-c_1440.webp"), Some(Named { kind: Kind::Direction("c".into()), width: 1440 }));
        assert_eq!(n("mock-1440.png"), Some(Named { kind: Kind::Direction(String::new()), width: 1440 }));
        assert_eq!(n("mock-a-mobile.png"), Some(Named { kind: Kind::Direction("a".into()), width: 390 }));
        assert_eq!(n("home-1440.png"), Some(Named { kind: Kind::Other, width: 1440 }));
        assert_eq!(n("logo.png"), None, "no word, no width: no render");
        assert_eq!(n("mock-a-1440.dark.png"), None, "only the light renders are paired");
        assert_eq!(n("icon-512.png"), Some(Named { kind: Kind::Other, width: 512 }));
        assert_eq!(n("favicon-16.png"), None);
    }

    #[test]
    fn a_ticket_is_named_by_its_folder_exactly() {
        for yes in ["t2562-dashboard", "T2562", "t-2562-x", "t2562"] {
            assert!(names_ticket(yes, 2562), "{yes}");
        }
        for no in ["t25620-x", "t256", "x2562", "tasks2562", "t2562x", "2562"] {
            assert!(!names_ticket(no, 2562), "{no}");
        }
    }

    #[test]
    fn a_direction_is_spoken_of_by_its_key_and_an_article_is_not_one() {
        assert!(mentions("Direction A", "a"));
        assert!(mentions("A — Calm ledger", "a"));
        assert!(mentions("option b: dense", "b"));
        assert!(mentions("Chose: Calm Ledger (B)", "calm-ledger"));
        assert!(!mentions("a calmer ledger with a table", "a"));
        assert!(!mentions("Chose a different path", "a"));
        assert!(!mentions("Direction AB", "a"));
        assert!(mentions("Direction C — Dense", "c"));
        assert!(!mentions("anything", ""));
    }

    #[test]
    fn change_notes_split_per_direction_and_keep_what_is_for_all() {
        let md = "# T-2562 dashboard — changes\n\nWhat the three have in common.\n\n## Direction A — Calm ledger\n### Before → After → Why\n- Rows 56px → 40px, as in the references' lists\n\n## Direction B\nBefore: cards. After: a table.\n\n## Direction D — never rendered\nText.\n\n## Unchanged\nFeatures, copy.\n";
        let keys = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let sections = parse_changes(md, &keys);
        let by: Vec<(&str, &str)> = sections.iter().map(|s| (s.direction.as_str(), s.title.as_str())).collect();
        assert_eq!(by, [("", ""), ("a", "Direction A — Calm ledger"), ("b", "Direction B"), ("d", "Direction D — never rendered"), ("", "Unchanged")]);
        assert!(sections[0].text.starts_with("What the three"), "the preface is for all, without the lone title");
        assert!(sections[1].text.contains("### Before → After → Why") && sections[1].text.contains("Rows 56px → 40px"), "a sub-heading stays in its section");
        // One heading level only: still sections. No headings: the text whole. Empty: none.
        assert_eq!(parse_changes("# A\n- x\n# B\n- y", &["a".into(), "b".into()]).iter().map(|s| s.direction.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(parse_changes("just words\n", &keys).len(), 1);
        assert!(parse_changes("  \n", &keys).is_empty());
        assert!(parse_changes("# Title only\n", &keys).is_empty());
    }

    #[test]
    fn change_notes_are_bounded() {
        let long = "x".repeat(SECTION_CUT * 3);
        let mut md = String::new();
        for i in 0..SECTIONS + 5 {
            md.push_str(&format!("## Direction {i}\n{long}\n"));
        }
        let sections = parse_changes(&md, &[]);
        assert_eq!(sections.len(), SECTIONS);
        assert!(sections[0].text.chars().count() <= SECTION_CUT + 1 && sections[0].text.ends_with('…'));
    }

    #[test]
    fn links_are_found_in_prose_cleaned_and_only_https_with_a_real_host() {
        let text = "Previews: A https://t2562-a-git-x-team.vercel.app/a, B (https://t2562-b.vercel.app/b). [c](https://t2562-c.vercel.app/c?x=1#top).\nhttp://insecure.example.com/x https://user:pw@evil.example.com/ https://localhost/ https://10.0.0.1/ https://a.example.com:8080/ https://a.vercel.app/a again https://ok.example.com/\n";
        assert_eq!(urls_in(text), ["https://t2562-a-git-x-team.vercel.app/a", "https://t2562-b.vercel.app/b", "https://t2562-c.vercel.app/c?x=1#top", "https://a.vercel.app/a", "https://ok.example.com/"]);
        assert!(urls_in("no links, https:// alone, or https://x").is_empty());
    }

    #[test]
    fn a_link_knows_its_direction_from_its_path_or_its_line() {
        let keys = vec!["a".to_owned(), "b".to_owned(), "calm-ledger".to_owned()];
        assert_eq!(link_direction("see https://p.vercel.app/b", "https://p.vercel.app/b", &keys), "b");
        assert_eq!(link_direction("Direction A: https://p.vercel.app/x", "https://p.vercel.app/x", &keys), "a");
        assert_eq!(link_direction("Calm Ledger — https://p.vercel.app/x", "https://p.vercel.app/x", &keys), "calm-ledger");
        assert_eq!(link_direction("all of it: https://p.vercel.app/", "https://p.vercel.app/", &keys), "");
    }

    #[test]
    fn a_hand_in_names_its_hero_and_its_before_in_a_line_each() {
        let text = "Done.\n- **Hero:** `design/directions/t7-x/boards/mock-b-1440.png`\n* Before render: ![before](design/directions/t7-x/boards/before-1440.png)\nBefore: the old dense table\nHero render: https://example.com/x.png\n";
        let (hero, before) = named_pictures(text);
        assert_eq!(hero.as_deref(), Some("design/directions/t7-x/boards/mock-b-1440.png"));
        assert_eq!(before.as_deref(), Some("design/directions/t7-x/boards/before-1440.png"));
        assert_eq!(named_pictures("Before: the old table, Hero: none"), (None, None), "words and addresses are no pictures");
        assert_eq!(named_pictures("Hero: /abs/path/hero.png.").0.as_deref(), Some("/abs/path/hero.png"));
    }

    #[test]
    fn a_decision_record_is_the_ticket_s_by_its_name_or_its_words_and_exact_numbers() {
        assert!(mentions_ticket("Ticket: T-2562", 2562));
        assert!(mentions_ticket("2026-11-14-t2562-direction.md", 2562));
        assert!(mentions_ticket("see (T2562).", 2562));
        assert!(!mentions_ticket("T-25620", 2562));
        assert!(!mentions_ticket("at2562", 2562));
        assert!(!mentions_ticket("T-256", 2562));
        assert_eq!(field("- **Chose:** Direction B\n", "chose").as_deref(), Some("Direction B"));
        assert_eq!(field("Picture: /x/mock-b-1440.png", "picture").as_deref(), Some("/x/mock-b-1440.png"));
        assert_eq!(field("Choice: x", "chose"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-design-results-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(4, 4, image::Rgb([20, 90, 250])).save(path).unwrap();
    }

    fn ticket(id: u32, status: &str, worktree: Option<&Path>, project: Option<&Path>, images: &[String], note: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": "Dashboard", "body": "", "status": status, "label": "design", "owner": "designer", "assignee": "designer",
            "project": project, "depends_on": [], "branch": null, "worktree": worktree, "created_by": "pm", "note": note, "images": images, "updated_at": 1_700_000_000u64,
        }))
        .unwrap()
    }

    fn entry(by: &str, kind: &str, text: &str) -> TicketEntry {
        TicketEntry { at: 1, by: by.into(), kind: kind.into(), text: text.into() }
    }

    const CHANGES: &str = "# Changes\n\n## Direction A — Calm ledger\n- Rows 56 → 40, because the references' lists do\n\n## Direction B\n- Cards → table\n\n## Unchanged\nCopy.\n";

    /// A project with a ticket's folders in it, the way a designer leaves them.
    fn project(name: &str) -> (PathBuf, PathBuf) {
        let root = scratch(name);
        let design = root.join("design");
        for file in ["directions/t7-dashboard/app/public/logo.png", "directions/t7-references/boards/before-1440.png", "directions/t7-references/boards/before-390.png", "directions/t7-references/boards/mock-a-1440.png", "directions/t7-references/boards/mock-b-1440.png", "directions/t7-references/boards/mock-b-390.png", "directions/t8-other/boards/mock-z-1440.png"] {
            png(&design.join(file));
        }
        std::fs::write(design.join("directions/t7-dashboard/CHANGES.md"), CHANGES).unwrap();
        std::fs::create_dir_all(design.join("decisions")).unwrap();
        std::fs::write(design.join("decisions/2026-11-14-foundation-direction.md"), "# Foundation direction\n\nChose: Direction B\nTicket: T-7\nWhy: denser\n").unwrap();
        std::fs::write(design.join("decisions/2026-11-01-other.md"), "# Other\nChose: Direction A\nTicket: T-70\n").unwrap();
        (root, design)
    }

    #[test]
    fn a_card_pairs_before_with_each_direction_reads_the_notes_and_names_the_chosen_one() {
        let (root, design) = project("card");
        let at = Where { squad_home: root.join("home"), projects: &[] };
        let t = ticket(7, "review", None, Some(&root), &[], "Three directions.\nPreviews: A https://t7-a.vercel.app/a");
        let follow = vec![entry("frontend", "comment", "Preview B: https://t7-b-git-x.vercel.app/b")];
        let work = build(&t, &[entry("designer", "handed in", "Done.")], &[follow], &at).expect("a card");
        assert_eq!((work.ticket, work.state), (7, DesignState::HandedIn));
        // Only this ticket's folders: t7-…, not t8-… and not the app's logo.
        let keys: Vec<&str> = work.directions.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["a", "b"]);
        let names = |d: &DesignDirection| d.shots.iter().map(|s| (s.name.clone(), s.width)).collect::<Vec<_>>();
        assert_eq!(names(&work.directions[1]), [("mock-b-1440.png".to_owned(), 1440), ("mock-b-390.png".to_owned(), 390)], "widest first");
        assert_eq!(work.before.iter().map(|b| b.width).collect::<Vec<_>>(), [1440, 390]);
        assert!(work.pictures.is_empty());
        // The record that is this ticket's (not T-70's) names B: B is chosen and is the hero.
        assert_eq!(work.decision.as_ref().map(|d| (d.file.as_str(), d.chose.as_str())), Some(("2026-11-14-foundation-direction.md", "Direction B")));
        assert!(work.directions[1].chosen && !work.directions[0].chosen);
        assert_eq!(work.hero.as_deref(), Some(design.join("directions/t7-references/boards/mock-b-1440.png").to_str().unwrap()));
        // Before, at the hero's width, is what the pictures to show first pair with it.
        let head = work.headline();
        assert_eq!(head.iter().map(|(w, p)| (*w, p.rsplit('/').next().unwrap())).collect::<Vec<_>>(), [("after", "mock-b-1440.png"), ("before", "before-1440.png")]);
        assert!(embed(&work).starts_with("![T-7 after](") && embed(&work).lines().count() == 2);
        // Notes per direction; links from the hand-in and from a follow-up's comment, each with its direction.
        assert_eq!(work.changes.iter().map(|c| c.direction.as_str()).collect::<Vec<_>>(), ["a", "b", ""]);
        let links: Vec<(&str, &str, &str)> = work.links.iter().map(|l| (l.url.as_str(), l.direction.as_str(), l.from.as_str())).collect();
        assert_eq!(links, [("https://t7-a.vercel.app/a", "a", "hand-in"), ("https://t7-b-git-x.vercel.app/b", "b", "comment")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn without_a_record_the_first_mock_is_the_hero_and_a_named_hero_wins_over_both() {
        let (root, design) = project("hero");
        std::fs::remove_dir_all(design.join("decisions")).unwrap();
        let at = Where { squad_home: root.join("home"), projects: &[] };
        let t = ticket(7, "done", None, Some(&root), &[], "");
        let work = build(&t, &[], &[], &at).unwrap();
        assert_eq!(work.state, DesignState::Accepted);
        assert!(work.decision.is_none() && work.directions.iter().all(|d| !d.chosen));
        assert!(work.hero.as_deref().unwrap().ends_with("mock-a-1440.png"));
        let named = ticket(7, "done", None, Some(&root), &[], "Hero: design/directions/t7-references/boards/mock-b-390.png\nBefore: design/directions/t7-references/boards/before-390.png");
        let work = build(&named, &[], &[], &at).unwrap();
        assert!(work.hero.as_deref().unwrap().ends_with("mock-b-390.png"), "what the hand-in names, however the files are");
        assert!(work.before[0].path.ends_with("before-390.png") && work.headline()[1].1.ends_with("before-390.png"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_picture_outside_design_or_through_a_link_out_of_it_is_never_given() {
        use std::os::unix::fs::symlink;
        let (root, design) = project("safe");
        let outside = scratch("safe-outside");
        png(&outside.join("secret/mock-s-1440.png"));
        png(&root.join("src/mock-t-1440.png"));
        // A link inside a ticket's folder to a folder outside `design/`, and one to a file.
        symlink(outside.join("secret"), design.join("directions/t7-dashboard/leak")).unwrap();
        symlink(outside.join("secret/mock-s-1440.png"), design.join("directions/t7-dashboard/mock-linked-1440.png")).unwrap();
        // A ticket's folder that is itself a link to a folder elsewhere.
        symlink(&outside, design.join("directions/t7-elsewhere")).unwrap();
        let at = Where { squad_home: root.join("home"), projects: &[] };
        let hostile = format!("Hero: ../src/mock-t-1440.png\nBefore: {}\n![x]({})", outside.join("secret/mock-s-1440.png").display(), root.join("src/mock-t-1440.png").display());
        let t = ticket(7, "review", None, Some(&root), &[], &hostile);
        let work = build(&t, &[], &[], &at).unwrap();
        let all: Vec<&str> = work.directions.iter().flat_map(|d| d.shots.iter()).chain(work.before.iter()).chain(work.pictures.iter()).map(|s| s.path.as_str()).collect();
        assert!(all.iter().all(|p| p.starts_with(design.to_str().unwrap())), "{all:?}");
        assert!(!all.iter().any(|p| p.contains("leak") || p.contains("linked") || p.contains("elsewhere")), "{all:?}");
        assert!(work.directions.iter().all(|d| d.key != "s" && d.key != "t"), "{:?}", work.directions);
        assert!(work.hero.as_deref().is_some_and(|h| h.starts_with(design.to_str().unwrap())), "a hero outside design/ is not taken: {:?}", work.hero);
        // A picture the ticket itself showed may be anywhere it was shown from.
        let shown = outside.join("secret/mock-s-1440.png").to_string_lossy().into_owned();
        let t = ticket(7, "review", None, Some(&root), std::slice::from_ref(&shown), "");
        let work = build(&t, &[], &[], &at).unwrap();
        assert!(work.pictures.iter().any(|p| p.path == shown));
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn a_worktree_is_read_before_the_project_and_a_ticket_with_nothing_made_has_no_card() {
        let (root, _) = project("wt-main");
        let wt = scratch("wt");
        png(&wt.join("design/directions/t7-new/mock-c-1440.png"));
        let at = Where { squad_home: root.join("home"), projects: &[] };
        let t = ticket(7, "in_progress", Some(&wt), Some(&root), &[], "");
        let work = build(&t, &[], &[], &at).unwrap();
        assert_eq!(work.state, DesignState::Working);
        assert_eq!(work.directions.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(), ["c", "a", "b"], "the worktree's own first, then what main has");
        let empty = ticket(9, "in_progress", None, Some(&root), &[], "");
        assert!(build(&empty, &[], &[], &at).is_none());
        let cancelled = ticket(7, "cancelled", None, Some(&root), &[], "");
        assert!(build(&cancelled, &[], &[], &at).is_none());
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&wt).unwrap();
    }

    #[test]
    fn the_walk_is_bounded_and_skips_what_an_app_keeps() {
        let root = scratch("bounded");
        let design = root.join("design");
        png(&design.join("directions/t7-app/node_modules/pkg/mock-x-1440.png"));
        png(&design.join("directions/t7-app/.next/mock-y-1440.png"));
        for i in 0..WALK_ENTRIES + 50 {
            std::fs::create_dir_all(design.join(format!("directions/t7-app/d{i:04}"))).unwrap();
        }
        let found = scan(&design, 7);
        assert!(found.shots.is_empty());
        assert!(is_design(&ticket(1, "review", None, None, &[], "")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
