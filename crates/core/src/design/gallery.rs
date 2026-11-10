//! The components a design system has, as the screen's Components section
//! shows them (`docs/DESIGN-SYSTEM.md`). Read from what the squad already
//! keeps, never at a fixed list:
//!
//! - the project's registry (`registry.json` at the top of its repository:
//!   each `registry:ui`, `registry:component` and `registry:block` item, its
//!   title, description, first file, and `meta.docs` / `meta.preview` — the
//!   page that shows it live) and its components' folder
//!   (`components.json`'s `ui` alias, `components/ui`);
//! - the previews the designer rendered into the design folder's
//!   `components/`: `button.png` the component as it first draws, a variant
//!   or state as `button/outline.png` or `button.hover.png`; a `button.md`
//!   beside them says what it is for where no registry does;
//! - what its matrix found and the contact sheets it drew
//!   (`checks/components/<name>.json`, `checks/components/<name>/sheet-<width>.png`),
//!   with the page it was drawn from.
//!
//! A component a ticket is still making, or has in review, is said with
//! that ticket ([`mark_work`]): the drafts' components laid over the
//! accepted system's.

use std::path::{Path, PathBuf};

use equipe_protocol::design::{Asset, AssetKind, Component, ComponentCheck, ComponentWork, DesignSystem, Preview};

use super::system::{self, catalog, matrix, registry};
use crate::visuals;

/// The biggest `registry.json`, matrix record and note read.
const REGISTRY_MAX: u64 = 2 << 20;
const RECORD_MAX: u64 = 1 << 20;
const NOTE_MAX: u64 = 16 << 10;
/// The most components sent, and the longest description.
pub const COMPONENTS_MAX: usize = 400;
const DESCRIPTION_MAX: usize = 400;
/// The registry items that are components.
const KINDS: [&str; 3] = ["registry:ui", "registry:component", "registry:block"];
/// Where the matrices are kept in a design folder.
const RECORDS: [&str; 2] = ["checks", "components"];
/// The keys of a registry item's `meta` that may hold its live page.
const PAGE_KEYS: [&str; 6] = ["docs", "preview", "page", "url", "story", "storybook"];

/// A component's name in words: `evidence-card` → `Evidence Card`.
pub fn titled(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The component and the label a picture in `components/` is of, by its
/// name there: `button.png` → (`button`, none), `button/outline.png` →
/// (`button`, `outline`), `button.hover.dark.png` → (`button`, `hover · dark`).
pub fn of_picture(rel: &str) -> Option<(String, Option<String>)> {
    let rel = rel.strip_prefix("components/").unwrap_or(rel);
    let mut parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let file = parts.pop()?;
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let (name, labels): (&str, Vec<&str>) = match parts.split_first() {
        Some((dir, rest)) => (dir, rest.iter().copied().chain(stem.split('.')).collect()),
        None => {
            let mut words = stem.split('.');
            (words.next()?, words.collect())
        }
    };
    let name = system::known_name(name);
    if name.is_empty() {
        return None;
    }
    let labels: Vec<&str> = labels.into_iter().filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("default")).collect();
    Some((name, (!labels.is_empty()).then(|| labels.join(" · "))))
}

/// A matrix's contact sheet's width, by its name: `sheet-1280.png` → 1280.
fn sheet_width(path: &str) -> Option<u32> {
    let file = path.rsplit(['/', '\\']).next()?;
    file.strip_prefix("sheet-")?.split('.').next()?.parse().ok()
}

fn short(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= DESCRIPTION_MAX {
        return Some(text.to_owned());
    }
    Some(format!("{}…", text.chars().take(DESCRIPTION_MAX).collect::<String>().trim_end()))
}

/// A note's first paragraph, past its heading.
fn first_paragraph(text: &str) -> Option<String> {
    let para: Vec<&str> = text.lines().map(str::trim).skip_while(|l| l.is_empty() || l.starts_with('#')).take_while(|l| !l.is_empty()).collect();
    short(&para.join(" "))
}

fn small_file(path: &Path, max: u64) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= max).then(|| std::fs::read_to_string(path).ok()).flatten()
}

/// A page to see a component live: an `http(s)` address.
fn web_page(value: &str) -> Option<String> {
    let v = value.trim();
    (v.starts_with("https://") || v.starts_with("http://")).then(|| v.to_owned())
}

/// The registry at the top of `repo`, when there is one small enough to read.
fn registry_of(repo: &Path) -> Option<registry::Registry> {
    let path = repo.join("registry.json");
    small_file(&path, REGISTRY_MAX)?;
    registry::Registry::read(&path).ok()
}

