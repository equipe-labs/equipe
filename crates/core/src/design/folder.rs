//! Where a squad's design systems are and what they hold. The squad's own is
//! `design/` in its folder (`~/.equipe/squads/<squad>/home/design`), which
//! the designer's tasks land in when the person accepts them; a project with
//! a look of its own keeps one in its repository (`<repo>/design`); a ticket
//! in review whose worktree changed one shows it as a draft, and a ticket
//! still at work on one shows what it has made so far (a draft `working`).
//!
//! A folder counts when it holds `tokens.json` or `guidelines.md`. Files are
//! read only inside it — links resolved, nothing bigger than a limit — and
//! its pictures are the images in `assets/` and `components/` that pass
//! [`visuals::check`], the same check `read_image` makes when a screen asks.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use equipe_protocol::design::{Asset, AssetKind, DesignSystem, DraftOf, FlowMap};
use equipe_protocol::{TicketKind, TicketStatus};

use super::tokens::{self, Problem};
use crate::i18n::Lang;
use crate::visuals;

/// The folder's name, in the squad's folder and in a project.
pub const DIR: &str = "design";
pub const TOKENS: &str = "tokens.json";
pub const GUIDELINES: &str = "guidelines.md";
/// The designer's brief, and the folder design QA's reports land in.
pub const BRIEF: &str = "brand.md";
pub const QA: &str = "qa";
const PICTURE_DIRS: [&str; 2] = ["assets", "components"];
/// Every folder the stamp follows: the pictures', then the UI map's and
/// design QA's and the decisions' — walked last, so a big map never crowds
/// the pictures out.
const WATCHED_DIRS: [&str; 5] = [crate::taste::store::DECISIONS_DIR, QA, super::flows::DIR, "assets", "components"];
/// The files right in the folder the stamp follows.
const WATCHED_FILES: [&str; 5] = [TOKENS, GUIDELINES, BRIEF, crate::taste::store::NOTES_FILE, crate::taste::store::DECISIONS_FILE];
/// The biggest `tokens.json` read.
pub const TOKENS_MAX: u64 = 1 << 20;
/// How much of `guidelines.md` is sent: past it, cut with a word.
pub const GUIDELINES_MAX: usize = 256 << 10;
/// How many pictures, and how many files looked at to find them.
pub const ASSETS_MAX: usize = 240;
const FILES_MAX: usize = 5000;
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
/// not), each project's that has one, each draft a ticket in review has —
/// and the work a ticket in progress (or waiting on the person) is making in
/// its worktree, marked `working`, so the screen fills in as it lands.
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
        let working = match ticket.status {
            TicketStatus::Review => false,
            TicketStatus::InProgress | TicketStatus::Waiting => true,
            _ => continue,
        };
        let Some(worktree) = ticket.worktree.as_ref() else { continue };
        let dir = worktree.join(DIR);
        if is_design_dir(&dir) {
            let project = if ticket.kind == TicketKind::Code { ticket.project.as_deref().map(crate::board::project_name) } else { None };
            out.push(Place { project, draft: Some(DraftOf { ticket: ticket.id, title: ticket.title.clone(), working }), dir });
        }
    }
    out
}

/// Where `squad`'s accepted design system of `project` is (`None`: the
/// squad's own, there or not) — not a draft's.
pub fn accepted_dir(home: &Path, squad: &str, project: Option<&str>) -> Option<PathBuf> {
    places(home, squad).into_iter().find(|p| p.draft.is_none() && p.project.as_deref() == project).map(|p| p.dir)
}

