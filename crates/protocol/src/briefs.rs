//! What the squad thought before it built: the briefs the PM leaves after
//! office hours with the person and the plans that follow them — kept in the
//! squad's own folder, never in a project's repository (which may be public):
//! `docs/<project>/briefs/<date>-<slug>.md` and `docs/<project>/plans/…` for a
//! project, `docs/briefs/…` and `docs/plans/…` for work outside the projects.
//! Each opens with a front matter (`title`, `date`, `status`, …); screens list
//! them on a project's page and all together from [`crate::Event::ProjectDocs`],
//! which also says which are saved under more than one project ([`DocCopies`])
//! and which older copies still sit in a project's repository ([`InRepo`]).

use serde::{Deserialize, Serialize};

use crate::TicketId;

/// A brief, or a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Brief,
    Plan,
}

impl DocKind {
    pub const ALL: [DocKind; 2] = [DocKind::Brief, DocKind::Plan];

    /// Its folder for the squad's own work, from the top of the squad's
    /// folder — and where equipe once wrote it in a project's repository.
    pub fn dir(self) -> &'static str {
        match self {
            DocKind::Brief => "docs/briefs",
            DocKind::Plan => "docs/plans",
        }
    }

    /// Its folder's own name, beside the other kind's: `briefs`, `plans`.
    pub fn folder(self) -> &'static str {
        match self {
            DocKind::Brief => "briefs",
            DocKind::Plan => "plans",
        }
    }

    /// The word for it in a file and a commit: `brief`, `plan`.
    pub fn word(self) -> &'static str {
        match self {
            DocKind::Brief => "brief",
            DocKind::Plan => "plan",
        }
    }
}

/// One brief or plan, as its front matter says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDoc {
    pub kind: DocKind,
    /// The project it is in, by name; none: the squad's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Where it is, on the daemon's machine.
    pub path: String,
    pub title: String,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub date: String,
    /// `draft`, `settled`, `dropped` — a plan's `in_review` too; as written.
    #[serde(default)]
    pub status: String,
    /// The ticket it came from (a brief), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    /// `startup` or `builder` (a brief), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The tickets it was cut into (a plan's `tickets:`), when it says.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
}

impl ProjectDoc {
    /// Every ticket it names: the one it came from, then those it was cut
    /// into, each once.
    pub fn ticket_ids(&self) -> Vec<TicketId> {
        let mut out: Vec<TicketId> = Vec::new();
        for id in self.ticket.iter().chain(&self.tickets) {
            if !out.contains(id) {
                out.push(*id);
            }
        }
        out
    }

    /// Its file's name: `2026-11-04-checkout.md`.
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

/// One brief or plan saved under more than one project of the squad's folder
/// (or a project's and the squad's own): the same slug and title, or the
/// same words. The person keeps one ([`crate::Command::KeepDocCopy`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocCopies {
    pub kind: DocKind,
    pub title: String,
    /// Where each copy is, as [`ProjectDoc::path`] says it, newest first.
    pub paths: Vec<String>,
    /// Whether their words differ — then which to keep matters.
    #[serde(default)]
    pub differ: bool,
}

/// A brief or plan equipe once committed into a project's repository and
/// that still sits there: told, never touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InRepo {
    /// The project, by name.
    pub project: String,
    pub kind: DocKind,
    /// Where it is in the repository: `docs/plans/2026-11-04-sign-in.md`.
    pub path: String,
    /// The squad folder's copy of it, when there is one ([`ProjectDoc::path`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
}
