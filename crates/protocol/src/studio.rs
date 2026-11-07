//! A ticket's design work as a screen watches it (the desktop's designer
//! studio): what the member keeps in its worktree's `design/` folder while it
//! works — each screen's latest check and second look, the edits not checked
//! yet, the directions it proposed and the one chosen, the critiques, the
//! reference board — read by the daemon on a screen's ask (`design_studio`,
//! [`crate::rpc::DESIGN_STUDIO`]), size-limited and from inside that folder
//! only. Its pictures come through `read_image`, by the paths named here.
//! What the member ran and said comes from its conversation, which the
//! screen already has; this is only what it left on disk.

use serde::{Deserialize, Serialize};

use crate::{SquadName, TicketId};

/// Ask for a ticket's design work: only the squad the screen follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioAsk {
    pub squad: SquadName,
    pub ticket: TicketId,
}

/// What a ticket's worktree holds of its design work, as far as it has come.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Studio {
    pub ticket: TicketId,
    /// The design folder read, on the daemon's machine; `None`: the ticket
    /// has no worktree, or no design folder in it yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// Each screen's latest check, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<StudioCheck>,
    /// UI files edited and not checked since, as the ledger names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// The directions proposed, oldest first: this ticket's, and those with
    /// no ticket named.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directions: Vec<StudioDirection>,
    /// The critiques, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critiques: Vec<StudioCritique>,
    /// The reference board: each reference kept, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<StudioReference>,
    /// Other pictures it made (mockups, renders, comparison sheets), newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<StudioPicture>,
    /// Changes whenever anything read here does.
    #[serde(default)]
    pub stamp: u64,
}

/// A picture on the daemon's machine: its path, when it last changed (so a
/// screen shows it again when it is drawn again under the same name), and
/// the width it was drawn at, when it was one of a check's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioPicture {
    pub path: String,
    #[serde(default)]
    pub modified: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
}

/// A screen's latest check (`design/checks/<screen>.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioCheck {
    /// `onboarding/login`.
    pub screen: String,
    /// When it was checked, in unix seconds.
    #[serde(default)]
    pub at: u64,
    /// Drawn in a browser, not only read.
    #[serde(default)]
    pub rendered: bool,
    /// Floor rules passed, of those run.
    #[serde(default)]
    pub floor: (u32, u32),
    #[serde(default)]
    pub habits: u32,
    #[serde(default)]
    pub finish: u32,
    /// The floor rules that failed, by id, and their first evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing: Vec<(String, String)>,
    /// The habits left open, by id, and their first evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<(String, String)>,
    /// Its pictures, one per width, narrowest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<StudioPicture>,
    /// The second look at its first screenful alone, and at the whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<StudioLook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<StudioLook>,
    /// Looks in a row that asked for fixes.
    #[serde(default)]
    pub fixes_in_a_row: u32,
}

/// A second look's verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioLook {
    pub pass: bool,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub at: u64,
    /// Who looked: `claude`, `codex`, `subagent`.
    #[serde(default)]
    pub by: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<StudioFix>,
    /// Facts it found missing: what, and the wording meanwhile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<(String, String)>,
    /// Which engine looked, and why that one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<crate::engines::Reviewed>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioFix {
    #[serde(rename = "where")]
    pub place: String,
    pub what: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// A direction proposed (`design/directions/log.json`), and its pictures —
/// those in `design/directions/<its name>/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioDirection {
    pub name: String,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub chosen: bool,
    #[serde(default)]
    pub layout: String,
    #[serde(default, rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub palette: String,
    #[serde(default)]
    pub density: String,
    #[serde(default)]
    pub motion: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<StudioPicture>,
}

/// A critique a fresh reviewer wrote (`design/critiques/<stamp>.md`), cut short.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioCritique {
    pub path: String,
    #[serde(default)]
    pub at: u64,
    pub text: String,
    /// Which engine wrote it, and why that one; `None` for one kept before it was said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<crate::engines::Reviewed>,
}

/// A reference kept on the board (`design/references/<name>/`): its link,
/// its notes — what to take from it and what to leave — cut short, and its shots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioReference {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default)]
    pub modified: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<StudioPicture>,
}

impl Studio {
    /// Every picture it names, once each.
    pub fn picture_paths(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let all = self
            .checks
            .iter()
            .flat_map(|c| &c.pictures)
            .chain(self.directions.iter().flat_map(|d| &d.pictures))
            .chain(self.references.iter().flat_map(|r| &r.pictures))
            .chain(&self.pictures);
        for p in all {
            if !out.contains(&p.path.as_str()) {
                out.push(&p.path);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_studio_travels_and_names_each_picture_once() {
        let pic = |path: &str| StudioPicture { path: path.into(), modified: 1, width: None };
        let studio = Studio {
            ticket: 7,
            dir: Some("/w/design".into()),
            checks: vec![StudioCheck { screen: "a".into(), pictures: vec![pic("/w/design/checks/a/360.png")], ..StudioCheck::default() }],
            directions: vec![StudioDirection { name: "calm".into(), pictures: vec![pic("/w/design/directions/calm/home.png")], ..StudioDirection::default() }],
            pictures: vec![pic("/w/design/checks/a/360.png"), pic("/w/design/mock.png")],
            ..Studio::default()
        };
        let line = serde_json::to_string(&studio).unwrap();
        assert!(!line.contains("critiques"), "empty lists stay off the wire: {line}");
        assert_eq!(serde_json::from_str::<Studio>(&line).unwrap(), studio);
        assert_eq!(studio.picture_paths(), ["/w/design/checks/a/360.png", "/w/design/directions/calm/home.png", "/w/design/mock.png"]);
        let fix = StudioFix { place: "the CTA".into(), what: "darker".into(), why: String::new() };
        assert_eq!(serde_json::to_value(&fix).unwrap(), serde_json::json!({ "where": "the CTA", "what": "darker" }));
    }
}