/// The files under `dir`'s picture folders, its UI map's and design QA's, and
/// its own three files: path and what the disk says of each. Links are
/// listed, not followed.
fn files(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut out = Vec::new();
    for name in WATCHED_FILES {
        if let Ok(meta) = std::fs::symlink_metadata(dir.join(name)) {
            out.push((dir.join(name), meta));
        }
    }
    let mut stack: Vec<(PathBuf, usize)> = WATCHED_DIRS.iter().map(|d| (dir.join(d), 0)).collect();
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
        place.draft.as_ref().map(|d| (d.ticket, &d.title, d.working)).hash(&mut h);
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

/// Where file `name` in `dir` really is, when it is one and stays in `dir`.
pub(crate) fn within(dir: &Path, name: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(dir).ok()?;
    let real = std::fs::canonicalize(dir.join(name)).ok()?;
    (real.starts_with(&root) && std::fs::metadata(&real).is_ok_and(|m| m.is_file())).then_some(real)
}

/// A file inside `dir`, no bigger than `max` — a link only when it stays in `dir`.
pub(crate) fn inside(dir: &Path, name: &str, max: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let real = within(dir, name)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&real).ok()?.take(max + 1).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// What a picture is, by where it is and what it is called.
pub(super) fn kind_of(name: &str) -> AssetKind {
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
    // A project's folder is in its repository, where the screens' code is.
    let repo = place.project.as_ref().and_then(|_| place.dir.parent());
    system.flows = super::flows::read(&place.dir, repo, lang);
    system.setup = super::flows::setup(&place.dir);
    system.stamp = stamp(std::slice::from_ref(place));
    system
}

/// The biggest picture a hash is worked out for; past it, none.
const HASH_MAX: u64 = 32 << 20;
/// How many pictures' hashes are kept before the list starts again.
const HASHES_KEPT: usize = 20_000;

/// What a picture holds, as a short hex digest of its bytes — read once
/// while its length and time stay the same. `None`: not a file, or too big.
pub fn picture_hash(path: &Path) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    type Kept = HashMap<PathBuf, (u64, Option<std::time::SystemTime>, String)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file() && m.len() <= HASH_MAX)?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    if let Some((_, _, hash)) = KEPT.lock().ok()?.get_or_insert_with(HashMap::new).get(path).filter(|(l, m, _)| *l == len && *m == modified) {
        return Some(hash.clone());
    }
    let bytes = std::fs::read(path).ok()?;
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    let hash = format!("{:016x}", h.finish());
    let mut kept = KEPT.lock().ok()?;
    let kept = kept.get_or_insert_with(HashMap::new);
    if kept.len() >= HASHES_KEPT {
        kept.clear();
    }
    kept.insert(path.to_owned(), (len, modified, hash.clone()));
    Some(hash)
}

/// Each UI map picture's hash, in the systems that have a draft in review
/// and in their drafts — so a screen can tell what a draft changed from the
/// copies its worktree only kept. Nothing is read when there is no draft.
pub fn fingerprint(systems: &mut [DesignSystem]) {
    let drafted: Vec<Option<String>> = systems.iter().filter(|s| s.draft.is_some()).map(|s| s.project.clone()).collect();
    for system in systems.iter_mut().filter(|s| drafted.contains(&s.project)) {
        let Some(map) = system.flows.as_mut() else { continue };
        for picture in map.flows.iter_mut().flat_map(|f| &mut f.screens).flat_map(|s| &mut s.states).flat_map(|s| &mut s.pictures) {
            picture.hash = picture_hash(Path::new(&picture.path));
        }
    }
}

/// A UI map as what it shows: where its pictures are, when they were taken
/// and its recordings left out, so a worktree's copy reads as the same.
fn map_shown(map: Option<&FlowMap>) -> Option<FlowMap> {
    let mut map = map.cloned()?;
    for flow in &mut map.flows {
        flow.videos.clear();
        for screen in &mut flow.screens {
            screen.stale = false;
            for picture in screen.states.iter_mut().flat_map(|s| &mut s.pictures) {
                picture.path.clear();
                picture.taken = 0;
            }
        }
    }
    Some(map)
}

/// Whether two systems hold the same: a draft that changed nothing isn't one.
fn same_content(a: &DesignSystem, b: &DesignSystem) -> bool {
    let pictures = |s: &DesignSystem| s.assets.iter().map(|a| (a.name.clone(), a.size)).collect::<Vec<_>>();
    a.tokens == b.tokens && a.guidelines == b.guidelines && a.problems == b.problems && pictures(a) == pictures(b) && map_shown(a.flows.as_ref()) == map_shown(b.flows.as_ref())
}

/// `squad`'s design systems as screens show them, and where its own would be.
pub fn catalog(home: &Path, squad: &str, lang: Lang) -> (Vec<DesignSystem>, String) {
    catalog_of(&places(home, squad), lang)
}

