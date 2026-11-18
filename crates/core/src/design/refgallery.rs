//! The references a designer keeps (`skills/design-references`), read for a
//! screen: `design/references/<pattern>/` — pictures and a `notes.md` of
//! entries (`## <product> — <screen>` with Source / Take / Leave / Fit lines)
//! — and `design/references/sources.md`, in a project's design folder and, for
//! the patterns the project has none of, the squad's own.
//!
//! Read only: a pattern is a real folder (never a link) right in the
//! references folder, a picture one `read_image` may give
//! ([`visuals::check`]: no `..`, no link out of the folder, an image of a kind
//! taken), a notes file a regular file. A picture lives in its pattern's
//! folder, or is linked from an entry of `notes.md` with an `Image:` line (a
//! path from the notes file): that one counts only when it really is a file
//! inside the same `design/` folder — no `..` out of it, no link out of it.
//! The places read are the project's own references and each ticket's
//! worktree that has some; the one with the latest work shows first. Everything is bounded — patterns
//! looked at, the page sent, pictures per pattern, how much of a notes file or
//! a sources file is read, and how long a line may be — so a folder that grew
//! big costs a screen one page. Pictures are served through `read_image`
//! (`visuals`' references gate), by the paths named here.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use equipe_protocol::design::{RefNote, RefPattern, RefPicture, RefPlace, RefSource};

use crate::visuals;

/// The folder, in a design folder.
pub const DIR: &str = super::refs::DIR;
pub const NOTES: &str = "notes.md";
pub const SOURCES: &str = "sources.md";
/// Patterns looked at, at most, and sent in one answer.
pub const PATTERNS_SCAN: usize = 400;
pub const PAGE: usize = 60;
/// Pictures sent for a pattern, and files looked at to count them.
pub const PICTURES: usize = 48;
const FILES_SCAN: usize = 600;
/// How much of a notes file is read, and of a sources file.
pub const NOTES_MAX: u64 = 64 << 10;
pub const SOURCES_MAX: u64 = 32 << 10;
/// Entries kept of a notes file, sources of a sources file, and the longest field.
pub const ENTRIES: usize = 40;
pub const SOURCE_LINES: usize = 60;
pub const FIELD_CHARS: usize = 600;
/// The longest query looked for.
pub const QUERY_CHARS: usize = 80;

/// What an ask found.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// How many patterns match; `patterns` is the page asked for.
    pub total: usize,
    pub patterns: Vec<RefPattern>,
    pub sources: Vec<RefSource>,
    /// Where the references are kept (the project's, else the squad's).
    pub folder: String,
    /// Every place that has references, and the one read (`0`: the project's own).
    pub places: Vec<RefPlace>,
    pub at: u64,
}

/// A place that has references, on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub info: RefPlace,
    /// Its `design/references` folder.
    pub dir: PathBuf,
}

/// The project's own `design/` folder (the squad's when none), where it is.
fn main_design(home: &Path, squad: &str, project: Option<&str>) -> Option<PathBuf> {
    let Some(project) = project else { return Some(crate::squad_home::path(home, squad).join(super::folder::DIR)) };
    let journal = crate::journal::Journal::open(&crate::journal::squad_dir(home, squad).join("squad.db")).ok()?;
    journal.projects().unwrap_or_default().into_iter().find(|p| crate::board::project_name(p) == project).map(|p| p.join(super::folder::DIR))
}

/// A `design/references` folder is there when it and `design/` are real folders.
fn has_refs(dir: &Path) -> bool {
    dir.parent().is_some_and(real_dir) && real_dir(dir)
}

