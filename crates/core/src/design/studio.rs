//! A ticket's design work as it is on disk, for a screen that watches it
//! (`equipe_protocol::studio`, the `design_studio` request): the design
//! folder in the ticket's worktree read — each screen's latest check and
//! second look (`lint::record`), the edits not checked yet, the directions
//! proposed (`directions`), the critiques (`critique`), the reference board
//! (`design/references/`) and the other pictures it made. Read-only, from
//! inside that folder only (links that leave it are not followed), every
//! file and list held to a limit, so an ask is cheap and a big folder never
//! floods a screen.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use equipe_protocol::studio::{PictureFrom, Studio, StudioCheck, StudioCritique, StudioDirection, StudioFix, StudioLook, StudioPicture, StudioReference, StudioReport};
use equipe_protocol::{Entry, TicketId};

use super::lint::record::{self, Record, Verdict};
use crate::visuals;

/// The biggest JSON file read (a check record, the directions' log).
const JSON_MAX: u64 = 1 << 20;
/// How much of a critique or a reference's notes is sent.
const CRITIQUE_CUT: usize = 4000;
const NOTES_CUT: usize = 1600;
/// At most this many of each.
const CHECKS_MAX: usize = 40;
const DIRECTIONS_MAX: usize = 24;
const CRITIQUES_MAX: usize = 24;
const REFERENCES_MAX: usize = 40;
const PER_REFERENCE: usize = 6;
const PER_DIRECTION: usize = 6;
const PICTURES_MAX: usize = 24;
/// How many entries, and how deep, a walk looks at.
const ENTRIES_MAX: usize = 4000;
const DEPTH_MAX: usize = 6;

/// The folders kept apart from "other pictures": each has its own place.
const OWN_DIRS: [&str; 8] = ["checks", "references", "directions", "critiques", "assets", "components", "flows", "decisions"];

fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// The files under `dir` (inside `root`), at most `depth` down: each path
/// and what the disk says of it. Links are not followed; hidden names skipped.
fn walk(dir: &Path, depth: usize) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((at, d)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        let mut entries: Vec<_> = entries.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            seen += 1;
            if seen > ENTRIES_MAX {
                return out;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                if d < depth {
                    stack.push((entry.path(), d + 1));
                }
            } else if meta.is_file() {
                out.push((entry.path(), meta));
            }
        }
    }
    out
}

/// A small text file inside `root`, as text, when it is one and no bigger than `max`.
fn text_inside(root: &Path, path: &Path, max: u64) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let bytes = super::folder::inside(root, &rel.to_string_lossy(), max)?;
    (bytes.len() as u64 <= max).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// `text`, cut at `most` bytes on a character's edge, with an ellipsis.
fn cut(text: &str, most: usize) -> String {
    let text = text.trim();
    if text.len() <= most {
        return text.to_owned();
    }
    let mut end = most;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", text[..end].trim_end())
}

/// A picture inside `root`, as screens ask for it: its real path, when it is one.
fn picture(root: &Path, path: &Path, width: Option<u32>) -> Option<StudioPicture> {
    let real = visuals::check(path, std::slice::from_ref(&root.to_path_buf())).ok()?;
    let meta = std::fs::metadata(&real).ok()?;
    Some(StudioPicture { path: real.to_string_lossy().into_owned(), modified: modified(&meta), width, from: PictureFrom::Design })
}

/// The width a check's picture was drawn at, from its name: `360.png`, `home-1280.light.png`.
pub fn width_of(path: &str) -> Option<u32> {
    let name = path.rsplit('/').next()?;
    let stem = name.split('.').next()?;
    let digits: String = stem.rsplit(|c: char| !c.is_ascii_digit()).next()?.to_owned();
    digits.parse().ok().filter(|w| (200..=4000).contains(w))
}

/// A name as a folder is called: `Calm ledger` → `calm-ledger`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn look(v: &Verdict) -> StudioLook {
    StudioLook {
        pass: v.pass,
        summary: cut(&v.summary, NOTES_CUT),
        at: v.at,
        by: v.by.clone(),
        fixes: v.fixes.iter().take(12).map(|f| StudioFix { place: f.place.clone(), what: f.what.clone(), why: f.why.clone() }).collect(),
        questions: v.questions.iter().take(8).map(|q| (q.what.clone(), q.wording.clone())).collect(),
    }
}

