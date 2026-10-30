//! What a command shows the person besides a line under the box: a sheet of
//! rows (changes, memory, usage, help), a question asked first, and what a
//! side command's answer turns into. Plain data and rules — the window only
//! draws them (`views/sheet.rs`) — so each can be tested.

use equipe_core::protocol::Command;

use crate::sidecar::Job;
use crate::{help, usage};

/// A question asked before something that is hard to take back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub title: String,
    pub body: Vec<String>,
    /// The button that says yes.
    pub yes: String,
    pub then: Then,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Then {
    /// Send these commands to the daemon.
    Send(Vec<Command>),
    /// Run a side command on the daemon's machine, and do `After` with its answer.
    Job(Job, After),
}

/// What to do with a side command's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    /// Say it in the line under the box.
    Flash,
    /// Rows in a sheet; `undo` gives each numbered row a button that takes that change back.
    Changes { title: String, undo: bool },
    /// What the squad remembers.
    Memory,
    /// The check passed: ask, and on yes run the real thing.
    Confirm { title: String, yes: String, then: Box<(Job, After)> },
    /// The squad's folders changed (or a squad was made): say `say`, reconnect to
    /// see it, and open `open` if that is a new squad — with its team to start.
    Squads { open: Option<String>, say: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub title: String,
    /// A line above the rows.
    pub lead: Option<String>,
    pub rows: Vec<Row>,
    pub buttons: Vec<Button>,
    /// A line below.
    pub foot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub head: String,
    pub text: String,
    /// A bar to fill, 0–100.
    pub bar: Option<u8>,
    /// The change this row is, for a button that takes it back.
    pub undo: Option<usize>,
    /// A heading for the rows below it.
    pub heading: bool,
}

impl Row {
    pub fn text(head: impl Into<String>, text: impl Into<String>) -> Self {
        Self { head: head.into(), text: text.into(), bar: None, undo: None, heading: false }
    }

    pub fn heading(text: impl Into<String>) -> Self {
        Self { heading: true, ..Self::text("", text) }
    }
}

/// `/usage`: the plan's windows, then each member.
pub fn usage_sheet(view: &usage::View) -> Sheet {
    let row = |r: &usage::Row| Row { bar: r.percent, ..Row::text(r.label.clone(), r.text.clone()) };
    let rows = std::iter::once(Row::heading(equipe_core::t!("sheet.claude_plan")))
        .chain(view.plan.iter().map(row))
        .chain(std::iter::once(Row::heading(equipe_core::t!("engines.codex_heading"))))
        .chain(view.codex.iter().map(row))
        .chain(std::iter::once(Row::heading(equipe_core::t!("engines.settings_section"))))
        .chain(std::iter::once(row(&view.switching)))
        .chain(std::iter::once(Row::heading(equipe_core::t!("sheet.members"))))
        .chain(view.members.iter().map(row))
        .collect();
    Sheet { title: equipe_core::t!("sheet.usage_title").into(), lead: None, rows, buttons: Vec::new(), foot: Some(equipe_core::t!("usage.note").into()) }
}

/// `/help`: this window's keys and every command.
pub fn help_sheet() -> Sheet {
    let rows = help::sections()
        .into_iter()
        .flat_map(|s| std::iter::once(Row::heading(s.title)).chain(s.rows.into_iter().map(|(k, w)| Row::text(k, w))))
        .collect();
    Sheet { title: equipe_core::t!("help.title").into(), lead: Some(help::intro().into()), rows, buttons: Vec::new(), foot: Some(equipe_core::t!("help.foot").into()) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Button {
    Copy(String, String),
    /// Run a command line as if typed.
    Run(String, String),
    /// Open a folder on this Mac.
    Open(String, std::path::PathBuf),
}

/// What `after` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Flash(String),
    Sheet(Sheet),
    Ask(Ask),
    /// Reconnect; open `open` if given, with `team` joining it when it is new.
    Squads { say: String, open: Option<String> },
}

/// A side command finished: what the window does about it.
pub fn after(after: &After, answer: Result<Vec<String>, String>) -> Outcome {
    let lines = match answer {
        Ok(lines) => lines,
        Err(why) => return Outcome::Flash(why),
    };
    match after {
        After::Flash => Outcome::Flash(lines.join(" · ")),
        After::Changes { title, undo } => {
            let rows: Vec<Row> = lines
                .iter()
                .map(|line| match numbered(line) {
                    Some((n, rest)) => Row { undo: undo.then_some(n), ..Row::text(n.to_string(), rest) },
                    None => Row::text("", line.clone()),
                })
                .collect();
            Outcome::Sheet(Sheet { title: title.clone(), lead: None, rows, buttons: Vec::new(), foot: undo.then(|| equipe_core::t!("sheet.changes_foot").to_owned()) })
        }
        After::Memory => {
            let mut lead = None;
            let mut rows = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    lead = Some(line.clone());
                } else if let Some(recent) = line.strip_prefix("lately: ") {
                    rows.push(Row::text(equipe_core::t!("sheet.lately"), recent));
                } else if let Some((name, about)) = line.split_once(" — ") {
                    rows.push(Row::text(name, about));
                } else {
                    rows.push(Row::text("", line.clone()));
                }
            }
            Outcome::Sheet(Sheet {
                title: equipe_core::t!("sheet.memory_title").into(),
                lead,
                rows,
                buttons: Vec::new(),
                foot: Some(equipe_core::t!("sheet.memory_foot").into()),
            })
        }
        After::Confirm { title, yes, then } => {
            let (job, next) = (**then).clone();
            Outcome::Ask(Ask { title: title.clone(), body: lines, yes: yes.clone(), then: Then::Job(job, next) })
        }
        After::Squads { open, say } => Outcome::Squads { say: say.clone(), open: open.clone() },
    }
}

