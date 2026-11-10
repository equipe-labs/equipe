//! Squads: which projects (repositories) a team works on.
//!
//! A squad is a team that outlives any one project — the way a company's squad
//! owns a product's web app, API and site at once. Its members, board and
//! record live in `~/.equipe/squads/<name>/`; each project belongs to one squad.
//! Screens read and write this registry directly (it is just the squad's
//! database); the daemon reads it fresh whenever it needs it.

use std::path::{Path, PathBuf};

use crate::journal::{Journal, squad_dir};
use crate::squad_home::git;

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

/// Whether there is a squad called `name` — with projects or not yet.
pub fn exists(home: &Path, name: &str) -> bool {
    valid_name(name) && squad_dir(home, name).join("squad.db").exists()
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
    let dir = squad_dir(home, name);
    if dir.join("squad.db").exists() {
        return Err(format!("there is already a squad called {name}"));
    }
    let new = !dir.exists();
    let made = match project {
        Some(project) => add_project(home, name, project),
        None => Ok(()),
    }
    .and_then(|()| {
        let journal = Journal::for_squad(home, name).map_err(|err| err.to_string())?;
        // Its heartbeat: a weekly check-in, on — the person's to change or turn
        // off — and the board counted as quiet from now, not from ever.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let lang = crate::i18n::Lang::resolve_in(home, crate::i18n::Front::App);
        let mut check_in = crate::heartbeat::weekly_check_in(lang);
        check_in.next_run = Some(crate::routines::next_run(check_in.when, now, &crate::routines::Local));
        journal.set_routines(&[check_in]).map_err(|err| err.to_string())?;
        journal.set_active_at(now).map_err(|err| err.to_string())?;
        drop(journal);
        crate::squad_home::ensure(home, name, None).map(|_| ())
    });
    // Not half made (git missing on a new Mac, say): the name stays free to try again.
    if made.is_err() && new {
        let _ = std::fs::remove_dir_all(&dir);
    }
    made
}

/// The squad to open where no project says which: the one opened last, else
/// the only squad, or the first by name.
pub fn default_squad(home: &Path) -> Option<String> {
    let squads = list(home);
    last_squad(home).filter(|last| squads.iter().any(|s| s.name == *last)).or_else(|| squads.first().map(|s| s.name.clone()))
}