/// A check record's pictures, inside `design`: as it names them (relative to
/// the folder the design folder is in, or to it), else what its screen's
/// picture folder holds — narrowest first.
fn check_pictures(design: &Path, r: &Record) -> Vec<StudioPicture> {
    let root = design.parent().unwrap_or(design);
    let mut out: Vec<StudioPicture> = Vec::new();
    for named in &r.pictures {
        let given = Path::new(named);
        let candidates = if given.is_absolute() { vec![given.to_path_buf()] } else { vec![root.join(given), design.join(given)] };
        if let Some(p) = candidates.iter().find(|c| c.is_file()).and_then(|c| picture(design, c, width_of(named))) {
            out.push(p);
        }
    }
    if out.is_empty() {
        let dir = record::pictures_dir(design, &r.screen);
        out = walk(&dir, 0).into_iter().filter(|(p, _)| visuals::is_image_name(&p.to_string_lossy())).filter_map(|(p, _)| picture(design, &p, width_of(&p.to_string_lossy()))).collect();
    }
    let one = out.len() == 1;
    out.retain(|p| p.width.is_some() || one);
    out.sort_by_key(|p| p.width.unwrap_or(u32::MAX));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn checks(design: &Path) -> Vec<StudioCheck> {
    let dir = design.join(record::DIR);
    let mut out: Vec<StudioCheck> = Vec::new();
    for (path, _) in walk(&dir, DEPTH_MAX) {
        if path.extension().is_none_or(|x| x != "json") || path.file_name().is_some_and(|n| n == record::PENDING) {
            continue;
        }
        let Some(r) = text_inside(design, &path, JSON_MAX).and_then(|t| serde_json::from_str::<Record>(&t).ok()).filter(|r| !r.screen.is_empty()) else { continue };
        let pictures = check_pictures(design, &r);
        let s = &r.summary;
        out.push(StudioCheck {
            screen: r.screen.clone(),
            at: r.at,
            rendered: r.rendered,
            floor: (u32::try_from(s.floor.0).unwrap_or(u32::MAX), u32::try_from(s.floor.1).unwrap_or(u32::MAX)),
            habits: u32::try_from(s.habits).unwrap_or(u32::MAX),
            finish: u32::try_from(s.finish).unwrap_or(u32::MAX),
            failing: s.failing.iter().take(12).map(|(id, e)| (id.clone(), cut(e, 240))).collect(),
            open: s.open.iter().take(12).map(|(id, e)| (id.clone(), cut(e, 240))).collect(),
            pictures,
            first: r.first.as_ref().map(look),
            look: r.verdict.as_ref().map(look),
            fixes_in_a_row: r.fixes_in_a_row,
        });
    }
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.screen.cmp(&b.screen)));
    out.truncate(CHECKS_MAX);
    out
}

fn directions(design: &Path, ticket: TicketId) -> Vec<StudioDirection> {
    let text = text_inside(design, &super::directions::path(design), JSON_MAX);
    let log: Vec<super::directions::Direction> = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let ours = format!("T-{ticket}");
    let named_ours = |t: &Option<String>| t.as_deref().is_none_or(|t| t.trim().trim_start_matches('#').eq_ignore_ascii_case(&ours) || t.trim() == ticket.to_string());
    let files: Vec<(PathBuf, std::fs::Metadata)> = walk(&design.join("directions"), 2).into_iter().filter(|(p, _)| visuals::is_image_name(&p.to_string_lossy())).collect();
    let mut out: Vec<StudioDirection> = Vec::new();
    for d in log.into_iter().filter(|d| named_ours(&d.ticket)) {
        let key = slug(&d.name);
        let dir = design.join("directions").join(&key);
        let mut pictures: Vec<StudioPicture> = files
            .iter()
            .filter(|(p, _)| p.starts_with(&dir) || p.parent() == Some(&design.join("directions")) && p.file_stem().is_some_and(|s| slug(&s.to_string_lossy()).starts_with(&key)))
            .filter_map(|(p, _)| picture(design, p, None))
            .collect();
        pictures.truncate(PER_DIRECTION);
        // Proposed again under the same name: the latest word on it.
        out.retain(|o| slug(&o.name) != key);
        out.push(StudioDirection { name: d.name, at: d.at, chosen: d.chosen, layout: d.layout, type_: d.type_, palette: d.palette, density: d.density, motion: d.motion, pictures });
    }
    out.sort_by_key(|d| d.at);
    if out.len() > DIRECTIONS_MAX {
        out.drain(..out.len() - DIRECTIONS_MAX);
    }
    out
}

fn critiques(design: &Path) -> Vec<StudioCritique> {
    let mut out: Vec<StudioCritique> = walk(&design.join("critiques"), 0)
        .into_iter()
        .filter(|(p, _)| p.extension().is_some_and(|x| x == "md" || x == "txt"))
        .filter_map(|(p, meta)| {
            let text = text_inside(design, &p, 256 << 10)?;
            let at = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or_else(|| modified(&meta));
            Some(StudioCritique { path: p.to_string_lossy().into_owned(), at, text: cut(&text, CRITIQUE_CUT) })
        })
        .collect();
    out.sort_by_key(|c| c.at);
    if out.len() > CRITIQUES_MAX {
        out.drain(..out.len() - CRITIQUES_MAX);
    }
    out
}

/// The first web address in `text`, if any.
pub fn first_link(text: &str) -> Option<String> {
    let at = text.find("https://").or_else(|| text.find("http://"))?;
    let link: String = text[at..].chars().take_while(|c| !c.is_whitespace() && !matches!(c, ')' | '>' | ']' | '"' | '\'' | '`')).collect();
    Some(link.trim_end_matches(['.', ',', ';', ':']).to_owned())
}

/// The notes files a reference folder may keep, first found first.
const NOTE_NAMES: [&str; 6] = ["notes.md", "README.md", "reference.md", "index.md", "notes.txt", "link.txt"];

