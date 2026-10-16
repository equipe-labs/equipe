//! The contract between equipe's brain (`equipe-core`) and its screens.
//!
//! Screens — the command line, the terminal UI, the desktop app — send
//! [`Command`]s and render [`Event`]s. They never call an engine and hold no
//! state beyond what they show; every decision happens in the core.
//!
//! Inside one process these travel as Rust values over a channel. Every message
//! also serializes to JSON, so the same contract can cross a socket later
//! (`equipe serve`) as JSON-RPC — the shape ACP, MCP and LSP already use.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod asks;
pub mod decide;
pub mod rpc;
pub mod stream;
pub mod tags;
pub mod team;
pub mod transcript;

pub use transcript::Transcript;

/// One conversation with one agent. Chosen by the screen that opens it — at
/// random, so screens attached to one daemon never collide — and kept below
/// 2^53 so it survives JSON in any language.
pub type SessionId = u64;

/// A fresh session id: random, below 2^53.
pub fn new_session_id() -> SessionId {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    (hasher.finish() & ((1 << 53) - 1)).max(1)
}

/// The engines equipe can run a turn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// The user's ChatGPT plan, signed in by equipe.
    Codex,
    /// The user's installed Claude Code, signed in by itself.
    Claude,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Codex => "codex",
            Engine::Claude => "claude code",
        }
    }
}

/// What a session may do to files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Answer and read; never write.
    ReadOnly,
    /// Edit files — on a new branch, in a worktree of its own.
    EditFiles,
}

/// Who a session is on the team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// A built-in role id (`pm`, `frontend`, …) or a name the person chose.
    pub role: String,
    /// Unique in its project, so the person can address it: `frontend`, `frontend-2`.
    pub name: String,
}

/// When a member expects what it is doing to be done, and — when it is
/// shipping something — to be live, as it said (unix seconds).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<u64>,
    /// What is being done, in a few words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_at: Option<u64>,
    /// What goes live: "v0.28.1 on Homebrew", "equipe.im".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_what: Option<String>,
    /// `done_at` is equipe's guess from how long such work took before, not
    /// the member's word.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub guessed: bool,
}

impl Estimate {
    pub fn is_empty(&self) -> bool {
        self.done_at.is_none() && self.live_at.is_none()
    }
}

/// A member's conversation about one project (`None`: the whole squad's),
/// with turns of its own beside the member's other conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    /// The member's own session.
    pub of: SessionId,
    pub project: Option<String>,
}

/// A squad's name: lowercase letters, digits and dashes.
pub type SquadName = String;

/// A ticket's number on its squad's board: `T-3` is 3.
pub type TicketId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    /// Suggested by a member; the PM or the person makes it real.
    Proposed,
    /// Ready to be picked up once its dependencies are done.
    Todo,
    InProgress,
    /// Its member can't go on without something only the person can do (a
    /// token, an account); the note lists what. The member is free for other
    /// work meanwhile, and gets this back, in the same worktree, once resumed.
    Waiting,
    /// Handed in; waiting for the person to accept it.
    Review,
    Done,
    Cancelled,
}

impl TicketStatus {
    pub fn label(self) -> &'static str {
        match self {
            TicketStatus::Proposed => "proposed",
            TicketStatus::Todo => "todo",
            TicketStatus::InProgress => "in progress",
            TicketStatus::Waiting => "waiting",
            TicketStatus::Review => "review",
            TicketStatus::Done => "done",
            TicketStatus::Cancelled => "cancelled",
        }
    }

    /// Finished one way or the other: nothing more happens to it.
    pub fn is_closed(self) -> bool {
        matches!(self, TicketStatus::Done | TicketStatus::Cancelled)
    }
}

/// What kind of work a ticket is: where it happens and what the person's
/// approval does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketKind {
    /// Changing a repository: a worktree of the ticket's project, a branch, and
    /// the person's merge question once accepted.
    #[default]
    Code,
    /// Finding out, writing, deciding: a worktree of the squad home; accepted,
    /// it goes into the squad's folder at once.
    Task,
    /// Doing something in the world — in an app or the browser — from a worktree
    /// of the squad home, each outward step approved before it happens.
    Action,
}

impl TicketKind {
    pub fn label(self) -> &'static str {
        match self {
            TicketKind::Code => "code",
            TicketKind::Task => "task",
            TicketKind::Action => "action",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "code" => Some(TicketKind::Code),
            "task" => Some(TicketKind::Task),
            "action" => Some(TicketKind::Action),
            _ => None,
        }
    }

    /// Worked in the squad home rather than a project.
    pub fn in_home(self) -> bool {
        self != TicketKind::Code
    }
}

/// How soon a ticket should be done — the person's call. Ready work is picked
/// up urgent first, then by its place on the board.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Urgent,
    High,
    #[default]
    Normal,
    Low,
}

impl Priority {
    pub const ALL: [Priority; 4] = [Priority::Urgent, Priority::High, Priority::Normal, Priority::Low];

    pub fn label(self) -> &'static str {
        match self {
            Priority::Urgent => "urgent",
            Priority::High => "high",
            Priority::Normal => "normal",
            Priority::Low => "low",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Priority::ALL.into_iter().find(|p| p.label() == text.trim().to_ascii_lowercase())
    }

    pub fn is_normal(&self) -> bool {
        *self == Priority::Normal
    }
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// A unit of work on the squad's board — the board is where the team agrees
/// on what is being done, by whom, in which project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: TicketId,
    pub title: String,
    /// What to do and how to know it's done.
    pub body: String,
    pub status: TicketStatus,
    /// Where its work happens and what accepting it does.
    #[serde(default)]
    pub kind: TicketKind,
    /// Who it is for: a role (`frontend`) or a member (`frontend-2`).
    pub owner: Option<String>,
    /// The member working on it, once it has been picked up.
    pub assignee: Option<String>,
    /// The project (repository) it belongs to — for a `task` or `action`, what
    /// it is about, if anything; it never writes there.
    pub project: Option<PathBuf>,
    /// Tickets that must be done first.
    pub depends_on: Vec<TicketId>,
    /// Where an editing member's work lives.
    pub branch: Option<String>,
    /// Its checkout, while there is one — the person can look at the work there.
    #[serde(default)]
    pub worktree: Option<PathBuf>,
    /// Whether its branch is in the project's main branch, as of its acceptance
    /// or merge — `Some(false)`: accepted, but not on main yet. `None`: unknown.
    #[serde(default)]
    pub merged: Option<bool>,
    /// Why its accepted work isn't on main, when equipe knows: the person left
    /// it on its branch, or merging it failed. `None` otherwise (and once merged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_main: Option<OffMain>,
    /// `you`, or the member who wrote it.
    pub created_by: String,
    /// The latest word on it: a hand-in summary, why it was reopened.
    pub note: Option<String>,
    /// While it waits for the person: what it asks of them, one item each
    /// ([`Ticket::asks`] reads an older daemon's note when this is empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<String>,
    /// What the person left open when they last answered its asks — not yet
    /// answered, or put off till later ([`asks::still_open`]): the member goes
    /// on without it, and it is asked again with its next wait.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining: Vec<String>,
    /// The pictures its hand-in shows — renders, mockups, screenshots: the
    /// ones its member named, then the images its branch changed — by their
    /// paths on the daemon's machine, which screens ask for (`read_image`).
    /// Left out when none, and absent from an older daemon.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// How soon it should be done; normal unless the person said.
    #[serde(default, skip_serializing_if = "Priority::is_normal")]
    pub priority: Priority,
    /// Its place among the tickets of its status, lower first, as the person
    /// put it; 0: by its number ([`Ticket::order_key`]).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub rank: i64,
    /// When it last changed, in unix seconds, as the squad's journal says —
    /// so a screen that just attached knows what finished today. `None` from
    /// an older daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

/// The gap between two tickets' ranks when they are spread out again.
pub const RANK_STEP: i64 = 1024;

impl Ticket {
    /// Where it sorts among the tickets of its status: its rank, else its
    /// number spread out, so tickets never placed keep the order they were made in.
    pub fn order_key(&self) -> i64 {
        if self.rank != 0 { self.rank } else { i64::from(self.id) * RANK_STEP }
    }

    /// Accepted, but its work isn't on the main branch yet.
    pub fn not_on_main(&self) -> bool {
        self.kind == TicketKind::Code && self.status == TicketStatus::Done && self.branch.is_some() && self.merged == Some(false)
    }

    /// Accepted, not on main, and still the person's call — not left on its
    /// branch by their choice. What the inbox lists.
    pub fn waits_to_merge(&self) -> bool {
        self.not_on_main() && self.off_main != Some(OffMain::Kept)
    }
}

/// Why a done ticket's work isn't on the main branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum OffMain {
    /// The person chose to leave it on its branch: no longer asked about.
    Kept,
    /// Merging it met a conflict with main; nothing was merged.
    Conflict { detail: String },
    /// Merging it failed otherwise (main checked out with changes, the merge
    /// refused); nothing was merged.
    Failed { detail: String },
}