/// What equipe remembers between runs, in `<home>/state.json`: the squad
/// opened last, and the repositories the person said not to add.
pub(crate) fn state(home: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read(home.join("state.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

pub(crate) fn save_state(home: &Path, state: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
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

/// A folder the person names, added to `squad` as a project: the repository
/// it is in, or — a folder with no history yet, made if it isn't there —
/// started as one with a first commit, so members can branch from it. `~`
/// is this machine's home. Never home itself or a folder above it, nor
/// equipe's own. Blocking (git): off the daemon's loop. Where it went.
pub fn add_folder(home: &Path, squad: &str, typed: &str, user_home: Option<&Path>) -> Result<PathBuf, String> {
    let typed = typed.trim();
    let path = match (typed.strip_prefix("~"), user_home) {
        (Some(rest), Some(user)) if rest.is_empty() || rest.starts_with('/') => user.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(typed),
    };
    if !path.is_absolute() {
        return Err(format!("{typed}: a full path, or one from ~"));
    }
    if !path.exists() {
        path.parent().filter(|p| p.is_dir()).ok_or_else(|| format!("{}: its folder isn't there", path.display()))?;
        std::fs::create_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let path = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{}: not a folder", path.display()));
    }
    let too_wide = |p: &Path| p.components().count() <= 2 || user_home.and_then(|u| u.canonicalize().ok()).is_some_and(|u| u.starts_with(p));
    let equipes = |p: &Path| home.canonicalize().is_ok_and(|h| p.starts_with(&h) || h.starts_with(p));
    if too_wide(&path) || equipes(&path) {
        return Err(format!("{}: too wide a folder for a project — pick the project's own", path.display()));
    }
    let repo = match crate::worktree::repo_root(&path) {
        Ok(root) if !too_wide(&root) => root.canonicalize().unwrap_or(root),
        Ok(root) => return Err(format!("{} is inside {}, a repository too wide for a project", path.display(), root.display())),
        Err(_) => {
            git(&path, &["init", "-q", "-b", "main"])?;
            path.clone()
        }
    };
    // Members branch from a commit: a repository with none gets its first.
    if git(&repo, &["rev-parse", "-q", "--verify", "HEAD"]).is_err() {
        git(&repo, &["add", "-A"])?;
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "equipe: the project's start"])?;
    }
    add_project(home, squad, &repo)?;
    Ok(repo)
}

/// A project the PM adds when the person asks it to in words: a repository
/// already there, or a new folder for a new one — never a folder of the
/// person's files that would be committed as a repository without them, nor
/// a hidden one. Otherwise as [`add_folder`].
pub fn add_folder_asked(home: &Path, squad: &str, typed: &str, user_home: Option<&Path>) -> Result<PathBuf, String> {
    let shown = typed.trim();
    if Path::new(shown).components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
        return Err(format!("{shown}: a hidden folder isn't a project"));
    }
    let path = match (shown.strip_prefix("~"), user_home) {
        (Some(rest), Some(user)) if rest.is_empty() || rest.starts_with('/') => user.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(shown),
    };
    if path.exists() && crate::worktree::repo_root(&path).is_err() {
        return Err(format!(
            "{}: a folder that isn't a git repository yet — adding it would commit its files as one. The person adds it \
themselves with the + beside the sidebar's projects.",
            path.display()
        ));
    }
    add_folder(home, squad, shown, user_home)
}

/// Take a project off a squad. Its tickets and branches stay.
pub fn remove_project(home: &Path, name: &str, project: &Path) -> Result<(), String> {
    let journal = Journal::for_squad(home, name).map_err(|err| err.to_string())?;
    journal.remove_project(project).map_err(|err| err.to_string())
}

// ── deleting a squad ──
//
// What a deleted squad can make again goes at once: the worktrees its
// tickets used (their branches stay, in the projects' repositories), its
// members' build caches, and build output in its folder. The rest of its
// folder moves, whole, to `<home>/deleted/<name>-<YYYYMMDD-HHMMSS>` — on the
// same disk as the squad, so the move is a rename that can't stop half-way,
// and on the machine the squad lived on. Moving it back to
// `<home>/squads/<name>` (with the daemon stopped) brings the squad back as
// it was: its board, history, members' memory and home. It is kept for
// [`KEPT_DAYS`], then the daily tidy deletes it ([`purge_deleted`]); deleted
// for good, it goes at once. The projects' checkouts are never touched.

/// How long a deleted squad's folder is kept to restore from.
pub const KEPT_DAYS: u64 = 7;
const KEPT_FOR: u64 = KEPT_DAYS * 24 * 60 * 60;
/// The file in a deleted folder that says when it was deleted, in unix seconds.
const DELETED_AT: &str = ".deleted-at";
/// Folders in a squad's own folder that builds made, and make again.
const REMADE: [&str; 3] = ["node_modules", "target", ".next"];
/// How long measuring what a deletion gives back may take, all told.
const MEASURE_FOR: std::time::Duration = std::time::Duration::from_secs(60);

/// Where deleted squads' folders go.
pub fn deleted_dir(home: &Path) -> PathBuf {
    home.join("deleted")
}

/// Squads being deleted now in this process, by folder: nothing opens their
/// journal again meanwhile (which would make the squad anew).
static GOING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Say that squad `name` is being deleted (`going`), or no longer is.
pub fn mark_going(home: &Path, name: &str, going: bool) {
    let dir = squad_dir(home, name);
    let mut list = GOING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    list.retain(|d| *d != dir);
    if going {
        list.push(dir);
    }
}

/// Whether squad `name` is being deleted, or was and hasn't been made again:
/// its journal isn't to be opened — opening it would make it anew.
pub fn is_gone(home: &Path, name: &str) -> bool {
    let dir = squad_dir(home, name);
    if GOING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&dir) {
        return true;
    }
    !dir.join("squad.db").exists() && was_deleted(home, name)
}

/// Whether a folder of squad `name`'s is among the deleted ones.
fn was_deleted(home: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(deleted_dir(home)) else {
        return false;
    };
    entries.flatten().any(|e| e.file_name().to_str().and_then(|n| n.strip_prefix(name)).and_then(|n| n.strip_prefix('-')).is_some_and(is_stamp))
}

