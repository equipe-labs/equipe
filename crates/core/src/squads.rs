//! Squads: which projects (repositories) a team works on.
//!
//! A squad is a team that outlives any one project — the way a company's squad
//! owns a product's web app, API and site at once. Its members, board and
//! record live in `~/.equipe/squads/<name>/`; each project belongs to one squad.
//! Screens read and write this registry directly (it is just the squad's
//! database); the daemon reads it fresh whenever it needs it.

use std::path::{Path, PathBuf};

use crate::journal::{Journal, squad_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquadSummary {
    pub name: String,
    pub projects: Vec<PathBuf>,
}

/// Every squad under `home`, by name.
pub fn list(home: &Path) -> Vec<SquadSummary> {
    let Ok(dirs) = std::fs::read_dir(home.join("squads")) else {
        return Vec::new();
    };
    let mut squads: Vec<SquadSummary> = dirs
        .flatten()
        .filter(|d| d.path().join("squad.db").exists())
        .filter_map(|d| {
            let name = d.file_name().to_string_lossy().into_owned();
            let projects = Journal::for_squad(home, &name).and_then(|j| j.projects()).ok()?;
            Some(SquadSummary { name, projects })
        })
        .collect();
    squads.sort_by(|a, b| a.name.cmp(&b.name));
    squads
}

/// The squad `project` belongs to.
pub fn find(home: &Path, project: &Path) -> Option<String> {
    list(home).into_iter().find(|s| s.projects.iter().any(|p| p == project)).map(|s| s.name)
}

/// A squad name is a short slug: it names a directory and is typed in commands.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// A name for a new squad, from the project it starts with: `My App` → `my-app`.
pub fn suggest_name(home: &Path, project: &Path) -> String {
    let raw = project.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut slug = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let base = if slug.is_empty() { "squad" } else { slug };
    let base: String = base.chars().take(32).collect();
    let taken = list(home);
    (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|name| !taken.iter().any(|s| &s.name == name))
        .expect("some name is free")
}

/// Start a squad with its first project.
pub fn create(home: &Path, name: &str, project: &Path) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("'{name}' isn't a squad name — use lowercase letters, digits and dashes"));
    }
    if squad_dir(home, name).join("squad.db").exists() {
        return Err(format!("there is already a squad called {name}"));
    }
    add_project(home, name, project)
}

/// Add a project to a squad. A project belongs to one squad.
pub fn add_project(home: &Path, name: &str, project: &Path) -> Result<(), String> {
    if let Some(other) = find(home, project).filter(|other| other != name) {
        return Err(format!("{} already belongs to the squad {other}", project.display()));
    }
    let journal = Journal::for_squad(home, name).map_err(|err| err.to_string())?;
    journal.add_project(project).map_err(|err| err.to_string())
}

/// Take a project off a squad. Its tickets and branches stay.
pub fn remove_project(home: &Path, name: &str, project: &Path) -> Result<(), String> {
    let journal = Journal::for_squad(home, name).map_err(|err| err.to_string())?;
    journal.remove_project(project).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn home() -> PathBuf {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-squads-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn one_squad_works_on_several_projects() {
        let home = home();
        create(&home, "acme", Path::new("/code/app")).unwrap();
        add_project(&home, "acme", Path::new("/code/site")).unwrap();
        assert_eq!(find(&home, Path::new("/code/site")).as_deref(), Some("acme"));
        assert_eq!(find(&home, Path::new("/code/other")), None);
        assert_eq!(list(&home), [SquadSummary { name: "acme".into(), projects: vec!["/code/app".into(), "/code/site".into()] }]);
        remove_project(&home, "acme", Path::new("/code/site")).unwrap();
        assert_eq!(find(&home, Path::new("/code/site")), None, "taken off, it belongs to no squad");
    }

    #[test]
    fn a_project_belongs_to_one_squad() {
        let home = home();
        create(&home, "acme", Path::new("/code/app")).unwrap();
        let err = create(&home, "other", Path::new("/code/app")).unwrap_err();
        assert!(err.contains("already belongs to the squad acme"), "{err}");
        assert!(create(&home, "acme", Path::new("/code/x")).unwrap_err().contains("already a squad"));
    }

    #[test]
    fn names_are_slugs_suggested_from_the_project() {
        let home = home();
        assert_eq!(suggest_name(&home, Path::new("/code/My App!")), "my-app");
        create(&home, "my-app", Path::new("/code/My App!")).unwrap();
        assert_eq!(suggest_name(&home, Path::new("/elsewhere/my app")), "my-app-2");
        assert_eq!(suggest_name(&home, Path::new("/")), "squad");
        assert!(valid_name("equipe-2") && !valid_name("Equipe") && !valid_name("") && !valid_name("a/b"));
        assert!(create(&home, "Bad Name", Path::new("/x")).is_err());
    }
}
