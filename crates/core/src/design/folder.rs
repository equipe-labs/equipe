//! Where a squad's design systems are and what they hold. The squad's own is
//! `design/` in its folder (`~/.equipe/squads/<squad>/home/design`), which
//! the designer's tasks land in when the person accepts them; a project with
//! a look of its own keeps one in its repository (`<repo>/design`); a ticket
//! in review whose worktree changed one shows it as a draft.
//!
//! A folder counts when it holds `tokens.json` or `guidelines.md`. Files are
//! read only inside it — links resolved, nothing bigger than a limit — and
//! its pictures are the images in `assets/` and `components/` that pass
//! [`visuals::check`], the same check `read_image` makes when a screen asks.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use equipe_protocol::design::{Asset, AssetKind, DesignSystem, DraftOf};
use equipe_protocol::{TicketKind, TicketStatus};

use super::tokens::{self, Problem};
use crate::i18n::Lang;
use crate::visuals;

/// The folder's name, in the squad's folder and in a project.
pub const DIR: &str = "design";
pub const TOKENS: &str = "tokens.json";
pub const GUIDELINES: &str = "guidelines.md";
const PICTURE_DIRS: [&str; 2] = ["assets", "components"];
/// The biggest `tokens.json` read.
pub const TOKENS_MAX: u64 = 1 << 20;
/// How much of `guidelines.md` is sent: past it, cut with a word.
pub const GUIDELINES_MAX: usize = 256 << 10;
/// How many pictures, and how many files looked at to find them.
pub const ASSETS_MAX: usize = 240;
const FILES_MAX: usize = 2000;
const DEPTH_MAX: usize = 5;

/// One place a design system may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub project: Option<String>,
    pub draft: Option<DraftOf>,
    pub dir: PathBuf,
}

/// Whether `dir` is a design system: a folder (not a link to one) with
/// `tokens.json` or `guidelines.md` in it.
pub fn is_design_dir(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) && [TOKENS, GUIDELINES].iter().any(|f| dir.join(f).is_file())
}

/// Where `squad`'s design systems are, or may be: its own (always, there or
/// not), each project's that has one, each draft a ticket in review has.
pub fn places(home: &Path, squad: &str) -> Vec<Place> {
    let mut out = vec![Place { project: None, draft: None, dir: crate::squad_home::path(home, squad).join(DIR) }];
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    if !crate::squads::valid_name(squad) || !db.exists() {
        return out;
    }
    let Ok(journal) = crate::journal::Journal::open(&db) else { return out };
    for project in journal.projects().unwrap_or_default() {
        let dir = project.join(DIR);
        if is_design_dir(&dir) {
            out.push(Place { project: Some(crate::board::project_name(&project)), draft: None, dir });
        }
    }
    for ticket in journal.tickets().unwrap_or_default() {
        let Some(worktree) = ticket.worktree.as_ref().filter(|_| ticket.status == TicketStatus::Review) else { continue };
        let dir = worktree.join(DIR);
        if is_design_dir(&dir) {
            let project = if ticket.kind == TicketKind::Code { ticket.project.as_deref().map(crate::board::project_name) } else { None };
            out.push(Place { project, draft: Some(DraftOf { ticket: ticket.id, title: ticket.title.clone() }), dir });
        }
    }
    out
}

/// The files under `dir`'s picture folders, and its two files: path and
/// what the disk says of each. Links are listed, not followed.
fn files(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut out = Vec::new();
    for name in [TOKENS, GUIDELINES] {
        if let Ok(meta) = std::fs::symlink_metadata(dir.join(name)) {
            out.push((dir.join(name), meta));
        }
    }
    let mut stack: Vec<(PathBuf, usize)> = PICTURE_DIRS.iter().map(|d| (dir.join(d), 0)).collect();
    while let Some((at, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else { continue };
        let mut entries: Vec<_> = entries.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if out.len() >= FILES_MAX {
                return out;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                if depth < DEPTH_MAX {
                    stack.push((entry.path(), depth + 1));
                }
            } else {
                out.push((entry.path(), meta));
            }
        }
    }
    out
}

/// A number that changes whenever any of `places` does: one appears or goes,
/// a file in one is added, removed, grows or is saved again. Only looks at
/// the disk's metadata, so it can be asked often.
pub fn stamp(places: &[Place]) -> u64 {
    let mut h = DefaultHasher::new();
    for place in places {
        place.dir.hash(&mut h);
        place.project.hash(&mut h);
        place.draft.as_ref().map(|d| (d.ticket, &d.title)).hash(&mut h);
        is_design_dir(&place.dir).hash(&mut h);
        for (path, meta) in files(&place.dir) {
            path.hash(&mut h);
            meta.len().hash(&mut h);
            meta.modified().ok().hash(&mut h);
            // A link's target counts too: retargeted, it shows something else.
            std::fs::metadata(&path).ok().map(|m| (m.len(), m.modified().ok())).hash(&mut h);
        }
    }
    h.finish()
}