/// A ticket the person writes ([`Command::CreateTicket`]). It goes on the
/// board ready (`todo`): equipe's rules hand it to its owner when it can start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketDraft {
    pub title: String,
    /// What to do and how to know it's done (Markdown).
    #[serde(default)]
    pub body: String,
    /// One of the squad's projects; `None`: the squad's only one for code, or
    /// the squad's folder for a task or action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    /// Who it is for: a role or a member. `None`: the PM decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Priority::is_normal")]
    pub priority: Priority,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TicketId>,
    /// `None`: code with a project, a task without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TicketKind>,
}

/// What the person changes in a ticket ([`Command::EditTicket`]): each field
/// given is set, the rest stay. For project and assignee, `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "some_of")]
    pub project: Option<Option<PathBuf>>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "some_of")]
    pub assignee: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<TicketId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TicketKind>,
}

impl TicketEdit {
    /// Nothing to change.
    pub fn is_empty(&self) -> bool {
        *self == TicketEdit::default()
    }
}

/// A field present on the wire, even as `null`, is a change: `Some(None)` clears.
fn some_of<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// A line of a ticket's record ([`Event::TicketHistory`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketEntry {
    /// Unix seconds.
    pub at: u64,
    /// `you`, `equipe`, or a member.
    pub by: String,
    /// `created`, `assigned`, `handed in`, `edited`, `moved`, `comment`, `merged`…
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

/// How much a member may do on its own while it works a ticket — and, at its
/// desk, in the squad's home; repositories it only reads there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Read only: look around and plan; change nothing.
    ReadOnly,
    /// Ask the person before every change and every command.
    Ask,
    /// Edit and run commands inside its own worktree, fenced in by the operating
    /// system; ask the person for anything beyond that.
    #[default]
    Auto,
    /// Anything the person's account can do, except what equipe always blocks.
    Full,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::ReadOnly, Mode::Ask, Mode::Auto, Mode::Full];

    pub fn label(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::Ask => "ask me",
            Mode::Auto => "auto",
            Mode::Full => "full access",
        }
    }

    /// What it means, short enough for a menu line.
    pub fn short(self) -> &'static str {
        match self {
            Mode::ReadOnly => "reads and plans; changes nothing",
            Mode::Ask => "asks before every edit and command",
            Mode::Auto => "works alone, sandboxed; asks for anything beyond",
            Mode::Full => "asks nothing, except to act in your name",
        }
    }

    /// What it means, in full.
    pub fn summary(self) -> &'static str {
        match self {
            Mode::ReadOnly => "reads and plans — changes nothing, runs nothing but looking around",
            Mode::Ask => "asks you before every edit and every command",
            Mode::Auto => "edits, installs, builds and tests on its own, sandboxed in its worktree; asks before anything beyond — network, pushes, your accounts",
            Mode::Full => "does anything your account can without asking, except what equipe always blocks; still asks before acting in your name",
        }
    }

    /// `read-only`, `ask`, `auto`, `full` — as typed in a command.
    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim().to_lowercase().as_str() {
            "read-only" | "readonly" | "read" => Some(Mode::ReadOnly),
            "ask" | "ask-me" | "askme" | "manual" => Some(Mode::Ask),
            "auto" => Some(Mode::Auto),
            "full" | "full-access" | "bypass" => Some(Mode::Full),
            _ => None,
        }
    }

    /// As written in a squad's journal — `plan` from before read-only was
    /// called that.
    pub fn stored(text: &str) -> Option<Mode> {
        if text == "plan" { Some(Mode::ReadOnly) } else { Mode::parse(text) }
    }
}

/// A member asking the person for permission — something its mode doesn't
/// allow on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub id: u64,
    pub session: SessionId,
    pub member: String,
    pub ticket: Option<TicketId>,
    /// The engine's tool: `Bash`, `Write`, `WebFetch`…
    pub tool: String,
    /// What exactly: the command, the file, the address.
    pub action: String,
    /// Why, in the member's words, when it gave one.
    pub reason: Option<String>,
    /// A picture of what's being allowed — the screen before a send from a
    /// desktop app: a file on this Mac, deleted once answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// The person's answer to an [`Approval`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// This once.
    Allow,
    /// This, and the same kind of thing from now on, for the whole squad.
    Always,
    Deny,
}

/// How to open a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    /// `None`: the first engine that is ready.
    pub engine: Option<Engine>,
    /// `None`: the engine's own default.
    pub model: Option<String>,
    pub access: Access,
    /// Where the agent works. With [`Access::EditFiles`], the repository the
    /// session's branch is cut from.
    pub cwd: PathBuf,
    /// The role the session plays on the team. `None`: a plain agent, no role.
    #[serde(default)]
    pub role: Option<String>,
    /// The squad a member joins. A member works on its squad's projects.
    #[serde(default)]
    pub squad: Option<SquadName>,
}

/// What a message is about, as a screen says it: left out, not said
/// (`None`); `null`, the whole squad (`Some(None)`); a name, that project.
mod about {
    use serde::{Deserialize, Deserializer};

    pub fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(d).map(Some)
    }
}

