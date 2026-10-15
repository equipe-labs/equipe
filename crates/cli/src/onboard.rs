//! Which squad `equipe` opens, wherever it is run — and the first squad, in
//! under a minute.
//!
//! A squad is the person's team, not a repository: the first time, they name it
//! and pick who is on it, from any folder; repositories join it later. Engines
//! are checked first, so nobody builds a team that can't start.

use std::path::{Path, PathBuf};

use equipe_core::protocol::Mode;
use equipe_core::t;
use equipe_core::{runtime, squad_home, squads};
use equipe_providers::CredentialStore;
use equipe_providers::claude_code;
use equipe_providers::credentials::equipe_home;

use crate::ui;

/// The squad to open, and the members to bring in as it opens.
#[derive(Debug)]
pub struct Start {
    pub squad: String,
    /// Roles to add, in order — the PM first. Empty when the squad already has its team.
    pub members: Vec<String>,
    /// The repository it was run in, which no squad has: the console asks once
    /// whether to add it.
    pub offer: Option<Folder>,
}

/// The roles a new squad may start with, besides the PM, and which are ticked.
const STARTERS: [(&str, bool); 5] =
    [("designer", true), ("frontend", true), ("backend", false), ("qa", false), ("data", false)];

/// A folder too wide to be a project: the home folder, or the disk's root.
pub fn is_too_wide(project: &Path) -> bool {
    project == Path::new("/") || dirs::home_dir().is_some_and(|home| project == home)
}

/// A folder the person typed for `/squad add` or `/squad new`, read exactly as
/// typed and checked before anything is registered.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    /// Absolute and canonical: what goes into the squad.
    pub path: PathBuf,
    pub name: String,
    /// Where its `origin` points, for the person to recognise it by.
    pub remote: Option<String>,
    pub git: bool,
}

impl Folder {
    /// Its name and remote, for the person to recognise it by.
    pub fn describe(&self) -> String {
        let mut line = self.name.clone();
        match (&self.remote, self.git) {
            (Some(remote), _) => line.push_str(&format!(" · {remote}")),
            (None, true) => line.push_str(&format!(" · {}", t!("onboard.no_remote"))),
            (None, false) => line.push_str(&format!(" · {}", t!("onboard.not_git"))),
        }
        line
    }
}

/// Read `typed` — `~`, `~/x`, absolute, or relative to `cwd`, with or without a
/// trailing `/` — into the folder it names. Never another folder: one inside a
/// repository is refused with the repository's top to type instead, not quietly
/// swapped for it. The home folder and `/` are refused outright.
pub fn resolve_folder(typed: &str, home: &Path, cwd: &Path) -> Result<Folder, String> {
    let typed = typed.trim();
    let path = match typed.strip_prefix('~') {
        Some("") => home.to_owned(),
        Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        // `~user` isn't read: it would be a guess.
        Some(_) => return Err(t!("onboard.no_folder_full_path", typed = typed)),
        None => cwd.join(typed),
    };
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    let path = match path.canonicalize() {
        Ok(path) if path.is_dir() => path,
        Ok(_) => return Err(t!("onboard.is_a_file", typed = typed)),
        Err(_) => return Err(t!("onboard.no_folder", typed = typed)),
    };
    if path == Path::new("/") || path == canon(home) {
        return Err(t!("onboard.too_wide", path = path.display()));
    }
    let git = match equipe_core::worktree::repo_root(&path) {
        Ok(root) if canon(&root) == path => true,
        Ok(root) => {
            let root = canon(&root);
            if root == Path::new("/") || root == canon(home) {
                return Err(t!("onboard.inside_wide_repo", path = path.display(), root = root.display()));
            }
            // A folder of a repository — an app in a monorepo — is a project of
            // its own: the squad works on it in the repository's worktrees.
            true
        }
        Err(_) => false,
    };
    let name = equipe_core::board::project_name(&path);
    let remote = if git { remote_url(&path) } else { None };
    Ok(Folder { path, name, remote, git })
}