/// A file inside `dir`, no bigger than `max` — a link only when it stays in `dir`.
fn inside(dir: &Path, name: &str, max: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let root = std::fs::canonicalize(dir).ok()?;
    let real = std::fs::canonicalize(dir.join(name)).ok()?;
    let meta = std::fs::metadata(&real).ok()?;
    if !real.starts_with(&root) || !meta.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&real).ok()?.take(max + 1).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// What a picture is, by where it is and what it is called.
fn kind_of(name: &str) -> AssetKind {
    let lower = name.to_ascii_lowercase();
    let mut parts = lower.split('/');
    let top = parts.next().unwrap_or_default();
    let sub = if top == "assets" { parts.next().unwrap_or_default() } else { "" };
    let file = lower.rsplit('/').next().unwrap_or_default();
    if top == "components" {
        AssetKind::Component
    } else if sub.starts_with("logo") || sub == "brand" || file.contains("logo") || file.contains("wordmark") || file.starts_with("mark") {
        AssetKind::Logo
    } else if sub.starts_with("icon") || file.contains("icon") {
        AssetKind::Icon
    } else if sub.starts_with("illustration") || sub.starts_with("illus") || sub == "art" {
        AssetKind::Illustration
    } else {
        AssetKind::Other
    }
}

/// The pictures in `dir`: images in `assets/` and `components/` that are
/// what they say and stay inside it, logos first.
pub fn assets(dir: &Path) -> Vec<Asset> {
    let Ok(root) = std::fs::canonicalize(dir) else { return Vec::new() };
    let mut out: Vec<Asset> = Vec::new();
    for (path, _) in files(dir) {
        if out.len() >= ASSETS_MAX {
            break;
        }
        let Ok(rel) = path.strip_prefix(dir) else { continue };
        let name = rel.to_string_lossy().replace('\\', "/");
        if !PICTURE_DIRS.iter().any(|d| name.starts_with(&format!("{d}/"))) || !visuals::is_image_name(&name) {
            continue;
        }
        let Ok(real) = visuals::check(&path, std::slice::from_ref(&root)) else { continue };
        let size = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(0);
        out.push(Asset { path: real.to_string_lossy().into_owned(), kind: kind_of(&name), name, size });
    }
    out.sort_by_key(|a| (a.kind != AssetKind::Logo, a.kind == AssetKind::Component, a.name.clone()));
    out
}

/// A problem with `tokens.json`, in the person's language.
pub fn say(problem: &Problem, lang: Lang) -> String {
    let words = crate::i18n::core(lang);
    match problem {
        Problem::NotJson { line, column } => words.fill("design.problem_not_json", &[("line", line.to_string()), ("column", column.to_string())]),
        Problem::NotGroups => words.get("design.problem_not_groups").to_owned(),
        Problem::Stray { path } => words.fill("design.problem_stray", &[("path", path.clone())]),
        Problem::Unresolved { token, reference } => words.fill("design.problem_unresolved", &[("token", token.clone()), ("reference", reference.clone())]),
        Problem::Cycle { token } => words.fill("design.problem_cycle", &[("token", token.clone())]),
        Problem::Invalid { token, kind, value } => words.fill("design.problem_invalid", &[("token", token.clone()), ("kind", kind.clone()), ("value", value.clone())]),
        Problem::TooMany => words.fill("design.problem_too_many", &[("limit", tokens::MAX_TOKENS.to_string())]),
    }
}

/// What `place` holds, read.
pub fn read(place: &Place, lang: Lang) -> DesignSystem {
    let words = crate::i18n::core(lang);
    let mut system = DesignSystem { project: place.project.clone(), draft: place.draft.clone(), dir: place.dir.to_string_lossy().into_owned(), ..DesignSystem::default() };
    if !is_design_dir(&place.dir) {
        return system;
    }
    match inside(&place.dir, TOKENS, TOKENS_MAX) {
        Some(bytes) if bytes.len() as u64 > TOKENS_MAX => system.problems.push(words.fill("design.problem_too_big", &[("file", TOKENS.into()), ("limit", format!("{} MB", TOKENS_MAX >> 20))])),
        Some(bytes) => {
            let parsed = tokens::parse(&String::from_utf8_lossy(&bytes));
            system.modes = parsed.modes;
            system.tokens = parsed.tokens;
            system.problems.extend(parsed.problems.iter().map(|p| say(p, lang)));
        }
        None => {}
    }
    if let Some(bytes) = inside(&place.dir, GUIDELINES, GUIDELINES_MAX as u64) {
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() > GUIDELINES_MAX {
            let mut at = GUIDELINES_MAX;
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            text.truncate(at);
            text.push_str(&format!("\n\n*{}*\n", words.get("design.guidelines_cut")));
        }
        system.guidelines = Some(text);
    }
    system.assets = assets(&place.dir);
    system.stamp = stamp(std::slice::from_ref(place));
    system
}