/// `20261110-093005`, or the same with `-2`… after it.
fn is_stamp(text: &str) -> bool {
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let mut parts = text.split('-');
    let (Some(day), Some(time)) = (parts.next(), parts.next()) else { return false };
    digits(day, 8) && digits(time, 6) && parts.next().is_none_or(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) && parts.next().is_none()
}

/// The moment `unix` as a deleted folder's name ends: `20261110-093005`, the machine's local time.
fn stamp(unix: u64) -> String {
    match equipe_providers::platform::local_time(unix) {
        Some(t) => format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second),
        None => format!("{unix}"),
    }
}

/// What deleting a squad did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    /// Where its folder went — deleted too when `purged`.
    pub moved_to: PathBuf,
    /// Its tickets' worktrees, removed; their branches stay.
    pub worktrees_removed: Vec<PathBuf>,
    /// Its tickets' worktrees another squad's ticket uses too: left.
    pub worktrees_kept: Vec<PathBuf>,
    /// The squad to open now, if any is left: what `state.json` remembers.
    pub next: Option<String>,
    /// The bytes given back on the disk.
    pub freed: u64,
    /// The bytes kept to restore from, for [`KEPT_DAYS`].
    pub kept: u64,
    /// Deleted for good: nothing kept.
    pub purged: bool,
}

/// What deleting squad `name` takes away at once.
struct Plan {
    /// Its worktrees no other squad's ticket uses: removed.
    worktrees: Vec<PathBuf>,
    /// Its worktrees another squad's ticket uses too: left.
    shared: Vec<PathBuf>,
    /// The repositories they belong to, to forget them after.
    repos: Vec<PathBuf>,
    /// Where its home's worktrees go: removed once empty.
    home_worktrees: PathBuf,
    /// Its members' build caches.
    cache: PathBuf,
}

/// What deleting squad `name` would take away at once, looked at only:
/// its tickets' worktrees under `worktrees/`, its home's, and every worktree
/// equipe made of its projects' repositories (a ticket's, a run of checks')
/// that no other squad's project shares — never a project's own checkout.
fn plan(home: &Path, name: &str) -> Plan {
    let worktrees_root = home.join("worktrees");
    let checks_root = crate::runtime::checks_scratch(home);
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    let tickets_worktrees = |squad: &str| -> Vec<PathBuf> {
        Journal::open(&squad_dir(home, squad).join("squad.db")).and_then(|j| j.tickets()).unwrap_or_default().into_iter().filter_map(|t| t.worktree).collect()
    };
    // Its tickets' worktrees under `worktrees/`, and every one of its home's.
    let mut own: Vec<PathBuf> = tickets_worktrees(name).into_iter().filter(|w| w.starts_with(&worktrees_root)).collect();
    let home_worktrees = crate::worktree::worktrees_of(&crate::squad_home::path(home, name), home);
    if let Ok(entries) = std::fs::read_dir(&home_worktrees) {
        own.extend(entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()));
    }
    // Every worktree equipe made of its projects' repositories, unless another
    // squad works on the same repository.
    let squads = list(home);
    let repo_of = |project: &Path| crate::worktree::repo_root(project).ok().map(|r| canon(&r));
    let others_repos: Vec<PathBuf> = squads.iter().filter(|s| s.name != name).flat_map(|s| s.projects.iter().filter_map(|p| repo_of(p))).collect();
    let mut repos: Vec<PathBuf> = squads.iter().filter(|s| s.name == name).flat_map(|s| s.projects.iter().filter_map(|p| repo_of(p))).filter(|r| !others_repos.contains(r)).collect();
    repos.sort();
    repos.dedup();
    let (roots, project_roots) = ([canon(&worktrees_root), canon(&checks_root)], repos.clone());
    for repo in &repos {
        own.extend(crate::worktree::listed(repo).into_iter().filter(|w| {
            let at = canon(w);
            roots.iter().any(|r| at.starts_with(r)) && !project_roots.contains(&at)
        }));
    }
    own.sort();
    own.dedup();
    let others: Vec<PathBuf> = squads.iter().filter(|s| s.name != name).flat_map(|s| tickets_worktrees(&s.name)).map(|w| canon(&w)).collect();
    let (shared, worktrees): (Vec<PathBuf>, Vec<PathBuf>) = own.into_iter().partition(|w| {
        let at = canon(w);
        others.iter().any(|o| o.starts_with(&at) || at.starts_with(o))
    });
    Plan { worktrees, shared, repos, home_worktrees, cache: home.join("cache").join("build").join(name) }
}