/// The latest change among the files under `refs` (notes, pictures, sources),
/// skipping the ones `same_as` has as they are: a worktree's checkout of
/// what the project has already is no work of its own.
fn activity(refs: &Path, same_as: Option<&Path>) -> u64 {
    let mut latest = 0;
    let mut seen = 0usize;
    let mut stack = vec![(refs.to_owned(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > FILES_SCAN * 4 {
                return latest;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() && depth < 2 {
                stack.push((entry.path(), depth + 1));
            } else if meta.is_file() {
                let same = same_as.zip(entry.path().strip_prefix(refs).ok()).and_then(|(base, rel)| std::fs::symlink_metadata(base.join(rel)).ok()).is_some_and(|m| m.is_file() && m.len() == meta.len());
                if !same {
                    latest = latest.max(secs(&meta));
                }
            }
        }
    }
    latest
}

/// Every place of `project` (the squad's own when none) that has
/// `design/references/`: its own folder, and each ticket's worktree — at work
/// or in review — with references of its own. The project's first, then the
/// tickets by the latest references work.
pub fn places(home: &Path, squad: &str, project: Option<&str>) -> Vec<Place> {
    if !crate::squads::valid_name(squad) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let main = main_design(home, squad, project).map(|d| d.join(DIR)).filter(|d| has_refs(d));
    if let Some(dir) = &main {
        out.push(Place { info: RefPlace { patterns: pattern_dirs(dir).len() as u32, updated: activity(dir, None), ..RefPlace::default() }, dir: dir.clone() });
    }
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !db.exists() {
        return out;
    }
    let Ok(journal) = crate::journal::Journal::open(&db) else { return out };
    let mut tickets = Vec::new();
    for ticket in journal.tickets().unwrap_or_default() {
        use equipe_protocol::{TicketKind, TicketStatus};
        let review = match ticket.status {
            TicketStatus::Review => true,
            TicketStatus::InProgress | TicketStatus::Waiting => false,
            _ => continue,
        };
        let of = if ticket.kind == TicketKind::Code { ticket.project.as_deref().map(crate::board::project_name) } else { None };
        let Some(worktree) = ticket.worktree.as_ref().filter(|_| of.as_deref() == project) else { continue };
        let dir = worktree.join(super::folder::DIR).join(DIR);
        if !has_refs(&dir) {
            continue;
        }
        let updated = activity(&dir, main.as_deref());
        // With the project's own to compare to, a checkout that added nothing is no place of its own.
        if main.is_some() && updated == 0 {
            continue;
        }
        tickets.push(Place { info: RefPlace { ticket: Some(u64::from(ticket.id)), title: ticket.title.clone(), review, patterns: pattern_dirs(&dir).len() as u32, updated }, dir });
    }
    tickets.sort_by_key(|p| std::cmp::Reverse(p.info.updated));
    out.extend(tickets);
    out
}

/// The place to read of `places`: the one asked for (`0`: the project's own,
/// else a ticket's id), else the one with the latest references work — the
/// project's own on a tie.
fn pick(places: &[Place], ticket: Option<u64>) -> Option<&Place> {
    let asked = ticket.and_then(|t| places.iter().find(|p| p.info.ticket == (t != 0).then_some(t)));
    asked.or_else(|| places.iter().rev().max_by_key(|p| p.info.updated))
}

/// The references folders to read: the one picked first, then the squad's
/// own when it is a project that is asked about.
fn folders(home: &Path, squad: &str, project: Option<&str>, picked: Option<&Place>) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    match picked {
        Some(place) => out.push((place.dir.clone(), false)),
        None => out.extend(main_design(home, squad, project).filter(|_| project.is_some()).map(|d| (d.join(DIR), false))),
    }
    if project.is_some() || picked.is_none() {
        let own = crate::squad_home::path(home, squad).join(super::folder::DIR).join(DIR);
        out.push((own, project.is_some()));
    }
    out
}

/// Whether `path` is a real folder: not a link to one.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// The references a screen asks for: `project`'s (the squad's own when
/// none) — in the place `ticket` names, else where the latest work is — the
/// squad's for the patterns the project has none of, the patterns holding
/// `query` (any case) in their name or notes, and the page at `offset`.
pub fn read(home: &Path, squad: &str, project: Option<&str>, ticket: Option<u64>, query: &str, offset: usize) -> Found {
    if !crate::squads::valid_name(squad) {
        return Found::default();
    }
    let all = places(home, squad, project);
    let picked = pick(&all, ticket);
    let at = picked.and_then(|p| p.info.ticket).unwrap_or(0);
    let mut found = read_in(folders(home, squad, project, picked), query, offset);
    found.at = at;
    found.places = all.iter().map(|p| p.info.clone()).collect();
    found
}

/// [`read`] of the references folders given — each with whether it is the
/// squad's, shown only for the patterns a folder before it has none of.
pub fn read_in(places: Vec<(PathBuf, bool)>, query: &str, offset: usize) -> Found {
    let mut found = Found { folder: places.first().map(|(d, _)| d.to_string_lossy().into_owned()).unwrap_or_default(), ..Found::default() };
    // `design/` and `references/` must themselves be real folders.
    let places: Vec<(PathBuf, bool)> = places.into_iter().filter(|(dir, _)| dir.parent().is_some_and(real_dir) && real_dir(dir)).collect();
    found.sources = places.iter().find_map(|(dir, _)| sources_in(dir)).unwrap_or_default();
    let mut named: Vec<(String, PathBuf, bool)> = Vec::new();
    for (dir, shared) in &places {
        for (name, path) in pattern_dirs(dir) {
            if !named.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(&name)) {
                named.push((name, path, *shared));
            }
        }
    }
    named.sort_by_key(|(n, _, _)| n.to_lowercase());
    named.truncate(PATTERNS_SCAN);
    let query: String = query.trim().chars().take(QUERY_CHARS).collect::<String>().to_lowercase();
    let matching: Vec<(String, PathBuf, bool)> = named.into_iter().filter(|(name, path, _)| query.is_empty() || name.to_lowercase().contains(&query) || read_text(&path.join(NOTES), NOTES_MAX).to_lowercase().contains(&query)).collect();
    found.total = matching.len();
    found.patterns = matching.into_iter().skip(offset).take(PAGE).map(|(name, path, shared)| pattern(&name, &path, shared)).collect();
    found
}

/// The pattern folders right in `refs`, by name: real folders, not hidden.
fn pattern_dirs(refs: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(refs) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            let sane = !name.starts_with('.') && name.chars().count() <= 80 && !name.chars().any(|c| c.is_control() || c == '/' || c == '\\');
            (sane && real_dir(&e.path())).then(|| (name, e.path()))
        })
        .collect();
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out.truncate(PATTERNS_SCAN);
    out
}