/// Screen → core: "do this".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Open a session. Answered by [`Event::SessionStarted`] or [`Event::Error`].
    Start { session: SessionId, spec: SessionSpec },
    /// Send the person's message. One turn runs at a time per session: a
    /// message sent while a turn runs waits as a memo ([`Event::MessageQueued`])
    /// and is delivered when the turn ends — or at once, if the turn is stopped.
    ///
    /// `about`: the project shown on the screen that sent it (`null`: the
    /// whole squad) — what the message is about, whatever another screen
    /// looks at. Left out (an older screen), the project in focus
    /// ([`Command::Focus`]).
    Submit {
        session: SessionId,
        text: String,
        #[serde(default, deserialize_with = "about::read", skip_serializing_if = "Option::is_none")]
        about: Option<Option<String>>,
    },
    /// A message for after the turn running now (a memo): it doesn't steer
    /// the work under way. With no turn running, the same as `Submit`.
    SubmitLater {
        session: SessionId,
        text: String,
        #[serde(default, deserialize_with = "about::read", skip_serializing_if = "Option::is_none")]
        about: Option<Option<String>>,
    },
    /// Stop the turn in progress (Esc). Answered by a `TurnEnded` with
    /// [`TurnOutcome::Interrupted`].
    Interrupt { session: SessionId },
    /// End the session. With [`Access::EditFiles`] this lands the branch first.
    /// For a member: it leaves the squad, and a ticket it was on goes back to the board.
    /// Answered by [`Event::SessionClosed`].
    Close { session: SessionId },
    /// The person accepts a ticket's work: it is done, its branch stays to merge.
    /// Only the person can do this — never a member, the PM included.
    Accept { squad: SquadName, ticket: TicketId },
    /// The person merges a done ticket's branch into the project's main branch,
    /// and pushes main to `origin` when `push`. Never without their word: a
    /// merge is the person's gate. A conflict merges nothing.
    Merge { squad: SquadName, ticket: TicketId, push: bool },
    /// The person leaves a done ticket's work on its branch, unmerged: it stops
    /// asking to be merged. A later `Merge` still lands it.
    KeepOnBranch { squad: SquadName, ticket: TicketId },
    /// The person sends a ticket back with what to change.
    Reopen { squad: SquadName, ticket: TicketId, note: String },
    /// The person did what a `waiting` ticket needed: it goes back to its
    /// member, in the same worktree, with `note` (what was done) to go on from.
    Resume { squad: SquadName, ticket: TicketId, note: String },
    /// The person drops a ticket.
    Cancel { squad: SquadName, ticket: TicketId },
    /// The person puts a ticket of their own on the board, ready (`todo`).
    /// Answered by [`Event::TicketChanged`]; the PM and its owner are told.
    CreateTicket { squad: SquadName, draft: TicketDraft },
    /// The person changes a ticket's fields. Refused (an [`Event::Error`]) when
    /// a change doesn't make sense: a dependency loop, a member nobody is,
    /// a project the squad hasn't, work under way moved to another project.
    /// The PM and the members it concerns are told what changed.
    EditTicket { squad: SquadName, ticket: TicketId, changes: TicketEdit },
    /// The same change to several tickets at once (the person chose them):
    /// each checked as [`Command::EditTicket`] is, those that can't take it
    /// left as they are, and the PM told once for all of them.
    EditTickets { squad: SquadName, tickets: Vec<TicketId>, changes: TicketEdit },
    /// The person moves a ticket on the board: to `status` (each move means
    /// what its button would — to done accepts, to cancelled cancels, a
    /// proposal to todo takes it on; see `equipe_core::board::move_rule`), and
    /// within its column before `before` (`None`: last).
    MoveTicket {
        squad: SquadName,
        ticket: TicketId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<TicketStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<TicketId>,
    },
    /// The person writes on a ticket: kept in its record, and passed to the
    /// member on it (or its owner) and the PM with their next message.
    CommentTicket { squad: SquadName, ticket: TicketId, text: String },
    /// A ticket's record, oldest first: answered by [`Event::TicketHistory`].
    TicketHistory { squad: SquadName, ticket: TicketId },
    /// Set how much members may do on their own: the squad's default, or one
    /// member's (`member`); `mode: None` puts a member back on the squad's default.
    SetMode { squad: SquadName, member: Option<String>, mode: Option<Mode> },
    /// The person answers a member's request for permission.
    Decide { squad: SquadName, approval: u64, decision: Decision },
    /// The person's answers to a member's questions ([`Event::QuestionsAsked`]),
    /// one per question in order — an empty one skips it. They go to the member
    /// as its next message. Answered by [`Event::QuestionsClosed`].
    /// `about` as [`Command::Submit`]'s.
    AnswerQuestions {
        squad: SquadName,
        id: u64,
        answers: Vec<decide::Answer>,
        #[serde(default, deserialize_with = "about::read", skip_serializing_if = "Option::is_none")]
        about: Option<Option<String>>,
    },
    /// The person closes a member's questions without answering: the member is
    /// told to go on without them.
    DismissQuestions { squad: SquadName, id: u64 },
    /// The person approves a change to the team the PM proposed
    /// ([`Event::TeamChangeProposed`]): the members join as `/add` brings
    /// them in, or the member leaves as `/close` lets it go. Only ever the
    /// person's: no agent sends it. Answered by [`Event::TeamChangeClosed`].
    ApproveTeamChange { squad: SquadName, id: u64 },
    /// The person says no to it: nothing changes, and the PM is told.
    DeclineTeamChange { squad: SquadName, id: u64 },
    /// How many extra members of a role equipe may bring in when several of
    /// its tickets are ready at once; 0 brings in none.
    Scale { squad: SquadName, extra: usize },
    /// Start a member's next conversation at its desk afresh: carrying a short
    /// summary of the last one (`summary`), or clean. Its memory and the board
    /// carry over either way.
    Refresh { session: SessionId, summary: bool },
    /// The project the person is looking at, by name — new work goes there
    /// unless they say otherwise; `None`: all of the squad's projects.
    Focus { squad: SquadName, project: Option<String> },
    /// The screen watches `session`'s browser (`None`: none): equipe sends it
    /// a picture of the page every moment or so ([`Event::BrowserFrame`]).
    WatchBrowser { squad: SquadName, session: Option<SessionId> },
    /// The model a member's turns run on from now — any its engine offers.
    /// Its conversation carries on.
    SetModel { session: SessionId, model: String },
    /// Members' browsers open on screen (`show`) or run hidden: one member's,
    /// or (`member: None`) everyone's. Kept for the squad.
    ShowBrowser { squad: SquadName, member: Option<String>, show: bool },
    /// The person takes the Mac's screen back from the members (`open: false`,
    /// `/desktop stop`): whoever had it loses it at once and nobody gets it
    /// until they open it again (`open: true`, `/desktop resume`). For every squad.
    Desktop { open: bool },
    /// Whether the squad's empty composer offers a guess at the person's next
    /// message after a member answers them ([`Event::Suggestion`]). On unless
    /// turned off; kept for the squad.
    Suggestions { squad: SquadName, on: bool },
    /// Take back a memo left while the member works ([`Event::MessageQueued`])
    /// before it is delivered — or a steer ([`Event::SteerQueued`]) it hasn't
    /// read yet: the first one with this text. Answered by
    /// [`Event::MemoWithdrawn`]; nothing when it was delivered (or read) already.
    Unqueue { session: SessionId, text: String },
    /// Send what waits for a member at work now: its turn stops (as
    /// [`Command::Interrupt`]) and the steers it hasn't read and the memos
    /// left — `texts` first, in that order, then any others — go to it at
    /// once, together, as one message. Answered by [`Event::QueueSent`], then
    /// the turn's end and the message.
    SendNow {
        session: SessionId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        texts: Vec<String>,
    },
    /// How hard a member thinks from its next turn: Claude Code's `--effort`
    /// (`low` · `medium` · `high` · `xhigh` · `max`), Codex's reasoning effort
    /// (`minimal` · `low` · `medium` · `high`). `None`: the engine's default.
    SetEffort { session: SessionId, effort: Option<String> },
    /// The browser panel moves a tab of the shared Chrome: back, forward,
    /// reload, stop — or to `url`, an address or words to search.
    BrowserNav { page: u64, action: BrowserNavAction, url: Option<String> },
    /// A tab of the person's own in the shared Chrome, at `url` (blank without
    /// one). Answered by [`Event::BrowserTabs`] for the person's lane (`""`).
    BrowserNewTab { url: Option<String> },
    BrowserCloseTab { page: u64 },
    /// The person takes a member's browser (`lane`, as [`Event::BrowserTabs`]
    /// names it) into their own hands, or gives it back. While they have it the
    /// member's browser tools wait; given back, the member is told.
    BrowserControl { lane: String, take: bool },
    /// The person answers a page's `alert`, `confirm` or `prompt` (`text`).
    BrowserDialog { page: u64, accept: bool, text: Option<String> },
    /// The real Chrome window, with `page` in front — beside the app when
    /// `bounds` says where (screen points) — for what a picture can't do.
    BrowserPopOut { page: u64, bounds: Option<ScreenBounds> },
    /// The person switched their screen to `lang` (`ko`, `zh-Hant`): what equipe
    /// says to them from now on is in it, as when a screen says hello.
    Language { lang: String },
    /// The person takes back one of the squad's "always allow" rules (an
    /// answer of [`Decision::Always`]): that kind of thing asks again. Answered
    /// by [`Event::Allowed`].
    Disallow { squad: SquadName, rule: String },
    /// Pair a phone (the equipe app) with the daemon's machine: a code opens
    /// at the relay. Answered, to this screen only, by [`Event::PairOffer`]
    /// (then [`Event::PairAsked`], [`Event::Paired`]) or [`Event::PairEnded`].
    /// Asked again while a code is open, the old code closes and a new one opens.
    PairStart,
    /// The person's answer to [`Event::PairAsked`]: pair that phone, or not.
    PairAnswer { approve: bool },
    /// The person closed the pairing: the code stops working.
    PairCancel,
    /// The phones paired with the daemon's machine: answered by [`Event::Devices`].
    Devices,
    /// Forget a paired phone (its `id` in [`Event::Devices`]): it can't reach
    /// the daemon again. Answered by [`Event::Devices`].
    Unpair { device: String },
}

/// A phone paired with the daemon's machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    /// `ios`, `android`.
    #[serde(default)]
    pub platform: String,
    /// When it was paired, Unix seconds.
    #[serde(default)]
    pub added: u64,
    /// Connected through the relay right now.
    #[serde(default)]
    pub online: bool,
    /// When it was last connected, Unix seconds, when the daemon knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<u64>,
}

/// Why a pairing ended without a phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairEnd {
    /// The person said no to the phone that asked.
    Refused,
    /// The code ran out of time before a phone brought it.
    Expired,
    /// Another screen (or `equipe pair`) has a code open.
    Busy,
    /// The relay couldn't be reached.
    Relay,
    /// The daemon doesn't pair phones (its phone link isn't running).
    Unavailable,
    /// Something else; the words say what.
    Failed,
}

/// Where a window goes on the screen, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenBounds {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

/// How the browser panel moves a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserNavAction {
    Back,
    Forward,
    Reload,
    Stop,
    /// To the `url` given.
    Go,
}

/// One tab of the shared Chrome. `id` is the browser's own, stable while the
/// tab is open and never given to another tab — not even by a browser started
/// again: what [`crate::stream`] frames and commands name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserPage {
    pub id: u64,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub loading: bool,
    /// The page's icon, where the page says it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    /// Something on the page only a person can do now: a "verify you are
    /// human" check, a sign-in, a code from their phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<HumanCheck>,
    /// The tab's page crashed ("Aw, Snap!"): it shows nothing until reloaded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crashed: bool,
}

/// What a page waits for a person to do — no member can, and no member
/// decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanCheck {
    /// A whole-page "checking your browser" wall (Cloudflare's "Just a moment…").
    Challenge,
    /// A CAPTCHA on the page: Turnstile, reCAPTCHA, hCaptcha.
    Captcha,
    /// A sign-in wall: a password to type.
    SignIn,
    /// A code from the person's phone or authenticator.
    TwoFactor,
}