/// Whether two systems hold the same: a draft that changed nothing isn't one.
fn same_content(a: &DesignSystem, b: &DesignSystem) -> bool {
    let pictures = |s: &DesignSystem| s.assets.iter().map(|a| (a.name.clone(), a.size)).collect::<Vec<_>>();
    a.tokens == b.tokens && a.guidelines == b.guidelines && a.problems == b.problems && pictures(a) == pictures(b)
}

/// `squad`'s design systems as screens show them, and where its own would be.
pub fn catalog(home: &Path, squad: &str, lang: Lang) -> (Vec<DesignSystem>, String) {
    catalog_of(&places(home, squad), lang)
}

/// The same, of places already found.
pub fn catalog_of(places: &[Place], lang: Lang) -> (Vec<DesignSystem>, String) {
    let folder = places.first().map(|p| p.dir.to_string_lossy().into_owned()).unwrap_or_default();
    let read: Vec<DesignSystem> = places.iter().filter(|p| is_design_dir(&p.dir)).map(|p| read(p, lang)).collect();
    let systems = read
        .iter()
        .filter(|s| match &s.draft {
            // A ticket's worktree holds a copy of the accepted folder: a draft only where it differs.
            Some(_) => !read.iter().any(|a| a.draft.is_none() && a.project == s.project && same_content(a, s)),
            None => true,
        })
        .cloned()
        .collect();
    (systems, folder)
}

