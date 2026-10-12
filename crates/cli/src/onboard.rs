//! The first `equipe` in a repository: a squad to work in, in under a minute.
//!
//! A squad is the person's team, not a repository: the first time, they name it
//! and pick who is on it; every repository after that just joins it. Engines are
//! checked first, so nobody builds a team that can't start.

use std::path::Path;

use equipe_core::protocol::Mode;
use equipe_core::{runtime, squads};
use equipe_providers::CredentialStore;
use equipe_providers::claude_code;
use equipe_providers::credentials::equipe_home;

use crate::ui;

/// The squad to open, and the members to bring in as it opens.
pub struct Start {
    pub squad: String,
    /// Roles to add, in order — the PM first. Empty when the squad already has its team.
    pub members: Vec<String>,
}

/// The roles a new squad may start with, besides the PM, and which are ticked.
const STARTERS: [(&str, bool); 5] =
    [("designer", true), ("frontend", true), ("backend", false), ("qa", false), ("data", false)];

/// The squad `cwd` works in — asking, the first time, which one.
pub async fn squad_for(cwd: &Path) -> Result<Start, String> {
    let home = equipe_home();
    let project = runtime::project_of(cwd);
    if let Some(squad) = squads::find(&home, &project) {
        return Ok(Start { squad, members: Vec::new() });
    }
    let e = ui::err();
    let theme = ui::select_theme();
    eprintln!("{}  {}\n", e.wordmark(), e.dim("a software squad of AI agents"));

    // Engines first: a squad needs one to run on.
    let codex = matches!(CredentialStore::default_location().load("openai"), Ok(Some(_)));
    let claude = claude_code::auth_status(claude_code::DEFAULT_BINARY).await.ok().flatten();
    let claude_ready = claude.as_ref().is_some_and(|s| s.logged_in);
    let mark = |ready: bool| if ready { e.accent("✓") } else { e.dim("○") };
    eprintln!("  {} {:<13}{}", mark(claude_ready), "claude code", e.dim(if claude_ready { "signed in" } else { "not signed in" }));
    eprintln!("  {} {:<13}{}", mark(codex), "codex", e.dim(if codex { "signed in" } else { "not signed in" }));
    if !claude_ready && !codex {
        eprintln!();
        return Err("No engine is signed in yet — a squad needs one to run on.".into());
    }
    if !claude_ready {
        eprintln!("\n{}", e.note(2, "The PM and the engineers run on Claude Code; without it they can't start."));
    }
    if equipe_core::worktree::repo_root(&project).is_err() {
        eprintln!("\n{}", e.note(2, "This folder isn't a git repository: members can read it, but engineers need one to work on branches."));
    }
    eprintln!();

    // Every menu line fits the terminal: a menu that wraps redraws over itself.
    let menu = ui::width().saturating_sub(6).max(20);
    let name = squads::list(&home);
    let place = equipe_core::board::project_name(&project);
    if !name.is_empty() {
        // Most people have one squad: offer it first.
        let mut items: Vec<String> = name
            .iter()
            .map(|s| {
                let projects: Vec<String> = s.projects.iter().map(|p| equipe_core::board::project_name(p)).collect();
                menu_item(&format!("Add {place} to {}  · works on {}", s.name, projects.join(", ")), menu)
            })
            .collect();
        items.push("Start a new squad".into());
        let picked = dialoguer::Select::with_theme(&*theme)
            .with_prompt(ui::fit(&format!("{place} isn't in a squad yet"), menu))
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(|err| err.to_string())?
            .ok_or("Nothing changed.")?;
        if let Some(squad) = name.get(picked) {
            squads::add_project(&home, &squad.name, &project)?;
            eprintln!();
            return Ok(Start { squad: squad.name.clone(), members: Vec::new() });
        }
    }

    // A new squad: a team, named by the person. The repository's owner on its
    // host — an organisation, usually — is the team it most likely belongs to.
    let owner = remote_url(&project).as_deref().and_then(owner_of).map(str::to_owned);
    let suggestion = squads::suggest_name(&home, Path::new(&squad_name_for(owner.as_deref())));
    eprintln!("{}", e.note(2, "A squad is your team, not this repository: it can work on all your projects."));
    let squad: String = dialoguer::Input::with_theme(&*theme)
        .with_prompt("Name your squad")
        .default(suggestion)
        .validate_with(|name: &String| {
            if squads::valid_name(name) { Ok(()) } else { Err("lowercase letters, digits and dashes") }
        })
        .interact_text()
        .map_err(|err| err.to_string())?;
    let labels: Vec<String> = STARTERS
        .iter()
        .map(|(id, _)| {
            let role = equipe_core::roles::role(id).expect("a built-in role");
            menu_item(&format!("{id:<10}{}", role.summary), menu)
        })
        .collect();
    let ticked: Vec<bool> = STARTERS.iter().map(|(_, on)| *on).collect();
    eprintln!("{}", e.note(2, "The PM joins first. Space toggles, enter confirms."));
    let picked = dialoguer::MultiSelect::with_theme(&*theme)
        .with_prompt(ui::fit("Who's on the team?", menu))
        .items(&labels)
        .defaults(&ticked)
        .report(false)
        .interact_opt()
        .map_err(|err| err.to_string())?
        .ok_or("Nothing changed.")?;
    // The answer in a line, not the whole menu again.
    let team: Vec<&str> = std::iter::once("pm").chain(picked.iter().map(|&i| STARTERS[i].0)).collect();
    eprintln!("{} {} {} {}", e.accent("✓"), e.bold("Team"), e.dim("·"), team.join(", "));
    // How much the squad does on its own: Auto unless the person says otherwise.
    let modes: Vec<String> = Mode::ALL
        .iter()
        .map(|m| {
            // Up front, so a narrow terminal cuts the description, not the advice.
            let note = if *m == Mode::Auto { "recommended · " } else { "" };
            menu_item(&format!("{:<13}{note}{}", m.label(), m.summary()), menu)
        })
        .collect();
    let auto = Mode::ALL.iter().position(|m| *m == Mode::Auto).unwrap_or(0);
    eprintln!("{}", e.note(2, "How much may members do on their own? /mode changes it any time."));
    let picked_mode = dialoguer::Select::with_theme(&*theme)
        .with_prompt(ui::fit("Mode", menu))
        .items(&modes)
        .default(auto)
        .report(false)
        .interact_opt()
        .map_err(|err| err.to_string())?
        .ok_or("Nothing changed.")?;
    let mode = Mode::ALL[picked_mode];
    eprintln!("{} {} {} {}", e.accent("✓"), e.bold("Mode"), e.dim("·"), mode.label());
    squads::create(&home, &squad, &project)?;
    equipe_core::journal::Journal::for_squad(&home, &squad)
        .and_then(|j| j.set_mode(mode))
        .map_err(|err| err.to_string())?;
    let mut members = vec![equipe_core::roles::PM.to_owned()];
    members.extend(picked.into_iter().map(|i| STARTERS[i].0.to_owned()));
    eprintln!();
    Ok(Start { squad, members })
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
/// suffix dropped, then `-squad`; `my-squad` when there is no owner.
fn squad_name_for(owner: Option<&str>) -> String {
    let Some(owner) = owner.map(str::to_lowercase) else {
        return "my-squad".into();
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
    fn a_menu_item_fits_in_bytes_as_well_as_columns() {
        let item = menu_item("designer  UX and UI — flows, layout, visual language, design tokens", 40);
        assert!(item.len() <= 40, "{item:?} is {} bytes", item.len());
        assert!(item.starts_with("designer  UX and UI —") && item.ends_with('…'), "{item:?}");
        assert_eq!(menu_item("qa  reviews", 40), "qa  reviews");
    }

    #[test]
    fn a_squad_is_named_after_the_team_it_belongs_to() {
        assert_eq!(squad_name_for(Some("equipe-labs")), "equipe-squad");
        assert_eq!(squad_name_for(Some("Acme")), "acme-squad");
        assert_eq!(squad_name_for(Some("rocket-squad")), "rocket-squad");
        assert_eq!(squad_name_for(None), "my-squad");
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