/// A member, wherever it is: its squad and its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemberRef {
    pub squad: SquadName,
    pub member: String,
}

impl std::fmt::Display for MemberRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.squad, self.member)
    }
}

/// The Mac's screen: one mouse, one keyboard, so one member at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopState {
    /// The member using the screen.
    #[serde(default)]
    pub holder: Option<MemberRef>,
    /// Members waiting for it, next first.
    #[serde(default)]
    pub queue: Vec<MemberRef>,
    /// The person took it back (`/desktop stop`): no member uses it until they reopen it.
    #[serde(default)]
    pub stopped: bool,
}

impl DesktopState {
    /// Nothing to show: nobody on it, nobody waiting, not stopped.
    pub fn is_quiet(&self) -> bool {
        self.holder.is_none() && self.queue.is_empty() && !self.stopped
    }

    /// One line for a screen following squad `here` (members of other squads
    /// show as `squad/member`); `None` when there is nothing to say.
    pub fn summary(&self, here: Option<&str>) -> Option<String> {
        if self.stopped {
            return Some("screen taken back · /desktop resume".into());
        }
        let name = |m: &MemberRef| if here == Some(m.squad.as_str()) { m.member.clone() } else { m.to_string() };
        let waiting = (!self.queue.is_empty()).then(|| format!("{} waiting", self.queue.len()));
        let parts: Vec<String> = self.holder.as_ref().map(|m| format!("{} on the screen", name(m))).into_iter().chain(waiting).collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// Token counts for one turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Input read from or written to the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_tokens: Option<u64>,
    /// How full the conversation's context was at the end of the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// The model's context window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

impl Usage {
    /// Everything the turn processed: input, cached input, and output.
    pub fn total(&self) -> u64 {
        self.input_tokens.unwrap_or(0) + self.cache_tokens.unwrap_or(0) + self.output_tokens.unwrap_or(0)
    }
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    /// The engine stopped early but left usable output.
    Incomplete,
    /// The user stopped it.
    Interrupted,
    /// It failed; an [`Event::Error`] says why.
    Failed,
}

/// The isolated place an editing session works in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub branch: String,
    /// The commit the branch was cut from.
    pub base: String,
    pub path: PathBuf,
}

/// One of the Claude plan's usage windows: how much of it is used, in
/// percent, and when it starts over (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanWindow {
    pub used_percent: u8,
    pub resets_at: i64,
}

/// The Claude plan's usage, as Claude Code last reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanUsage {
    pub five_hour: Option<PlanWindow>,
    pub seven_day: Option<PlanWindow>,
}

/// Core → screen: "this happened".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        session: SessionId,
        /// The project the session belongs to (its repository root, or the
        /// directory it was opened in), so every screen on that project shows it.
        project: PathBuf,
        engine: Engine,
        /// The model, once known. Codex resolves it at start; Claude Code
        /// reports it with the first turn.
        model: Option<String>,
        /// Set for [`Access::EditFiles`] sessions.
        workspace: Option<Workspace>,
        /// Set for a session that plays a role on the team.
        #[serde(default)]
        member: Option<Member>,
        #[serde(default)]
        squad: Option<SquadName>,
    },
    /// The session just started is a lane of member `of`: its conversation
    /// about `project` (`None`: the whole squad's), running beside the member's
    /// others — one turn at a time in each, any number of them at once.
    LaneOpened { session: SessionId, of: SessionId, project: Option<String> },
    /// A member picked up a ticket. Its turns now run in the ticket's project —
    /// in `workspace` when it edits.
    WorkStarted {
        session: SessionId,
        ticket: TicketId,
        title: String,
        project: PathBuf,
        workspace: Option<Workspace>,
    },
    /// The member put the ticket down (handed in, accepted, cancelled, or left);
    /// its turns are back at its desk.
    WorkEnded { session: SessionId, ticket: TicketId },
    /// A ticket was created or changed. Carries the whole ticket.
    TicketChanged { squad: SquadName, ticket: Ticket },
    /// A ticket's record, oldest first ([`Command::TicketHistory`]).
    TicketHistory { squad: SquadName, ticket: TicketId, entries: Vec<TicketEntry> },
    /// The squad's default mode, or one member's, changed. `mode: None`: the
    /// member follows the squad again.
    ModeChanged { squad: SquadName, member: Option<String>, mode: Option<Mode> },
    /// A member waits for the person's permission.
    ApprovalRequested { squad: SquadName, approval: Approval },
    /// It was answered — by the person, or by nobody in time (`Deny`).
    ApprovalResolved { squad: SquadName, approval: u64, decision: Decision },
    /// A member puts a decision to the person (`ask_decision`): questions with
    /// options, waiting until they answer — it doesn't hold the member up.
    QuestionsAsked { squad: SquadName, questions: decide::QuestionSet },
    /// No longer waiting: answered (`answers`, on any screen), or closed
    /// unanswered by the person or because its member left (`None`).
    QuestionsClosed {
        squad: SquadName,
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answers: Option<Vec<decide::Answer>>,
    },
    /// The PM proposes a change to the team (`propose_team_change`): it waits
    /// for the person's [`Command::ApproveTeamChange`] or
    /// [`Command::DeclineTeamChange`].
    TeamChangeProposed { squad: SquadName, change: team::TeamChange },
    /// No longer waiting: approved (`members`: who joined, or left), declined,
    /// or moot (its member left anyway; `approved: false`).
    TeamChangeClosed {
        squad: SquadName,
        id: u64,
        approved: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<String>,
    },
    /// The squad's "always allow" rules changed (one added by an answer of
    /// [`Decision::Always`], one taken back): all of them, oldest first.
    Allowed { squad: SquadName, rules: Vec<String> },
    /// The plan's usage windows — for every screen, whichever squad it follows.
    PlanUsage { usage: PlanUsage },
    /// Who has the Mac's screen and who waits for it changed — for every screen.
    Desktop { desktop: DesktopState },
    /// Output a member's turn has produced so far, in tokens.
    Output { session: SessionId, tokens: u64 },
    /// What the person asked — sent to every screen, not only the one it was typed in.
    UserMessage { session: SessionId, text: String },
    /// A message left while the agent works. It becomes a [`Event::UserMessage`]
    /// when the turn ends; memos left in the meantime arrive together.
    MessageQueued {
        session: SessionId,
        text: String,
        /// The project it was left about: it goes into that project's
        /// conversation, whichever the person looks at when the turn ends.
        #[serde(default)]
        project: Option<String>,
    },
    /// A memo left while it works — or a steer it hadn't read — was taken
    /// back before it was delivered.
    MemoWithdrawn { session: SessionId, text: String },
    /// What waited for the member ([`Command::SendNow`]) goes to it now, as
    /// one message, in this order: none of it waits any more.
    QueueSent { session: SessionId, texts: Vec<String> },
    /// A line from equipe for this session's conversation — e.g. the PM being
    /// told that the person spoke to a member directly.
    Notice { session: SessionId, text: String },
    /// A fragment of the agent's answer.
    TextDelta { session: SessionId, text: String },
    /// The agent is using a tool, e.g. `Edit` on a file.
    ToolUse {
        session: SessionId,
        tool: String,
        target: Option<String>,
    },
    /// A tool use came back: the latest [`Event::ToolUse`] with the same
    /// `tool` and `target` still open ended so. A screen that doesn't know
    /// this event shows the step as before.
    ToolResult {
        session: SessionId,
        tool: String,
        target: Option<String>,
        outcome: ToolOutcome,
    },
    /// A fragment of the model's reasoning, as the engine shows it. Not part
    /// of the conversation: a screen may show it while it comes.
    Thinking { session: SessionId, text: String },
    /// The engine named its model (Claude Code does on its first turn), or the
    /// person changed it.
    ModelKnown { session: SessionId, model: String },
    /// What a watched member's browser shows now: a picture (JPEG, or PNG from
    /// a browser of its own), base64, and the page.
    BrowserFrame { session: SessionId, image: String, url: Option<String> },
    /// The tabs of one lane of the shared Chrome changed: a member
    /// conversation's browser (`lane` = `squad/key`; `session` its
    /// conversation), or the person's own (`lane` = `""`, `squad` none).
    /// `agent_page`: the tab its member's tools last worked in.
    BrowserTabs {
        squad: Option<SquadName>,
        lane: String,
        #[serde(default)]
        session: Option<SessionId>,
        #[serde(default)]
        member: Option<String>,
        /// The member's colour, `r, g, b`.
        #[serde(default)]
        color: Option<String>,
        pages: Vec<BrowserPage>,
        #[serde(default)]
        agent_page: Option<u64>,
    },
    /// Who has a member's browser in hand: the person (`by: "person"`) or the
    /// member again (`None`). `why`: the member asked the person to do
    /// something there (sign in) — the screen shows that browser.
    BrowserControl {
        squad: Option<SquadName>,
        lane: String,
        by: Option<String>,
        #[serde(default)]
        why: Option<String>,
        /// The daemon put the browser in the person's hands itself: a page
        /// waits for a person ([`HumanCheck`]). Its member waits meanwhile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<HumanCheck>,
    },
    /// A page asks: `kind` is `alert`, `confirm`, `prompt` or `beforeunload`.
    BrowserDialog {
        squad: Option<SquadName>,
        lane: String,
        page: u64,
        kind: String,
        message: String,
        #[serde(default)]
        default: Option<String>,
    },
    /// The page's question was answered (here or elsewhere), or it went away.
    BrowserDialogClosed { page: u64 },
    /// A member asked to do something to a tab the person is on, and was
    /// refused for it — `action` `close`: the screen offers to let it, at
    /// one click (closing the tab), or keep using it.
    BrowserWants { squad: Option<SquadName>, lane: String, page: u64, action: String },
    /// The member handed `what` to a subagent, which works alongside it.
    SubagentStarted { session: SessionId, id: String, what: String },
    /// A subagent's latest step.
    SubagentStep { session: SessionId, id: String, step: String },
    /// A subagent handed back what it found.
    SubagentEnded { session: SessionId, id: String },
    /// What follows in the conversation is about `project` (`None`: the whole
    /// squad) — a message of the person's sent looking at it, news of its
    /// ticket, or a ticket's work. See [`transcript::scope`].
    Thread { session: SessionId, project: Option<String> },
    /// A guess at what the person will say next to the member who just
    /// answered them, for their empty composer. Not part of the conversation.
    Suggestion { session: SessionId, text: String },
    /// A message the person sent while the member works: it reads it at its
    /// next step, without stopping (steering).
    SteerQueued {
        session: SessionId,
        text: String,
        /// The project the person was looking at when they said it (`None`:
        /// the whole squad): it shows in that project's conversation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
    },
    /// The member read a steering message, at this point of its turn.
    Steered { session: SessionId, text: String },
    /// When the member expects its work done, and what it ships live — as it
    /// said, or nothing any more (`None`).
    Estimated { session: SessionId, estimate: Option<Estimate> },
    TurnEnded {
        session: SessionId,
        outcome: TurnOutcome,
        usage: Option<Usage>,
    },
    /// An editing session closed with changes, committed to its branch.
    BranchLanded {
        session: SessionId,
        branch: String,
        base: String,
        /// `git diff --stat` of the branch.
        stat: String,
    },
    /// An editing session closed without changing anything; no branch was kept.
    NothingChanged { session: SessionId },
    SessionClosed { session: SessionId },
    /// Something went wrong. `next` is the command that fixes it, when there is one.
    Error {
        session: Option<SessionId>,
        message: String,
        next: Option<String>,
    },
    /// A code to pair a phone with ([`Command::PairStart`]), for a QR code:
    /// `seconds` it still works. A secret: shown only while the person looks,
    /// never logged or kept.
    PairOffer { url: String, seconds: u64 },
    /// A phone brought the code and asks to pair: the person says yes or no
    /// ([`Command::PairAnswer`]).
    PairAsked { name: String, platform: String },
    /// The phone `name` is paired.
    Paired { name: String },
    /// The pairing ended without a phone; `message` in the person's words.
    PairEnded {
        why: PairEnd,
        #[serde(default)]
        message: String,
    },
    /// The phones paired with the daemon's machine; `relay` is where they
    /// reach it, `connected` whether the daemon is connected there now.
    Devices {
        devices: Vec<PairedDevice>,
        #[serde(default)]
        relay: String,
        #[serde(default)]
        connected: bool,
    },
}