fn secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// A regular file's text (not a link, not another kind), at most `max` bytes, cut at a character.
fn read_text(path: &Path, max: u64) -> String {
    use std::io::Read;
    let Ok(meta) = std::fs::symlink_metadata(path) else { return String::new() };
    if !meta.is_file() {
        return String::new();
    }
    let Ok(file) = std::fs::File::open(path) else { return String::new() };
    let mut bytes = Vec::new();
    if file.take(max).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    // A cut in the middle of a character left a replacement mark at the end.
    if meta.len() > max {
        while text.ends_with('\u{FFFD}') {
            text.pop();
        }
    }
    text
}

/// `rel`, a path an `Image:` line gives, from `dir` (where `notes.md` is), as
/// a picture of the same `design/` folder: the path written out without `..`
/// (it must stay inside `design`), and the real file it is (links resolved,
/// still inside `design`, a regular file, an image of a kind taken, not too
/// big — [`visuals::check`]).
fn linked(dir: &Path, design: &Path, rel: &str) -> Option<(PathBuf, PathBuf)> {
    use std::path::Component;
    let rel = Path::new(rel);
    let mut path = dir.to_owned();
    for part in rel.components() {
        match part {
            Component::Normal(p) => path.push(p),
            Component::CurDir => {}
            Component::ParentDir if path.pop() => {}
            _ => return None,
        }
    }
    if !path.starts_with(design) || path == design {
        return None;
    }
    let real = visuals::check(&path, std::slice::from_ref(&design.to_path_buf())).ok()?;
    Some((path, real))
}

fn pattern(name: &str, dir: &Path, shared: bool) -> RefPattern {
    let raw = read_text(&dir.join(NOTES), NOTES_MAX);
    let (notes, images, agree) = parse_notes_with_images(&raw);
    let mut updated = std::fs::symlink_metadata(dir.join(NOTES)).ok().filter(|m| m.is_file()).map_or(0, |m| secs(&m));
    let root = dir.parent().and_then(|p| std::fs::canonicalize(p).ok());
    // The `design/` folder the references are in: where linked pictures may be.
    let design = dir.parent().filter(|refs| refs.file_name().is_some_and(|n| n == DIR)).and_then(Path::parent).map(Path::to_path_buf);
    let mut files: Vec<(String, std::fs::Metadata)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten().take(FILES_SCAN) {
            let Some(file) = entry.file_name().to_str().map(str::to_owned) else { continue };
            if file.starts_with('.') || !visuals::is_image_name(&file) {
                continue;
            }
            if let Ok(meta) = std::fs::symlink_metadata(entry.path())
                && (meta.is_file() || meta.file_type().is_symlink())
            {
                files.push((file, meta));
            }
        }
    }
    files.sort_by_key(|(n, _)| n.to_lowercase());
    let mut pictures_total = files.len() as u32;
    let mut pictures: Vec<RefPicture> = Vec::new();
    // What each picture really is, to tell the same file linked twice.
    let mut reals: Vec<PathBuf> = Vec::new();
    for (file, link_meta) in files {
        if pictures.len() >= PICTURES {
            updated = updated.max(secs(&link_meta));
            continue;
        }
        let path = dir.join(&file);
        let Some(real) = root.as_ref().and_then(|root| visuals::check(&path, std::slice::from_ref(root)).ok()) else { continue };
        let meta = std::fs::metadata(&path).unwrap_or(link_meta);
        updated = updated.max(secs(&meta));
        reals.push(real);
        pictures.push(RefPicture { path: path.to_string_lossy().into_owned(), name: file, size: meta.len(), modified: secs(&meta), note: None });
    }
    // The pictures the entries link, by `Image:` lines — each to its own entry.
    if let Some(design) = &design {
        let mut looked = 0;
        for (n, links) in images.iter().enumerate() {
            for rel in links {
                looked += 1;
                if looked > PICTURES * 2 {
                    break;
                }
                let Some((path, real)) = linked(dir, design, rel) else { continue };
                if let Some(at) = reals.iter().position(|r| *r == real) {
                    // Already there (in the pattern's folder, or linked by another entry): the first entry keeps it.
                    if let Some(pic) = pictures.get_mut(at).filter(|p| p.note.is_none()) {
                        pic.note = Some(n as u32);
                    }
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&real) else { continue };
                pictures_total += 1;
                updated = updated.max(secs(&meta));
                if pictures.len() >= PICTURES {
                    continue;
                }
                let name = path.file_name().and_then(|f| f.to_str()).unwrap_or_default().to_owned();
                reals.push(real);
                pictures.push(RefPicture { path: path.to_string_lossy().into_owned(), name, size: meta.len(), modified: secs(&meta), note: Some(n as u32) });
            }
        }
    }
    RefPattern { name: name.to_owned(), shared, notes, agree, raw, pictures, pictures_total, updated }
}

// ── notes.md ──

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= FIELD_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(FIELD_CHARS).collect();
    format!("{}…", cut.trim_end())
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Source,
    Take,
    Leave,
    Fit,
    Image,
}