/// The build output in a squad's folder ([`REMADE`]): never inside `.git`,
/// links never followed, nothing looked for inside what is found.
fn remade(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            if REMADE.iter().any(|r| name == *r) {
                found.push(entry.path());
            } else if name != ".git" {
                stack.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

/// What deleting squad `name` would give back, measured within `budget`.
/// Blocking (git, folders): off the daemon's loop.
pub fn footprint(home: &Path, name: &str, budget: std::time::Duration) -> Result<equipe_protocol::rpc::SquadFootprint, String> {
    if !exists(home, name) {
        return Err(format!("there is no squad called {name}"));
    }
    let plan = plan(home, name);
    let mut sizer = crate::disk::Sizer::new(budget);
    let dir = squad_dir(home, name);
    let built: u64 = remade(&dir).iter().map(|d| sizer.size(d)).sum();
    let mut freed: u64 = plan.worktrees.iter().map(|w| sizer.size(w)).sum();
    freed += sizer.size(&plan.cache) + built;
    let kept = sizer.size(&dir).saturating_sub(built);
    Ok(equipe_protocol::rpc::SquadFootprint { freed, kept, complete: sizer.complete() })
}

/// Delete squad `name` at `now` (unix seconds): what can be made again goes —
/// its worktrees no other squad's ticket uses (their branches stay, in the
/// projects' repositories, which are otherwise never touched), its members'
/// build caches, build output in its folder — and its folder moves to
/// [`deleted_dir`], kept for [`KEPT_DAYS`]; `purge` deletes it too, now. If
/// it was the squad opened last, the one to open now is remembered instead
/// (or none, when it was the last squad). Its members must have stopped and
/// its journal closed: the daemon's to do first ([`crate::runtime`]).
/// Blocking (git, folders): off the daemon's loop.
pub fn delete(home: &Path, name: &str, now: u64, purge: bool) -> Result<Deleted, String> {
    if !exists(home, name) {
        return Err(format!("there is no squad called {name}"));
    }
    let dir = squad_dir(home, name);
    let plan = plan(home, name);
    let mut sizer = crate::disk::Sizer::new(MEASURE_FOR);
    let mut freed = 0;
    let mut worktrees_removed = Vec::new();
    for worktree in plan.worktrees {
        if !worktree.exists() {
            continue;
        }
        let size = sizer.size(&worktree);
        match crate::worktree::remove_at(&worktree) {
            Ok(()) => {
                freed += size;
                worktrees_removed.push(worktree);
            }
            Err(why) => crate::daemon_log!("equipe: deleting {name}: {} stays: {why}", worktree.display()),
        }
    }
    for repo in &plan.repos {
        crate::worktree::prune(repo);
    }
    let _ = std::fs::remove_dir(&plan.home_worktrees);
    // Its members' builds, the checks' among them: made again whenever wanted.
    let size = sizer.size(&plan.cache);
    if std::fs::remove_dir_all(&plan.cache).is_ok() {
        freed += size;
    }
    // The folder, whole, under a name no other deleted folder has.
    let deleted = deleted_dir(home);
    std::fs::create_dir_all(&deleted).map_err(|e| format!("{}: {e}", deleted.display()))?;
    let base = format!("{name}-{}", stamp(now));
    let moved_to = (1..)
        .map(|n| deleted.join(if n == 1 { base.clone() } else { format!("{base}-{n}") }))
        .find(|p| !p.exists())
        .expect("some name is free");
    std::fs::rename(&dir, &moved_to).map_err(|e| format!("{} → {}: {e}", dir.display(), moved_to.display()))?;
    let _ = std::fs::write(moved_to.join(DELETED_AT), now.to_string());
    for built in remade(&moved_to) {
        let size = sizer.size(&built);
        if std::fs::remove_dir_all(&built).is_ok() {
            freed += size;
        }
    }
    let mut kept = sizer.size(&moved_to);
    if purge {
        match std::fs::remove_dir_all(&moved_to) {
            Ok(()) => freed += std::mem::take(&mut kept),
            // What is left goes with the daily tidy, a week on.
            Err(why) => crate::daemon_log!("equipe: deleting {name} for good: {} stays for now: {why}", moved_to.display()),
        }
    }
    // What opens next: the one remembered, unless that was this one.
    let mut state = state(home);
    let next = default_squad(home);
    if state.get("last_squad").and_then(|v| v.as_str()) == Some(name) {
        match &next {
            Some(next) => state.insert("last_squad".into(), next.clone().into()),
            None => state.remove("last_squad"),
        };
        save_state(home, state)?;
    }
    Ok(Deleted { moved_to, worktrees_removed, worktrees_kept: plan.shared, next, freed, kept, purged: purge && kept == 0 })
}

/// Deleted squads' folders kept past [`KEPT_DAYS`], deleted, at `now` (unix
/// seconds). Only folders named as a deleted squad's are touched; one with no
/// word of when it was deleted (deleted before equipe kept one) starts its
/// week now. What went, one line each. Blocking: off the daemon's loop.
pub fn purge_deleted(home: &Path, now: u64) -> Vec<String> {
    let mut done = Vec::new();
    for entry in std::fs::read_dir(deleted_dir(home)).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let named = name.char_indices().filter(|(_, c)| *c == '-').any(|(i, _)| is_stamp(&name[i + 1..]));
        if !named || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        let at = std::fs::read_to_string(path.join(DELETED_AT)).ok().and_then(|t| t.trim().parse::<u64>().ok());
        let Some(at) = at else {
            let _ = std::fs::write(path.join(DELETED_AT), now.to_string());
            continue;
        };
        if now.saturating_sub(at) >= KEPT_FOR && std::fs::remove_dir_all(&path).is_ok() {
            done.push(format!("removed {}, a squad deleted {} days ago", path.display(), now.saturating_sub(at) / 86_400));
        }
    }
    done
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
    fn a_deleted_folder_is_named_by_its_squad_and_when_and_only_its_own_counts() {
        assert!(is_stamp("20261110-093005") && is_stamp("20261110-093005-2"));
        assert!(!is_stamp("20261110") && !is_stamp("2026111-093005") && !is_stamp("20261110-093005-") && !is_stamp("x-093005"));
        let home = home();
        create_without_project(&home, "acme").unwrap();
        create_without_project(&home, "acme-2").unwrap();
        create_without_project(&home, "beta").unwrap();
        remember_last(&home, "beta").unwrap();
        let first = delete(&home, "acme-2", 1_762_000_000, false).unwrap();
        assert!(first.moved_to.file_name().unwrap().to_string_lossy().starts_with("acme-2-"));
        assert_eq!(first.next.as_deref(), Some("beta"), "the one remembered, as it was");
        assert_eq!(last_squad(&home).as_deref(), Some("beta"), "not this one: left as it was");
        assert!(is_gone(&home, "acme-2") && !is_gone(&home, "acme"), "acme-2's folder isn't acme's");
        // The same name, the same second: a name of its own.
        create_without_project(&home, "acme-2").unwrap();
        assert!(!is_gone(&home, "acme-2"), "made again: not gone");
        let second = delete(&home, "acme-2", 1_762_000_000, false).unwrap();
        assert_ne!(first.moved_to, second.moved_to);
        assert!(second.moved_to.to_string_lossy().ends_with("-2"));
        assert!(delete(&home, "nope", 0, false).unwrap_err().contains("nope"));
        mark_going(&home, "beta", true);
        assert!(is_gone(&home, "beta"), "while it goes");
        mark_going(&home, "beta", false);
        assert!(!is_gone(&home, "beta"));
    }

    /// A repository with one commit, at `path`.
    fn repo_at(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(path.join("README.md"), "hi\n").unwrap();
        git(path, &["add", "."]).unwrap();
        git(path, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]).unwrap();
        path.canonicalize().unwrap()
    }

    fn file(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn deleting_gives_back_what_can_be_made_again_at_once_and_keeps_the_rest_to_restore() {
        let home = home().canonicalize().unwrap();
        let shop = repo_at(&home.join("code/shop"));
        let site = repo_at(&home.join("code/site"));
        create(&home, "acme", &shop).unwrap();
        create(&home, "beta", &site).unwrap();
        // A ticket's worktree and a run of checks' — no ticket names them — and another squad's.
        let ticket = home.join("worktrees/shop/t3-abc");
        let checks = crate::runtime::checks_scratch(&home).join("t3-merge-abc");
        let theirs = home.join("worktrees/site/t1-def");
        for (repo, at) in [(&shop, &ticket), (&shop, &checks), (&site, &theirs)] {
            git(repo, &["worktree", "add", "-q", "--detach", &at.to_string_lossy()]).unwrap();
        }
        file(&ticket.join("node_modules/pkg/index.js"), 64 * 1024);
        file(&home.join("cache/build/acme/backend/debug/app"), 64 * 1024);
        let dir = squad_dir(&home, "acme");
        file(&dir.join("home/site/node_modules/pkg/index.js"), 64 * 1024);
        file(&dir.join("home/tool/target/debug/tool"), 64 * 1024);
        file(&dir.join("home/notes/plan.md"), 100);
        file(&dir.join("members/backend/memory/MEMORY.md"), 100);
        let look = footprint(&home, "acme", std::time::Duration::from_secs(30)).unwrap();
        assert!(look.complete && look.freed >= 4 * 64 * 1024 && look.kept > 0, "{look:?}");
        assert!(footprint(&home, "nope", std::time::Duration::from_secs(1)).is_err());

        let deleted = delete(&home, "acme", 1_762_000_000, false).unwrap();
        assert!(!ticket.exists() && !checks.exists(), "its worktrees, gone");
        let listed = git(&shop, &["worktree", "list", "--porcelain"]).unwrap();
        assert!(!listed.contains("t3-abc") && !listed.contains("t3-merge-abc"), "and forgotten by git: {listed}");
        assert!(shop.join("README.md").exists(), "the project's own checkout, untouched");
        assert!(theirs.exists(), "another squad's worktree stays");
        assert!(!home.join("cache/build/acme").exists(), "its build cache, gone");
        let kept = &deleted.moved_to;
        assert!(kept.join("squad.db").exists() && kept.join("home/.git").exists() && kept.join("home/notes/plan.md").exists());
        assert!(kept.join("members/backend/memory/MEMORY.md").exists(), "members' memory, kept");
        assert!(!kept.join("home/site/node_modules").exists() && !kept.join("home/tool/target").exists(), "build output, gone");
        assert!(deleted.freed >= look.freed / 2 && deleted.kept > 0 && !deleted.purged, "{deleted:?}");
        assert_eq!(std::fs::read_to_string(kept.join(DELETED_AT)).unwrap(), "1762000000");
        assert!(purge_deleted(&home, 1_762_000_000 + KEPT_FOR - 1).is_empty() && kept.exists(), "kept for its week");
        assert_eq!(purge_deleted(&home, 1_762_000_000 + KEPT_FOR).len(), 1);
        assert!(!kept.exists(), "then gone");
    }

    #[test]
    fn deleted_for_good_nothing_is_kept() {
        let home = home();
        create_without_project(&home, "acme").unwrap();
        file(&squad_dir(&home, "acme").join("home/notes/plan.md"), 100);
        let deleted = delete(&home, "acme", 1_762_000_000, true).unwrap();
        assert!(deleted.purged && deleted.kept == 0 && deleted.freed > 0, "{deleted:?}");
        assert!(!deleted.moved_to.exists() && !exists(&home, "acme"));
    }

    #[test]
    fn the_daily_tidy_deletes_only_deleted_squads_past_their_week() {
        let home = home();
        let deleted = deleted_dir(&home);
        let now = 1_762_000_000u64;
        for (name, at) in [("old-20261101-090000", Some(now - KEPT_FOR - 60)), ("new-20261110-090000", Some(now - 86_400)), ("before-20261001-090000", None)] {
            std::fs::create_dir_all(deleted.join(name)).unwrap();
            if let Some(at) = at {
                std::fs::write(deleted.join(name).join(DELETED_AT), at.to_string()).unwrap();
            }
        }
        std::fs::create_dir_all(deleted.join("notes")).unwrap();
        let done = purge_deleted(&home, now);
        assert_eq!(done.len(), 1, "{done:?}");
        assert!(done[0].contains("old-20261101-090000") && done[0].contains("7 days"), "{done:?}");
        assert!(deleted.join("new-20261110-090000").exists() && deleted.join("notes").exists(), "not yet; not a deleted squad's");
        // Deleted before equipe said when: its week starts now.
        assert_eq!(std::fs::read_to_string(deleted.join("before-20261001-090000").join(DELETED_AT)).unwrap(), now.to_string());
        assert!(purge_deleted(&home, now + KEPT_FOR).iter().any(|l| l.contains("before-20261001-090000")));
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

    #[test]
    fn the_pm_adds_a_repository_or_a_new_folder_but_never_a_folder_of_the_persons_files_or_a_hidden_one() {
        let home = home();
        let user = home.with_file_name(format!("{}-pmuser", home.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(user.join("code/site")).unwrap();
        git(&user.join("code/site"), &["init", "-q", "-b", "main"]).unwrap();
        std::fs::create_dir_all(user.join("Documents/taxes")).unwrap();
        std::fs::write(user.join("Documents/taxes/2026.pdf"), "x").unwrap();
        std::fs::create_dir_all(user.join(".ssh")).unwrap();
        crate::squad_home::ensure(&home, "acme", None).ok();
        let site = add_folder_asked(&home, "acme", "~/code/site", Some(&user)).unwrap();
        assert_eq!(site, user.join("code/site").canonicalize().unwrap());
        let fresh = add_folder_asked(&home, "acme", "~/code/new-app", Some(&user)).unwrap();
        assert!(git(&fresh, &["rev-parse", "HEAD"]).is_ok(), "a new project, started");
        let plain = add_folder_asked(&home, "acme", "~/Documents/taxes", Some(&user)).unwrap_err();
        assert!(plain.contains("isn't a git repository yet") && plain.contains("+"), "{plain}");
        assert!(!user.join("Documents/taxes/.git").exists(), "its files weren't committed");
        assert!(add_folder_asked(&home, "acme", "~/.ssh", Some(&user)).unwrap_err().contains("hidden"));
        assert!(add_folder_asked(&home, "acme", "~", Some(&user)).is_err(), "too wide, as ever");
    }

    #[test]
    fn a_folder_named_becomes_a_project_started_as_a_repository_if_need_be() {
        let home = home();
        // The person's home beside equipe's, not in it.
        let user = home.with_file_name(format!("{}-user", home.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(user.join("code/notes")).unwrap();
        std::fs::write(user.join("code/notes/plan.md"), "# plan\n").unwrap();
        crate::squad_home::ensure(&home, "acme", None).ok();
        // A plain folder: a repository now, its files in a first commit.
        let notes = add_folder(&home, "acme", "~/code/notes", Some(&user)).unwrap();
        assert_eq!(notes, user.join("code/notes").canonicalize().unwrap());
        assert!(notes.join(".git").exists());
        assert!(git(&notes, &["ls-files"]).unwrap().contains("plan.md"));
        // A new folder: made, and started with an empty commit.
        let fresh = add_folder(&home, "acme", &user.join("code/fresh").display().to_string(), Some(&user)).unwrap();
        assert!(git(&fresh, &["rev-parse", "HEAD"]).is_ok());
        assert_eq!(list(&home).into_iter().find(|s| s.name == "acme").unwrap().projects, vec![notes.clone(), fresh]);
        // A folder inside a repository: the repository.
        std::fs::create_dir_all(notes.join("sub")).unwrap();
        assert_eq!(add_folder(&home, "acme", &notes.join("sub").display().to_string(), Some(&user)).unwrap(), notes);
        // Too wide, equipe's own, relative, or with no parent: refused.
        assert!(add_folder(&home, "acme", "~", Some(&user)).is_err());
        assert!(add_folder(&home, "acme", "/", Some(&user)).is_err());
        assert!(add_folder(&home, "acme", &home.join("squads").display().to_string(), Some(&user)).is_err());
        assert!(add_folder(&home, "acme", "code/notes", Some(&user)).is_err());
        assert!(add_folder(&home, "acme", &user.join("no/such/place").display().to_string(), Some(&user)).is_err());
    }
}