impl Event {
    /// The squad a squad-wide event (the board, modes, approvals) belongs to.
    pub fn squad(&self) -> Option<&str> {
        match self {
            Event::TicketChanged { squad, .. }
            | Event::TicketHistory { squad, .. }
            | Event::ModeChanged { squad, .. }
            | Event::ApprovalRequested { squad, .. }
            | Event::ApprovalResolved { squad, .. }
            | Event::QuestionsAsked { squad, .. }
            | Event::QuestionsClosed { squad, .. }
            | Event::TeamChangeProposed { squad, .. }
            | Event::TeamChangeClosed { squad, .. }
            | Event::Allowed { squad, .. } => Some(squad),
            Event::BrowserTabs { squad, .. } | Event::BrowserControl { squad, .. } | Event::BrowserDialog { squad, .. } | Event::BrowserWants { squad, .. } => squad.as_deref(),
            _ => None,
        }
    }

    /// The session this event belongs to, if any.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Event::SessionStarted { session, .. }
            | Event::UserMessage { session, .. }
            | Event::MessageQueued { session, .. }
            | Event::MemoWithdrawn { session, .. }
            | Event::QueueSent { session, .. }
            | Event::Notice { session, .. }
            | Event::TextDelta { session, .. }
            | Event::ToolUse { session, .. }
            | Event::ToolResult { session, .. }
            | Event::Thinking { session, .. }
            | Event::ModelKnown { session, .. }
            | Event::Thread { session, .. }
            | Event::Suggestion { session, .. }
            | Event::SteerQueued { session, .. }
            | Event::LaneOpened { session, .. }
            | Event::Steered { session, .. }
            | Event::Estimated { session, .. }
            | Event::SubagentStarted { session, .. }
            | Event::BrowserFrame { session, .. }
            | Event::SubagentStep { session, .. }
            | Event::SubagentEnded { session, .. }
            | Event::TurnEnded { session, .. }
            | Event::BranchLanded { session, .. }
            | Event::NothingChanged { session }
            | Event::SessionClosed { session }
            | Event::WorkStarted { session, .. }
            | Event::WorkEnded { session, .. }
            | Event::Output { session, .. } => Some(*session),
            Event::Error { session, .. } => *session,
            Event::TicketChanged { .. }
            | Event::TicketHistory { .. }
            | Event::ModeChanged { .. }
            | Event::ApprovalRequested { .. }
            | Event::ApprovalResolved { .. }
            | Event::QuestionsAsked { .. }
            | Event::QuestionsClosed { .. }
            | Event::TeamChangeProposed { .. }
            | Event::TeamChangeClosed { .. }
            | Event::Allowed { .. }
            | Event::PlanUsage { .. }
            | Event::Desktop { .. }
            // A browser's own: never part of a conversation.
            | Event::BrowserTabs { .. }
            | Event::BrowserControl { .. }
            | Event::BrowserDialog { .. }
            | Event::BrowserDialogClosed { .. }
            | Event::BrowserWants { .. }
            // Pairing a phone: the daemon machine's, told only the screen that asked.
            | Event::PairOffer { .. }
            | Event::PairAsked { .. }
            | Event::Paired { .. }
            | Event::PairEnded { .. }
            | Event::Devices { .. } => None,
        }
    }

    /// Whether every screen gets it, whatever squad it follows.
    pub fn for_everyone(&self) -> bool {
        matches!(
            self,
            Event::PlanUsage { .. }
                | Event::Desktop { .. }
                | Event::BrowserDialogClosed { .. }
                | Event::BrowserTabs { squad: None, .. }
                | Event::BrowserControl { squad: None, .. }
                | Event::BrowserDialog { squad: None, .. }
        )
    }
}

/// How a tool use ended: whether it worked, a command's exit code, how long it
/// took, the end of what it printed, and — for an edit — the change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// Milliseconds from the call to its answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// The last lines it printed (see [`ToolOutcome::tail`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// Lines of output left out before `output`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skipped: u32,
    /// An edit's or a write's change, as a unified diff headed `diff --git`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl ToolOutcome {
    /// How many lines of output travel with an outcome: the end, where errors are.
    pub const OUTPUT_LINES: usize = 200;
    /// And at most this many bytes of them.
    pub const OUTPUT_BYTES: usize = 16 * 1024;

    /// `text`'s last lines within the limits, and how many lines came before them.
    pub fn tail(text: &str) -> (String, u32) {
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            return (String::new(), 0);
        }
        let lines: Vec<&str> = text.split('\n').collect();
        let (mut kept, mut bytes) = (0, 0);
        for line in lines.iter().rev() {
            if kept == Self::OUTPUT_LINES || (kept > 0 && bytes + line.len() + 1 > Self::OUTPUT_BYTES) {
                break;
            }
            bytes += line.len() + 1;
            kept += 1;
        }
        let from = lines.len() - kept;
        let mut tail = lines[from..].join("\n");
        // One enormous line: its end.
        if tail.len() > Self::OUTPUT_BYTES {
            let mut at = tail.len() - Self::OUTPUT_BYTES;
            while !tail.is_char_boundary(at) {
                at += 1;
            }
            tail = format!("…{}", &tail[at..]);
        }
        (tail, from as u32)
    }
}