/// A bullet line `- Key: value` (any bold around the key) of one of the four fields.
fn field_line(line: &str) -> Option<(Field, &str)> {
    let rest = line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* "))?;
    let colon = rest.find([':', '：'])?;
    let key = rest[..colon].trim_matches(|c: char| c == '*' || c == '_' || c.is_whitespace()).to_lowercase();
    let after = &rest[colon..];
    let after = &after[after.chars().next()?.len_utf8()..];
    let value = after.trim_start_matches(['*', '_']).trim();
    let field = match key.as_str() {
        "source" => Field::Source,
        "take" => Field::Take,
        "leave" => Field::Leave,
        "fit" => Field::Fit,
        "image" => Field::Image,
        _ => return None,
    };
    Some((field, value))
}

#[derive(Default)]
struct Draft {
    title: String,
    fields: [String; 4],
    /// The paths its `Image:` lines give.
    images: Vec<String>,
    seen: bool,
    /// Text after the fields, past a blank line: what the examples agree on.
    tail: Vec<String>,
}

impl Draft {
    fn field(&mut self, f: Field) -> &mut String {
        &mut self.fields[f as usize]
    }
}

/// `notes.md` as entries, and the closing lines on what the examples agree on.
pub fn parse_notes(raw: &str) -> (Vec<RefNote>, String) {
    let (notes, _, agree) = parse_notes_with_images(raw);
    (notes, agree)
}

/// The path an `Image:` value gives: in backticks, the target of a Markdown
/// image or link, in `<…>`, or the plain text. Never an address or an
/// absolute path.
fn image_path(value: &str) -> Option<String> {
    let value = value.trim();
    let path = if let Some(ticked) = backticked(value) {
        ticked
    } else if let Some(open) = value.find("](") {
        let rest = &value[open + 2..];
        rest[..rest.find(')')?].trim()
    } else {
        value.trim_matches(['<', '>', '"', '\'', ' ']).trim_end_matches(['.', ',', ';'])
    };
    let path = path.trim_matches(['<', '>']);
    let bad = path.is_empty() || path.contains("://") || path.starts_with('/') || path.starts_with('\\') || path.contains(':') || path.chars().any(char::is_control) || path.len() > 400;
    (!bad).then(|| path.to_owned())
}

/// [`parse_notes`], with each entry's `Image:` paths too (in the entries' order).
fn parse_notes_with_images(raw: &str) -> (Vec<RefNote>, Vec<Vec<String>>, String) {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut last: Option<Field> = None;
    let mut blank = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("## ") {
            drafts.push(Draft { title: title.trim().to_owned(), ..Draft::default() });
            last = None;
            blank = false;
            continue;
        }
        let Some(draft) = drafts.last_mut() else { continue };
        if trimmed.is_empty() {
            blank = true;
            last = None;
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some((Field::Image, value)) = field_line(line) {
            draft.seen = true;
            if draft.images.len() < 8 {
                draft.images.extend(image_path(value));
            }
            last = None;
            blank = false;
        } else if let Some((field, value)) = field_line(line) {
            draft.seen = true;
            let slot = draft.field(field);
            if !slot.is_empty() {
                slot.push(' ');
            }
            slot.push_str(value);
            last = Some(field);
            blank = false;
        } else if let Some(field) = last.filter(|_| !blank) {
            // A wrapped line, or a sub-bullet, of the field above.
            let text = trimmed.trim_start_matches(['-', '*']).trim();
            let slot = draft.field(field);
            slot.push(' ');
            slot.push_str(text);
        } else {
            draft.tail.push(trimmed.trim_start_matches(['-', '*']).trim().to_owned());
        }
    }
    let mut notes = Vec::new();
    let mut images = Vec::new();
    let mut agree = String::new();
    let count = drafts.len();
    for (i, d) in drafts.into_iter().enumerate() {
        if !d.seen {
            // A heading with no fields: the closing summary, when it is last.
            if i + 1 == count && !d.tail.is_empty() {
                agree = clip(&d.tail.join(" "));
            }
            continue;
        }
        if i + 1 == count && !d.tail.is_empty() {
            agree = clip(&d.tail.join(" "));
        }
        if notes.len() < ENTRIES {
            notes.push(note(&d));
            images.push(d.images.clone());
        }
    }
    (notes, images, agree)
}

fn note(d: &Draft) -> RefNote {
    let [source, take, leave, fit] = &d.fields;
    let linked = d.images.first().and_then(|p| p.rsplit('/').next()).filter(|p| !p.is_empty()).map(str::to_owned);
    let (product, screen) = [" — ", " – ", " - "].iter().find_map(|sep| d.title.split_once(sep)).map_or((d.title.as_str(), ""), |(a, b)| (a.trim(), b.trim()));
    let url = url_in(source);
    let host = url.as_deref().and_then(host_of);
    let after = url.as_deref().and_then(|u| source.find(u).map(|at| &source[at + u.len()..])).unwrap_or(source.as_str());
    let name = after.find('(').and_then(|open| after[open + 1..].find(')').map(|close| after[open + 1..open + 1 + close].trim()));
    RefNote {
        title: clip(&d.title),
        product: clip(product),
        screen: clip(screen),
        url,
        host,
        source: name.filter(|n| !n.is_empty()).map(clip),
        date: date_in(source),
        picture: linked.or_else(|| backticked(source).and_then(|p| p.rsplit('/').next().map(str::to_owned)).filter(|p| !p.is_empty())),
        take: clip(take),
        leave: clip(leave),
        fit: clip(fit),
    }
}

