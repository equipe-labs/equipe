//! Where taste notes and decisions are kept, read for the screens and
//! written when the person (or a pick) changes them.
//!
//! - The squad's notes: its memory, the fact `design taste notes`
//!   (`memory/design-taste-notes.md`). The memory is what every member is
//!   given each turn, it is there before any design folder is, it is saved
//!   and can be taken back like the rest of the squad's folder, and what the
//!   person wrote last wins until they agree — the confirmation a note the
//!   designer proposes needs. Beside the learned record (`design taste`),
//!   which stays as it was: the evidence a note is drawn from.
//! - A project's notes: `design/taste.md` in its repository, versioned with
//!   its design, in the same line format.
//! - Decisions: `design/decisions/`, a file each and its picture beside it —
//!   the squad's in its folder (saved there), a project's in its repository.
//!
//! Read the way `design::folder` reads: inside the folder only, links that
//! lead out refused, nothing past a size.

use std::path::{Path, PathBuf};

use equipe_protocol::design::{Decision, Taste, TasteChange};

use super::{decisions, evidence, notes};
use crate::design::folder::{self, Place};
use crate::squad_memory;

/// The squad's notes in its memory.
pub const NOTES_NAME: &str = "design taste notes";
pub const NOTES_DESCRIPTION: &str = "The person's design taste, a note a line: read them before any design proposal and follow each.";
/// A project's notes, in its `design/`.
pub const NOTES_FILE: &str = "taste.md";
/// Decisions, a file each, in `design/`; and one file holding many.
pub const DECISIONS_DIR: &str = "decisions";
pub const DECISIONS_FILE: &str = "decisions.md";
/// The biggest notes or decision file read.
pub const TEXT_MAX: u64 = 64 << 10;
/// How many decision files are looked at.
const FILES_MAX: usize = 400;
/// The template a project's `taste.md` starts from.
pub const TEMPLATE: &str = include_str!("../../assets/design/taste.md");

/// Why a change to the notes wasn't made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Invalid(notes::Invalid),
    /// No such project, or it has no design folder.
    NoProject,
    /// The disk or git said no.
    Failed(String),
}

impl From<notes::Invalid> for Refused {
    fn from(why: notes::Invalid) -> Self {
        Refused::Invalid(why)
    }
}

/// The memory file of a fact named `name`.
fn memory_file(folder: &Path, name: &str) -> PathBuf {
    squad_memory::dir(folder).join(format!("{}.md", squad_memory::slug(name)))
}

/// A fact's body, read safely: its file inside the memory folder, no link out, not too big.
fn fact_body(folder: &Path, name: &str) -> Option<String> {
    let file = format!("{}.md", squad_memory::slug(name));
    let bytes = folder::inside(&squad_memory::dir(folder), &file, TEXT_MAX).filter(|b| b.len() as u64 <= TEXT_MAX)?;
    Some(squad_memory::parse_fact(&file, &String::from_utf8_lossy(&bytes)).body)
}