/// One line of a session's conversation, as kept and as replayed to a screen
/// that attaches later. Streaming fragments are joined into one `Reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    User { text: String },
    Reply { text: String },
    Tool {
        tool: String,
        target: Option<String>,
        /// How it ended, once it has (and the screen was told).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<Box<ToolOutcome>>,
    },
    /// A quiet line from equipe itself.
    Note { text: String },
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
    /// What follows is about `project` — or, with `None`, about the whole
    /// squad (see [`transcript::scope`]). Never shown itself; a screen shows
    /// one project's slice of the conversation.
    Thread { project: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Ready,
    Working,
}

/// Where an open session stands, for a screen that attaches mid-flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session: SessionId,
    pub project: PathBuf,
    #[serde(default)]
    pub member: Option<Member>,
    pub engine: Engine,
    pub model: Option<String>,
    pub access: Access,
    pub workspace: Option<Workspace>,
    pub state: SessionState,
    /// The first line of the first message.
    pub title: Option<String>,
    pub tokens: u64,
    /// How full its conversation's context was after its latest turn, and the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<(u64, u64)>,
    /// The latest part of the conversation, oldest first. A reply still
    /// streaming is its last `Reply`.
    pub recent: Vec<Entry>,
    /// Memos waiting for the turn in progress to end.
    #[serde(default)]
    pub queued: Vec<String>,
    /// The project the first memo was left about (for older screens).
    #[serde(default)]
    pub queued_project: Option<String>,
    /// The project each memo was left about (`None`: the whole squad).
    #[serde(default)]
    pub queued_in: Vec<Option<String>>,
    /// Messages sent while it works that it hasn't read yet (steering).
    #[serde(default)]
    pub steering: Vec<String>,
    /// The project each of those was said about (`None`: the whole squad).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steering_in: Vec<Option<String>>,
    /// Set when this is a lane of another member's session: its conversation
    /// about one project, running beside the others.
    #[serde(default)]
    pub lane: Option<Lane>,
    /// When it expects its work done, and what it ships live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    /// The ticket it is working on, and where.
    #[serde(default)]
    pub work: Option<WorkInfo>,
    /// Its own mode, when it doesn't follow the squad's.
    #[serde(default)]
    pub mode: Option<Mode>,
    /// How hard it thinks ([`Command::SetEffort`]); `None`: its engine's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The subagents it handed work to this turn that haven't handed back,
    /// in the order they started ([`Event::SubagentStarted`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentInfo>,
}

/// A subagent at work for a member, for a screen that attaches mid-turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentInfo {
    /// As its events name it.
    pub id: String,
    /// What it was handed.
    pub what: String,
    /// Its latest step, once it has taken one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// How long it had been at it when the snapshot was taken, in seconds.
    #[serde(default)]
    pub secs: u64,
}

/// A member's current ticket, for a screen that attaches mid-work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkInfo {
    pub ticket: TicketId,
    pub project: PathBuf,
    pub workspace: Option<Workspace>,
}

