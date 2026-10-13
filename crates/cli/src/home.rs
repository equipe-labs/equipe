//! `/home` and `equipe home`: the squad's folder, for the person — open it, see
//! what changed there lately, take a change back. No git words. `/memory` and
//! `equipe memory`: what the squad remembers, for the person to read and fix.

use std::path::{Path, PathBuf};

use equipe_core::squad_home::{self, Change};
use equipe_core::squad_memory;

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

/// What `/memory <words>` does: with none, opens the memory's folder and lists
/// every fact and the latest changes; `changes` lists the latest ten changes.
/// Undoing one is `/home undo`, like any change to the squad's folder.
pub fn memory(dir: &Path, words: &[&str]) -> Result<Vec<String>, String> {
    if !dir.join(".git").exists() {
        return Err("The squad's folder isn't made yet — it is when equipe's daemon next starts.".into());
    }
    let now = now();
    match words {
        [] => {
            let folder = squad_memory::dir(dir);
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            open(&folder);
            let facts = squad_memory::facts(dir);
            let mut lines = vec![match facts.len() {
                0 => format!("The squad remembers nothing yet: {}", folder.display()),
                n => format!("The squad remembers {n} {}: {}", if n == 1 { "thing" } else { "things" }, folder.display()),
            }];
            lines.extend(facts.iter().map(|f| format!("{} — {}", f.name, f.description)));
            let recent = squad_memory::changes(dir, 3)?;
            lines.extend(recent.iter().enumerate().map(|(i, c)| format!("lately: {}", line(i + 1, c, now))));
            Ok(lines)
        }
        ["changes"] | ["recent"] => {
            let changes = squad_memory::changes(dir, 10)?;
            if changes.is_empty() {
                return Ok(vec!["Nothing has changed in the squad's memory yet.".into()]);
            }
            Ok(changes.iter().enumerate().map(|(i, c)| line(i + 1, c, now)).collect())
        }
        _ => Err("/memory opens what the squad remembers · /memory changes".into()),
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

    #[test]
    fn the_person_reads_what_the_squad_remembers_and_what_changed() {
        let home = std::env::temp_dir().join(format!("equipe-cli-memory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        squad_home::ensure(&home, "acme", None).unwrap();
        let dir = dir(&home, "acme");
        assert_eq!(memory(&dir, &["changes"]).unwrap(), ["Nothing has changed in the squad's memory yet."]);
        let fact = squad_memory::Remember { name: "tax-filing", description: "매년 5월 종합소득세 직접 신고", kind: "deadline", ..Default::default() };
        squad_memory::remember(&dir, "backend", &fact, "2026-10-13", None).unwrap();
        // A change elsewhere in the folder isn't the memory's.
        std::fs::write(dir.join("notes/plan.md"), "x").unwrap();
        squad_home::save(&dir, "pm", "write the plan down").unwrap();
        assert_eq!(
            memory(&dir, &["changes"]).unwrap(),
            ["1 · just now · backend saved memory/tax-filing.md — remembered tax-filing: 매년 5월 종합소득세 직접 신고"]
        );
        let all = memory(&dir, &[]).unwrap();
        assert!(all[0].starts_with("The squad remembers 1 thing: "), "{all:?}");
        assert_eq!(all[1], "tax-filing — 매년 5월 종합소득세 직접 신고");
        assert!(all[2].starts_with("lately: 1 · just now · backend saved memory/tax-filing.md"), "{all:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
