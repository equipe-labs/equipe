//! What the person changes about their squads with a command — `/send-apps`,
//! `/browser max`, `/squad new|add|remove` — in one place, so the terminal
//! console and the desktop app (which runs `equipe …` on the daemon's machine,
//! whichever it is) do the same thing.

use std::path::Path;

/// `/send-apps [add|remove <app> · reset]` for `squad`: what to tell the person.
pub fn send_apps(home: &Path, squad: &str, words: &str) -> Result<String, String> {
    let journal = equipe_core::journal::Journal::for_squad(home, squad).map_err(|err| format!("Couldn't open the squad's settings: {err}"))?;
    let apps = journal.messaging_apps();
    if words.trim().is_empty() {
        let list = if apps.is_empty() { "no app".to_owned() } else { apps.join(", ") };
        return Ok(format!("Sends from {list} wait for your yes · /send-apps add|remove <app> · reset"));
    }
    let apps = equipe_core::desktop_gate::edit_apps(&apps, words)?;
    journal.set_messaging_apps(&apps).map_err(|err| format!("Couldn't keep it: {err}"))?;
    Ok(if apps.is_empty() {
        "No app asks before a send now · /send-apps reset brings the usual ones back".into()
    } else {
        format!("Sends wait for your yes in: {}", apps.join(", "))
    })
}

/// `/browser max [n]`: how many browsers may be open at once.
pub fn browser_max(home: &Path, n: Option<&str>) -> Result<String, String> {
    use equipe_core::browser;
    let Some(n) = n else {
        return Ok(format!("Up to {} browsers open at once · /browser max <n> changes it", browser::at_once(home)));
    };
    match n.parse::<usize>() {
        Ok(max @ 1..=16) => browser::set_at_once(home, max)
            .map(|()| format!("Up to {max} browsers open at once, from the next one that opens"))
            .map_err(|err| format!("Couldn't keep it: {err}")),
        _ => Err("A number from 1 to 16: /browser max 6".into()),
    }
}

/// What `equipe squad <words>` does. `dry_run` only checks the folder and says
/// what would happen, so a screen can ask before it does.
pub fn squad(home: &Path, cwd: &Path, current: Option<String>, words: &[&str], dry_run: bool) -> Result<Vec<String>, String> {
    let folder = |typed: &str| -> Result<crate::onboard::Folder, String> {
        let user = dirs::home_dir().ok_or("No home folder to read ~ from")?;
        crate::onboard::resolve_folder(typed, &user, cwd)
    };
    let current = || current.clone().ok_or_else(|| "Which squad? --squad <name>".to_owned());
    match words {
        ["add", typed] => {
            let squad = current()?;
            let folder = folder(typed)?;
            if equipe_core::squads::list(home).iter().any(|s| s.name == squad && s.projects.contains(&folder.path)) {
                return Err(format!("{squad} already works on {}", folder.path.display()));
            }
            if let Some(owner) = equipe_core::squads::find(home, &folder.path).filter(|o| *o != squad) {
                return Err(format!("{} already belongs to the squad {owner}", folder.path.display()));
            }
            if !dry_run {
                equipe_core::squads::add_project(home, &squad, &folder.path)?;
            }
            Ok(vec![folder.path.display().to_string(), folder.describe()])
        }
        ["remove", name] => {
            let squad = current()?;
            let project = equipe_core::squads::list(home)
                .into_iter()
                .find(|s| s.name == squad)
                .and_then(|s| s.projects.into_iter().find(|p| equipe_core::board::project_name(p) == *name || p.to_string_lossy() == *name))
                .ok_or_else(|| format!("{squad} doesn't work on {name}"))?;
            if !dry_run {
                equipe_core::squads::remove_project(home, &squad, &project)?;
            }
            Ok(vec![project.display().to_string(), format!("{squad} no longer works on {name} — its tickets and branches stay")])
        }
        ["new", name] => {
            if !dry_run {
                equipe_core::squads::create_without_project(home, name)?;
            } else if equipe_core::squads::exists(home, name) || !equipe_core::squads::valid_name(name) {
                return Err(format!("{name} can't be a new squad's name"));
            }
            Ok(vec![(*name).to_owned(), format!("Start the squad {name}")])
        }
        ["new", name, typed] => {
            let folder = folder(typed)?;
            if let Some(owner) = equipe_core::squads::find(home, &folder.path) {
                return Err(format!("{} is already {owner}'s · /squad {owner}", folder.name));
            }
            if !dry_run {
                equipe_core::squads::create(home, name, &folder.path)?;
            } else if equipe_core::squads::exists(home, name) || !equipe_core::squads::valid_name(name) {
                return Err(format!("{name} can't be a new squad's name"));
            }
            Ok(vec![folder.path.display().to_string(), folder.describe()])
        }
        _ => Err("equipe squad new <name> [folder] · add <folder> · remove <project> · delete <name>".into()),
    }
}

