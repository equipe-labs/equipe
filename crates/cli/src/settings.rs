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
        _ => Err("equipe squad new <name> [folder] · add <folder> · remove <project>".into()),
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
}