/// The team from the onboarding menu's picks: item 0 is the PM, who joins
/// whether or not it was left ticked; the rest are the starters.
fn team_from_picks(picked: &[usize]) -> Vec<&'static str> {
    std::iter::once(equipe_core::roles::PM).chain(picked.iter().filter(|&&i| i > 0).map(|&i| STARTERS[i - 1].0)).collect()
}

/// The team a squad started from the console begins with: the PM, then the
/// roles ticked by default.
pub fn starting_team() -> Vec<String> {
    std::iter::once(equipe_core::roles::PM).chain(STARTERS.iter().filter(|(_, on)| *on).map(|(id, _)| *id)).map(str::to_owned).collect()
}

/// The squad to open from `cwd`, first match wins: the one named; the one
/// whose project or home `cwd` is in; the one opened last (or the only one, or
/// the first) — offering to add a repository no squad has; else onboarding.
pub async fn squad_for(cwd: &Path, named: Option<String>) -> Result<Start, String> {
    let home = equipe_home();
    match pick(&home, cwd, named)? {
        Pick::Open(start) => Ok(start),
        Pick::Onboard(repo) => onboard(&home, repo).await,
    }
}

#[derive(Debug)]
enum Pick {
    Open(Start),
    /// No squad yet; the repository `cwd` is in, if any, to offer at the end.
    Onboard(Option<Folder>),
}

/// Which squad opens, decided without asking anything.
fn pick(home: &Path, cwd: &Path, named: Option<String>) -> Result<Pick, String> {
    let open = |squad: String, offer: Option<Folder>| Ok(Pick::Open(Start { squad, members: Vec::new(), offer }));
    if let Some(name) = named {
        if squads::list(home).iter().any(|s| s.name == name) {
            return open(name, None);
        }
        return Err(t!("onboard.no_squad_called", name = name));
    }
    let project = runtime::project_of(cwd);
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    // A worktree is its repository's: a ticket's branch belongs to the squad.
    let main = main_repository(cwd);
    let found = squads::find(home, &project)
        .or_else(|| main.as_deref().and_then(|main| squads::find(home, main)))
        .or_else(|| squad_home::squad_of(&canon(home), &canon(cwd)))
        .or_else(|| main.as_deref().and_then(|main| squad_home::squad_of(&canon(home), main)));
    if let Some(squad) = found {
        return open(squad, None);
    }
    let repo = repository_here(home, cwd);
    match squads::default_squad(home) {
        Some(squad) => open(squad, repo.filter(|r| !squads::is_declined(home, &r.path))),
        None => Ok(Pick::Onboard(repo)),
    }
}

/// The git repository `cwd` is in, when it could be a project: not the home
/// folder or the disk's root (a dotfiles repository), not equipe's own folders.
fn repository_here(home: &Path, cwd: &Path) -> Option<Folder> {
    let root = equipe_core::worktree::repo_root(cwd).ok()?;
    let root = root.canonicalize().unwrap_or(root);
    if is_too_wide(&root) || root.starts_with(home.canonicalize().unwrap_or_else(|_| home.to_owned())) {
        return None;
    }
    if squads::find(home, &root).is_some() {
        return None;
    }
    let name = equipe_core::board::project_name(&root);
    let remote = remote_url(&root);
    Some(Folder { path: root, name, remote, git: true })
}

/// The repository a worktree was cut from (the repository itself when `dir`
/// isn't a linked worktree).
fn main_repository(dir: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let common = common.canonicalize().unwrap_or(common);
    (common.file_name()? == ".git").then(|| common.parent().map(Path::to_owned)).flatten()
}

