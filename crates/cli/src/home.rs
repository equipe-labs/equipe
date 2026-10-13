//! `/home` and `equipe home`: the squad's folder, for the person — open it, see
//! what changed there lately, take a change back. No git words.

use std::path::{Path, PathBuf};

use equipe_core::squad_home::{self, Change};

/// The squad `equipe home` is about, run from `cwd`: the one named, the one
/// whose project or folder `cwd` is in, else the only squad (or the first).
pub fn squad_for(home: &Path, cwd: &Path, named: Option<String>) -> Option<String> {
    if named.is_some() {
        return named;
    }
    let squads = equipe_core::squads::list(home);
    let project = equipe_core::runtime::project_of(cwd);
    squads
        .iter()
        .find(|s| s.projects.contains(&project))
        .map(|s| s.name.clone())
        .or_else(|| squad_home::squad_of(home, cwd))
        .or_else(|| squads.first().map(|s| s.name.clone()))
}

/// Open the folder in Finder (or the desktop's file manager).
pub fn open(dir: &Path) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener)
        .arg(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// "2 · 5m ago · backend saved notes/a.md — what the person asked"
pub fn line(n: usize, change: &Change, now: i64) -> String {
    format!("{n} · {} · {} {} — {}", ago(now - change.at), change.by, change.what(), change.why)
}

fn ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => "just now".to_owned(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// What `/home <words>` does, as lines for the person. `Ok` lines are news,
/// `Err` a reason it couldn't.
pub fn run(dir: &Path, words: &[&str]) -> Result<Vec<String>, String> {
    if !dir.join(".git").exists() {
        return Err("The squad's folder isn't made yet — it is when equipe's daemon next starts.".into());
    }
    match words {
        [] => {
            open(dir);
            Ok(vec![format!("The squad's folder: {}", dir.display())])
        }
        ["changes"] | ["recent"] => {
            let changes = squad_home::changes(dir, 10)?;
            if changes.is_empty() {
                return Ok(vec!["Nothing has changed in the squad's folder yet.".into()]);
            }
            let now = now();
            Ok(changes.iter().enumerate().map(|(i, c)| line(i + 1, c, now)).collect())
        }
        ["undo"] => undo(dir, 1),
        ["undo", n] => match n.trim_start_matches('#').parse::<usize>() {
            Ok(n) if n >= 1 => undo(dir, n),
            _ => Err("Which change? /home changes numbers them: /home undo 2".into()),
        },
        _ => Err("/home opens the squad's folder · /home changes · /home undo [n]".into()),
    }
}

fn undo(dir: &Path, n: usize) -> Result<Vec<String>, String> {
    let change = squad_home::undo(dir, n)?;
    Ok(vec![format!("Taken back: {} · /home undo brings it back", change.what())])
}

/// The squad's folder under equipe's home.
pub fn dir(home: &Path, squad: &str) -> PathBuf {
    squad_home::path(home, squad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_person_sees_changes_and_takes_one_back_in_plain_words() {
        let home = std::env::temp_dir().join(format!("equipe-cli-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        squad_home::ensure(&home, "acme", None).unwrap();
        let dir = dir(&home, "acme");
        assert_eq!(run(&dir, &["changes"]).unwrap(), ["Nothing has changed in the squad's folder yet."]);
        std::fs::write(dir.join("notes/plan.md"), "x").unwrap();
        squad_home::save(&dir, "pm", "write the plan down").unwrap();
        let list = run(&dir, &["changes"]).unwrap();
        assert_eq!(list, ["1 · just now · pm saved notes/plan.md — write the plan down"]);
        assert_eq!(run(&dir, &["undo"]).unwrap(), ["Taken back: removed notes/plan.md · /home undo brings it back"]);
        assert!(!dir.join("notes/plan.md").exists());
        assert!(run(&dir, &["undo", "x"]).unwrap_err().contains("/home undo 2"));
        assert!(!list.concat().contains("commit"), "no git words");
        let _ = std::fs::remove_dir_all(&home);
    }
}