/// The first `http(s)` address in `text` — in `<…>`, a Markdown link or bare.
fn url_in(text: &str) -> Option<String> {
    let at = text.find("https://").or_else(|| text.find("http://"))?;
    let end = text[at..].find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | '`' | ',' | ']')).map_or(text.len(), |e| at + e);
    let url = text[at..end].trim_end_matches(['.', ';']);
    host_of(url).map(|_| url.to_owned())
}

/// The host of an `http(s)` address, without `www.`: none for one with a
/// login in it or a host that isn't a name.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let host = authority.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(authority, |(h, _)| h);
    let host = host.strip_prefix("www.").unwrap_or(host);
    let ok = !host.is_empty() && host.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') && !host.starts_with('.') && !host.ends_with('.');
    ok.then(|| host.to_ascii_lowercase())
}

/// The first `YYYY-MM-DD` in `text`.
fn date_in(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(9)).find_map(|i| {
        let w = &b[i..i + 10];
        let ok = w.iter().enumerate().all(|(k, c)| if k == 4 || k == 7 { *c == b'-' } else { c.is_ascii_digit() });
        let boundary = (i == 0 || !b[i - 1].is_ascii_digit()) && b.get(i + 10).is_none_or(|c| !c.is_ascii_digit());
        (ok && boundary).then(|| text[i..i + 10].to_owned())
    })
}

fn backticked(text: &str) -> Option<&str> {
    let open = text.find('`')?;
    let close = text[open + 1..].find('`')?;
    Some(text[open + 1..open + 1 + close].trim()).filter(|s| !s.is_empty())
}

// ── sources.md ──

fn sources_in(refs: &Path) -> Option<Vec<RefSource>> {
    let raw = read_text(&refs.join(SOURCES), SOURCES_MAX);
    let found = parse_sources(&raw);
    (!found.is_empty()).then_some(found)
}