/// The pictures `squad`'s design systems show, and where they may be — for
/// `read_image`.
pub fn shown(home: &Path, squad: &str) -> (Vec<String>, Vec<PathBuf>) {
    let mut paths = Vec::new();
    let mut roots = Vec::new();
    for place in places(home, squad).into_iter().filter(|p| is_design_dir(&p.dir)) {
        paths.extend(assets(&place.dir).into_iter().map(|a| a.path));
        roots.push(place.dir);
    }
    (paths, roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-design-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 250, 255])).save(path).unwrap();
    }

    const TOKENS_JSON: &str = r##"{ "color": { "$type": "color", "bg": { "$value": "#F6F5F1" } } }"##;

    #[test]
    fn a_folder_counts_with_tokens_or_guidelines_and_only_pictures_inside_it_show() {
        let root = dir("assets");
        let design = root.join("design");
        let outside = root.join("secret");
        std::fs::create_dir_all(&design).unwrap();
        assert!(!is_design_dir(&design), "empty");
        std::fs::write(design.join(GUIDELINES), "# Brand").unwrap();
        assert!(is_design_dir(&design));
        png(&design.join("assets/logo/mark.png"));
        png(&design.join("assets/icons/bell.png"));
        png(&design.join("components/button.png"));
        png(&outside.join("private.png"));
        std::fs::write(design.join("assets/notes.txt"), "not a picture").unwrap();
        std::fs::write(design.join("assets/fake.png"), "not a picture either").unwrap();
        png(&design.join("stray.png"));
        // Links: one out of the folder, one to a folder outside, one that stays in.
        symlink(outside.join("private.png"), design.join("assets/out.png")).unwrap();
        symlink(&outside, design.join("assets/up")).unwrap();
        symlink(design.join("assets/logo/mark.png"), design.join("assets/same.png")).unwrap();
        let found = assets(&design);
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["assets/logo/mark.png", "assets/fake.png", "assets/icons/bell.png", "assets/same.png", "components/button.png"], "logos first; nothing outside, nothing loose at the top");
        assert!(found.iter().all(|a| a.path.starts_with(design.to_str().unwrap())), "real paths, all inside");
        assert_eq!(found[0].kind, AssetKind::Logo);
        assert_eq!(found.iter().find(|a| a.name.ends_with("bell.png")).unwrap().kind, AssetKind::Icon);
        assert_eq!(found.last().unwrap().kind, AssetKind::Component);
        // Named a picture, it is listed; read, its bytes are refused.
        let fake = found.iter().find(|a| a.name.ends_with("fake.png")).unwrap();
        assert!(visuals::read(Path::new(&fake.path), std::slice::from_ref(&design), None).is_err());
        // A link to the folder itself isn't one.
        symlink(&design, root.join("linked")).unwrap();
        assert!(!is_design_dir(&root.join("linked")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tokens_and_guidelines_are_read_from_inside_only_and_cut_when_long() {
        let root = dir("read");
        let design = root.join("design");
        std::fs::create_dir_all(&design).unwrap();
        std::fs::write(root.join("elsewhere.json"), TOKENS_JSON).unwrap();
        symlink(root.join("elsewhere.json"), design.join(TOKENS)).unwrap();
        std::fs::write(design.join(GUIDELINES), "가".repeat(GUIDELINES_MAX)).unwrap();
        let place = Place { project: None, draft: None, dir: design.clone() };
        let system = read(&place, Lang::En);
        assert!(system.tokens.is_empty(), "a tokens.json that is a link out of the folder isn't read");
        let text = system.guidelines.unwrap();
        assert!(text.len() < GUIDELINES_MAX + 200 && text.contains("guidelines.md"), "cut at a character, with a word");
        std::fs::remove_file(design.join(TOKENS)).unwrap();
        std::fs::write(design.join(TOKENS), "{ not json").unwrap();
        let system = read(&place, Lang::Ko);
        assert_eq!(system.problems.len(), 1);
        assert!(system.problems[0].contains("JSON") && system.problems[0].contains('1'), "{:?}", system.problems);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_stamp_moves_when_anything_in_the_folder_does() {
        let root = dir("stamp");
        let design = root.join("design");
        let place = [Place { project: None, draft: None, dir: design.clone() }];
        let missing = stamp(&place);
        std::fs::create_dir_all(&design).unwrap();
        std::fs::write(design.join(TOKENS), TOKENS_JSON).unwrap();
        let first = stamp(&place);
        assert_ne!(missing, first);
        assert_eq!(first, stamp(&place), "steady while nothing changes");
        png(&design.join("assets/logo.png"));
        let with_logo = stamp(&place);
        assert_ne!(first, with_logo);
        // Linux stamps a file's time from a coarse clock (a few ms): a save
        // within it reads as the same time. A person's edits are further apart.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(design.join(TOKENS), r##"{ "color": { "$type": "color", "bg": { "$value": "#0A0A0C" } } }"##).unwrap();
        assert_ne!(with_logo, stamp(&place), "the same size, saved again");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_squad_shows_its_own_its_projects_and_drafts_that_differ() {
        let home = dir("catalog");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let folder = crate::squad_home::path(&home, "acme").join(DIR);
        let (systems, at) = catalog(&home, "acme", Lang::En);
        assert!(systems.is_empty());
        assert_eq!(at, folder.to_string_lossy(), "where the squad's would be");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(TOKENS), TOKENS_JSON).unwrap();
        png(&folder.join("assets/logo/mark.png"));
        // A project with its own.
        let app = home.join("code/app");
        std::fs::create_dir_all(app.join(DIR)).unwrap();
        std::fs::write(app.join(DIR).join(GUIDELINES), "# App").unwrap();
        journal.add_project(&app).unwrap();
        // Two tickets in review: one changed the tokens, one only has the folder's copy.
        let worktree = |n: u32, tokens: &str| {
            let w = home.join(format!("worktrees/t{n}"));
            std::fs::create_dir_all(w.join(DIR)).unwrap();
            std::fs::write(w.join(DIR).join(TOKENS), tokens).unwrap();
            png(&w.join(DIR).join("assets/logo/mark.png"));
            w
        };
        let changed = worktree(144, r##"{ "color": { "$type": "color", "bg": { "$value": "#0A0A0C" } } }"##);
        let copy = worktree(150, TOKENS_JSON);
        for (n, w) in [(144, changed), (150, copy)] {
            let ticket = equipe_protocol::Ticket {
                id: 0, title: format!("T{n}"), body: String::new(), status: TicketStatus::Review, kind: TicketKind::Task, owner: Some("designer".into()), assignee: None,
                project: None, depends_on: vec![], branch: None, worktree: None, merged: None, off_main: None, design_qa: None, created_by: "pm".into(), note: None,
                asks: vec![], remaining: vec![], images: vec![], priority: Default::default(), rank: 0, updated_at: None,
            };
            let id = journal.create_ticket(&ticket).unwrap();
            journal.set_ticket_work(id, &crate::journal::TicketWork { worktree: Some(w), ..Default::default() }).unwrap();
        }
        let tickets = journal.tickets().unwrap();
        let (systems, _) = catalog(&home, "acme", Lang::En);
        let keys: Vec<String> = systems.iter().map(|s| s.key()).collect();
        assert_eq!(keys, ["squad".to_owned(), "project:app".into(), format!("squad#T-{}", tickets[0].id)]);
        assert_eq!(systems[0].tokens[0].value, "#F6F5F1");
        assert_eq!(systems[2].tokens[0].value, "#0A0A0C");
        // What read_image may give: the pictures of each, in the folders only.
        let (paths, roots) = shown(&home, "acme");
        assert_eq!(roots.len(), 4, "{roots:?}");
        assert!(paths.iter().any(|p| p.ends_with("home/design/assets/logo/mark.png")));
        std::fs::remove_dir_all(home).unwrap();
    }
}
