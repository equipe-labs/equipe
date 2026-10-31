//! The first run on this Mac: no squad yet. Rather than send the person to a
//! terminal, the window starts one itself — git checked (a new Mac has only
//! Apple's stub until its developer tools are installed), a project folder
//! picked or none, a name — with `equipe squad new`, then opens the AI
//! platforms page so an engine gets connected.

use std::path::{Path, PathBuf};

use crate::sheet::After;
use crate::sidecar::Job;

/// How often the window looks again while macOS installs its developer tools.
pub const GIT_POLL: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Default)]
pub struct Welcome {
    /// Whether git runs here; `None` until looked at.
    pub git: Option<bool>,
    /// macOS was asked to install its developer tools: the window watches.
    pub git_asked: bool,
    /// The project the squad starts with, if the person picked one.
    pub folder: Option<PathBuf>,
    /// `equipe squad new` runs.
    pub creating: bool,
    /// Why the last try didn't make the squad.
    pub problem: Option<String>,
    /// Made from here: once attached, the AI platforms page opens.
    pub then_engines: bool,
}

impl Welcome {
    /// The name the field starts with: the folder's, else the person's.
    pub fn suggestion(&self, home: &Path, user: Option<&str>) -> String {
        let from = match (&self.folder, user) {
            (Some(folder), _) => folder.clone(),
            (None, Some(user)) => PathBuf::from(user),
            (None, None) => PathBuf::from("squad"),
        };
        equipe_core::squads::suggest_name(home, &from)
    }

    /// Whether 시작 may be pressed with `name` typed.
    pub fn can_start(&self, name: &str) -> bool {
        self.git == Some(true) && !self.creating && equipe_core::squads::valid_name(name.trim())
    }

    /// 시작: the job that makes the squad, and what follows it — the window
    /// opens it with its starting team. `None` when it can't start yet.
    pub fn start(&mut self, name: &str, say: String) -> Option<(Job, After)> {
        let name = name.trim().to_owned();
        if !self.can_start(&name) {
            return None;
        }
        let mut words = vec!["new".to_owned(), name.clone()];
        words.extend(self.folder.as_ref().map(|f| f.display().to_string()));
        self.creating = true;
        self.problem = None;
        Some((Job::Squad { squad: String::new(), words, dry_run: false }, After::Squads { open: Some(name), say }))
    }

    /// The job ended: made (the AI platforms page next), or why not.
    pub fn finished(&mut self, answer: &Result<Vec<String>, String>) {
        self.creating = false;
        match answer {
            Ok(_) => self.then_engines = true,
            Err(why) => self.problem = Some(why.clone()),
        }
    }

    /// Attached to a squad: whether to open the AI platforms page, once.
    pub fn take_then_engines(&mut self) -> bool {
        std::mem::take(&mut self.then_engines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-welcome-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_name_comes_from_the_folder_else_the_person() {
        let home = home("name");
        let mut w = Welcome::default();
        assert_eq!(w.suggestion(&home, Some("abel")), "abel");
        assert_eq!(w.suggestion(&home, None), "squad");
        w.folder = Some("/Users/abel/Projects/My Shop".into());
        assert_eq!(w.suggestion(&home, Some("abel")), "my-shop");
    }

    #[test]
    fn a_squad_starts_only_once_git_runs_and_the_name_is_one() {
        let mut w = Welcome::default();
        assert!(!w.can_start("shop"), "git not looked at yet");
        w.git = Some(false);
        assert!(!w.can_start("shop"), "a new Mac's stub");
        w.git = Some(true);
        assert!(w.can_start("shop") && !w.can_start("My Shop") && !w.can_start(""));
    }

    #[test]
    fn starting_makes_the_squad_with_its_folder_then_opens_it_and_the_ai_page() {
        let mut w = Welcome { git: Some(true), folder: Some("/p/shop".into()), ..Default::default() };
        let (job, after) = w.start(" shop ", "made".into()).unwrap();
        assert_eq!(job.args(), ["squad", "new", "shop", "/p/shop"]);
        assert_eq!(after, After::Squads { open: Some("shop".into()), say: "made".into() });
        assert!(w.creating && w.start("shop", String::new()).is_none(), "once at a time");
        w.finished(&Err("git: the developer tools aren't installed".into()));
        assert_eq!(w.problem.as_deref(), Some("git: the developer tools aren't installed"));
        assert!(!w.take_then_engines());
        w.start("shop", String::new()).unwrap();
        w.finished(&Ok(vec![]));
        assert!(w.problem.is_none() && w.take_then_engines() && !w.take_then_engines(), "the AI page opens once");
        let mut none = Welcome { git: Some(true), ..Default::default() };
        assert_eq!(none.start("solo", String::new()).unwrap().0.args(), ["squad", "new", "solo"], "no folder: the squad's own home");
    }
}