/// "2 · 5m ago · backend saved a.md — why" → (2, "5m ago · backend saved a.md — why").
pub fn numbered(line: &str) -> Option<(usize, String)> {
    let (n, rest) = line.split_once(" · ")?;
    Some((n.trim().parse().ok()?, rest.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(l: &[&str]) -> Result<Vec<String>, String> {
        Ok(l.iter().map(|l| (*l).to_owned()).collect())
    }

    #[test]
    fn changes_become_numbered_rows_with_a_way_back() {
        let sheet = |undo| match after(&After::Changes { title: "t".into(), undo }, lines(&["1 · just now · pm saved notes/a.md — plan", "Nothing else"])) {
            Outcome::Sheet(s) => s,
            other => panic!("{other:?}"),
        };
        let with = sheet(true);
        assert_eq!((with.rows[0].head.as_str(), with.rows[0].text.as_str(), with.rows[0].undo), ("1", "just now · pm saved notes/a.md — plan", Some(1)));
        assert_eq!(with.rows[1].undo, None, "a line that isn't a change has no button");
        assert!(sheet(false).rows.iter().all(|r| r.undo.is_none()));
    }

    #[test]
    fn usage_and_help_are_sheets_of_headed_rows() {
        let row = |label: &str, percent| usage::Row { label: label.into(), text: String::new(), percent };
        let u = usage_sheet(&usage::View { plan: vec![row("5-hour", Some(42))], codex: vec![row("week", Some(18))], switching: row("제안", None), members: vec![] });
        assert_eq!(u.rows[0], Row::heading("CLAUDE 플랜"));
        assert_eq!(u.rows[1].bar, Some(42));
        // Both engines' plans, then how the squad moves members between them.
        assert_eq!((u.rows[2].clone(), u.rows[3].bar), (Row::heading("CODEX 플랜"), Some(18)));
        assert_eq!((u.rows[4].clone(), u.rows[5].head.as_str()), (Row::heading("엔진 자동 전환"), "제안"));
        assert!(u.rows[6].heading);
        let h = help_sheet();
        assert!(h.rows.iter().filter(|r| r.heading).count() >= 5);
        assert!(h.rows.iter().any(|r| r.head.starts_with("/usage")));
    }

    #[test]
    fn a_failure_is_just_said() {
        assert_eq!(after(&After::Flash, Err("nope".into())), Outcome::Flash("nope".into()));
        assert_eq!(after(&After::Memory, Err("x".into())), Outcome::Flash("x".into()));
    }

    #[test]
    fn memory_lists_each_fact_and_what_changed_lately() {
        let Outcome::Sheet(s) = after(&After::Memory, lines(&["The squad remembers 2 things: /m", "tone — be brief", "lately: 1 · 5m ago · pm saved tone.md — why"])) else { panic!() };
        assert_eq!(s.lead.as_deref(), Some("The squad remembers 2 things: /m"));
        assert_eq!((s.rows[0].head.as_str(), s.rows[0].text.as_str()), ("tone", "be brief"));
        assert_eq!(s.rows[1].head, "최근");
    }

    #[test]
    fn a_checked_folder_is_asked_about_before_it_is_added() {
        let real = Job::Squad { squad: "a".into(), words: vec!["add".into(), "~/x".into()], dry_run: false };
        let next = After::Confirm { title: "추가".into(), yes: "추가".into(), then: Box::new((real.clone(), After::Squads { open: None, say: "ok".into() })) };
        let Outcome::Ask(ask) = after(&next, lines(&["/Users/me/x", "x · git@h:me/x.git"])) else { panic!() };
        assert_eq!(ask.body, ["/Users/me/x", "x · git@h:me/x.git"]);
        assert_eq!(ask.then, Then::Job(real, After::Squads { open: None, say: "ok".into() }));
        // The check failing is the end of it: nothing to ask.
        assert_eq!(after(&next, Err("No folder at ~/x".into())), Outcome::Flash("No folder at ~/x".into()));
    }
}
