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

/// Start a squad with its first project, and its home.
pub fn create(home: &Path, name: &str, project: &Path) -> Result<(), String> {
    start(home, name, Some(project))
}

/// Start a squad with no project yet: its home is where it works until one joins.
pub fn create_without_project(home: &Path, name: &str) -> Result<(), String> {
    start(home, name, None)
}

fn start(home: &Path, name: &str, project: Option<&Path>) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("'{name}' isn't a squad name — use lowercase letters, digits and dashes"));
    }
    if squad_dir(home, name).join("squad.db").exists() {
        return Err(format!("there is already a squad called {name}"));
    }
    match project {
        Some(project) => add_project(home, name, project)?,
        None => drop(Journal::for_squad(home, name).map_err(|err| err.to_string())?),
    }
    crate::squad_home::ensure(home, name, None).map(|_| ())
}

/// The squad to open where no project says which: the one opened last, else
/// the only squad, or the first by name.
pub fn default_squad(home: &Path) -> Option<String> {
    let squads = list(home);
    last_squad(home).filter(|last| squads.iter().any(|s| s.name == *last)).or_else(|| squads.first().map(|s| s.name.clone()))
}

/// What equipe remembers between runs, in `<home>/state.json`: the squad
/// opened last, and the repositories the person said not to add.
fn state(home: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read(home.join("state.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn save_state(home: &Path, state: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|err| err.to_string())?;
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(state)).map_err(|err| err.to_string())?;
    // Whole or not at all: two screens may write it at once.
    let tmp = home.join(format!("state.json.{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|err| err.to_string())?;
    std::fs::rename(&tmp, home.join("state.json")).map_err(|err| err.to_string())
}

/// The squad opened last, as remembered — it may since be gone.
pub fn last_squad(home: &Path) -> Option<String> {
    state(home).get("last_squad")?.as_str().map(str::to_owned)
}

/// Remember `name` as the squad opened last.
pub fn remember_last(home: &Path, name: &str) -> Result<(), String> {
    let mut state = state(home);
    if state.get("last_squad").and_then(|v| v.as_str()) == Some(name) {
        return Ok(());
    }
    state.insert("last_squad".into(), name.into());
    save_state(home, state)
}

/// The person said not to add `repo` to a squad: don't offer it again.
pub fn is_declined(home: &Path, repo: &Path) -> bool {
    let repo = repo.to_string_lossy();
    state(home).get("declined").and_then(|v| v.as_array()).is_some_and(|d| d.iter().any(|r| r.as_str() == Some(&repo)))
}

/// Remember that `repo` isn't to be offered again.
pub fn decline(home: &Path, repo: &Path) -> Result<(), String> {
    if is_declined(home, repo) {
        return Ok(());
    }
    let mut state = state(home);
    let declined = state.entry("declined").or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if !declined.is_array() {
        *declined = serde_json::Value::Array(Vec::new());
    }
    declined.as_array_mut().expect("an array").push(repo.to_string_lossy().into_owned().into());
    save_state(home, state)
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
        assert!(crate::squad_home::path(&home, "acme").join(".git").is_dir(), "a new squad has its home");
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

    #[test]
    fn a_squad_may_start_with_no_project() {
        let home = home();
        create_without_project(&home, "solo").unwrap();
        assert_eq!(list(&home), [SquadSummary { name: "solo".into(), projects: vec![] }]);
        assert!(crate::squad_home::path(&home, "solo").join(".git").is_dir(), "it has its home");
        assert!(create_without_project(&home, "solo").unwrap_err().contains("already a squad"));
    }

    #[test]
    fn the_squad_opened_last_opens_where_no_project_says() {
        let home = home();
        assert_eq!(default_squad(&home), None, "no squad, nothing to open");
        create_without_project(&home, "beta").unwrap();
        create_without_project(&home, "acme").unwrap();
        assert_eq!(default_squad(&home).as_deref(), Some("acme"), "none opened yet: the first by name");
        remember_last(&home, "beta").unwrap();
        assert_eq!(default_squad(&home).as_deref(), Some("beta"));
        remember_last(&home, "gone").unwrap();
        assert_eq!(default_squad(&home).as_deref(), Some("acme"), "a squad since removed is passed over");
    }

    #[test]
    fn a_repository_said_no_to_is_remembered() {
        let home = home();
        remember_last(&home, "acme").unwrap();
        assert!(!is_declined(&home, Path::new("/code/site")));
        decline(&home, Path::new("/code/site")).unwrap();
        decline(&home, Path::new("/code/site")).unwrap();
        assert!(is_declined(&home, Path::new("/code/site")));
        assert!(!is_declined(&home, Path::new("/code/app")));
        assert_eq!(last_squad(&home).as_deref(), Some("acme"), "one remembered thing keeps the other");
        let text = std::fs::read_to_string(home.join("state.json")).unwrap();
        assert_eq!(text.matches("/code/site").count(), 1, "{text}");
    }
}