/// The first squad, from anywhere: engines, a name, a team, a mode — and, run
/// inside a repository, whether it joins. Nothing is written into a repository.
async fn onboard(home: &Path, repo: Option<Folder>) -> Result<Start, String> {
    let e = ui::err();
    let theme = ui::select_theme();
    eprintln!("{}  {}\n", e.wordmark(), e.dim(t!("onboard.tagline")));

    // Engines first: a squad needs one to run on.
    let codex = matches!(CredentialStore::default_location().load("openai"), Ok(Some(_)));
    let claude = claude_code::auth_status(claude_code::DEFAULT_BINARY).await.ok().flatten();
    let claude_ready = claude.as_ref().is_some_and(|s| s.logged_in);
    let mark = |ready: bool| if ready { e.accent("✓") } else { e.dim("○") };
    let signed_in = |ready: bool| if ready { t!("onboard.signed_in") } else { t!("onboard.not_signed_in") };
    eprintln!("  {} {:<13}{}", mark(claude_ready), "claude code", e.dim(signed_in(claude_ready)));
    eprintln!("  {} {:<13}{}", mark(codex), "codex", e.dim(signed_in(codex)));
    if !claude_ready && !codex {
        eprintln!();
        return Err(t!("onboard.no_engine").into());
    }
    if !claude_ready {
        eprintln!("\n{}", e.note(2, t!("onboard.needs_claude")));
    }
    eprintln!();

    // Every menu line fits the terminal: a menu that wraps redraws over itself.
    let menu = ui::width().saturating_sub(6).max(20);
    // A new squad: a team, named by the person. The repository's owner on its
    // host — an organisation, usually — is the team it most likely belongs to.
    let owner = repo.as_ref().and_then(|r| r.remote.as_deref()).and_then(owner_of).map(str::to_owned);
    let user = std::env::var("USER").ok();
    let suggestion = squads::suggest_name(home, Path::new(&squad_name_for(owner.as_deref(), user.as_deref())));
    eprintln!("{}", e.note(2, t!("onboard.squad_is_team")));
    let squad: String = dialoguer::Input::with_theme(&*theme)
        .with_prompt(t!("onboard.name_prompt"))
        .default(suggestion)
        .validate_with(|name: &String| {
            if squads::valid_name(name) { Ok(()) } else { Err(t!("onboard.name_rule")) }
        })
        .interact_text()
        .map_err(|err| err.to_string())?;
    // The PM heads the list, ticked, so the person sees who leads the team; it
    // joins whatever is toggled — a squad always has its PM.
    let mut labels = vec![menu_item(&format!("{:<10}{}", "pm", t!("onboard.pm_item")), menu)];
    labels.extend(STARTERS.iter().map(|(id, _)| {
        let role = equipe_core::roles::role(id).expect("a built-in role");
        menu_item(&format!("{id:<10}{}", role.summary), menu)
    }));
    let ticked: Vec<bool> = std::iter::once(true).chain(STARTERS.iter().map(|(_, on)| *on)).collect();
    eprintln!("{}", e.note(2, t!("onboard.menu_keys")));
    let picked = dialoguer::MultiSelect::with_theme(&*theme)
        .with_prompt(ui::fit(t!("onboard.team_prompt"), menu))
        .items(&labels)
        .defaults(&ticked)
        .report(false)
        .interact_opt()
        .map_err(|err| err.to_string())?
        .ok_or(t!("onboard.nothing_changed"))?;
    let team = team_from_picks(&picked);
    // The answer in a line, not the whole menu again.
    eprintln!("{} {} {} {}", e.accent("✓"), e.bold(t!("onboard.team")), e.dim("·"), team.join(", "));
    // How much the squad does on its own: Auto unless the person says otherwise.
    let modes: Vec<String> = Mode::ALL
        .iter()
        .map(|m| {
            // Up front, so a narrow terminal cuts the description, not the advice.
            let note = if *m == Mode::Auto { format!("{} · ", t!("onboard.recommended")) } else { String::new() };
            menu_item(&format!("{:<13}{note}{}", m.label(), m.short()), menu)
        })
        .collect();
    let auto = Mode::ALL.iter().position(|m| *m == Mode::Auto).unwrap_or(0);
    eprintln!("{}", e.note(2, t!("onboard.mode_note")));
    let picked_mode = dialoguer::Select::with_theme(&*theme)
        .with_prompt(ui::fit(t!("onboard.mode"), menu))
        .items(&modes)
        .default(auto)
        .report(false)
        .interact_opt()
        .map_err(|err| err.to_string())?
        .ok_or(t!("onboard.nothing_changed"))?;
    let mode = Mode::ALL[picked_mode];
    eprintln!("{} {} {} {}", e.accent("✓"), e.bold(t!("onboard.mode")), e.dim("·"), mode.label());
    squads::create_without_project(home, &squad)?;
    equipe_core::journal::Journal::for_squad(home, &squad)
        .and_then(|j| j.set_mode(mode))
        .map_err(|err| err.to_string())?;
    // Run inside a repository: the one thing left to ask.
    if let Some(repo) = repo {
        eprintln!("{}", e.note(2, &format!("{} · {}", repo.path.display(), repo.describe())));
        let add = dialoguer::Confirm::with_theme(&*theme)
            .with_prompt(ui::fit(&t!("onboard.add_repo", name = repo.name), menu))
            .default(true)
            .interact_opt()
            .map_err(|err| err.to_string())?
            .unwrap_or(false);
        if add {
            squads::add_project(home, &squad, &repo.path)?;
        } else {
            let _ = squads::decline(home, &repo.path);
        }
    }
    let members: Vec<String> = team.iter().map(|r| (*r).to_owned()).collect();
    eprintln!();
    Ok(Start { squad, members, offer: None })
}

