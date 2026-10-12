//! The first `equipe` in a repository: a squad to work in, in under a minute.
//!
//! A squad is the person's team, not a repository: the first time, they name it
//! and pick who is on it; every repository after that just joins it. Engines are
//! checked first, so nobody builds a team that can't start.

use std::path::Path;

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

    let name = squads::list(&home);
    let place = equipe_core::board::project_name(&project);
    if !name.is_empty() {
        // Most people have one squad: offer it first.
        let mut items: Vec<String> = name
            .iter()
            .map(|s| {
                let projects: Vec<String> = s.projects.iter().map(|p| equipe_core::board::project_name(p)).collect();
                format!("Add {place} to {}  {}", e.bold(&s.name), e.dim(&format!("works on {}", projects.join(", "))))
            })
            .collect();
        items.push("Start a new squad".into());
        let picked = dialoguer::Select::with_theme(&*theme)
            .with_prompt(format!("{place} isn't in a squad yet"))
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

    // A new squad: the person's team, named by them.
    let suggestion = squads::suggest_name(&home, Path::new(&whoami()));
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
            format!("{id:<10}{}", e.dim(role.summary))
        })
        .collect();
    let ticked: Vec<bool> = STARTERS.iter().map(|(_, on)| *on).collect();
    let picked = dialoguer::MultiSelect::with_theme(&*theme)
        .with_prompt("Who's on the team? The PM joins first · space toggles, enter confirms")
        .items(&labels)
        .defaults(&ticked)
        .interact_opt()
        .map_err(|err| err.to_string())?
        .ok_or("Nothing changed.")?;
    squads::create(&home, &squad, &project)?;
    let mut members = vec![equipe_core::roles::PM.to_owned()];
    members.extend(picked.into_iter().map(|i| STARTERS[i].0.to_owned()));
    eprintln!();
    Ok(Start { squad, members })
}

/// The person's login name, for a squad name they can accept or change.
fn whoami() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "my-squad".into())
}
