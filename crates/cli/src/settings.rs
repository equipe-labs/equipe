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

/// The squad's speed as a few words: `Full speed — up to 6 tickets per member…`.
pub fn speed_words(parallel: &equipe_core::protocol::lanes::Parallel) -> (String, String) {
    use equipe_core::t;
    if parallel.is_max() {
        (t!("speed.full").to_owned(), t!("speed.full_about", per_member = parallel.per_member, per_engine = parallel.per_engine))
    } else {
        (t!("speed.balanced").to_owned(), t!("speed.balanced_about").to_owned())
    }
}

/// What `equipe speed [full|balanced] [--per-member N] [--per-engine N]`
/// asks for, against the speed the squad has: the new speed, or `None` when
/// nothing was asked (say what it is). A cap named with no speed is a cap on
/// full speed. Caps are kept on balanced too, so full speed comes back as the
/// person left it.
pub fn speed_change(now: equipe_core::protocol::lanes::Parallel, mode: Option<&str>, per_member: Option<u32>, per_engine: Option<u32>) -> Result<Option<equipe_core::protocol::lanes::Parallel>, String> {
    use equipe_core::protocol::lanes::{MOST_PER_ENGINE, MOST_PER_MEMBER, Parallel, ParallelMode};
    use equipe_core::t;
    let cap = |name: &str, n: Option<u32>, most: u32| match n {
        Some(n) if !(1..=most).contains(&n) => Err(t!("speed.bad_cap", name = name, most = most)),
        other => Ok(other),
    };
    let (per_member, per_engine) = (cap("--per-member", per_member, MOST_PER_MEMBER)?, cap("--per-engine", per_engine, MOST_PER_ENGINE)?);
    let mode = match mode {
        Some(word) => ParallelMode::parse(word).ok_or_else(|| t!("speed.usage").to_owned())?,
        None if per_member.is_some() || per_engine.is_some() => ParallelMode::Max,
        None => return Ok(None),
    };
    Ok(Some(Parallel { mode, per_member: per_member.unwrap_or(now.per_member), per_engine: per_engine.unwrap_or(now.per_engine) }.clamped()))
}

/// `equipe speed …` for `squad`: what to tell the person. Through the daemon
/// at `socket` when it runs — its screens and the work move at once — else
/// kept in the squad's journal for when it does.
pub async fn speed(home: &Path, socket: &Path, squad: &str, mode: Option<&str>, per_member: Option<u32>, per_engine: Option<u32>) -> Result<String, String> {
    use equipe_core::protocol::{Command, Event};
    use equipe_core::t;
    let journal = equipe_core::journal::Journal::for_squad(home, squad).map_err(|err| format!("Couldn't open the squad's settings: {err}"))?;
    let now = journal.parallel();
    let Some(next) = speed_change(now, mode, per_member, per_engine)? else {
        let (name, about) = speed_words(&now);
        return Ok(t!("speed.now", mode = name, about = about));
    };
    if equipe_core::gateway::is_running(socket).await {
        let (mut client, _) = equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), squad).await.map_err(|e| e.to_string())?;
        client.send(Command::SetParallel { squad: squad.to_owned(), parallel: next });
        let answer = async {
            while let Some(event) = client.next_event().await {
                if matches!(&event, Event::ParallelChanged { squad: s, parallel } if s == squad && *parallel == next) {
                    return true;
                }
            }
            false
        };
        match tokio::time::timeout(std::time::Duration::from_secs(10), answer).await {
            Ok(true) => {}
            _ => return Err(t!("speed.no_answer").to_owned()),
        }
    } else {
        journal.set_parallel(next).map_err(|err| format!("Couldn't keep it: {err}"))?;
    }
    let (name, about) = speed_words(&next);
    Ok(t!("speed.set", mode = name, about = about))
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
/// ask; with no terminal to ask at, `--yes` is needed), saying how much room
/// it gives back. Through the daemon at `socket` when it runs — it stops the
/// squad's members first, and its screens move on — else right here, with no
/// member running. What can be made again goes at once; the rest is kept for
/// a week to restore, or — `purge` — deleted now too. What to tell the person.
pub async fn delete_squad(home: &Path, socket: &Path, name: &str, yes: bool, purge: bool) -> Result<Vec<String>, String> {
    use equipe_core::t;
    if !equipe_core::squads::exists(home, name) {
        return Err(t!("squad.delete_no_squad", name = name));
    }
    let place = shown(&equipe_core::squads::deleted_dir(home));
    let days = equipe_core::squads::KEPT_DAYS.to_string();
    if !yes {
        if !crate::ui::interactive() {
            return Err(t!("squad.delete_needs_yes", name = name));
        }
        eprintln!("{}", if purge { t!("squad.delete_what_purge", name = name) } else { t!("squad.delete_what", name = name, path = place, days = days) });
        let (home_for, name_for) = (home.to_owned(), name.to_owned());
        let look = tokio::task::spawn_blocking(move || equipe_core::squads::footprint(&home_for, &name_for, FOOTPRINT_FOR)).await.ok().and_then(Result::ok);
        if let Some(look) = look {
            eprintln!("{}", space_words(&look, purge, &days));
        }
        let go = dialoguer::Confirm::new().with_prompt(t!("squad.delete_confirm", name = name)).default(false).interact().map_err(|e| e.to_string())?;
        if !go {
            return Ok(vec![t!("squad.delete_cancelled").to_owned()]);
        }
    }
    let done = if equipe_core::gateway::is_running(socket).await {
        through_daemon(socket, name, purge).await?
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let deleted = equipe_core::squads::delete(home, name, now, purge)?;
        Done { moved_to: deleted.moved_to, next: deleted.next, freed: deleted.freed, purged: deleted.purged }
    };
    let freed = equipe_core::disk::size(done.freed);
    let mut lines = vec![if done.purged {
        t!("squad.delete_done_purge", name = name, freed = freed)
    } else {
        t!("squad.delete_done", name = name, path = shown(&done.moved_to), freed = freed, days = days)
    }];
    lines.push(match done.next {
        Some(next) => t!("squad.delete_next", next = next),
        None => t!("squad.delete_none_left").to_owned(),
    });
    Ok(lines)
}