/// A menu line that never wraps. dialoguer measures its items in bytes when it
/// redraws a menu, so a line whose bytes (style codes, `—`, Hangul) outrun the
/// terminal's width makes it clear lines above the menu. Items are plain text,
/// cut to fit in bytes as well as in columns.
fn menu_item(text: &str, width: usize) -> String {
    let fitted = ui::fit(text, width);
    if fitted.len() <= width {
        return fitted;
    }
    let mut cut = String::new();
    for c in fitted.trim_end_matches('…').chars() {
        if cut.len() + c.len_utf8() + '…'.len_utf8() > width {
            break;
        }
        cut.push(c);
    }
    format!("{}…", cut.trim_end())
}

/// `equipe-squad` for the owner `equipe-labs`: the team's name with a company
/// suffix dropped, then `-squad`; with no owner, the person's user name; else
/// `my-squad`.
fn squad_name_for(owner: Option<&str>, user: Option<&str>) -> String {
    let Some(owner) = owner.map(str::to_lowercase) else {
        return user.filter(|u| !u.is_empty()).map_or_else(|| "my-squad".into(), str::to_lowercase);
    };
    if owner.ends_with("-squad") || owner.ends_with("squad") {
        return owner;
    }
    let base = ["-labs", "-lab", "-hq", "-inc", "-dev", "-io", "-app", "-team", "-org", "-co"]
        .iter()
        .find_map(|suffix| owner.strip_suffix(suffix))
        .filter(|base| !base.is_empty())
        .unwrap_or(&owner);
    format!("{base}-squad")
}

/// Where `project`'s `origin` points, if it has one.
fn remote_url(project: &Path) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(project).args(["remote", "get-url", "origin"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|u| !u.is_empty())
}