/// `equipe squad delete <name>`: asked first at a terminal (`yes` doesn't
/// ask; with no terminal to ask at, `--yes` is needed). Through the daemon at
/// `socket` when it runs — it stops the squad's members first, and its
/// screens move on — else right here, with no member running. What to tell
/// the person.
pub async fn delete_squad(home: &Path, socket: &Path, name: &str, yes: bool) -> Result<Vec<String>, String> {
    use equipe_core::t;
    if !equipe_core::squads::exists(home, name) {
        return Err(t!("squad.delete_no_squad", name = name));
    }
    let place = shown(&equipe_core::squads::deleted_dir(home));
    if !yes {
        if !crate::ui::interactive() {
            return Err(t!("squad.delete_needs_yes", name = name));
        }
        eprintln!("{}", t!("squad.delete_what", name = name, path = place));
        let go = dialoguer::Confirm::new().with_prompt(t!("squad.delete_confirm", name = name)).default(false).interact().map_err(|e| e.to_string())?;
        if !go {
            return Ok(vec![t!("squad.delete_cancelled").to_owned()]);
        }
    }
    let (moved_to, next) = if equipe_core::gateway::is_running(socket).await {
        through_daemon(socket, name).await?
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let deleted = equipe_core::squads::delete(home, name, now)?;
        (deleted.moved_to, deleted.next)
    };
    let mut lines = vec![t!("squad.delete_done", name = name, path = shown(&moved_to))];
    lines.push(match next {
        Some(next) => t!("squad.delete_next", next = next),
        None => t!("squad.delete_none_left").to_owned(),
    });
    Ok(lines)
}

/// Ask the daemon to delete `name`, and wait for its answer.
async fn through_daemon(socket: &Path, name: &str) -> Result<(std::path::PathBuf, Option<String>), String> {
    use equipe_core::protocol::{Command, Event};
    use equipe_core::t;
    let (mut client, _) = equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), name).await.map_err(|e| e.to_string())?;
    client.send(Command::DeleteSquad { squad: name.to_owned() });
    let answer = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::SquadDeleted { squad, moved_to, next } if squad == name => return Ok((moved_to, next)),
                Event::SquadNotDeleted { squad, why } if squad == name => return Err(why),
                _ => {}
            }
        }
        Err(t!("squad.delete_daemon_gone").to_owned())
    };
    // Removing big worktrees takes a while; a daemon too old to know the command never answers.
    tokio::time::timeout(std::time::Duration::from_secs(300), answer).await.map_err(|_| t!("squad.delete_no_answer", name = name))?
}

/// `path` with the home folder as `~`.
fn shown(path: &Path) -> String {
    match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn browser_max_reads_and_keeps_a_number_from_one_to_sixteen() {
        let home = scratch("max");
        assert_eq!(browser_max(&home, Some("6")).unwrap(), "Up to 6 browsers open at once, from the next one that opens");
        assert!(browser_max(&home, None).unwrap().starts_with("Up to 6 browsers"));
        assert!(browser_max(&home, Some("0")).is_err() && browser_max(&home, Some("x")).is_err() && browser_max(&home, Some("17")).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_dry_run_changes_nothing_and_new_squads_and_projects_follow_the_words() {
        let home = scratch("squad");
        let project = home.join("shop");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let words = |w: &[&str], dry| squad(&home, &home, Some("acme".into()), w, dry);
        // Checking a new squad's name and folder doesn't make it.
        assert!(words(&["new", "acme", project.to_str().unwrap()], true).is_ok());
        assert!(!equipe_core::squads::exists(&home, "acme"));
        assert!(words(&["new", "acme", project.to_str().unwrap()], false).is_ok());
        assert!(equipe_core::squads::exists(&home, "acme"));
        assert!(words(&["new", "acme"], true).is_err(), "taken");
        let second = home.join("site");
        std::fs::create_dir_all(second.join(".git")).unwrap();
        let described = words(&["add", second.to_str().unwrap()], true).unwrap();
        assert_eq!(described.len(), 2);
        assert_eq!(equipe_core::squads::list(&home)[0].projects.len(), 1, "a dry run adds nothing");
        words(&["add", second.to_str().unwrap()], false).unwrap();
        assert_eq!(equipe_core::squads::list(&home)[0].projects.len(), 2);
        assert!(words(&["add", second.to_str().unwrap()], false).unwrap_err().contains("already works on"));
        assert!(words(&["remove", "nope"], false).is_err());
        words(&["remove", "site"], false).unwrap();
        assert_eq!(equipe_core::squads::list(&home)[0].projects.len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn with_no_daemon_a_squad_is_deleted_right_here_into_the_deleted_folder() {
        let home = scratch("delete");
        let socket = home.join("equipe.sock");
        equipe_core::squads::create_without_project(&home, "acme").unwrap();
        equipe_core::squads::create_without_project(&home, "beta").unwrap();
        equipe_core::squads::remember_last(&home, "acme").unwrap();
        let err = delete_squad(&home, &socket, "nope", true).await.unwrap_err();
        assert!(err.contains("nope"), "{err}");
        let lines = delete_squad(&home, &socket, "acme", true).await.unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("acme") && lines[0].contains("deleted"), "{lines:?}");
        assert!(lines[1].contains("beta"), "what opens next: {lines:?}");
        assert!(!equipe_core::squads::exists(&home, "acme"));
        assert_eq!(equipe_core::squads::last_squad(&home).as_deref(), Some("beta"));
        let moved: Vec<_> = std::fs::read_dir(equipe_core::squads::deleted_dir(&home)).unwrap().flatten().collect();
        assert_eq!(moved.len(), 1);
        assert!(moved[0].path().join("squad.db").exists(), "the folder, whole");
        let lines = delete_squad(&home, &socket, "beta", true).await.unwrap();
        assert!(lines[1].contains("equipe"), "none left: how to start one: {lines:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