/// The same, of places already found.
pub fn catalog_of(places: &[Place], lang: Lang) -> (Vec<DesignSystem>, String) {
    let folder = places.first().map(|p| p.dir.to_string_lossy().into_owned()).unwrap_or_default();
    let mut read: Vec<DesignSystem> = places.iter().filter(|p| is_design_dir(&p.dir)).map(|p| read(p, lang)).collect();
    fingerprint(&mut read);
    let systems: Vec<DesignSystem> = read
        .iter()
        .filter(|s| match &s.draft {
            // A ticket's worktree holds a copy of the accepted folder: a draft only where it differs.
            Some(_) => !read.iter().any(|a| a.draft.is_none() && a.project == s.project && same_content(a, s)),
            None => true,
        })
        .cloned()
        .collect();
    // Work under way beside a draft in review of the same system: the one in review is shown.
    let reviewed: Vec<Option<String>> = systems.iter().filter(|s| s.draft.as_ref().is_some_and(|d| !d.working)).map(|s| s.project.clone()).collect();
    let systems = systems.into_iter().filter(|s| !(s.draft.as_ref().is_some_and(|d| d.working) && reviewed.contains(&s.project))).collect();
    (systems, folder)
}

/// The pictures `squad`'s design systems show, their UI maps' included, and
/// where they may be — for `read_image`.
pub fn shown(home: &Path, squad: &str) -> (Vec<String>, Vec<PathBuf>) {
    let mut paths = Vec::new();
    let mut roots = Vec::new();
    for place in places(home, squad) {
        // Decisions' pictures: the squad's own folder's too, before it holds a system.
        paths.extend(crate::taste::store::pictures(&place.dir));
        if !is_design_dir(&place.dir) {
            continue;
        }
        paths.extend(assets(&place.dir).into_iter().map(|a| a.path));
        let map = super::flows::read(&place.dir, None, Lang::En).unwrap_or_default();
        let states = map.flows.into_iter().flat_map(|f| f.screens).flat_map(|s| s.states);
        paths.extend(states.flat_map(|s| s.pictures).map(|p| p.path));
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
        // The UI map, its pictures deep in it, the brief and design QA count too.
        let mut last = stamp(&place);
        let moved = |last: &mut u64, what: &str| {
            let now = stamp(&place);
            assert_ne!(*last, now, "{what}");
            *last = now;
        };
        std::fs::create_dir_all(design.join("flows")).unwrap();
        std::fs::write(design.join("flows/flows.json"), r#"{ "flows": [] }"#).unwrap();
        moved(&mut last, "flows.json");
        png(&design.join("flows/onboarding/login/default.light.390.png"));
        moved(&mut last, "a screen's picture");
        std::thread::sleep(std::time::Duration::from_millis(20));
        png(&design.join("flows/onboarding/login/default.light.390.png"));
        moved(&mut last, "a picture shot again");
        std::fs::write(design.join("brand.md"), "# Brief").unwrap();
        moved(&mut last, "the brief");
        std::fs::create_dir_all(design.join("qa/2026-11-04")).unwrap();
        std::fs::write(design.join("qa/2026-11-04/report.md"), "# QA").unwrap();
        moved(&mut last, "a QA report");
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
        std::fs::create_dir_all(folder.join("flows")).unwrap();
        std::fs::write(folder.join("flows/flows.json"), r#"{ "flows": [ { "id": "app", "title": "App", "screens": [ { "id": "home", "title": "Home", "states": [ { "id": "default", "label": "Default", "kind": "default" } ] } ] } ] }"#).unwrap();
        png(&folder.join("flows/app/home/default.light.390.png"));
        png(&folder.join("flows/app/home/stray.light.390.png"));
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
            // Its copy of the UI map too, as a worktree has.
            std::fs::create_dir_all(w.join(DIR).join("flows")).unwrap();
            std::fs::copy(folder.join("flows/flows.json"), w.join(DIR).join("flows/flows.json")).unwrap();
            png(&w.join(DIR).join("flows/app/home/default.light.390.png"));
            png(&w.join(DIR).join("flows/app/home/stray.light.390.png"));
            w
        };
        let changed = worktree(144, r##"{ "color": { "$type": "color", "bg": { "$value": "#0A0A0C" } } }"##);
        let copy = worktree(150, TOKENS_JSON);
        for (n, w) in [(144, changed), (150, copy)] {
            let ticket = equipe_protocol::Ticket {
                id: 0, title: format!("T{n}"), body: String::new(), status: TicketStatus::Review, kind: TicketKind::Task, owner: Some("designer".into()), assignee: None,
                project: None, depends_on: vec![], branch: None, worktree: None, merged: None, off_main: None, design_qa: None, plan: None, created_by: "pm".into(), note: None,
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
        assert!(paths.iter().any(|p| p.ends_with("home/design/flows/app/home/default.light.390.png")), "the UI map's pictures too");
        assert!(!paths.iter().any(|p| p.ends_with("stray.light.390.png")), "not one of no state");
        assert_eq!(systems[0].flows.as_ref().unwrap().flows[0].screens[0].states[0].pictures.len(), 1);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn work_in_progress_is_shown_as_it_lands_and_a_draft_in_review_comes_first() {
        let home = dir("working");
        let journal = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let folder = crate::squad_home::path(&home, "acme").join(DIR);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(TOKENS), TOKENS_JSON).unwrap();
        // Each ticket's worktree starts as a copy of the squad's folder.
        let worktree = |n: u32| {
            let w = home.join(format!("worktrees/t{n}"));
            std::fs::create_dir_all(w.join(DIR)).unwrap();
            std::fs::write(w.join(DIR).join(TOKENS), TOKENS_JSON).unwrap();
            w
        };
        let ticket = |status: TicketStatus, title: &str, w: PathBuf| {
            let t = equipe_protocol::Ticket {
                id: 0, title: title.into(), body: String::new(), status, kind: TicketKind::Task, owner: Some("designer".into()), assignee: None,
                project: None, depends_on: vec![], branch: None, worktree: None, merged: None, off_main: None, design_qa: None, plan: None, created_by: "pm".into(), note: None,
                asks: vec![], remaining: vec![], images: vec![], priority: Default::default(), rank: 0, updated_at: None,
            };
            let id = journal.create_ticket(&t).unwrap();
            journal.set_ticket_work(id, &crate::journal::TicketWork { worktree: Some(w.clone()), ..Default::default() }).unwrap();
            (id, w.join(DIR))
        };
        let (map_id, map) = ticket(TicketStatus::InProgress, "화면 지도", worktree(378));
        let (waits_id, _) = ticket(TicketStatus::Waiting, "로고", worktree(379));
        ticket(TicketStatus::Todo, "아직", worktree(380));
        ticket(TicketStatus::Done, "끝난 것", worktree(381));
        let found = places(&home, "acme");
        let mut drafts: Vec<(u32, bool)> = found.iter().filter_map(|p| p.draft.as_ref()).map(|d| (d.ticket, d.working)).collect();
        drafts.sort();
        assert_eq!(drafts, [(map_id, true), (waits_id, true)],"in progress and waiting: work under way; not one to do or done");
        // Nothing made yet: its worktree is the squad's folder, and nothing shows.
        let keys = |home: &Path| catalog(home, "acme", Lang::En).0.iter().map(|s| s.key()).collect::<Vec<_>>();
        assert_eq!(keys(&home), ["squad"]);
        // The map lands, a picture at a time: the folder's watch follows it, and the work shows.
        let before = stamp(&places(&home, "acme"));
        std::fs::create_dir_all(map.join("flows")).unwrap();
        std::fs::write(map.join("flows/flows.json"), r#"{ "flows": [ { "id": "app", "title": "App", "screens": [ { "id": "home", "title": "Home", "states": [ { "id": "default" } ] } ] } ] }"#).unwrap();
        let listed = stamp(&places(&home, "acme"));
        assert_ne!(before, listed, "flows.json in the worktree of a ticket in progress");
        png(&map.join("flows/app/home/default.light.390.png"));
        assert_ne!(listed, stamp(&places(&home, "acme")), "its first picture");
        let (systems, _) = catalog(&home, "acme", Lang::En);
        let work = systems.iter().find(|s| s.draft.is_some()).unwrap();
        assert_eq!(work.key(), format!("squad#T-{map_id}"));
        assert!(work.draft.as_ref().unwrap().working);
        assert_eq!(work.flows.as_ref().unwrap().flows[0].screens[0].states[0].pictures.len(), 1);
        let (paths, _) = shown(&home, "acme");
        assert!(paths.iter().any(|p| p.ends_with("t378/design/flows/app/home/default.light.390.png")), "its pictures can be read");
        // Handed in, it is in review; a ticket still at work on the same system waits behind it.
        let mut handed_in = journal.ticket(map_id).unwrap().unwrap();
        handed_in.status = TicketStatus::Review;
        journal.save_ticket(&handed_in).unwrap();
        std::fs::write(home.join("worktrees/t379/design").join(GUIDELINES), "# 로고").unwrap();
        let (systems, _) = catalog(&home, "acme", Lang::En);
        let drafts: Vec<(u32, bool)> = systems.iter().filter_map(|s| s.draft.as_ref()).map(|d| (d.ticket, d.working)).collect();
        assert_eq!(drafts, [(map_id, false)], "the one in review is shown");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_draft_that_only_shot_its_screens_again_is_one_and_its_pictures_say_what_they_hold() {
        const MAP: &str = r#"{ "flows": [ { "id": "app", "title": "App", "screens": [ { "id": "home", "title": "Home", "states": [ { "id": "default" } ] }, { "id": "pay", "title": "Pay", "states": [ { "id": "default" } ] } ] } ] }"#;
        let root = dir("draft-map");
        let folder = |name: &str, home_colour: [u8; 4]| {
            let design = root.join(name).join(DIR);
            std::fs::create_dir_all(design.join("flows")).unwrap();
            std::fs::write(design.join(TOKENS), TOKENS_JSON).unwrap();
            std::fs::write(design.join("flows/flows.json"), MAP).unwrap();
            let shot = |path: PathBuf, c: [u8; 4]| image::RgbaImage::from_pixel(4, 4, image::Rgba(c)).save(path).unwrap();
            for screen in ["home", "pay"] {
                std::fs::create_dir_all(design.join(format!("flows/app/{screen}"))).unwrap();
            }
            shot(design.join("flows/app/home/default.light.390.png"), home_colour);
            shot(design.join("flows/app/pay/default.light.390.png"), [9, 9, 9, 255]);
            design
        };
        let accepted = folder("main", [20, 90, 250, 255]);
        let reshot = folder("t170", [250, 90, 20, 255]);
        let copy = folder("t171", [20, 90, 250, 255]);
        let draft = |n: u32, dir: PathBuf| Place { project: None, draft: Some(DraftOf { ticket: n, title: format!("T{n}"), working: false }), dir };
        let alone = [Place { project: None, draft: None, dir: accepted.clone() }];
        let (systems, _) = catalog_of(&alone, Lang::En);
        let pictures = |s: &DesignSystem| s.flows.as_ref().unwrap().flows[0].screens.iter().map(|sc| sc.states[0].pictures[0].hash.clone()).collect::<Vec<_>>();
        assert!(pictures(&systems[0]).iter().all(Option::is_none), "no draft: nothing read for it");
        let places = [alone[0].clone(), draft(170, reshot), draft(171, copy)];
        let (systems, _) = catalog_of(&places, Lang::En);
        assert_eq!(systems.iter().map(|s| s.key()).collect::<Vec<_>>(), ["squad", "squad#T-170"], "the copy that changed nothing isn't a draft");
        let (now, after) = (pictures(&systems[0]), pictures(&systems[1]));
        assert!(now.iter().chain(&after).all(|h| h.as_ref().is_some_and(|h| h.len() == 16)), "{now:?} {after:?}");
        assert_ne!(now[0], after[0], "home was shot again");
        assert_eq!(now[1], after[1], "pay is the same picture, in another folder");
        assert_eq!(picture_hash(&accepted.join("flows/app/home/default.light.390.png")), now[0], "kept, and the same when asked again");
        assert!(picture_hash(&accepted.join("nowhere.png")).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