fn references(design: &Path) -> Vec<StudioReference> {
    let board = design.join("references");
    let Ok(entries) = std::fs::read_dir(&board) else { return Vec::new() };
    let mut out: Vec<StudioReference> = Vec::new();
    let mut loose: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    let mut entries: Vec<_> = entries.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries.into_iter().take(ENTRIES_MAX) {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        if meta.is_file() {
            loose.push((entry.path(), meta));
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let dir = entry.path();
        let files = walk(&dir, 2);
        let notes_file = NOTE_NAMES.iter().map(|n| dir.join(n)).find(|p| p.is_file()).or_else(|| files.iter().map(|(p, _)| p.clone()).find(|p| p.extension().is_some_and(|x| x == "md" || x == "txt")));
        let notes = notes_file.as_ref().and_then(|p| text_inside(design, p, 64 << 10)).unwrap_or_default();
        let mut pictures: Vec<StudioPicture> = files.iter().filter(|(p, _)| visuals::is_image_name(&p.to_string_lossy())).filter_map(|(p, _)| picture(design, p, None)).collect();
        pictures.sort_by_key(|p| std::cmp::Reverse(p.modified));
        pictures.truncate(PER_REFERENCE);
        let newest = files.iter().map(|(_, m)| modified(m)).max().unwrap_or_else(|| modified(&meta));
        out.push(StudioReference { name: entry.file_name().to_string_lossy().into_owned(), link: first_link(&notes), notes: cut(&strip_link_lines(&notes), NOTES_CUT), modified: newest, pictures });
    }
    // A shot kept loose on the board, with its notes beside it (`stripe.png`, `stripe.md`).
    for (path, meta) in &loose {
        let name = path.to_string_lossy();
        if !visuals::is_image_name(&name) {
            continue;
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let notes = ["md", "txt"].iter().map(|x| path.with_extension(x)).find(|p| p.is_file()).and_then(|p| text_inside(design, &p, 64 << 10)).unwrap_or_default();
        let Some(pic) = picture(design, path, None) else { continue };
        out.push(StudioReference { name: stem, link: first_link(&notes), notes: cut(&strip_link_lines(&notes), NOTES_CUT), modified: modified(meta), pictures: vec![pic] });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    out.truncate(REFERENCES_MAX);
    out
}

/// Notes without the lines that are only a link (the link is shown on its own).
fn strip_link_lines(notes: &str) -> String {
    notes
        .lines()
        .filter(|l| {
            let l = l.trim().trim_start_matches(['-', '*', ' ']).trim();
            let bare = l.strip_prefix("Link:").or_else(|| l.strip_prefix("link:")).or_else(|| l.strip_prefix("Source:")).unwrap_or(l).trim();
            !(bare.starts_with("http://") || bare.starts_with("https://")) || bare.contains(' ')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The pictures not kept in a folder of their own: mockups, renders,
/// comparison sheets — newest first.
fn others(design: &Path) -> Vec<StudioPicture> {
    let mut files: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    for (path, meta) in walk(design, DEPTH_MAX) {
        let Ok(rel) = path.strip_prefix(design) else { continue };
        let top = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
        if rel.components().count() > 1 && OWN_DIRS.contains(&top.as_str()) || !visuals::is_image_name(&path.to_string_lossy()) {
            continue;
        }
        files.push((path, meta));
    }
    files.sort_by(|a, b| modified(&b.1).cmp(&modified(&a.1)).then_with(|| a.0.cmp(&b.0)));
    files.into_iter().filter_map(|(p, _)| picture(design, &p, None)).take(PICTURES_MAX).collect()
}

/// What `design` holds of `ticket`'s work.
pub fn read(design: &Path, ticket: TicketId) -> Studio {
    if !std::fs::symlink_metadata(design).is_ok_and(|m| m.is_dir()) {
        return Studio { ticket, ..Studio::default() };
    }
    let design = std::fs::canonicalize(design).unwrap_or_else(|_| design.to_path_buf());
    let pending = record::read_pending(&design).edited.into_iter().map(|e| e.path).take(40).collect();
    let mut studio = Studio {
        ticket,
        dir: Some(design.to_string_lossy().into_owned()),
        checks: checks(&design),
        pending,
        directions: directions(&design, ticket),
        critiques: critiques(&design),
        references: references(&design),
        pictures: others(&design),
        reports: Vec::new(),
        stamp: 0,
    };
    studio.stamp = stamp_of(&studio);
    studio
}

fn stamp_of(studio: &Studio) -> u64 {
    let mut h = DefaultHasher::new();
    serde_json::to_string(studio).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Where a worktree keeps its design work: its `design/` folder, else the
/// one a check was kept in a level or two down (a project's own folder).
pub fn design_of(worktree: &Path) -> Option<PathBuf> {
    let top = worktree.join(super::DIR);
    if std::fs::symlink_metadata(&top).is_ok_and(|m| m.is_dir()) {
        return Some(top);
    }
    let mut stack = vec![(worktree.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten().take(400) {
            let name = e.file_name();
            let p = e.path();
            if name.to_string_lossy().starts_with('.') || name == "node_modules" || name == "target" || !std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            if name == super::DIR && p.join(record::DIR).is_dir() {
                return Some(p);
            }
            if depth < 2 {
                stack.push((p, depth + 1));
            }
        }
    }
    None
}

/// A screen's ask for `ticket`'s work in `squad`: its worktree's design
/// folder read, and every other picture of the work found ([`gather`]);
/// nothing, said so, for a ticket the squad doesn't have. What it names is
/// what `read_image` may give for a while ([`given`]).
pub fn of_ticket(home: &Path, squad: &str, ticket: TicketId) -> Result<Studio, String> {
    if !crate::squads::valid_name(squad) {
        return Err("no such squad".into());
    }
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    let journal = crate::journal::Journal::open(&db).map_err(|_| "the squad's board couldn't be read".to_owned())?;
    let found = journal.tickets().unwrap_or_default().into_iter().find(|t| t.id == ticket).ok_or_else(|| format!("no T-{ticket} on the board"))?;
    let work = journal.ticket_lines(ticket).ok().flatten();
    // Where its work is checked out, as the board keeps it beside the ticket.
    let worktree = found.worktree.clone().or_else(|| journal.ticket_work(ticket).ok().and_then(|w| w.worktree));
    drop(journal);
    let studio = gather(&Gather {
        ticket,
        design: worktree.as_deref().and_then(design_of),
        worktree,
        recordings: Some(crate::phone::recordings_dir(&crate::squad_home::path(home, squad), ticket)),
        images: found.images.clone(),
        cwd: work.as_ref().and_then(|w| w.cwd.clone()),
        lines: work.map(|w| w.lines).unwrap_or_default(),
        also: vec![crate::squad_home::path(home, squad)],
    });
    remember(home, squad, &studio);
    Ok(studio)
}

// ── every picture of the work, not only the design folder's ──

/// What a ticket's work is gathered from.
#[derive(Debug, Clone, Default)]
pub struct Gather {
    pub ticket: TicketId,
    /// Its worktree's design folder, if it has one.
    pub design: Option<PathBuf>,
    pub worktree: Option<PathBuf>,
    /// The phone's recordings for the ticket (`<squad home>/recordings/T-<n>/`).
    pub recordings: Option<PathBuf>,
    /// What its hand-in names.
    pub images: Vec<String>,
    /// Where its conversation works: what a relative path in it is read against.
    pub cwd: Option<PathBuf>,
    /// Its conversation on the ticket, each line with when it was kept.
    pub lines: Vec<(Entry, Option<u64>)>,
    /// Other folders its hand-in folder (`outputs/T-<n>…`) may be in: the
    /// squad's folder, where a task's lands once accepted.
    pub also: Vec<PathBuf>,
}

/// How many pictures of each kind, and in all.
const RECORDINGS_MAX: usize = 24;
const OPENED_MAX: usize = 40;
const BESIDE_PER_FOLDER: usize = 8;
const BESIDE_FOLDERS: usize = 6;
const WORKTREE_MAX: usize = 24;
const ALL_MAX: usize = 80;
const REPORTS_MAX: usize = 4;
const REPORT_CUT: usize = 6000;
/// Folders a walk of a worktree never goes into.
const SKIP_DIRS: [&str; 10] = [".git", "node_modules", "target", "build", "DerivedData", "Pods", ".build", "dist", ".next", "vendor"];

/// The image paths a line of the conversation names: a tool's file, a
/// `--out` it was told to write and absolute paths in a command, absolute
/// paths in its words and a Markdown image (relative to where it works).
pub fn named_images(entry: &Entry, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut take = |raw: &str| {
        let raw = raw.trim_start_matches(['`', '"', '\'', '(', '<', '*', '!']);
        let raw = raw.trim_end_matches(['`', '"', '\'', ')', '>', ',', ';', ':', '*', '.']);
        if !visuals::is_image_name(raw) {
            return;
        }
        let path = Path::new(raw);
        let full = if path.is_absolute() { Some(path.to_path_buf()) } else { cwd.map(|c| c.join(path)) };
        if let Some(full) = full.filter(|p| !p.components().any(|c| matches!(c, std::path::Component::ParentDir))) {
            out.push(full);
        }
    };
    match entry {
        Entry::Tool { tool, target: Some(target), .. } => {
            if matches!(tool.as_str(), "Read" | "Write" | "Edit" | "MultiEdit") {
                take(target);
            } else if tool == "Bash" {
                let words: Vec<&str> = target.split_whitespace().collect();
                for (i, w) in words.iter().enumerate() {
                    if matches!(*w, "--out" | "-o" | ">") {
                        if let Some(next) = words.get(i + 1) {
                            take(next);
                        }
                    } else if w.starts_with('/') {
                        take(w);
                    }
                }
            }
        }
        Entry::Reply { text } | Entry::User { text, .. } => {
            for image in visuals::markdown::local_images(text) {
                take(&image.path);
            }
            // A path written out whole, in or out of backticks.
            for w in text.split_whitespace().map(|w| w.trim_start_matches(['`', '"', '\'', '(', '<'])).filter(|w| w.starts_with('/')) {
                take(w);
            }
        }
        _ => {}
    }
    out
}

/// A picture outside the design folder, as screens ask for it: a file named
/// as an image, no bigger than the most `read_image` gives.
fn outside(path: &Path, from: PictureFrom) -> Option<StudioPicture> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > visuals::FILE_MAX || !visuals::is_image_name(&path.to_string_lossy()) {
        return None;
    }
    let real = std::fs::canonicalize(path).ok()?;
    Some(StudioPicture { path: real.to_string_lossy().into_owned(), modified: modified(&meta), width: None, from })
}

/// The images right in `dir`, newest first, at most `most`.
fn images_in(dir: &Path, most: usize, from: PictureFrom) -> Vec<StudioPicture> {
    let mut found: Vec<StudioPicture> = walk(dir, 0).into_iter().filter_map(|(p, _)| outside(&p, from)).collect();
    found.sort_by_key(|p| std::cmp::Reverse(p.modified));
    found.truncate(most);
    found
}

/// The files under `dir` a walk of a worktree finds, never in [`SKIP_DIRS`].
fn walk_worktree(dir: &Path, depth: usize) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((at, d)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 2 * ENTRIES_MAX {
                return out;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                if d < depth {
                    stack.push((entry.path(), d + 1));
                }
            } else if meta.is_file() {
                out.push((entry.path(), meta));
            }
        }
    }
    out
}

/// The ticket's own folders in its worktree's `outputs/`: named for it
/// (`T-<n>`, `T-<n> …`).
fn output_dirs(worktree: &Path, ticket: TicketId) -> Vec<PathBuf> {
    let name = format!("T-{ticket}");
    let Ok(entries) = std::fs::read_dir(worktree.join("outputs")) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n == name || n.strip_prefix(&name).is_some_and(|rest| rest.starts_with([' ', '-', '_', '.']))
        })
        .filter(|e| std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir()))
        .map(|e| e.path())
        .collect()
}

/// A report's title: its first heading, else its file's name.
fn title_of(text: &str, path: &Path) -> String {
    text.lines().map(str::trim).find_map(|l| l.strip_prefix("# ")).map(|t| t.trim().to_owned()).unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Everything of `g`'s work: its design folder, then each picture found
/// elsewhere — the phone's recordings, the hand-in folder and its reports,
/// what it opened to look at and what lies beside those, what is new in its
/// worktree since the work began, what its hand-in names — each once,
/// newest first.
pub fn gather(g: &Gather) -> Studio {
    let mut studio = match &g.design {
        Some(design) => read(design, g.ticket),
        None => Studio { ticket: g.ticket, ..Studio::default() },
    };
    let since = g.lines.iter().filter_map(|(_, at)| *at).min();
    let mut more: Vec<StudioPicture> = Vec::new();
    // The phone's recordings: frames and shots.
    if let Some(dir) = &g.recordings {
        let mut found: Vec<StudioPicture> = walk(dir, 3).into_iter().filter_map(|(p, _)| outside(&p, PictureFrom::Recording)).collect();
        found.sort_by_key(|p| std::cmp::Reverse(p.modified));
        found.truncate(RECORDINGS_MAX);
        more.extend(found);
    }
    // The hand-in folder: its pictures and its reports — in its worktree, or where it landed.
    let mut seen_dirs: Vec<PathBuf> = Vec::new();
    for root in g.worktree.iter().chain(&g.also) {
        for dir in output_dirs(root, g.ticket) {
            let real = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
            if seen_dirs.contains(&real) {
                continue;
            }
            seen_dirs.push(real);
            for (path, meta) in walk(&dir, 2) {
                let name = path.to_string_lossy().into_owned();
                if let Some(p) = outside(&path, PictureFrom::Output) {
                    more.push(p);
                } else if name.ends_with(".md")
                    && studio.reports.len() < REPORTS_MAX
                    && let Some(text) = text_inside(&dir, &path, 256 << 10)
                {
                    studio.reports.push(StudioReport { path: name, at: modified(&meta), title: title_of(&text, &path), text: cut(&text, REPORT_CUT) });
                }
            }
        }
    }
    // What it opened, wrote or named while it worked, and what lies beside it.
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut opened = 0;
    for (entry, _) in &g.lines {
        for path in named_images(entry, g.cwd.as_deref()) {
            if opened >= OPENED_MAX {
                break;
            }
            if let Some(p) = outside(&path, PictureFrom::Opened) {
                opened += 1;
                if let Some(dir) = Path::new(&p.path).parent().map(Path::to_path_buf)
                    && !folders.contains(&dir)
                    && folders.len() < BESIDE_FOLDERS
                {
                    folders.push(dir);
                }
                more.push(p);
            }
        }
    }
    for dir in &folders {
        more.extend(images_in(dir, BESIDE_PER_FOLDER, PictureFrom::Made).into_iter().filter(|p| since.is_none_or(|s| p.modified + 60 >= s)));
    }
    // New in its worktree since the work began.
    if let (Some(worktree), Some(since)) = (&g.worktree, since) {
        // The design folder and the hand-in folders are read above, each its own way.
        let apart: Vec<PathBuf> = [g.design.clone(), Some(worktree.join("outputs"))].into_iter().flatten().filter_map(|d| std::fs::canonicalize(d).ok()).collect();
        let mut found: Vec<StudioPicture> = walk_worktree(worktree, DEPTH_MAX)
            .into_iter()
            .filter(|(_, meta)| modified(meta) + 60 >= since)
            .filter_map(|(p, _)| outside(&p, PictureFrom::Made))
            .filter(|p| !apart.iter().any(|d| Path::new(&p.path).starts_with(d)))
            .collect();
        found.sort_by_key(|p| std::cmp::Reverse(p.modified));
        found.truncate(WORKTREE_MAX);
        more.extend(found);
    }
    // What its hand-in names.
    more.extend(g.images.iter().filter_map(|p| outside(Path::new(p), PictureFrom::HandIn)));
    // Each once — the design folder's and the checks' as they are — newest first.
    let mut known: std::collections::HashSet<String> = studio.picture_paths().into_iter().map(str::to_owned).collect();
    for p in more {
        if known.insert(p.path.clone()) {
            studio.pictures.push(p);
        }
    }
    studio.pictures.sort_by_key(|p| std::cmp::Reverse(p.modified));
    studio.pictures.truncate(ALL_MAX);
    studio.reports.sort_by_key(|r| std::cmp::Reverse(r.at));
    studio.stamp = stamp_of(&studio);
    studio
}

// ── what read_image may give ──

/// How long the pictures a studio named stay given.
const GIVEN_FOR: std::time::Duration = std::time::Duration::from_secs(15 * 60);

type Given = std::collections::HashMap<(PathBuf, String), (std::time::Instant, std::collections::HashSet<String>)>;

fn given_store() -> &'static std::sync::Mutex<Given> {
    static GIVEN: std::sync::OnceLock<std::sync::Mutex<Given>> = std::sync::OnceLock::new();
    GIVEN.get_or_init(Default::default)
}

/// The pictures a studio answer named for `squad`: `read_image` gives them
/// for a while after ([`given`]).
pub fn remember(home: &Path, squad: &str, studio: &Studio) {
    let mut store = given_store().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, (at, _)| at.elapsed() < GIVEN_FOR);
    let entry = store.entry((home.to_path_buf(), squad.to_owned())).or_insert_with(|| (std::time::Instant::now(), Default::default()));
    entry.0 = std::time::Instant::now();
    if entry.1.len() > 4000 {
        entry.1.clear();
    }
    entry.1.extend(studio.picture_paths().into_iter().map(str::to_owned));
}

/// Whether a studio answer for `squad` named `path` lately.
pub fn given(home: &Path, squad: &str, path: &str) -> bool {
    let store = given_store().lock().unwrap_or_else(|e| e.into_inner());
    store.get(&(home.to_path_buf(), squad.to_owned())).is_some_and(|(at, paths)| at.elapsed() < GIVEN_FOR && paths.contains(path))
}

/// The pictures a watched ticket's design folder holds, for `read_image` to
/// give: each image inside the design folder of a ticket still at work, or
/// in review, in its worktree.
pub fn shown(tickets: &[equipe_protocol::Ticket]) -> Vec<String> {
    use equipe_protocol::TicketStatus;
    let mut out = Vec::new();
    for t in tickets.iter().filter(|t| matches!(t.status, TicketStatus::InProgress | TicketStatus::Waiting | TicketStatus::Review)) {
        let Some(design) = t.worktree.as_deref().and_then(design_of) else { continue };
        let Ok(root) = std::fs::canonicalize(&design) else { continue };
        for (path, _) in walk(&root, DEPTH_MAX) {
            if visuals::is_image_name(&path.to_string_lossy()) && let Ok(real) = visuals::check(&path, std::slice::from_ref(&root)) {
                out.push(real.to_string_lossy().into_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::lint::record::{Fix, Question};
    use crate::design::lint::Summary;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-studio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(4, 4, image::Rgb([20, 90, 250])).save(path).unwrap();
    }

    #[test]
    fn widths_and_names_are_read_as_the_tools_write_them() {
        assert_eq!(width_of("design/checks/home/360.png"), Some(360));
        assert_eq!(width_of("/w/design/checks/login/login-1280.light.png"), Some(1280));
        assert_eq!(width_of("/w/design/mock.png"), None);
        assert_eq!(width_of("/w/design/v2.png"), None, "a version is not a width");
        assert_eq!(slug("Calm ledger"), "calm-ledger");
        assert_eq!(slug("  Bold / Editorial "), "bold-editorial");
        assert_eq!(first_link("Linear's changelog: <https://linear.app/changelog>."), Some("https://linear.app/changelog".into()));
        assert_eq!(first_link("no link"), None);
        assert_eq!(cut("abcdef", 3), "abc…");
    }

    #[test]
    fn a_tickets_design_folder_is_read_whole_from_inside_it_only() {
        let root = dir("whole");
        let design = root.join("design");
        std::fs::create_dir_all(&design).unwrap();
        std::fs::write(design.join("tokens.json"), "{}").unwrap();
        // Two checks: the older failing, the newer drawn at two widths with a look that asked for fixes.
        let failing = Record {
            screen: "onboarding/login".into(),
            at: 100,
            rendered: true,
            summary: Summary { floor: (10, 11), failing: vec![("contrast".into(), "the CTA is 2.47:1".into())], ..Summary::default() },
            ..Record::default()
        };
        record::write(&design, &failing).unwrap();
        png(&design.join("checks/home/360.png"));
        png(&design.join("checks/home/1280.png"));
        let verdict = Verdict { pass: false, fixes: vec![Fix { place: "top bar".into(), what: "less in it".into(), why: String::new() }], questions: vec![Question { what: "the plan's price".into(), wording: "See plans".into() }], at: 210, by: "codex".into(), ..Verdict::default() };
        let home = Record { screen: "home".into(), at: 200, rendered: true, summary: Summary { floor: (11, 11), ..Summary::default() }, pictures: vec!["design/checks/home/1280.png".into(), "design/checks/home/360.png".into()], verdict: Some(verdict), fixes_in_a_row: 1, ..Record::default() };
        record::write(&design, &home).unwrap();
        record::write_pending(&design, &record::Pending { edited: vec![record::Edited { path: "web/index.html".into(), at: 220, created: false }], checked: vec![] }).unwrap();
        // Directions: two of this ticket's, one of another's; the chosen one's pictures in its folder.
        let d = |name: &str, ticket: &str, at: u64, chosen: bool| super::super::directions::Direction { name: name.into(), ticket: Some(ticket.into()), at, chosen, layout: "one column".into(), ..Default::default() };
        super::super::directions::write(&design, &[d("Calm ledger", "T-7", 50, false), d("Loud poster", "T-7", 60, false), d("Old one", "T-3", 10, true), d("Calm ledger", "T-7", 70, true)]).unwrap();
        png(&design.join("directions/calm-ledger/home.png"));
        png(&design.join("directions/loud-poster.png"));
        std::fs::create_dir_all(design.join("critiques")).unwrap();
        std::fs::write(design.join("critiques/150.md"), "The hierarchy is flat.").unwrap();
        // The board: a folder with notes and a shot, and a loose shot.
        std::fs::create_dir_all(design.join("references/linear")).unwrap();
        std::fs::write(design.join("references/linear/notes.md"), "- https://linear.app\nTake: the quiet density.\nLeave: the purple.").unwrap();
        png(&design.join("references/linear/shot.png"));
        png(&design.join("references/stripe.png"));
        png(&design.join("mockups/home-v2.png"));
        png(&design.join("assets/logo.png"));
        // A link out of the folder is never followed.
        let outside = root.join("secret");
        png(&outside.join("private.png"));
        std::os::unix::fs::symlink(&outside, design.join("mockups/leak")).unwrap();
        std::os::unix::fs::symlink(outside.join("private.png"), design.join("mockups/leak.png")).unwrap();

        let s = read(&design, 7);
        assert_eq!(s.dir.as_deref(), Some(design.to_string_lossy().as_ref()));
        assert_eq!(s.checks.iter().map(|c| c.screen.as_str()).collect::<Vec<_>>(), ["home", "onboarding/login"], "newest first");
        assert_eq!(s.checks[0].pictures.iter().map(|p| p.width).collect::<Vec<_>>(), [Some(360), Some(1280)], "narrowest first");
        let look = s.checks[0].look.as_ref().unwrap();
        assert!(!look.pass && look.fixes[0].place == "top bar" && look.questions == [("the plan's price".to_owned(), "See plans".to_owned())]);
        assert_eq!(s.checks[1].failing, [("contrast".to_owned(), "the CTA is 2.47:1".to_owned())]);
        assert_eq!(s.pending, ["web/index.html"]);
        assert_eq!(s.directions.iter().map(|d| (d.name.as_str(), d.chosen)).collect::<Vec<_>>(), [("Loud poster", false), ("Calm ledger", true)], "this ticket's, the latest word on each");
        assert_eq!(s.directions[1].pictures.len(), 1);
        assert_eq!(s.directions[0].pictures.len(), 1, "a loose picture named for it");
        assert_eq!(s.critiques.len(), 1);
        assert_eq!(s.critiques[0].at, 150);
        let linear = s.references.iter().find(|r| r.name == "linear").unwrap();
        assert_eq!(linear.link.as_deref(), Some("https://linear.app"));
        assert!(linear.notes.contains("quiet density") && !linear.notes.contains("https://"), "{}", linear.notes);
        assert_eq!(linear.pictures.len(), 1);
        assert!(s.references.iter().any(|r| r.name == "stripe"));
        assert_eq!(s.pictures.iter().map(|p| p.path.rsplit('/').next().unwrap()).collect::<Vec<_>>(), ["home-v2.png"], "assets, checks and the rest kept apart; links out never followed");
        let all = s.picture_paths();
        assert!(all.iter().all(|p| p.starts_with(design.to_string_lossy().as_ref())), "{all:?}");
        assert_eq!(read(&design, 7).stamp, s.stamp, "the same as long as nothing changes");
        png(&design.join("mockups/home-v3.png"));
        assert_ne!(read(&design, 7).stamp, s.stamp);
        // Nothing there yet: said as nothing.
        let empty = read(&root.join("nowhere"), 7);
        assert_eq!(empty.dir, None);
        assert!(empty.checks.is_empty());
    }

    /// A design QA ticket's conversation, in the shape the journal keeps it
    /// (`{"kind":"tool","tool":…,"target":…,"label":…}`), with made-up paths.
    fn qa_lines(scratch: &Path, worktree: &Path, other: &Path) -> Vec<(Entry, Option<u64>)> {
        let line = |json: String| serde_json::from_str::<Entry>(&json).unwrap();
        let s = scratch.display();
        let w = worktree.display();
        let o = other.display();
        vec![
            (line(r#"{"kind":"note","text":"── T-7 · Design QA: T-6 the composer at large text — on branch equipe/designer-t7-1"}"#.into()), Some(1_000)),
            (line(r#"{"kind":"tool","tool":"Skill","target":"equipe:design-qa"}"#.into()), Some(1_010)),
            (line(format!(r#"{{"kind":"tool","tool":"Bash","target":"cd {o}; git diff main...HEAD --stat"}}"#)), Some(1_020)),
            (line(format!(r#"{{"kind":"tool","tool":"Read","target":"{o}/design/flows/chat/large.dark.360.png"}}"#)), Some(1_030)),
            (line(format!(r#"{{"kind":"tool","tool":"Bash","target":"xcrun simctl io booted screenshot {s}/ax5.png","background":true}}"#)), Some(1_040)),
            (line(format!(r#"{{"kind":"tool","tool":"Read","target":"{s}/ax5.png"}}"#)), Some(1_050)),
            (line(format!(r#"{{"kind":"reply","text":"At AX5 the cancel glyph spills out of its circle: {s}/ax5.png"}}"#)), Some(1_060)),
            (line(format!(r#"{{"kind":"tool","tool":"Write","target":"{w}/outputs/T-7 Design QA/report.md","label":"Write the report"}}"#)), Some(1_070)),
            (line(r#"{"kind":"tool","tool":"Read","target":"/private/tmp/claude-501/-Users-someone--equipe-worktrees-squad-home-t7-1/668b1517-195e-4164-b67a-46deb0ae48"}"#.into()), Some(1_080)),
        ]
    }

    #[test]
    fn a_qa_tickets_pictures_come_from_everywhere_it_looked_and_made() {
        let root = dir("qa");
        let (scratch, worktree, other, recordings) = (root.join("scratch/t7"), root.join("wt"), root.join("other-wt"), root.join("home/recordings/T-7"));
        png(&scratch.join("ax5.png"));
        png(&scratch.join("before-after.png"));
        png(&other.join("design/flows/chat/large.dark.360.png"));
        png(&recordings.join("composer/01-0.0s.jpg"));
        png(&worktree.join("outputs/T-7 Design QA/1-ax5.png"));
        std::fs::write(worktree.join("outputs/T-7 Design QA/report.md"), "# T-7 Design QA — the composer\n\nVerdict: **fix and again**.\n\n## Blocking\n1. The cancel glyph spills at AX5.\n").unwrap();
        png(&worktree.join("outputs/T-70 Other/old.png"));
        png(&worktree.join("notes/sketch.png"));
        png(&worktree.join("node_modules/pkg/icon.png"));
        let lines = qa_lines(&scratch, &worktree, &other);
        let g = Gather { ticket: 7, design: None, worktree: Some(worktree.clone()), recordings: Some(recordings.clone()), images: vec![], cwd: Some(worktree.clone()), lines, also: vec![worktree.clone()] };
        let s = gather(&g);
        let name = |p: &StudioPicture| Path::new(&p.path).file_name().unwrap().to_string_lossy().into_owned();
        let from = |n: &str| s.pictures.iter().find(|p| name(p) == n).map(|p| p.from);
        assert_eq!(from("ax5.png"), Some(PictureFrom::Opened), "a shot it took and opened");
        assert_eq!(from("large.dark.360.png"), Some(PictureFrom::Opened), "the engineer's picture it looked at");
        assert_eq!(from("before-after.png"), Some(PictureFrom::Made), "made beside what it opened");
        assert_eq!(from("01-0.0s.jpg"), Some(PictureFrom::Recording));
        assert_eq!(from("1-ax5.png"), Some(PictureFrom::Output));
        assert_eq!(from("sketch.png"), Some(PictureFrom::Made), "new in its worktree since it began");
        assert_eq!(from("old.png"), None, "another ticket's hand-in folder isn't this one's");
        assert_eq!(from("icon.png"), None, "never in dependencies");
        assert_eq!(s.pictures.iter().filter(|p| name(p) == "ax5.png").count(), 1, "each once");
        assert_eq!(s.reports.len(), 1);
        assert_eq!(s.reports[0].title, "T-7 Design QA — the composer");
        assert!(s.reports[0].text.contains("cancel glyph"));
        // Newest first.
        assert!(s.pictures.windows(2).all(|w| w[0].modified >= w[1].modified));
        // A truncated path is no picture.
        assert!(s.pictures.iter().all(|p| !p.path.contains("668b1517")));
    }

    #[test]
    fn a_tickets_work_is_read_from_the_conversation_that_did_it() {
        let root = dir("journal");
        let journal = crate::journal::Journal::open(&root.join("squad.db")).unwrap();
        let note = |t: &str| Entry::Note { text: t.into(), undo: None };
        let read = |t: &str| Entry::Tool { tool: "Read".into(), target: Some(t.into()), outcome: None, label: None, background: false };
        for e in [note("── T-6 · Other — on branch b"), read("/a.png"), note("── T-7 · QA — on branch c"), read("/b.png"), note("Stopped."), read("/c.png"), note("── T-8 · Next — on branch d"), read("/d.png")] {
            journal.append(42, &e).unwrap();
        }
        let work = journal.ticket_lines(7).unwrap().unwrap();
        assert_eq!(work.session, 42);
        let targets: Vec<String> = work.lines.iter().filter_map(|(e, _)| if let Entry::Tool { target, .. } = e { target.clone() } else { None }).collect();
        assert_eq!(targets, ["/b.png", "/c.png"], "from its header to the next ticket's, a restart's note and all");
        assert!(work.lines.iter().all(|(_, at)| at.is_some()));
        assert!(journal.ticket_lines(70).unwrap().is_none(), "T-70 is not T-7");
    }

    #[test]
    fn the_studio_names_what_read_image_may_give_for_a_while() {
        let root = dir("given");
        let studio = Studio { ticket: 1, pictures: vec![StudioPicture { path: "/tmp/x/a.png".into(), modified: 1, width: None, from: PictureFrom::Opened }], ..Studio::default() };
        assert!(!given(&root, "acme", "/tmp/x/a.png"));
        remember(&root, "acme", &studio);
        assert!(given(&root, "acme", "/tmp/x/a.png"));
        assert!(!given(&root, "zeta", "/tmp/x/a.png"), "only for the squad it was asked for");
        assert!(!given(&root, "acme", "/tmp/x/b.png"));
    }

    #[test]
    fn a_line_names_the_pictures_it_opened_shot_or_wrote_of() {
        let entry = |json: &str| serde_json::from_str::<Entry>(json).unwrap();
        let cwd = Path::new("/w");
        assert_eq!(named_images(&entry(r#"{"kind":"tool","tool":"Read","target":"/tmp/a.png"}"#), Some(cwd)), [PathBuf::from("/tmp/a.png")]);
        assert_eq!(named_images(&entry(r#"{"kind":"tool","tool":"Bash","target":"equipe shoot https://x.com --out design/references/x/shot.png"}"#), Some(cwd)), [PathBuf::from("/w/design/references/x/shot.png")]);
        assert_eq!(named_images(&entry(r#"{"kind":"tool","tool":"Bash","target":"xcrun simctl io booted screenshot /tmp/t/s.png"}"#), None), [PathBuf::from("/tmp/t/s.png")]);
        assert_eq!(named_images(&entry(r#"{"kind":"reply","text":"Here: ![sheet](out/sheet.png) and `/tmp/t/b.jpg`."}"#), Some(cwd)), [PathBuf::from("/w/out/sheet.png"), PathBuf::from("/tmp/t/b.jpg")]);
        assert!(named_images(&entry(r#"{"kind":"tool","tool":"Read","target":"../up.png"}"#), Some(cwd)).is_empty(), "never up and out");
        assert!(named_images(&entry(r#"{"kind":"tool","tool":"Read","target":"/w/src/App.swift"}"#), Some(cwd)).is_empty());
    }

    #[test]
    fn a_worktree_keeps_its_design_work_at_its_top_or_in_a_project_folder() {
        let root = dir("where");
        assert_eq!(design_of(&root), None);
        std::fs::create_dir_all(root.join("web/design/checks")).unwrap();
        assert_eq!(design_of(&root), Some(root.join("web/design")));
        std::fs::create_dir_all(root.join("design")).unwrap();
        assert_eq!(design_of(&root), Some(root.join("design")));
    }
}