/// Everything a squad has going, sent to a screen when it attaches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub squad: SquadName,
    /// The squad's projects (repositories), in the order they joined.
    #[serde(default)]
    pub projects: Vec<PathBuf>,
    /// The members, the PM first.
    pub sessions: Vec<SessionInfo>,
    /// Open tickets and the latest closed ones.
    #[serde(default)]
    pub tickets: Vec<Ticket>,
    /// How much members may do on their own, unless one has its own mode.
    #[serde(default)]
    pub mode: Mode,
    /// Requests for permission waiting for the person, oldest first.
    #[serde(default)]
    pub approvals: Vec<Approval>,
    /// Members' questions waiting for the person, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<decide::QuestionSet>,
    /// Changes to the team the PM proposed, waiting for the person, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_changes: Vec<team::TeamChange>,
    /// How many extra members of a role equipe may bring in when its work waits.
    #[serde(default)]
    pub scale: Option<usize>,
    /// The person turned off guesses at their next message.
    #[serde(default)]
    pub suggestions_off: bool,
    /// The plan's usage, as last reported.
    #[serde(default)]
    pub plan: Option<PlanUsage>,
    /// Who has the Mac's screen, across squads.
    #[serde(default)]
    pub desktop: DesktopState,
    /// The squad's "always allow" rules, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    /// Every member's browser opens on screen ([`Command::ShowBrowser`] for
    /// everyone), rather than hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub browsers_shown: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_screens_line_names_other_squads_members_in_full() {
        let m = |squad: &str, member: &str| MemberRef { squad: squad.into(), member: member.into() };
        assert_eq!(DesktopState::default().summary(Some("acme")), None);
        let desktop = DesktopState { holder: Some(m("acme", "frontend")), queue: vec![m("tax", "pm"), m("acme", "qa")], stopped: false };
        assert_eq!(desktop.summary(Some("acme")).as_deref(), Some("frontend on the screen · 2 waiting"));
        assert_eq!(desktop.summary(Some("tax")).as_deref(), Some("acme/frontend on the screen · 2 waiting"));
        let stopped = DesktopState { stopped: true, ..Default::default() };
        assert_eq!(stopped.summary(None).as_deref(), Some("screen taken back · /desktop resume"));
    }

    fn ticket(id: TicketId) -> Ticket {
        Ticket {
            id,
            title: "Landing hero".into(),
            body: "Headline, subline, one button.".into(),
            status: TicketStatus::Todo,
            owner: Some("frontend".into()),
            assignee: None,
            project: Some("/site".into()),
            depends_on: vec![1],
            branch: None,
            worktree: None,
            merged: None,
            off_main: None,
            kind: TicketKind::Code,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            remaining: Vec::new(),
            images: Vec::new(),
            updated_at: None,
            priority: Default::default(), rank: 0,
        }
    }

    #[test]
    fn a_hand_ins_pictures_travel_as_paths_and_are_left_out_when_none() {
        let mut t = ticket(144);
        assert!(serde_json::to_value(&t).unwrap().get("images").is_none(), "no pictures: the wire as before");
        t.status = TicketStatus::Review;
        t.images = vec!["/h/worktrees/acme.home/144/brand/logo-a.png".into(), "/h/worktrees/acme.home/144/brand/logo-b.svg".into()];
        let wire = serde_json::to_value(&t).unwrap();
        assert_eq!(wire["images"][1], json!("/h/worktrees/acme.home/144/brand/logo-b.svg"));
        assert_eq!(serde_json::from_value::<Ticket>(wire.clone()).unwrap(), t);
        let mut old = wire;
        old.as_object_mut().unwrap().remove("images");
        assert!(serde_json::from_value::<Ticket>(old).unwrap().images.is_empty(), "an older daemon's ticket: none");
    }

    #[test]
    fn a_tickets_asks_travel_as_a_list_and_are_left_out_when_none() {
        let mut t = ticket(117);
        assert!(serde_json::to_value(&t).unwrap().get("asks").is_none(), "nothing asked: the wire as before");
        t.status = TicketStatus::Waiting;
        t.asks = vec!["심사 연락처를 알려 주세요.".into(), "리뷰 일정을 정해 주세요.".into()];
        let wire = serde_json::to_value(&t).unwrap();
        assert_eq!(wire["asks"], json!(["심사 연락처를 알려 주세요.", "리뷰 일정을 정해 주세요."]));
        let back: Ticket = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(back, t);
        // From a daemon that doesn't send them: none, and the note says them.
        let mut old = wire;
        old.as_object_mut().unwrap().remove("asks");
        old["note"] = json!("Waiting on you:\n- [ ] a\n- [ ] b");
        let old: Ticket = serde_json::from_value(old).unwrap();
        assert!(old.asks.is_empty());
        assert_eq!(old.asks(), ["a", "b"]);
    }

    #[test]
    fn a_tickets_priority_and_place_travel_only_when_set_and_older_tickets_are_normal() {
        let t = ticket(4);
        let wire = serde_json::to_value(&t).unwrap();
        assert!(wire.get("priority").is_none() && wire.get("rank").is_none(), "the wire as before: {wire}");
        assert_eq!((t.priority, t.order_key()), (Priority::Normal, 4 * RANK_STEP));
        let set = Ticket { priority: Priority::Urgent, rank: -3, ..ticket(4) };
        let wire = serde_json::to_value(&set).unwrap();
        assert_eq!((wire["priority"].clone(), wire["rank"].clone()), (json!("urgent"), json!(-3)));
        assert_eq!(serde_json::from_value::<Ticket>(wire).unwrap(), set);
        assert!(Priority::Urgent < Priority::High && Priority::Normal < Priority::Low, "sorted most pressing first");
        assert_eq!(Priority::parse(" High "), Some(Priority::High));
    }

    #[test]
    fn the_persons_board_commands_round_trip_and_an_edit_says_what_it_clears() {
        let commands = [
            Command::CreateTicket {
                squad: "acme".into(),
                draft: TicketDraft { title: "Hero".into(), project: Some("/site".into()), assignee: Some("frontend".into()), priority: Priority::High, depends_on: vec![1], ..Default::default() },
            },
            Command::EditTicket { squad: "acme".into(), ticket: 3, changes: TicketEdit { assignee: Some(None), title: Some("x".into()), ..Default::default() } },
            Command::EditTickets { squad: "acme".into(), tickets: vec![3, 4], changes: TicketEdit { priority: Some(Priority::Low), ..Default::default() } },
            Command::MoveTicket { squad: "acme".into(), ticket: 3, status: Some(TicketStatus::Done), before: None },
            Command::MoveTicket { squad: "acme".into(), ticket: 3, status: None, before: Some(2) },
            Command::CommentTicket { squad: "acme".into(), ticket: 3, text: "also the footer".into() },
            Command::TicketHistory { squad: "acme".into(), ticket: 3 },
        ];
        for command in commands {
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command, "{text}");
        }
        // `null` clears; left out, it stays.
        let clear: TicketEdit = serde_json::from_value(json!({ "assignee": null })).unwrap();
        assert_eq!(clear.assignee, Some(None));
        let keep: TicketEdit = serde_json::from_value(json!({ "title": "x" })).unwrap();
        assert_eq!((keep.assignee, keep.project), (None, None));
        assert!(TicketEdit::default().is_empty());
        let edit = Command::EditTicket { squad: "acme".into(), ticket: 3, changes: TicketEdit { project: Some(None), ..Default::default() } };
        assert_eq!(serde_json::to_value(&edit).unwrap(), json!({ "type": "edit_ticket", "squad": "acme", "ticket": 3, "changes": { "project": null } }));
        let history = Event::TicketHistory { squad: "acme".into(), ticket: 3, entries: vec![TicketEntry { at: 1, by: "you".into(), kind: "edited".into(), text: "title".into() }] };
        assert_eq!(history.squad(), Some("acme"));
        assert_eq!(serde_json::from_value::<Event>(serde_json::to_value(&history).unwrap()).unwrap(), history);
    }

    #[test]
    fn only_the_person_accepts_and_it_travels_as_json() {
        let cmd = Command::Accept { squad: "acme".into(), ticket: 3 };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "accept", "squad": "acme", "ticket": 3 }));
        assert_eq!(serde_json::to_value(TicketStatus::InProgress).unwrap(), json!("in_progress"));
    }

    #[test]
    fn why_work_is_off_main_travels_only_when_known_and_kept_work_stops_asking() {
        let cmd = Command::KeepOnBranch { squad: "acme".into(), ticket: 3 };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "keep_on_branch", "squad": "acme", "ticket": 3 }));
        let done = Ticket { status: TicketStatus::Done, branch: Some("equipe/x".into()), merged: Some(false), ..ticket(3) };
        // Unknown: left out of the wire, and an older daemon's ticket reads as unknown.
        let wire = serde_json::to_value(&done).unwrap();
        assert!(wire.get("off_main").is_none(), "{wire}");
        assert_eq!(serde_json::from_value::<Ticket>(wire).unwrap().off_main, None);
        let failed = Ticket { off_main: Some(OffMain::Conflict { detail: "a.rs".into() }), ..done.clone() };
        assert_eq!(serde_json::to_value(&failed).unwrap()["off_main"], json!({ "why": "conflict", "detail": "a.rs" }));
        assert!(done.waits_to_merge() && failed.waits_to_merge());
        let kept = Ticket { off_main: Some(OffMain::Kept), ..done };
        assert_eq!(serde_json::to_value(&kept).unwrap()["off_main"], json!({ "why": "kept" }));
        assert!(kept.not_on_main() && !kept.waits_to_merge(), "still off main, no longer asking");
    }

    #[test]
    fn modes_read_the_way_people_type_them_and_default_to_auto() {
        assert_eq!(Mode::default(), Mode::Auto);
        assert_eq!(Mode::parse("Ask"), Some(Mode::Ask));
        assert_eq!(Mode::parse("full-access"), Some(Mode::Full));
        assert_eq!(Mode::parse("yolo"), None);
        let cmd = Command::Decide { squad: "acme".into(), approval: 7, decision: Decision::Always };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "decide", "squad": "acme", "approval": 7, "decision": "always" }));
        let event = Event::ApprovalRequested {
            squad: "acme".into(),
            approval: Approval {
                id: 7,
                session: 2,
                member: "frontend".into(),
                ticket: Some(3),
                tool: "Bash".into(),
                action: "npm publish".into(),
                reason: None,
                preview: None,
            },
        };
        assert_eq!(event.squad(), Some("acme"));
        assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&event).unwrap()).unwrap(), event);
    }

    #[test]
    fn commands_serialize_with_a_type_tag() {
        let cmd = Command::Submit { session: 1, text: "hi".into(), about: None };
        assert_eq!(
            serde_json::to_value(&cmd).unwrap(),
            json!({ "type": "submit", "session": 1, "text": "hi" })
        );
    }

    #[test]
    fn events_round_trip_through_json() {
        // The contract must survive a socket, not just a channel.
        let events = [
            Event::SessionStarted {
                session: 1,
                project: "/repo".into(),
                engine: Engine::Claude,
                model: None,
                workspace: Some(Workspace { branch: "equipe/try-1".into(), base: "abc".into(), path: "/w".into() }),
                member: Some(Member { role: "frontend".into(), name: "frontend-2".into() }),
                squad: Some("acme".into()),
            },
            Event::TicketChanged { squad: "acme".into(), ticket: ticket(3) },
            Event::WorkStarted { session: 1, ticket: 3, title: "Hero".into(), project: "/site".into(), workspace: None },
            Event::MessageQueued { session: 1, text: "also the footer".into(), project: None },
            Event::Notice { session: 2, text: "↪ You → frontend: hi".into() },
            Event::TextDelta { session: 1, text: "안녕".into() },
            Event::ToolUse { session: 1, tool: "Edit".into(), target: Some("a.rs".into()) },
            Event::TurnEnded {
                session: 1,
                outcome: TurnOutcome::Interrupted,
                usage: Some(Usage { input_tokens: Some(3), output_tokens: None, ..Default::default() }),
            },
            Event::Error { session: None, message: "no engine".into(), next: Some("equipe login".into()) },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event, "{json}");
        }
    }

    #[test]
    fn a_session_spec_round_trips() {
        let spec = SessionSpec {
            engine: None,
            model: Some("haiku".into()),
            access: Access::EditFiles,
            cwd: "/repo".into(),
            role: Some("frontend".into()),
            squad: Some("acme".into()),
        };
        let cmd = Command::Start { session: 7, spec };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"edit_files\""), "{json}");
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    }

    #[test]
    fn session_ids_are_random_and_json_safe() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert!(a < (1 << 53) && a > 0);
    }

    #[test]
    fn a_snapshot_round_trips() {
        let snapshot = Snapshot {
            squad: "acme".into(),
            projects: vec!["/repo".into()],
            tickets: vec![ticket(1)],
            mode: Mode::ReadOnly,
            approvals: Vec::new(),
            questions: vec![decide::QuestionSet {
                id: 3,
                session: 9,
                member: "pm".into(),
                ticket: None,
                questions: vec![decide::Question {
                    question: "Which plan?".into(),
                    header: "Plan".into(),
                    options: vec![decide::Choice { label: "Pro".into(), description: String::new(), ..Default::default() }, decide::Choice { label: "Free".into(), description: "for now".into(), ..Default::default() }],
                    multi_select: false,
                    ..Default::default()
                }],
                asked_at: 1,
            }],
            team_changes: vec![team::TeamChange {
                id: 5,
                change: team::Change::Add,
                role: "mobile".into(),
                count: 1,
                member: None,
                engine: None,
                model: None,
                effort: None,
                reason: "T-1 waits".into(),
                unblocks: vec![1],
                by: "pm".into(),
                session: 9,
                proposed_at: 2,
                busy_on: None,
            }],
            scale: Some(2),
            suggestions_off: false,
            plan: None,
            desktop: DesktopState::default(),
            sessions: vec![SessionInfo {
                session: 9,
                project: "/repo".into(),
                member: Some(Member { role: "pm".into(), name: "pm".into() }),
                engine: Engine::Codex,
                model: Some("m".into()),
                access: Access::ReadOnly,
                workspace: None,
                state: SessionState::Working,
                title: Some("fix it".into()),
                tokens: 12,
                context: None,
                recent: vec![Entry::User { text: "fix it".into() }, Entry::Reply { text: "on it".into() }],
                queued: vec!["and the tests".into()],
                queued_project: None,
                queued_in: vec![None],
                steering: Vec::new(),
                steering_in: vec![],
                lane: None,
                estimate: None,
                work: Some(WorkInfo { ticket: 1, project: "/repo".into(), workspace: None }),
                mode: Some(Mode::Ask),
                effort: Some("high".into()),
                subagents: vec![SubagentInfo { id: "toolu_1".into(), what: "Read the logs".into(), step: Some("Read build.log".into()), secs: 42 }],
            }],
            allowed: vec!["Bash(npm install:*)".into()],
            browsers_shown: true,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    }

    #[test]
    fn questions_are_asked_answered_and_closed_as_json_and_older_snapshots_have_none() {
        let answers = vec![decide::Answer { picked: vec![0], other: None }, decide::Answer::skipped()];
        let cmd = Command::AnswerQuestions { squad: "acme".into(), id: 7, answers: answers.clone(), about: None };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "answer_questions", "squad": "acme", "id": 7, "answers": [{ "picked": [0] }, {}] }));
        let dismiss = Command::DismissQuestions { squad: "acme".into(), id: 7 };
        assert_eq!(serde_json::to_value(&dismiss).unwrap(), json!({ "type": "dismiss_questions", "squad": "acme", "id": 7 }));
        for command in [cmd, dismiss] {
            assert_eq!(serde_json::from_str::<Command>(&serde_json::to_string(&command).unwrap()).unwrap(), command);
        }
        let set = decide::QuestionSet { id: 7, session: 2, member: "backend".into(), ticket: Some(3), questions: vec![], asked_at: 5 };
        let events = [
            Event::QuestionsAsked { squad: "acme".into(), questions: set },
            Event::QuestionsClosed { squad: "acme".into(), id: 7, answers: Some(answers) },
            Event::QuestionsClosed { squad: "acme".into(), id: 7, answers: None },
        ];
        for event in events {
            assert_eq!((event.squad(), event.session()), (Some("acme"), None));
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event, "{json}");
        }
        assert_eq!(serde_json::to_value(Event::QuestionsClosed { squad: "a".into(), id: 1, answers: None }).unwrap(), json!({ "type": "questions_closed", "squad": "a", "id": 1 }));
        let old: Snapshot = serde_json::from_str(r#"{"squad":"acme","sessions":[]}"#).unwrap();
        assert!(old.questions.is_empty());
        assert!(serde_json::to_value(&old).unwrap().get("questions").is_none(), "none: the wire as before");
    }

    #[test]
    fn team_changes_are_proposed_approved_or_declined_as_json_and_older_snapshots_have_none() {
        let approve = Command::ApproveTeamChange { squad: "acme".into(), id: 7 };
        assert_eq!(serde_json::to_value(&approve).unwrap(), json!({ "type": "approve_team_change", "squad": "acme", "id": 7 }));
        let decline = Command::DeclineTeamChange { squad: "acme".into(), id: 7 };
        assert_eq!(serde_json::to_value(&decline).unwrap(), json!({ "type": "decline_team_change", "squad": "acme", "id": 7 }));
        for command in [approve, decline] {
            assert_eq!(serde_json::from_str::<Command>(&serde_json::to_string(&command).unwrap()).unwrap(), command);
        }
        let change = team::TeamChange {
            id: 7,
            change: team::Change::Remove,
            role: "qa".into(),
            count: 1,
            member: Some("qa-2".into()),
            engine: None,
            model: None,
            effort: None,
            reason: "idle".into(),
            unblocks: vec![],
            by: "pm".into(),
            session: 1,
            proposed_at: 3,
            busy_on: Some(4),
        };
        let events = [
            Event::TeamChangeProposed { squad: "acme".into(), change },
            Event::TeamChangeClosed { squad: "acme".into(), id: 7, approved: true, members: vec!["qa-2".into()] },
            Event::TeamChangeClosed { squad: "acme".into(), id: 7, approved: false, members: vec![] },
        ];
        for event in events {
            assert_eq!((event.squad(), event.session(), event.for_everyone()), (Some("acme"), None, false));
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event, "{json}");
        }
        assert_eq!(
            serde_json::to_value(Event::TeamChangeClosed { squad: "a".into(), id: 1, approved: false, members: vec![] }).unwrap(),
            json!({ "type": "team_change_closed", "squad": "a", "id": 1, "approved": false })
        );
        let old: Snapshot = serde_json::from_str(r#"{"squad":"acme","sessions":[]}"#).unwrap();
        assert!(old.team_changes.is_empty());
        assert!(serde_json::to_value(&old).unwrap().get("team_changes").is_none(), "none: the wire as before");
    }

    #[test]
    fn older_messages_without_members_still_parse() {
        // A screen from before members existed sends specs without a role.
        let spec: SessionSpec = serde_json::from_str(r#"{"engine":null,"model":null,"access":"read_only","cwd":"/r"}"#).unwrap();
        assert_eq!(spec.role, None);
    }

    #[test]
    fn always_allow_rules_are_listed_and_taken_back_per_squad() {
        let cmd = Command::Disallow { squad: "acme".into(), rule: "Bash(npm install:*)".into() };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "disallow", "squad": "acme", "rule": "Bash(npm install:*)" }));
        let event = Event::Allowed { squad: "acme".into(), rules: vec!["Bash(gh:*)".into()] };
        assert_eq!((event.squad(), event.session()), (Some("acme"), None));
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn a_snapshot_from_an_older_daemon_has_no_rules_and_browsers_hidden_and_says_nothing_new() {
        let snapshot: Snapshot = serde_json::from_str(r#"{"squad":"acme","sessions":[]}"#).unwrap();
        assert!(snapshot.allowed.is_empty() && !snapshot.browsers_shown);
        // Nothing new is written when there is nothing to say: older screens read the same bytes.
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json.get("allowed").is_none() && json.get("browsers_shown").is_none(), "{json}");
    }

    #[test]
    fn a_session_lists_its_subagents_only_when_it_has_some() {
        let old = r#"{"session":1,"project":"/r","engine":"claude","model":null,"access":"read_only","workspace":null,"state":"working","title":null,"tokens":0,"recent":[]}"#;
        let mut info: SessionInfo = serde_json::from_str(old).unwrap();
        assert!(info.subagents.is_empty(), "an older daemon's session has none");
        assert!(serde_json::to_value(&info).unwrap().get("subagents").is_none(), "none: the wire as before");
        info.subagents.push(SubagentInfo { id: "toolu_1".into(), what: "Read the logs".into(), step: None, secs: 3 });
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["subagents"], json!([{ "id": "toolu_1", "what": "Read the logs", "secs": 3 }]));
        assert_eq!(serde_json::from_value::<SessionInfo>(json).unwrap(), info);
    }

    #[test]
    fn every_session_event_names_its_session() {
        assert_eq!(Event::NothingChanged { session: 4 }.session(), Some(4));
        assert_eq!(Event::Error { session: None, message: String::new(), next: None }.session(), None);
    }

    #[test]
    fn pairing_a_phone_travels_as_json_and_names_no_squad_or_session() {
        for command in [Command::PairStart, Command::PairAnswer { approve: true }, Command::PairCancel, Command::Devices, Command::Unpair { device: "ab12".into() }] {
            let json = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
        }
        assert_eq!(serde_json::to_string(&Command::PairStart).unwrap(), r#"{"type":"pair_start"}"#);
        let device = PairedDevice { id: "ab12".into(), name: "Abel's iPhone".into(), platform: "ios".into(), added: 1_760_000_000, online: false, seen: Some(1_760_000_100) };
        let events = [
            Event::PairOffer { url: "equipe://pair?r=x".into(), seconds: 300 },
            Event::PairAsked { name: "Abel's iPhone".into(), platform: "ios".into() },
            Event::Paired { name: "Abel's iPhone".into() },
            Event::PairEnded { why: PairEnd::Expired, message: "The code ran out of time.".into() },
            Event::Devices { devices: vec![device], relay: "https://relay.equipe.im".into(), connected: true },
        ];
        for event in events {
            assert_eq!((event.squad(), event.session()), (None, None));
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
        }
        // A device the daemon has never seen leaves `seen` out; older lists parse.
        let old: Event = serde_json::from_str(r#"{"type":"devices","devices":[{"id":"a","name":"p"}]}"#).unwrap();
        let Event::Devices { devices, connected, .. } = old else { panic!() };
        assert_eq!((devices[0].seen, devices[0].online, connected), (None, false, false));
        assert_eq!(serde_json::to_value(PairEnd::Unavailable).unwrap(), "unavailable");
    }
}