/// `- <name> — <URL> — good for: … — fits: … — access: …`, one source a line.
pub fn parse_sources(raw: &str) -> Vec<RefSource> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("- ").or_else(|| line.trim().strip_prefix("* ")) else { continue };
        let mut parts = rest.split(" — ").map(str::trim);
        let name = parts.next().unwrap_or_default().trim_matches('*').trim();
        if name.is_empty() || !rest.contains(" — ") {
            continue;
        }
        let mut source = RefSource { name: clip(name), ..RefSource::default() };
        for part in parts {
            let lower = part.to_lowercase();
            if let Some(v) = lower.strip_prefix("good for:").map(|_| part["good for:".len()..].trim()) {
                source.good_for = clip(v);
            } else if lower.starts_with("fits:") {
                source.fits = clip(part["fits:".len()..].trim());
            } else if lower.starts_with("access:") {
                source.access = clip(part["access:".len()..].trim());
            } else if source.url.is_none() {
                source.url = url_in(part);
            }
        }
        out.push(source);
        if out.len() >= SOURCE_LINES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES_MD: &str = "# kpi-tile\n\n## Acme — Orders overview\n- Source: https://www.example.com/orders (Gallery One), 2026-10-02, `shots/acme-1440.png`\n- Take: label above value, one accent;\n  tabular figures\n- Leave: the mascot\n- Fit: high — same density\n\n## Beta - Revenue\n- **Source:** <https://beta.example.org/rev?x=1> (Library), 2026-09-20, `beta-390.png`\n- **Take:** sparkline under the number\n- **Leave:** gradients\n- **Fit:** medium\n\nThey agree on: one number, one delta, one hint of color.\n";

    #[test]
    fn notes_are_read_into_entries_and_what_they_agree_on() {
        let (notes, agree) = parse_notes(NOTES_MD);
        assert_eq!(notes.len(), 2);
        let a = &notes[0];
        assert_eq!((a.product.as_str(), a.screen.as_str()), ("Acme", "Orders overview"));
        assert_eq!(a.url.as_deref(), Some("https://www.example.com/orders"));
        assert_eq!(a.host.as_deref(), Some("example.com"));
        assert_eq!(a.source.as_deref(), Some("Gallery One"));
        assert_eq!(a.date.as_deref(), Some("2026-10-02"));
        assert_eq!(a.picture.as_deref(), Some("acme-1440.png"), "the file's name, not its folder");
        assert_eq!(a.take, "label above value, one accent; tabular figures", "a wrapped line joins its field");
        assert_eq!((a.leave.as_str(), a.fit.as_str()), ("the mascot", "high — same density"));
        let b = &notes[1];
        assert_eq!((b.product.as_str(), b.screen.as_str()), ("Beta", "Revenue"), "a plain hyphen splits too");
        assert_eq!(b.url.as_deref(), Some("https://beta.example.org/rev?x=1"));
        assert_eq!(b.take, "sparkline under the number", "bold keys are keys");
        assert_eq!(agree, "They agree on: one number, one delta, one hint of color.");
    }

    #[test]
    fn a_closing_heading_with_no_fields_is_the_summary_and_odd_notes_do_not_break_it() {
        let (notes, agree) = parse_notes("## One\n- Take: x\n\n## What they agree on\nDense rows, quiet borders.\n");
        assert_eq!((notes.len(), agree.as_str()), (1, "Dense rows, quiet borders."));
        assert_eq!(parse_notes(""), (vec![], String::new()));
        assert_eq!(parse_notes("no headings, just words\n- Take: lost").0.len(), 0);
        let (notes, _) = parse_notes("## Only a title\n- Source: not a link at all\n");
        assert_eq!((notes[0].url.clone(), notes[0].date.clone(), notes[0].picture.clone()), (None, None, None));
    }

    #[test]
    fn only_web_addresses_are_links_and_logins_are_not() {
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "ftp://x.example/a", "https://user:pw@example.com/", "https://", "https:///x"] {
            let (notes, _) = parse_notes(&format!("## P\n- Source: {bad}, 2026-01-01\n"));
            assert_eq!(notes[0].url, None, "{bad}");
        }
        assert_eq!(host_of("http://Sub.Example.COM:8080/x"), Some("sub.example.com".into()));
        assert_eq!(host_of("https://www.example.com"), Some("example.com".into()));
        assert_eq!(date_in("on 12026-01-011 or 2026-02-03."), Some("2026-02-03".into()));
    }

    #[test]
    fn long_fields_and_many_entries_are_bounded() {
        let long = "x".repeat(FIELD_CHARS * 3);
        let mut md = String::new();
        for i in 0..ENTRIES + 10 {
            md.push_str(&format!("## P{i}\n- Take: {long}\n"));
        }
        let (notes, _) = parse_notes(&md);
        assert_eq!(notes.len(), ENTRIES);
        assert!(notes[0].take.chars().count() <= FIELD_CHARS + 1 && notes[0].take.ends_with('…'));
    }

    #[test]
    fn sources_are_read_a_line_each() {
        let md = "# Sources\n\n- Gallery One — https://one.example.com — good for: kpi-tile, table — fits: dense tools — access: public\n- Own library — good for: forms — access: mcp:ui\n- not a source line\n- ** — https://x.example.com\n";
        let s = parse_sources(md);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].name.as_str(), s[0].url.as_deref(), s[0].good_for.as_str(), s[0].fits.as_str(), s[0].access.as_str()), ("Gallery One", Some("https://one.example.com"), "kpi-tile, table", "dense tools", "public"));
        assert_eq!((s[1].url.clone(), s[1].access.as_str()), (None, "mcp:ui"));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-refgallery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(4, 4, image::Rgb([20, 90, 250])).save(path).unwrap();
    }

    #[test]
    fn a_folder_is_read_by_pattern_with_its_pictures_and_never_beyond_it() {
        use std::os::unix::fs::symlink;
        let root = scratch("read");
        let refs = root.join("refs");
        png(&refs.join("kpi-tile/acme-1440.png"));
        png(&refs.join("kpi-tile/beta-390.png"));
        std::fs::write(refs.join("kpi-tile/notes.md"), NOTES_MD).unwrap();
        std::fs::write(refs.join("kpi-tile/readme.txt"), "not a picture").unwrap();
        png(&refs.join("data-table/a.png"));
        std::fs::create_dir_all(refs.join(".hidden")).unwrap();
        // Out of the folder: a link to a folder, a link to a picture outside, a link to a secret.
        let outside = root.join("outside");
        png(&outside.join("secret.png"));
        std::fs::write(outside.join("notes.md"), "## Leak\n- Take: secret").unwrap();
        symlink(&outside, refs.join("linked-pattern")).unwrap();
        symlink(outside.join("secret.png"), refs.join("kpi-tile/escape.png")).unwrap();
        symlink(outside.join("notes.md"), refs.join("data-table/notes.md")).unwrap();
        std::fs::write(refs.join("sources.md"), "- Gallery — https://g.example.com — good for: kpi-tile\n").unwrap();

        let places = [(refs.clone(), false)];
        let names: Vec<String> = pattern_dirs(&places[0].0).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["data-table", "kpi-tile"], "a link to a folder and a hidden one are no pattern");
        let kpi = pattern("kpi-tile", &refs.join("kpi-tile"), false);
        assert_eq!(kpi.notes.len(), 2);
        assert_eq!(kpi.pictures.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["acme-1440.png", "beta-390.png"], "no link out, no text file");
        assert_eq!(kpi.pictures_total, 3, "the link is counted by its name, never given");
        assert!(kpi.updated > 0 && kpi.raw.starts_with("# kpi-tile"));
        let table = pattern("data-table", &refs.join("data-table"), false);
        assert!(table.notes.is_empty() && table.raw.is_empty(), "a notes file that is a link is not read");
        assert_eq!(table.pictures.len(), 1);
        assert_eq!(sources_in(&refs).unwrap()[0].name, "Gallery");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_squads_references_are_listed_filtered_and_paged_by_name() {
        let home = scratch("squad");
        let refs = crate::squad_home::path(&home, "acme").join("design").join(DIR);
        for name in ["Filter-bar", "kpi-tile", "data-table", "toast"] {
            png(&refs.join(name).join("a.png"));
        }
        std::fs::write(refs.join("kpi-tile/notes.md"), NOTES_MD).unwrap();
        let all = read(&home, "acme", None, None, "", 0);
        assert_eq!(all.total, 4);
        assert_eq!(all.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["data-table", "Filter-bar", "kpi-tile", "toast"], "by name, any case");
        assert!(all.patterns.iter().all(|p| !p.shared) && all.folder.ends_with("design/references"));
        let by_name = read(&home, "acme", None, None, "  FILTER ", 0);
        assert_eq!((by_name.total, by_name.patterns[0].name.as_str()), (1, "Filter-bar"));
        let by_notes = read(&home, "acme", None, None, "sparkline", 0);
        assert_eq!((by_notes.total, by_notes.patterns[0].name.as_str()), (1, "kpi-tile"), "the notes are searched too");
        let paged = read(&home, "acme", None, None, "", 3);
        assert_eq!((paged.total, paged.patterns.len()), (4, 1));
        assert_eq!(read(&home, "acme", None, None, "", 99).patterns.len(), 0);
        assert_eq!(read(&home, "../acme", None, None, "", 0), Found::default(), "a squad name is checked");
        assert_eq!(read(&home, "nobody", None, None, "", 0).total, 0);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_big_folder_costs_one_page_and_a_big_notes_file_is_cut() {
        let root = scratch("limits");
        let refs = root.join("refs");
        for i in 0..PAGE + 25 {
            std::fs::create_dir_all(refs.join(format!("p{i:03}"))).unwrap();
        }
        for i in 0..PICTURES + 7 {
            png(&refs.join(format!("p000/s{i:03}.png")));
        }
        std::fs::write(refs.join("p000/notes.md"), format!("## Big\n- Take: {}\n", "é".repeat(NOTES_MAX as usize))).unwrap();
        let raw = read_text(&refs.join("p000/notes.md"), NOTES_MAX);
        assert!(raw.len() as u64 <= NOTES_MAX && !raw.contains('\u{FFFD}'), "cut at a character");
        let names = pattern_dirs(&refs);
        assert_eq!(names.len(), PAGE + 25);
        let first = pattern("p000", &refs.join("p000"), false);
        assert_eq!((first.pictures.len(), first.pictures_total as usize), (PICTURES, PICTURES + 7));
        assert!(first.notes[0].take.chars().count() <= FIELD_CHARS + 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A project's `design/` folder in a scratch dir, with its references folder.
    fn design_with_refs(root: &Path) -> PathBuf {
        let design = root.join("design");
        std::fs::create_dir_all(design.join("references")).unwrap();
        design
    }

    #[test]
    fn a_picture_an_entry_links_inside_design_is_counted_and_belongs_to_its_entry() {
        let root = scratch("linked");
        let design = design_with_refs(&root);
        let refs = design.join("references");
        png(&design.join("directions/t1-references/data-table/mercury.png"));
        png(&design.join("directions/t1-references/data-table/linear.png"));
        png(&refs.join("data-table/own.png"));
        std::fs::write(
            refs.join("data-table/notes.md"),
            "# data-table\n\n## Mercury — Transactions\n- Source: https://example.com/m (Gallery), 2026-11-01\n- Image: `../../directions/t1-references/data-table/mercury.png`\n- Take: quiet rows\n\n## Linear — Issues\n- Image: ../../directions/t1-references/data-table/linear.png\n- Take: dense\n\n## Own — Table\n- Take: with a picture in the folder\n",
        )
        .unwrap();
        let table = pattern("data-table", &refs.join("data-table"), false);
        assert_eq!(table.pictures_total, 3, "the folder's and the two linked");
        let by: Vec<(&str, Option<u32>)> = table.pictures.iter().map(|p| (p.name.as_str(), p.note)).collect();
        assert_eq!(by, [("own.png", None), ("mercury.png", Some(0)), ("linear.png", Some(1))]);
        assert_eq!(table.notes[0].picture.as_deref(), Some("mercury.png"));
        let mercury = &table.pictures[1];
        assert_eq!(Path::new(&mercury.path), design.join("directions/t1-references/data-table/mercury.png"), "written out, no `..`");
        assert!(visuals::check(Path::new(&mercury.path), std::slice::from_ref(&design)).is_ok(), "read_image's check passes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_link_out_of_design_through_dots_a_link_or_an_address_is_never_a_picture() {
        use std::os::unix::fs::symlink;
        let root = scratch("escape");
        let design = design_with_refs(&root);
        let refs = design.join("references");
        png(&root.join("outside.png"));
        png(&root.join("elsewhere/secret.png"));
        png(&design.join("fine.png"));
        symlink(root.join("outside.png"), design.join("sneaky.png")).unwrap();
        symlink(root.join("elsewhere"), design.join("door")).unwrap();
        symlink(design.join("fine.png"), design.join("inner-link.png")).unwrap();
        std::fs::create_dir_all(refs.join("p")).unwrap();
        let notes: String = [
            "../../../outside.png",
            "../../../elsewhere/secret.png",
            "../../sneaky.png",
            "../../door/secret.png",
            "/etc/passwd.png",
            "https://example.com/a.png",
            "../../../design/../outside.png",
            "../../missing.png",
            "../../fine.png/../../outside.png",
            "../../inner-link.png",
        ]
        .iter()
        .enumerate()
        .map(|(i, rel)| format!("## E{i}\n- Image: `{rel}`\n- Take: x\n\n"))
        .collect();
        std::fs::write(refs.join("p/notes.md"), notes).unwrap();
        let found = pattern("p", &refs.join("p"), false);
        let names: Vec<&str> = found.pictures.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["inner-link.png"], "only a link that stays inside design/ — {names:?}");
        assert_eq!(found.pictures_total, 1);
        // A pattern whose folder isn't in `design/references` gets no linked pictures at all.
        let loose = root.join("refs-only/p");
        png(&root.join("refs-only/x.png"));
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("notes.md"), "## A\n- Image: ../x.png\n- Take: x\n").unwrap();
        assert_eq!(pattern("p", &loose, false).pictures_total, 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A ticket in `worktree` (squad `acme`), at work or in review; its id.
    fn ticket_in(home: &Path, title: &str, worktree: &Path, review: bool) -> u64 {
        use crate::protocol::{Ticket, TicketKind, TicketStatus};
        let journal = crate::journal::Journal::for_squad(home, "acme").unwrap();
        let ticket = Ticket {
            id: 0,
            title: title.into(),
            body: "".into(),
            status: if review { TicketStatus::Review } else { TicketStatus::InProgress },
            kind: TicketKind::Task,
            owner: Some("designer".into()),
            assignee: Some("designer".into()),
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: None,
            model: None,
            flow: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: vec![],
            images: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
        };
        let id = journal.create_ticket(&ticket).unwrap();
        journal.set_ticket_work(id, &crate::journal::TicketWork { worktree: Some(worktree.to_owned()), ..Default::default() }).unwrap();
        u64::from(id)
    }

    fn touch(path: &Path, secs_ago: u64) {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn the_source_defaults_to_the_worktree_with_the_latest_references_work() {
        let home = scratch("places");
        let squad = crate::squad_home::path(&home, "acme");
        let main = squad.join("design/references");
        png(&main.join("kpi/a.png"));
        touch(&main.join("kpi/a.png"), 5000);
        let (old, new, bare, same) = (home.join("wt/old"), home.join("wt/new"), home.join("wt/bare"), home.join("wt/same"));
        png(&old.join("design/references/toast/a.png"));
        touch(&old.join("design/references/toast/a.png"), 3000);
        png(&new.join("design/references/data-table/b.png"));
        std::fs::write(new.join("design/references/data-table/notes.md"), "## A — B\n- Take: x\n").unwrap();
        std::fs::create_dir_all(bare.join("design")).unwrap();
        png(&same.join("design/references/kpi/a.png"));
        let bare_id = ticket_in(&home, "no references", &bare, false);
        let new_id = ticket_in(&home, "references", &new, false);
        let old_id = ticket_in(&home, "old", &old, true);
        ticket_in(&home, "a copy of main", &same, false);
        let all = places(&home, "acme", None);
        let ids: Vec<Option<u64>> = all.iter().map(|p| p.info.ticket).collect();
        assert_eq!(ids, [None, Some(new_id), Some(old_id)], "main, then by the latest work; none for no references or only a copy of main's");
        assert!(all[2].info.review && all[1].info.patterns == 1 && !all[1].info.review);
        let auto = read(&home, "acme", None, None, "", 0);
        assert_eq!(auto.at, new_id);
        assert_eq!(auto.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["data-table"]);
        assert_eq!(auto.places.len(), 3);
        assert!(auto.folder.ends_with("new/design/references"));
        let main_read = read(&home, "acme", None, Some(0), "", 0);
        assert_eq!((main_read.at, main_read.patterns[0].name.as_str()), (0, "kpi"));
        let old_read = read(&home, "acme", None, Some(old_id), "", 0);
        assert_eq!((old_read.at, old_read.patterns[0].name.as_str()), (old_id, "toast"));
        assert_eq!(read(&home, "acme", None, Some(bare_id), "", 0).at, new_id, "no references there: the default instead");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn without_a_project_folder_of_its_own_the_only_worktree_with_references_is_the_source() {
        // The designer's layout: notes in design/references/<pattern>/, pictures in design/directions/<ticket>-references/<pattern>/.
        let home = scratch("t2562");
        let wt = home.join("wt/t2562");
        let refs = wt.join("design/references");
        for name in ["data-table", "kpi-cards"] {
            png(&wt.join(format!("design/directions/t2562-references/{name}/M-one.png")));
            std::fs::create_dir_all(refs.join(name)).unwrap();
            std::fs::write(refs.join(name).join("notes.md"), format!("## One — {name}\n- Source: https://example.com (Gallery), 2026-11-10\n- Image: `../../directions/t2562-references/{name}/M-one.png`\n- Take: a\n- Leave: b\n- Fit: high\n")).unwrap();
        }
        let id = ticket_in(&home, "references", &wt, false);
        let found = read(&home, "acme", None, None, "", 0);
        assert_eq!(found.at, id);
        assert_eq!(found.patterns.len(), 2);
        assert!(found.patterns.iter().all(|p| p.pictures_total == 1 && p.pictures[0].note == Some(0)), "{:?}", found.patterns);
        std::fs::remove_dir_all(home).unwrap();
    }
}