/// What a matrix record found, and where its pictures are now: by their
/// names in its own folder, so a record made in a ticket's worktree reads
/// the same once merged.
fn record(design: &Path, path: &Path) -> Option<(String, matrix::Report)> {
    let report: matrix::Report = serde_json::from_str(&small_file(path, RECORD_MAX)?).ok()?;
    let name = if report.component.is_empty() { path.file_stem()?.to_string_lossy().into_owned() } else { report.component.clone() };
    let name = system::known_name(&name);
    let mut report = report;
    let dir = matrix::pictures_dir(design, &name);
    report.sheets = report.sheets.iter().filter_map(|s| Some(dir.join(s.rsplit(['/', '\\']).next()?).to_string_lossy().into_owned())).collect();
    Some((name, report))
}

/// The matrix records in `design`, by file.
fn records(design: &Path) -> Vec<PathBuf> {
    let dir = RECORDS.iter().fold(design.to_path_buf(), |d, p| d.join(p));
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    out.sort();
    out.truncate(COMPONENTS_MAX);
    out
}

/// The matrices' contact sheets in `design` that `read_image` may give.
pub fn pictures(design: &Path) -> Vec<String> {
    let Ok(root) = std::fs::canonicalize(design) else { return Vec::new() };
    records(design)
        .iter()
        .filter_map(|p| record(design, p))
        .flat_map(|(_, r)| r.sheets)
        .filter_map(|s| visuals::check(Path::new(&s), std::slice::from_ref(&root)).ok())
        .map(|real| real.to_string_lossy().into_owned())
        .collect()
}

/// The files whose change changes the components, beyond the design
/// folder's own pictures: the matrices' records, and in the repository its
/// registry, `components.json` and the usual components' folders.
pub fn watched(design: &Path, repo: Option<&Path>) -> Vec<PathBuf> {
    let mut out = records(design);
    if let Some(repo) = repo {
        out.extend(["registry.json", "components.json", "components/ui", "src/components/ui", "registry/ui", "src/registry/ui"].iter().map(|p| repo.join(p)));
    }
    out
}