/// How long the question waits for a look at what deleting gives back.
const FOOTPRINT_FOR: std::time::Duration = std::time::Duration::from_secs(10);

/// How much room deleting gives back, as the question says it.
fn space_words(look: &equipe_core::protocol::rpc::SquadFootprint, purge: bool, days: &str) -> String {
    use equipe_core::t;
    let at_least = |bytes: u64| format!("{}{}", if look.complete { "" } else { "≥ " }, equipe_core::disk::size(bytes));
    if purge {
        t!("squad.delete_space_purge", freed = at_least(look.freed + look.kept))
    } else {
        t!("squad.delete_space", freed = at_least(look.freed), kept = at_least(look.kept), days = days)
    }
}

/// What deleting did, as the daemon or this process says it.
struct Done {
    moved_to: std::path::PathBuf,
    next: Option<String>,
    freed: u64,
    purged: bool,
}

/// Ask the daemon to delete `name`, and wait for its answer.
async fn through_daemon(socket: &Path, name: &str, purge: bool) -> Result<Done, String> {
    use equipe_core::protocol::{Command, Event};
    use equipe_core::t;
    let (mut client, _) = equipe_core::gateway::Client::connect(socket, &crate::daemon::build_id(), name).await.map_err(|e| e.to_string())?;
    client.send(Command::DeleteSquad { squad: name.to_owned(), purge });
    let answer = async {
        while let Some(event) = client.next_event().await {
            match event {
                Event::SquadDeleted { squad, moved_to, next, freed, purged } if squad == name => return Ok(Done { moved_to, next, freed, purged }),
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
    fn speed_changes_follow_the_words_and_a_cap_alone_is_a_cap_on_full_speed() {
        use equipe_core::protocol::lanes::{Parallel, ParallelMode};
        let now = Parallel::default();
        assert_eq!(speed_change(now, None, None, None).unwrap(), None, "nothing asked: say what it is");
        assert_eq!(speed_change(now, Some("full"), None, None).unwrap(), Some(Parallel::max(6, 16)));
        assert_eq!(speed_change(now, Some("Full speed"), Some(4), None).unwrap(), Some(Parallel::max(4, 16)));
        assert_eq!(speed_change(now, None, Some(4), Some(10)).unwrap(), Some(Parallel::max(4, 10)), "caps alone: full speed at them");
        assert_eq!(speed_change(Parallel::max(8, 20), Some("balanced"), None, None).unwrap(), Some(Parallel { mode: ParallelMode::Auto, per_member: 8, per_engine: 20 }), "balanced keeps the caps full speed comes back to");
        assert!(speed_change(now, Some("full"), Some(13), None).unwrap_err().contains("--per-member") && speed_change(now, None, None, Some(0)).is_err());
        assert!(speed_change(now, Some("warp"), None, None).unwrap_err().contains("equipe speed"));
        let (name, about) = speed_words(&Parallel::max(8, 20));
        assert!(name == "Full speed" && about.contains('8') && about.contains("20"), "{name}: {about}");
    }

    #[tokio::test]
    async fn with_no_daemon_a_speed_is_kept_in_the_journal_and_read_back() {
        let home = scratch("speed");
        let socket = home.join("equipe.sock");
        equipe_core::squads::create_without_project(&home, "acme").unwrap();
        let said = speed(&home, &socket, "acme", None, None, None).await.unwrap();
        assert!(said.starts_with("Speed: Balanced"), "{said}");
        let said = speed(&home, &socket, "acme", Some("full"), Some(4), None).await.unwrap();
        assert!(said.starts_with("Speed set to Full speed") && said.contains("4 tickets per member"), "{said}");
        assert_eq!(equipe_core::journal::Journal::for_squad(&home, "acme").unwrap().parallel(), equipe_core::protocol::lanes::Parallel::max(4, 16));
        assert!(speed(&home, &socket, "acme", None, None, None).await.unwrap().starts_with("Speed: Full speed"));
        let _ = std::fs::remove_dir_all(&home);
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
        let err = delete_squad(&home, &socket, "nope", true, false).await.unwrap_err();
        assert!(err.contains("nope"), "{err}");
        let lines = delete_squad(&home, &socket, "acme", true, false).await.unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("acme") && lines[0].contains("deleted"), "{lines:?}");
        assert!(lines[1].contains("beta"), "what opens next: {lines:?}");
        assert!(!equipe_core::squads::exists(&home, "acme"));
        assert_eq!(equipe_core::squads::last_squad(&home).as_deref(), Some("beta"));
        let moved: Vec<_> = std::fs::read_dir(equipe_core::squads::deleted_dir(&home)).unwrap().flatten().collect();
        assert_eq!(moved.len(), 1);
        assert!(moved[0].path().join("squad.db").exists(), "the folder, whole");
        assert!(lines[0].contains('7'), "kept for a week: {lines:?}");
        let lines = delete_squad(&home, &socket, "beta", true, true).await.unwrap();
        assert!(lines[1].contains("equipe"), "none left: how to start one: {lines:?}");
        let kept: Vec<_> = std::fs::read_dir(equipe_core::squads::deleted_dir(&home)).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1, "deleted for good: nothing kept of beta");
        let _ = std::fs::remove_dir_all(&home);
    }
}