/// A text file in `dir`, read safely.
fn text_in(dir: &Path, name: &str) -> Option<String> {
    let bytes = folder::inside(dir, name, TEXT_MAX).filter(|b| b.len() as u64 <= TEXT_MAX)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Where a decision's picture is, when it is one inside `dir`: written
/// relative to the design folder (or to `decisions/`).
fn picture_at(dir: &Path, written: &str) -> Option<String> {
    if !crate::visuals::is_image_name(written) {
        return None;
    }
    let rel = written.strip_prefix(&format!("{}/", dir.to_string_lossy())).unwrap_or(written);
    [rel.to_owned(), format!("{DECISIONS_DIR}/{rel}")].iter().find_map(|r| folder::within(dir, r)).map(|p| p.to_string_lossy().into_owned())
}

/// The decisions kept in the design folder `dir`: a file each in
/// `decisions/`, `decisions.md`, and the old `taste.md`'s entries — their
/// pictures as `read_image` asks for them, the ones outside left out.
pub fn decisions_in(dir: &Path) -> Vec<Decision> {
    let mut found: Vec<Vec<Decision>> = Vec::new();
    let sub = dir.join(DECISIONS_DIR);
    if std::fs::symlink_metadata(&sub).is_ok_and(|m| m.is_dir())
        && let Ok(entries) = std::fs::read_dir(&sub)
    {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_file()))
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
            .take(FILES_MAX)
            .collect();
        names.sort();
        names.reverse();
        found.extend(names.iter().filter_map(|n| text_in(dir, &format!("{DECISIONS_DIR}/{n}"))).map(|t| decisions::parse(&t)));
    }
    for name in [DECISIONS_FILE, NOTES_FILE] {
        found.extend(text_in(dir, name).map(|t| decisions::parse(&t)));
    }
    let mut merged = decisions::merge(found);
    for d in &mut merged {
        d.picture = d.picture.take().and_then(|p| picture_at(dir, &p));
    }
    merged
}

/// The pictures of the decisions in `dir`: what `read_image` may give.
pub fn pictures(dir: &Path) -> Vec<String> {
    decisions_in(dir).into_iter().filter_map(|d| d.picture).collect()
}

/// The squad's taste, and each project's with a design system of its own
/// among `places` — for the screens, each note with what it was learned
/// from (tickets are the squad's, so a project's note finds the squad's
/// record too).
pub fn of(home: &Path, squad: &str, places: &[Place]) -> Vec<Taste> {
    let folder = crate::squad_home::path(home, squad);
    let own = folder.join(folder::DIR);
    let history = fact_body(&folder, super::NAME).unwrap_or_default();
    let sent_back = evidence::sent_backs(&history);
    let squad_decisions = decisions::merge(vec![decisions_in(&own), decisions::from_history(&history)]);
    let mut squad_notes = fact_body(&folder, NOTES_NAME).map(|b| notes::parse(&b)).unwrap_or_default();
    evidence::link(&mut squad_notes, &squad_decisions, &sent_back);
    let mut out = vec![Taste { project: None, notes_at: memory_file(&folder, NOTES_NAME).to_string_lossy().into_owned(), notes: squad_notes, decisions: squad_decisions }];
    for place in places.iter().filter(|p| p.draft.is_none() && p.project.is_some() && folder::is_design_dir(&p.dir)) {
        let decisions = decisions_in(&place.dir);
        let mut notes = text_in(&place.dir, NOTES_FILE).map(|t| notes::parse(&t)).unwrap_or_default();
        let known: Vec<_> = decisions.iter().chain(&out[0].decisions).cloned().collect();
        evidence::link(&mut notes, &known, &sent_back);
        out.push(Taste { project: place.project.clone(), notes_at: place.dir.join(NOTES_FILE).to_string_lossy().into_owned(), notes, decisions });
    }
    out
}

/// A number that changes when the squad's notes or learned taste do (its
/// design folders' are in `design::stamp`).
pub fn stamp(home: &Path, squad: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let folder = crate::squad_home::path(home, squad);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for name in [NOTES_NAME, super::NAME] {
        std::fs::metadata(memory_file(&folder, name)).ok().map(|m| (m.len(), m.modified().ok())).hash(&mut h);
    }
    h.finish()
}

/// The design folder of `project` (none: the squad's own), there or not yet.
fn design_dir(home: &Path, squad: &str, project: Option<&str>) -> Option<PathBuf> {
    let Some(name) = project else {
        return Some(crate::squad_home::path(home, squad).join(folder::DIR));
    };
    if let Some(place) = folder::places(home, squad).into_iter().find(|p| p.draft.is_none() && p.project.as_deref() == Some(name)) {
        return Some(place.dir);
    }
    let db = crate::journal::squad_dir(home, squad).join("squad.db");
    let journal = crate::journal::Journal::open(&db).ok()?;
    journal.projects().ok()?.into_iter().find(|p| crate::board::project_name(p) == name).map(|p| p.join(folder::DIR))
}

