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

use equipe_protocol::TicketId;
use equipe_protocol::studio::{Studio, StudioCheck, StudioCritique, StudioDirection, StudioFix, StudioLook, StudioPicture, StudioReference};

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
    Some(StudioPicture { path: real.to_string_lossy().into_owned(), modified: modified(&meta), width })
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
        questions: v.questions.iter().take(8).map(|q| (q.what.clone(), q.wording.clone())).collect(),        reviewer: v.reviewer,
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
            let (reviewer, text) = super::reviewer::read_mark(&text);
            Some(StudioCritique { path: p.to_string_lossy().into_owned(), at, text: cut(text, CRITIQUE_CUT), reviewer })
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
/// folder read; nothing, said so, for a ticket the squad doesn't have.
pub fn of_ticket(home: &Path, squad: &str, ticket: TicketId) -> Result<Studio, String> {
    if !crate::squads::valid_name(squad) {
        return Err("no such squad".into());
    }
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    let journal = crate::journal::Journal::open(&db).map_err(|_| "the squad's board couldn't be read".to_owned())?;
    let found = journal.tickets().unwrap_or_default().into_iter().find(|t| t.id == ticket).ok_or_else(|| format!("no T-{ticket} on the board"))?;
    Ok(match found.worktree.as_deref().and_then(design_of) {
        Some(design) => read(&design, ticket),
        None => Studio { ticket, ..Studio::default() },
    })
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
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 250, 255])).save(path).unwrap();
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
        let verdict = Verdict { pass: false, fixes: vec![Fix { place: "top bar".into(), what: "less in it".into(), why: String::new() }], questions: vec![Question { what: "the plan's price".into(), wording: "See plans".into() }], at: 210, by: "codex".into(), reviewer: Some(equipe_protocol::engines::Reviewed { engine: equipe_protocol::Engine::Codex, member: Some(equipe_protocol::Engine::Claude), why: equipe_protocol::engines::ReviewWhy::Room { used: Some(30) } }), ..Verdict::default() };
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
        // One kept with who wrote it, and why that engine.
        let fresh = equipe_protocol::engines::Reviewed { engine: equipe_protocol::Engine::Claude, member: Some(equipe_protocol::Engine::Claude), why: equipe_protocol::engines::ReviewWhy::SignedOut };
        std::fs::write(design.join("critiques/160.md"), format!("{}\n## Whole\nCramped.", super::super::reviewer::mark(&fresh))).unwrap();
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
        assert_eq!(look.reviewer.map(|r| r.engine), Some(equipe_protocol::Engine::Codex), "who looked travels with it");
        assert_eq!(s.checks[1].failing, [("contrast".to_owned(), "the CTA is 2.47:1".to_owned())]);
        assert_eq!(s.pending, ["web/index.html"]);
        assert_eq!(s.directions.iter().map(|d| (d.name.as_str(), d.chosen)).collect::<Vec<_>>(), [("Loud poster", false), ("Calm ledger", true)], "this ticket's, the latest word on each");
        assert_eq!(s.directions[1].pictures.len(), 1);
        assert_eq!(s.directions[0].pictures.len(), 1, "a loose picture named for it");
        assert_eq!(s.critiques.iter().map(|c| (c.at, c.reviewer.map(|r| r.why))).collect::<Vec<_>>(), [(150, None), (160, Some(equipe_protocol::engines::ReviewWhy::SignedOut))]);
        assert_eq!(s.critiques[1].text, "## Whole\nCramped.", "the mark is not the critique");
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