/// The components of the design system in `design` — its repository at
/// `repo` for a project's — given its pictures as read (`folder::assets`).
pub fn read(design: &Path, repo: Option<&Path>, assets: &[Asset]) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    fn at<'a>(out: &'a mut Vec<Component>, name: &str) -> &'a mut Component {
        match out.iter().position(|c| c.name == name) {
            Some(i) => &mut out[i],
            None => {
                out.push(Component { name: name.to_owned(), title: titled(name), ..Component::default() });
                out.last_mut().expect("just pushed")
            }
        }
    }
    if let Some(repo) = repo {
        if let Some(reg) = registry_of(repo) {
            for item in reg.items.iter().filter(|i| KINDS.contains(&i.kind.as_str())) {
                let name = system::known_name(&item.name);
                if name.is_empty() {
                    continue;
                }
                let c = at(&mut out, &name);
                if let Some(title) = item.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    c.title = title.to_owned();
                }
                c.description = item.description.as_deref().and_then(short);
                c.block = item.kind == "registry:block";
                c.source = item.files.first().map(|f| f.path.clone()).filter(|p| !p.is_empty());
                c.page = PAGE_KEYS.iter().find_map(|k| item.meta.get(*k)?.as_str().and_then(web_page));
            }
        }
        let dir = system::components_dir(repo, system::ComponentsJson::read(&repo.join("components.json")).as_ref());
        for (name, file) in dir.as_deref().map(system::names_in).unwrap_or_default() {
            let c = at(&mut out, &name);
            if c.source.is_none() {
                c.source = file.strip_prefix(repo).ok().map(|r| r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    for asset in assets.iter().filter(|a| a.kind == AssetKind::Component) {
        let Some((name, label)) = of_picture(&asset.name) else { continue };
        let c = at(&mut out, &name);
        c.previews.push(Preview { path: asset.path.clone(), label, sheet: None, size: asset.size });
    }
    let root = std::fs::canonicalize(design).ok();
    for path in records(design) {
        let Some((name, report)) = record(design, &path) else { continue };
        let c = at(&mut out, &name);
        let notes = report.findings.iter().filter(|f| f.level == matrix::Level::Note).count();
        c.check = Some(ComponentCheck { at: report.at, fails: report.fails(), notes });
        c.variants = report.examples.iter().map(|e| e.trim().to_owned()).filter(|e| !e.is_empty()).collect();
        if c.page.is_none() {
            c.page = web_page(&report.url).or_else(|| {
                let p = Path::new(report.url.trim());
                (p.is_absolute() && p.is_file()).then(|| report.url.trim().to_owned())
            });
        }
        let mut sheets: Vec<Preview> = report
            .sheets
            .iter()
            .filter_map(|s| {
                let real = visuals::check(Path::new(s), root.as_slice()).ok()?;
                let size = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(0);
                Some(Preview { path: real.to_string_lossy().into_owned(), label: None, sheet: Some(sheet_width(s).unwrap_or(0)), size })
            })
            .collect();
        sheets.sort_by_key(|p| std::cmp::Reverse(p.sheet));
        c.previews.extend(sheets);
    }
    for c in &mut out {
        // The component as it first draws, then its variants and states in order.
        c.previews.sort_by_key(|p| (p.sheet.is_some(), p.label.is_some()));
        if let Some(known) = catalog::catalog().get(&c.name) {
            c.states = known.states.clone();
        }
        if c.description.is_none() {
            c.description = small_file(&design.join("components").join(format!("{}.md", c.name)), NOTE_MAX).as_deref().and_then(first_paragraph);
        }
    }
    out.sort_by_key(|c| (c.block, c.title.to_lowercase()));
    out.truncate(COMPONENTS_MAX);
    out
}

/// What the person sees of a component, for telling whether a draft changed it.
fn shown(c: &Component) -> impl PartialEq + use<> {
    (c.title.clone(), c.description.clone(), c.previews.iter().map(|p| (p.label.clone(), p.sheet, p.size)).collect::<Vec<_>>(), c.variants.clone(), c.check.clone())
}

/// Each accepted system's components, with the drafts of it laid over: one
/// a ticket added is listed with that ticket, one it changed is marked with
/// it — in review before still being made.
pub fn mark_work(systems: &mut [DesignSystem]) {
    let mut drafts: Vec<(usize, ComponentWork)> = systems
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.draft.as_ref().map(|d| (i, ComponentWork { ticket: d.ticket, title: d.title.clone(), review: !d.working })))
        .collect();
    drafts.sort_by_key(|(_, w)| (!w.review, w.ticket));
    let mut marks: Vec<(usize, Component)> = Vec::new();
    for (accepted, system) in systems.iter().enumerate().filter(|(_, s)| s.draft.is_none()) {
        for (d, work) in &drafts {
            let draft = &systems[*d];
            if draft.project != system.project {
                continue;
            }
            for c in &draft.components {
                let changed = system.components.iter().find(|a| a.name == c.name).is_none_or(|a| shown(a) != shown(c));
                if changed && !marks.iter().any(|(i, m)| *i == accepted && m.name == c.name) {
                    marks.push((accepted, Component { work: Some(work.clone()), ..c.clone() }));
                }
            }
        }
    }
    for (i, mark) in marks {
        let list = &mut systems[i].components;
        match list.iter().position(|c| c.name == mark.name) {
            // Changed: as accepted, with who is changing it.
            Some(at) => list[at].work = mark.work,
            None if list.len() < COMPONENTS_MAX => list.push(mark),
            None => {}
        }
    }
    for s in systems.iter_mut().filter(|s| s.draft.is_none()) {
        s.components.sort_by_key(|c| (c.block, c.title.to_lowercase()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::design::DraftOf;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("equipe-gallery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn png(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(path).unwrap();
    }

    #[test]
    fn a_picture_names_its_component_and_what_it_shows() {
        assert_eq!(of_picture("components/button.png"), Some(("button".into(), None)));
        assert_eq!(of_picture("components/button/outline.png"), Some(("button".into(), Some("outline".into()))));
        assert_eq!(of_picture("components/button.hover.dark.svg"), Some(("button".into(), Some("hover · dark".into()))));
        assert_eq!(of_picture("components/EvidenceCard/default.png"), Some(("evidence-card".into(), None)));
        assert_eq!(of_picture("components/source_chip.png"), Some(("source-chip".into(), None)));
        assert_eq!(titled("question-step"), "Question Step");
        assert_eq!(sheet_width("/d/checks/components/button/sheet-1280.png"), Some(1280));
    }

    #[test]
    fn components_come_from_the_registry_the_folder_the_previews_and_the_matrix() {
        let repo = dir("read");
        let design = repo.join("design");
        std::fs::create_dir_all(design.join("components")).unwrap();
        std::fs::write(design.join("tokens.json"), "{}").unwrap();
        std::fs::write(
            repo.join("registry.json"),
            r#"{ "name": "acme", "items": [
                { "name": "button", "type": "registry:ui", "title": "Button", "description": "The one action an area asks for.", "files": [{ "path": "registry/ui/button.tsx", "type": "registry:ui" }], "meta": { "docs": "http://localhost:3000/docs/components/button" } },
                { "name": "evidence-card", "type": "registry:component", "title": "Evidence Card", "files": [] },
                { "name": "utils", "type": "registry:lib", "files": [] },
                { "name": "sign-in", "type": "registry:block", "title": "Sign in" }
            ] }"#,
        )
        .unwrap();
        std::fs::create_dir_all(repo.join("components/ui")).unwrap();
        std::fs::write(repo.join("components/ui/source-chip.tsx"), "export function SourceChip() {}").unwrap();
        png(&design.join("components/button.png"));
        png(&design.join("components/button/hover.png"));
        png(&design.join("components/question-step.png"));
        std::fs::write(design.join("components/question-step.md"), "# Question Step\n\nOne question at a time, with the step it is on.\n\nMore.").unwrap();
        // The matrix ran in a worktree: its sheets are found by name in the record's folder.
        let sheets = matrix::pictures_dir(&design, "button");
        png(&sheets.join("sheet-390.png"));
        png(&sheets.join("sheet-1280.png"));
        let report = matrix::Report {
            component: "button".into(),
            url: "http://localhost:6006/?path=/story/button".into(),
            at: 1_793_000_000,
            examples: vec!["variant=default".into(), "variant=outline".into()],
            findings: vec![matrix::Finding { level: matrix::Level::Fail, check: "contrast".into(), example: None, theme: None, width: None, words: "x".into() }],
            sheets: vec!["/elsewhere/worktree/design/checks/components/button/sheet-390.png".into(), "/elsewhere/worktree/design/checks/components/button/sheet-1280.png".into()],
            ..matrix::Report::default()
        };
        matrix::write(&design, &report).unwrap();
        let assets = super::super::folder::assets(&design);
        let found = read(&design, Some(&repo), &assets);
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["button", "evidence-card", "question-step", "source-chip", "sign-in"], "by title, blocks last; no lib");
        let button = &found[0];
        assert_eq!(button.description.as_deref(), Some("The one action an area asks for."));
        assert_eq!(button.page.as_deref(), Some("http://localhost:3000/docs/components/button"), "the registry's page before the matrix's");
        assert_eq!(button.source.as_deref(), Some("registry/ui/button.tsx"));
        let shown: Vec<(Option<&str>, Option<u32>)> = button.previews.iter().map(|p| (p.label.as_deref(), p.sheet)).collect();
        assert_eq!(shown, [(None, None), (Some("hover"), None), (None, Some(1280)), (None, Some(390))]);
        assert_eq!(button.check, Some(ComponentCheck { at: 1_793_000_000, fails: 1, notes: 0 }));
        assert_eq!(button.variants, ["variant=default", "variant=outline"]);
        assert!(button.states.contains(&"focus-visible".to_owned()), "the catalog's states");
        assert!(found[1].previews.is_empty() && found[1].check.is_none(), "listed with nothing to show yet");
        assert_eq!(found[2].description.as_deref(), Some("One question at a time, with the step it is on."), "the note beside its preview");
        assert_eq!(found[3].source.as_deref(), Some("components/ui/source-chip.tsx"));
        assert!(found[4].block);
        // `read_image` may give the sheets; the stamp follows the record and the registry.
        assert_eq!(pictures(&design).len(), 2);
        let watched = watched(&design, Some(&repo));
        assert!(watched.contains(&repo.join("registry.json")) && watched.iter().any(|p| p.ends_with("checks/components/button.json")));
        // The squad's own: its pictures alone.
        let squad = read(&design, None, &assets);
        assert_eq!(squad.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["button", "question-step"]);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_component_a_ticket_makes_or_changes_says_which_ticket() {
        let c = |name: &str, size: u64| Component { name: name.into(), title: titled(name), previews: vec![Preview { path: format!("/d/components/{name}.png"), size, ..Preview::default() }], ..Component::default() };
        let system = |draft: Option<(u32, bool)>, components: Vec<Component>| DesignSystem {
            project: Some("web".into()),
            draft: draft.map(|(ticket, working)| DraftOf { ticket, title: format!("T{ticket}"), working }),
            components,
            ..DesignSystem::default()
        };
        let mut systems = vec![
            system(None, vec![c("button", 10), c("card", 10)]),
            system(Some((412, true)), vec![c("button", 10), c("card", 10), c("source-chip", 5), c("question-step", 5)]),
            system(Some((170, false)), vec![c("button", 12), c("card", 10), c("question-step", 6)]),
            DesignSystem { project: None, components: vec![c("button", 1)], ..DesignSystem::default() },
        ];
        mark_work(&mut systems);
        let marked: Vec<(&str, Option<(u32, bool)>)> = systems[0].components.iter().map(|c| (c.name.as_str(), c.work.as_ref().map(|w| (w.ticket, w.review)))).collect();
        assert_eq!(marked, [("button", Some((170, true))), ("card", None), ("question-step", Some((170, true))), ("source-chip", Some((412, false)))], "in review before still being made");
        assert_eq!(systems[0].components[0].previews[0].size, 10, "a changed one shows as accepted");
        assert!(systems[1].components.iter().all(|c| c.work.is_none()), "a draft's own are left as they are");
        assert!(systems[3].components[0].work.is_none(), "another project's drafts don't count");
    }
}