/// Whether `path` may be written: not a link, not a folder.
fn writable(path: &Path) -> Result<(), Refused> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => Err(Refused::Failed(format!("{} isn't a plain file", path.display()))),
        _ => Ok(()),
    }
}

/// The person's change to a taste note — the squad's (saved in its memory,
/// by them) or a project's (`design/taste.md` in its repository).
pub fn change(home: &Path, squad: &str, project: Option<&str>, change: &TasteChange, today: &str, language: Option<&str>) -> Result<(), Refused> {
    let Some(project) = project else {
        let folder = crate::squad_home::path(home, squad);
        let before = fact_body(&folder, NOTES_NAME).unwrap_or_default();
        let now = notes::change(&notes::parse(&before), change, today)?;
        if notes::parse(&before) == now {
            return Ok(());
        }
        let body = notes::write(&before, &now);
        let fact = squad_memory::Remember { name: NOTES_NAME, description: NOTES_DESCRIPTION, kind: "person", body: &body, confirmed: true };
        return squad_memory::remember(&folder, crate::squad_home::PERSON, &fact, today, language).map(|_| ()).map_err(Refused::Failed);
    };
    let dir = design_dir(home, squad, Some(project)).filter(|d| folder::is_design_dir(d)).ok_or(Refused::NoProject)?;
    let path = dir.join(NOTES_FILE);
    writable(&path)?;
    let before = if path.exists() { text_in(&dir, NOTES_FILE).ok_or_else(|| Refused::Failed(format!("{} couldn't be read", path.display())))? } else { TEMPLATE.to_owned() };
    let now = notes::change(&notes::parse(&before), change, today)?;
    std::fs::write(&path, notes::write(&before, &now)).map_err(|e| Refused::Failed(e.to_string()))
}

/// A file name for a decision: its date and question, made unique in `dir`.
fn file_name(dir: &Path, d: &Decision, ext: &str) -> String {
    let slug: String = squad_memory::slug(&d.question).chars().take(40).collect();
    let stem = if slug.is_empty() { d.date.clone() } else { format!("{}-{slug}", d.date) };
    let mut n = 1;
    loop {
        let stem = if n == 1 { stem.clone() } else { format!("{stem}-{n}") };
        if !dir.join(format!("{stem}.md")).exists() && !dir.join(format!("{stem}.{ext}")).exists() {
            return stem;
        }
        n += 1;
    }
}

