//! The references a designer keeps (`skills/design-references`), read for a
//! screen: `design/references/<pattern>/` — pictures and a `notes.md` of
//! entries (`## <product> — <screen>` with Source / Take / Leave / Fit lines)
//! — and `design/references/sources.md`, in a project's design folder and, for
//! the patterns the project has none of, the squad's own.
//!
//! Read only, and only inside the references folder: a pattern is a real
//! folder (never a link) right in it, a picture one `read_image` may give
//! ([`visuals::check`]: no `..`, no link out of the folder, an image of a kind
//! taken), a notes file a regular file. Everything is bounded — patterns
//! looked at, the page sent, pictures per pattern, how much of a notes file or
//! a sources file is read, and how long a line may be — so a folder that grew
//! big costs a screen one page. Pictures are served through `read_image`
//! (`visuals`' references gate), by the paths named here.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use equipe_protocol::design::{RefNote, RefPattern, RefPicture, RefSource};

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
}

/// The references folders to read: the one asked for first, then the squad's
/// own when it is a project that is asked about.
fn folders(home: &Path, squad: &str, project: Option<&str>) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    if project.is_some()
        && let Some(design) = super::folder::accepted_dir(home, squad, project)
    {
        out.push((design.join(DIR), false));
    }
    let own = crate::squad_home::path(home, squad).join(super::folder::DIR).join(DIR);
    out.push((own, project.is_some()));
    out
}

/// Whether `path` is a real folder: not a link to one.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// The references a screen asks for: `project`'s (the squad's own when
/// none), the squad's for the patterns the project has none of, the patterns
/// holding `query` (any case) in their name or notes, and the page at `offset`.
pub fn read(home: &Path, squad: &str, project: Option<&str>, query: &str, offset: usize) -> Found {
    if !crate::squads::valid_name(squad) {
        return Found::default();
    }
    read_in(folders(home, squad, project), query, offset)
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

fn pattern(name: &str, dir: &Path, shared: bool) -> RefPattern {
    let raw = read_text(&dir.join(NOTES), NOTES_MAX);
    let (notes, agree) = parse_notes(&raw);
    let mut updated = std::fs::symlink_metadata(dir.join(NOTES)).ok().filter(|m| m.is_file()).map_or(0, |m| secs(&m));
    let root = dir.parent().and_then(|p| std::fs::canonicalize(p).ok());
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
    let pictures_total = files.len() as u32;
    let mut pictures = Vec::new();
    for (file, link_meta) in files {
        if pictures.len() >= PICTURES {
            updated = updated.max(secs(&link_meta));
            continue;
        }
        let path = dir.join(&file);
        let ok = root.as_ref().is_some_and(|root| visuals::check(&path, std::slice::from_ref(root)).is_ok());
        if !ok {
            continue;
        }
        let meta = std::fs::metadata(&path).unwrap_or(link_meta);
        updated = updated.max(secs(&meta));
        pictures.push(RefPicture { path: path.to_string_lossy().into_owned(), name: file, size: meta.len(), modified: secs(&meta) });
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
        _ => return None,
    };
    Some((field, value))
}

#[derive(Default)]
struct Draft {
    title: String,
    fields: [String; 4],
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
        if let Some((field, value)) = field_line(line) {
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
        }
    }
    (notes, agree)
}

fn note(d: &Draft) -> RefNote {
    let [source, take, leave, fit] = &d.fields;
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
        picture: backticked(source).and_then(|p| p.rsplit('/').next().map(str::to_owned)).filter(|p| !p.is_empty()),
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
        let all = read(&home, "acme", None, "", 0);
        assert_eq!(all.total, 4);
        assert_eq!(all.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["data-table", "Filter-bar", "kpi-tile", "toast"], "by name, any case");
        assert!(all.patterns.iter().all(|p| !p.shared) && all.folder.ends_with("design/references"));
        let by_name = read(&home, "acme", None, "  FILTER ", 0);
        assert_eq!((by_name.total, by_name.patterns[0].name.as_str()), (1, "Filter-bar"));
        let by_notes = read(&home, "acme", None, "sparkline", 0);
        assert_eq!((by_notes.total, by_notes.patterns[0].name.as_str()), (1, "kpi-tile"), "the notes are searched too");
        let paged = read(&home, "acme", None, "", 3);
        assert_eq!((paged.total, paged.patterns.len()), (4, 1));
        assert_eq!(read(&home, "acme", None, "", 99).patterns.len(), 0);
        assert_eq!(read(&home, "../acme", None, "", 0), Found::default(), "a squad name is checked");
        assert_eq!(read(&home, "nobody", None, "", 0).total, 0);
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
}