/// The owner in a repository URL: `equipe-labs` in `https://github.com/equipe-labs/equipe-site.git`,
/// `git@github.com:equipe-labs/equipe-site.git` or `ssh://git@host/equipe-labs/site`.
fn owner_of(url: &str) -> Option<&str> {
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1, // scp-like: git@host:owner/repo
    };
    let mut parts = path.trim_matches('/').split('/');
    let owner = parts.next().filter(|o| !o.is_empty())?;
    parts.next().filter(|r| !r.is_empty())?;
    Some(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pm_is_on_every_team_ticked_or_not() {
        assert_eq!(team_from_picks(&[0, 2, 3]), ["pm", "frontend", "backend"]);
        assert_eq!(team_from_picks(&[2]), ["pm", "frontend"], "unticking the PM doesn't leave it out");
        assert_eq!(team_from_picks(&[]), ["pm"]);
    }

    #[test]
    fn home_and_root_are_never_projects() {
        assert!(is_too_wide(&dirs::home_dir().unwrap()));
        assert!(is_too_wide(Path::new("/")));
        assert!(!is_too_wide(&dirs::home_dir().unwrap().join("code/app")));
    }

    /// A scratch home with `equipe` (a repository) beside `equipe-labs-profile`
    /// and `notes` (plain folders).
    fn scratch_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("equipe-resolve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for dir in ["Abel/equipe/crates", "Abel/equipe-labs-profile", "notes"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        std::fs::write(home.join("file.txt"), "").unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(home.join("Abel/equipe")).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", "https://github.com/equipe-labs/equipe.git"]);
        home.canonicalize().unwrap()
    }

    #[test]
    fn a_typed_folder_is_the_folder_typed_never_a_neighbour() {
        let home = scratch_home("exact");
        let cwd = home.join("Abel");
        let equipe = home.join("Abel/equipe");
        for typed in ["~/Abel/equipe", "~/Abel/equipe/", "~/Abel//equipe", "equipe", "./equipe/", "../Abel/equipe"] {
            let folder = resolve_folder(typed, &home, &cwd).unwrap_or_else(|err| panic!("{typed}: {err}"));
            assert_eq!(folder.path, equipe, "{typed}");
        }
        let abs = equipe.display().to_string();
        let folder = resolve_folder(&abs, &home, Path::new("/")).unwrap();
        assert_eq!(folder.path, equipe);
        assert_eq!(folder.name, "equipe");
        assert!(folder.git);
        assert_eq!(folder.remote.as_deref(), Some("https://github.com/equipe-labs/equipe.git"));
        assert!(folder.describe().contains("https://github.com/equipe-labs/equipe.git"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn home_root_and_missing_folders_are_refused() {
        let home = scratch_home("refused");
        for typed in ["~", "~/", " ~ ", "/", "notes/..", "~/Abel/.."] {
            let err = resolve_folder(typed, &home, &home).unwrap_err();
            assert!(err.contains("home folder"), "{typed}: {err}");
        }
        assert!(resolve_folder("~/Abel/equip", &home, &home).unwrap_err().contains("No folder"), "no prefix guessing");
        assert!(resolve_folder("~/file.txt", &home, &home).unwrap_err().contains("not a folder"));
        assert!(resolve_folder("~someone/x", &home, &home).unwrap_err().contains("full path"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_folder_inside_a_repository_is_a_project_of_its_own_not_swapped_for_its_top() {
        let home = scratch_home("inside");
        let folder = resolve_folder("~/Abel/equipe/crates", &home, &home).unwrap();
        assert!(folder.path.ends_with("Abel/equipe/crates") && folder.git, "{folder:?}");
        assert_eq!(folder.name, "equipe-crates");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_plain_folder_is_allowed_with_a_warning() {
        let home = scratch_home("plain");
        let folder = resolve_folder("~/Abel/equipe-labs-profile", &home, &home).unwrap();
        assert_eq!(folder.path, home.join("Abel/equipe-labs-profile"));
        assert!(!folder.git);
        assert!(folder.describe().contains("not a git repository"), "{}", folder.describe());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_menu_item_fits_in_bytes_as_well_as_columns() {
        let item = menu_item("designer  UX and UI — flows, layout, visual language, design tokens", 40);
        assert!(item.len() <= 40, "{item:?} is {} bytes", item.len());
        assert!(item.starts_with("designer  UX and UI —") && item.ends_with('…'), "{item:?}");
        assert_eq!(menu_item("qa  reviews", 40), "qa  reviews");
    }

    #[test]
    fn a_squad_is_named_after_the_team_it_belongs_to() {
        assert_eq!(squad_name_for(Some("equipe-labs"), None), "equipe-squad");
        assert_eq!(squad_name_for(Some("Acme"), None), "acme-squad");
        assert_eq!(squad_name_for(Some("rocket-squad"), None), "rocket-squad");
        assert_eq!(squad_name_for(None, Some("Abel")), "abel", "no remote: the person's user name");
        assert_eq!(squad_name_for(None, None), "my-squad");
        assert_eq!(squad_name_for(Some("acme"), Some("abel")), "acme-squad");
    }

    fn opens(pick: Result<Pick, String>) -> (String, Option<PathBuf>) {
        match pick {
            Ok(Pick::Open(start)) => {
                assert!(start.members.is_empty(), "an existing squad brings no one in");
                (start.squad, start.offer.map(|f| f.path))
            }
            Ok(Pick::Onboard(_)) => panic!("onboards"),
            Err(err) => panic!("{err}"),
        }
    }

    #[test]
    fn with_no_squad_it_onboards_from_anywhere() {
        let dirs = scratch_home("onboard");
        let home = dirs.join(".equipe");
        let Ok(Pick::Onboard(None)) = pick(&home, &dirs.join("notes"), None) else { panic!("a plain folder onboards with nothing to add") };
        let Ok(Pick::Onboard(Some(repo))) = pick(&home, &dirs.join("Abel/equipe/crates"), None) else { panic!("onboards") };
        assert_eq!(repo.path, dirs.join("Abel/equipe"), "its repository, offered at the end");
        assert_eq!(repo.remote.as_deref(), Some("https://github.com/equipe-labs/equipe.git"));
        assert!(!home.join("squads").exists(), "deciding writes nothing");
        std::fs::remove_dir_all(&dirs).unwrap();
    }

    #[test]
    fn anywhere_else_the_squad_opened_last_opens() {
        let dirs = scratch_home("anywhere");
        let home = dirs.join(".equipe");
        let site = dirs.join("Abel/equipe-labs-profile");
        squads::create(&home, "acme", &site).unwrap();
        squads::create_without_project(&home, "beta").unwrap();
        assert_eq!(opens(pick(&home, &dirs.join("notes"), None)), ("acme".into(), None), "none opened yet: the first");
        squads::remember_last(&home, "beta").unwrap();
        assert_eq!(opens(pick(&home, &dirs.join("notes"), None)), ("beta".into(), None));
        assert_eq!(opens(pick(&home, &site, None)), ("acme".into(), None), "a project opens its own squad");
        let beta_home = squad_home::path(&home, "beta");
        assert_eq!(opens(pick(&home, &beta_home.join("notes"), None)), ("beta".into(), None), "so does a squad's home");
        squads::remember_last(&home, "acme").unwrap();
        assert_eq!(opens(pick(&home, &beta_home, None)), ("beta".into(), None));
        assert_eq!(opens(pick(&home, &dirs.join("notes"), Some("beta".into()))), ("beta".into(), None), "--squad wins");
        assert!(pick(&home, &dirs, Some("nope".into())).unwrap_err().contains("No squad called nope"));
        std::fs::remove_dir_all(&dirs).unwrap();
    }

    #[test]
    fn a_repository_no_squad_has_is_offered_until_declined() {
        let dirs = scratch_home("offer");
        let home = dirs.join(".equipe");
        squads::create_without_project(&home, "acme").unwrap();
        let equipe = dirs.join("Abel/equipe");
        assert_eq!(opens(pick(&home, &equipe.join("crates"), None)), ("acme".into(), Some(equipe.clone())));
        squads::decline(&home, &equipe).unwrap();
        assert_eq!(opens(pick(&home, &equipe, None)), ("acme".into(), None), "no is remembered");
        squads::add_project(&home, "acme", &equipe).unwrap();
        assert_eq!(opens(pick(&home, &equipe, None)), ("acme".into(), None));
        std::fs::remove_dir_all(&dirs).unwrap();
    }

    #[test]
    fn a_worktree_opens_its_repositorys_squad() {
        let dirs = scratch_home("worktree");
        let home = dirs.join(".equipe");
        let equipe = dirs.join("Abel/equipe");
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&equipe).args(args).output().unwrap().status.success(), "{args:?}");
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "first"]);
        let tree = dirs.join("tree");
        git(&["worktree", "add", "-q", tree.to_str().unwrap()]);
        squads::create_without_project(&home, "other").unwrap();
        squads::create(&home, "acme", &equipe).unwrap();
        squads::remember_last(&home, "other").unwrap();
        assert_eq!(opens(pick(&home, &tree, None)), ("acme".into(), None));
        std::fs::remove_dir_all(&dirs).unwrap();
    }

    #[test]
    fn a_repositorys_owner_is_read_from_any_kind_of_remote() {
        assert_eq!(owner_of("https://github.com/equipe-labs/equipe-site.git"), Some("equipe-labs"));
        assert_eq!(owner_of("git@github.com:equipe-labs/equipe-site.git"), Some("equipe-labs"));
        assert_eq!(owner_of("ssh://git@gitlab.example.com/acme/web"), Some("acme"));
        assert_eq!(owner_of("/srv/git/site.git"), None);
        assert_eq!(owner_of("https://github.com/"), None);
    }
}