/// Keep a design decision in the record: `design/decisions/<date>-<question>.md`
/// in the squad's folder (`project` none; saved there by `by`) or the
/// project's repository, its picture copied beside it so the record keeps it
/// after the worktree it came from is gone. `picture` is where the chosen
/// option's picture is now, a picture the squad may show (its folder, a
/// ticket's worktree, a project, the scratch folders); `decision.picture`
/// is ignored. A date left empty is `today`. Returns the file written.
pub fn record(home: &Path, squad: &str, project: Option<&str>, decision: &Decision, picture: Option<&Path>, by: &str) -> Result<PathBuf, String> {
    if decision.question.trim().is_empty() || decision.chosen.trim().is_empty() {
        return Err("A decision needs its question and what was chosen.".into());
    }
    let dir = design_dir(home, squad, project).ok_or_else(|| format!("{squad} has no project called {}", project.unwrap_or_default()))?;
    let sub = dir.join(DECISIONS_DIR);
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) || std::fs::symlink_metadata(&sub).is_ok_and(|m| !m.is_dir()) {
        return Err(format!("{} isn't a folder", sub.display()));
    }
    std::fs::create_dir_all(&sub).map_err(|e| e.to_string())?;
    let mut d = decision.clone();
    if d.date.trim().is_empty() {
        d.date = squad_memory::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    }
    let source = match picture {
        Some(path) => {
            let shown = crate::visuals::shown(home, squad);
            let mut roots = shown.roots;
            roots.extend(shown.projects);
            roots.extend([std::env::temp_dir(), PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]);
            Some(crate::visuals::check(path, &roots)?)
        }
        None => None,
    };
    let ext = source.as_ref().and_then(|s| s.extension()).and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stem = file_name(&sub, &d, &ext);
    d.picture = None;
    if let Some(source) = source {
        let name = format!("{stem}.{ext}");
        std::fs::copy(&source, sub.join(&name)).map_err(|e| e.to_string())?;
        d.picture = Some(format!("{DECISIONS_DIR}/{name}"));
    }
    let path = sub.join(format!("{stem}.md"));
    std::fs::write(&path, decisions::text(&d)).map_err(|e| e.to_string())?;
    if project.is_none() {
        let folder = crate::squad_home::path(home, squad);
        let rel = format!("{}/{DECISIONS_DIR}", folder::DIR);
        crate::squad_home::save_in(&folder, by, &format!("recorded a design decision: {}", d.question.trim()), &[&rel])?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::design::{TasteFrom, TasteNote};

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn home() -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("equipe-taste-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        crate::journal::Journal::for_squad(&home, "acme").unwrap();
        crate::squad_home::ensure(&home, "acme", None).unwrap();
        home.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 250, 255])).save(path).unwrap();
    }

    fn add(text: &str) -> TasteChange {
        TasteChange::Add { text: text.into() }
    }

    #[test]
    fn the_squads_notes_live_in_its_memory_beside_what_equipe_learned() {
        let home = home();
        let folder = crate::squad_home::path(&home, "acme");
        // What equipe learned before notes were: still read, as decisions.
        // (A change by equipe itself is no news, so `remember` says nothing changed.)
        let learned = crate::taste::with(None, "## 2026-11-01 · T-272 (designer)\n- Which hero? \u{2192} chose B; passed over A");
        let learned = crate::taste::with(Some(&learned), "## 2026-11-02 · T-336 (designer)\n- Sent back \u{201c}Pricing\u{201d}: \u{201c}너무 쨍해\u{201d}");
        let _ = squad_memory::remember(&folder, "equipe", &squad_memory::Remember { name: crate::taste::NAME, description: crate::taste::DESCRIPTION, kind: "decision", body: &learned, confirmed: true }, "2026-11-01", None);
        assert!(folder.join("memory/design-taste.md").is_file());
        // The designer writes down what it saw, for the person to confirm.
        let proposed = "- 채도 높은 파랑은 피하기 _(sent back on T-336, 2026-11-02)_";
        squad_memory::remember(&folder, "designer", &squad_memory::Remember { name: NOTES_NAME, description: NOTES_DESCRIPTION, kind: "person", body: proposed, confirmed: false }, "2026-11-02", None).unwrap();
        let before = stamp(&home, "acme");
        change(&home, "acme", None, &add("모서리는 둥글게"), "2026-11-03", Some("Korean")).unwrap();
        change(&home, "acme", None, &add("여백은 넉넉하게"), "2026-11-04", None).unwrap();
        assert_ne!(before, stamp(&home, "acme"));
        let text = std::fs::read_to_string(folder.join("memory/design-taste-notes.md")).unwrap();
        assert!(text.contains("name: design taste notes") && text.contains(&format!("{proposed}\n- 모서리는 둥글게 _(the person, 2026-11-03)_\n- 여백은 넉넉하게 _(the person, 2026-11-04)_")), "{text}");
        assert_eq!(squad_memory::changes(&folder, 1).unwrap()[0].by, crate::squad_home::PERSON, "saved as the person's");
        assert_eq!(change(&home, "acme", None, &add("모서리는 둥글게"), "x", None), Err(Refused::Invalid(notes::Invalid::Already)));
        change(&home, "acme", None, &TasteChange::Edit { was: "모서리는 둥글게".into(), text: "모서리는 둥글게 (12px)".into() }, "2026-11-05", None).unwrap();
        change(&home, "acme", None, &TasteChange::Remove { text: "여백은 넉넉하게".into() }, "2026-11-05", None).unwrap();
        let taste = of(&home, "acme", &folder::places(&home, "acme"));
        assert_eq!(taste.len(), 1);
        assert_eq!(taste[0].notes[1], TasteNote { text: "모서리는 둥글게 (12px)".into(), from: TasteFrom::Person, date: Some("2026-11-03".into()), confirmed: true, ..TasteNote::default() });
        // The designer's: not confirmed yet, with what it was learned from.
        let theirs = &taste[0].notes[0];
        assert!(!theirs.confirmed && theirs.by_designer());
        assert_eq!(theirs.evidence.len(), 1);
        assert_eq!((theirs.evidence[0].ticket, theirs.evidence[0].words.as_deref()), (Some(336), Some("너무 쨍해")));
        // "맞아요": confirmed, in the file too.
        change(&home, "acme", None, &TasteChange::Confirm { text: "채도 높은 파랑은 피하기".into() }, "2026-11-05", None).unwrap();
        assert!(of(&home, "acme", &[])[0].notes[0].confirmed);
        assert!(std::fs::read_to_string(folder.join("memory/design-taste-notes.md")).unwrap().contains("2026-11-02, confirmed)_"));
        assert!(taste[0].notes_at.ends_with("memory/design-taste-notes.md"));
        assert_eq!(taste[0].decisions.len(), 1);
        assert_eq!((taste[0].decisions[0].chosen.as_str(), taste[0].decisions[0].ticket), ("B", Some(272)));
        // The learned record is as it was.
        let learned_now = squad_memory::facts(&folder).into_iter().find(|f| f.name == crate::taste::NAME).unwrap();
        assert!(learned_now.body.contains("chose B"));
        // All gone: none left, the fact still there.
        change(&home, "acme", None, &TasteChange::Remove { text: "모서리는 둥글게 (12px)".into() }, "2026-11-06", None).unwrap();
        change(&home, "acme", None, &TasteChange::Remove { text: "채도 높은 파랑은 피하기".into() }, "2026-11-06", None).unwrap();
        assert!(of(&home, "acme", &[])[0].notes.is_empty());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_decision_is_recorded_with_its_picture_kept_beside_it() {
        let home = home();
        let folder = crate::squad_home::path(&home, "acme");
        let option = folder.join("outputs/T-272/b.png");
        png(&option);
        let d = Decision { question: "히어로 레이아웃".into(), chosen: "B — 한 단".into(), why: Some("숨 쉬게".into()), ticket: Some(272), ..Decision::default() };
        let path = record(&home, "acme", None, &d, Some(&option), "designer").unwrap();
        assert!(path.starts_with(folder.join("design/decisions")), "{path:?}");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-히어로-레이아웃.md"), "{name}");
        assert!(path.with_extension("png").is_file(), "the picture beside it");
        let change = crate::squad_home::changes(&folder, 1).unwrap().remove(0);
        assert_eq!(change.by, "designer");
        // The same question again that day: another file, never overwritten.
        let again = record(&home, "acme", None, &d, None, "designer").unwrap();
        assert_ne!(again, path);
        let taste = of(&home, "acme", &[]);
        assert_eq!(taste[0].decisions.len(), 1, "the same question that day is one decision");
        let read = &taste[0].decisions[0];
        assert_eq!((read.why.as_deref(), read.ticket), (Some("숨 쉬게"), Some(272)));
        assert_eq!(read.picture.as_deref(), Some(path.with_extension("png").to_string_lossy().as_ref()), "where read_image finds it");
        assert_eq!(pictures(&folder.join("design")).len(), 1);
        // A picture from outside what the squad may show isn't taken.
        let outside = std::env::temp_dir().parent().unwrap().join("x.png");
        assert!(record(&home, "acme", None, &d, Some(&outside), "designer").is_err());
        assert!(record(&home, "acme", None, &Decision { chosen: " ".into(), ..d }, None, "designer").is_err());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_projects_notes_and_decisions_are_in_its_design_folder_read_from_inside_only() {
        use std::os::unix::fs::symlink;
        let home = home();
        let journal = crate::journal::Journal::open(&crate::journal::squad_dir(&home, "acme").join("squad.db")).unwrap();
        let app = home.join("code/web");
        let design = app.join("design");
        std::fs::create_dir_all(&design).unwrap();
        std::fs::write(design.join("guidelines.md"), "# Web").unwrap();
        journal.add_project(&app).unwrap();
        // The old template's taste.md: a note by hand, a dated entry.
        std::fs::write(design.join(NOTES_FILE), "# Taste\n\n- 선은 얇게\n- 점 하나로 _(noticed on T-120, 2026-10-05)_\n\n## 2026-10-02 · Logo\n- Chose: dot (`decisions/dot.png`)\n").unwrap();
        png(&design.join("decisions/dot.png"));
        // A decision file that is a link out, and a picture that leads out.
        let secret = home.join("secret.md");
        std::fs::write(&secret, "## 2026-10-03 · Secret\n- Chose: x\n").unwrap();
        symlink(&secret, design.join("decisions/leak.md")).unwrap();
        std::fs::write(design.join("decisions/2026-10-04-a.md"), "## 2026-10-04 · Accent\n- Chose: Clay\n- Picture: ../../../secret.png\n- Ticket: T-120\n").unwrap();
        change(&home, "acme", Some("web"), &add("숫자는 고정폭"), "2026-11-04", None).unwrap();
        let taste = of(&home, "acme", &folder::places(&home, "acme"));
        let web = taste.iter().find(|t| t.key() == "project:web").unwrap();
        assert_eq!(web.notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["선은 얇게", "점 하나로", "숫자는 고정폭"]);
        assert_eq!(web.notes[1].evidence.iter().map(|e| e.what.as_str()).collect::<Vec<_>>(), ["Accent \u{2192} Clay"], "linked by its ticket");
        assert_eq!(web.decisions.iter().map(|d| d.question.as_str()).collect::<Vec<_>>(), ["Accent", "Logo"], "nothing through the link");
        assert_eq!(web.decisions[0].picture, None, "a picture outside isn't one");
        assert!(web.decisions[1].picture.as_deref().is_some_and(|p| p.ends_with("design/decisions/dot.png")));
        let file = std::fs::read_to_string(design.join(NOTES_FILE)).unwrap();
        assert!(file.contains("- 선은 얇게 _(the person)_\n- 점 하나로 _(noticed on T-120, 2026-10-05)_\n- 숫자는 고정폭 _(the person, 2026-11-04)_\n\n## 2026-10-02 · Logo"), "{file}");
        assert_eq!(change(&home, "acme", Some("nope"), &add("x"), "x", None), Err(Refused::NoProject));
        // A project with none yet starts from the template.
        std::fs::remove_file(design.join(NOTES_FILE)).unwrap();
        change(&home, "acme", Some("web"), &add("둥글게"), "2026-11-04", None).unwrap();
        let file = std::fs::read_to_string(design.join(NOTES_FILE)).unwrap();
        assert!(file.starts_with("# Taste") && file.ends_with("- 둥글게 _(the person, 2026-11-04)_\n"), "{file}");
        assert_eq!(notes::parse(&file).len(), 1, "the template has no note of its own");
        // A taste.md that is a link isn't written through.
        std::fs::remove_file(design.join(NOTES_FILE)).unwrap();
        symlink(&secret, design.join(NOTES_FILE)).unwrap();
        assert!(matches!(change(&home, "acme", Some("web"), &add("y"), "x", None), Err(Refused::Failed(_))));
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "## 2026-10-03 · Secret\n- Chose: x\n");
        std::fs::remove_dir_all(home).unwrap();
    }
}
