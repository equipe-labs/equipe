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

pub mod rpc;
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
    /// `you`, or the member who wrote it.
    pub created_by: String,
    /// The latest word on it: a hand-in summary, why it was reopened.
    pub note: Option<String>,
}

impl Ticket {
    /// Accepted, but its work isn't on the main branch yet.
    pub fn not_on_main(&self) -> bool {
        self.kind == TicketKind::Code && self.status == TicketStatus::Done && self.branch.is_some() && self.merged == Some(false)
    }
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

/// Screen → core: "do this".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Open a session. Answered by [`Event::SessionStarted`] or [`Event::Error`].
    Start { session: SessionId, spec: SessionSpec },
    /// Send the person's message. One turn runs at a time per session: a
    /// message sent while a turn runs waits as a memo ([`Event::MessageQueued`])
    /// and is delivered when the turn ends — or at once, if the turn is stopped.
    Submit { session: SessionId, text: String },
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
    /// The person sends a ticket back with what to change.
    Reopen { squad: SquadName, ticket: TicketId, note: String },
    /// The person did what a `waiting` ticket needed: it goes back to its
    /// member, in the same worktree, with `note` (what was done) to go on from.
    Resume { squad: SquadName, ticket: TicketId, note: String },
    /// The person drops a ticket.
    Cancel { squad: SquadName, ticket: TicketId },
    /// Set how much members may do on their own: the squad's default, or one
    /// member's (`member`); `mode: None` puts a member back on the squad's default.
    SetMode { squad: SquadName, member: Option<String>, mode: Option<Mode> },
    /// The person answers a member's request for permission.
    Decide { squad: SquadName, approval: u64, decision: Decision },
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
    /// The squad's default mode, or one member's, changed. `mode: None`: the
    /// member follows the squad again.
    ModeChanged { squad: SquadName, member: Option<String>, mode: Option<Mode> },
    /// A member waits for the person's permission.
    ApprovalRequested { squad: SquadName, approval: Approval },
    /// It was answered — by the person, or by nobody in time (`Deny`).
    ApprovalResolved { squad: SquadName, approval: u64, decision: Decision },
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
    /// The engine named its model (Claude Code does on its first turn), or the
    /// person changed it.
    ModelKnown { session: SessionId, model: String },
    /// What a watched member's browser shows now: a PNG, base64, and the page.
    BrowserFrame { session: SessionId, image: String, url: Option<String> },
    /// The member handed `what` to a subagent, which works alongside it.
    SubagentStarted { session: SessionId, id: String, what: String },
    /// A subagent's latest step.
    SubagentStep { session: SessionId, id: String, step: String },
    /// A subagent handed back what it found.
    SubagentEnded { session: SessionId, id: String },
    /// The PM's conversation moved to its thread for `project` (`None`: the
    /// whole squad's). Each thread is a conversation of its own.
    Thread { session: SessionId, project: Option<String> },
    /// A guess at what the person will say next to the member who just
    /// answered them, for their empty composer. Not part of the conversation.
    Suggestion { session: SessionId, text: String },
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
}

impl Event {
    /// The squad a squad-wide event (the board, modes, approvals) belongs to.
    pub fn squad(&self) -> Option<&str> {
        match self {
            Event::TicketChanged { squad, .. }
            | Event::ModeChanged { squad, .. }
            | Event::ApprovalRequested { squad, .. }
            | Event::ApprovalResolved { squad, .. } => Some(squad),
            _ => None,
        }
    }

    /// The session this event belongs to, if any.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Event::SessionStarted { session, .. }
            | Event::UserMessage { session, .. }
            | Event::MessageQueued { session, .. }
            | Event::Notice { session, .. }
            | Event::TextDelta { session, .. }
            | Event::ToolUse { session, .. }
            | Event::ModelKnown { session, .. }
            | Event::Thread { session, .. }
            | Event::Suggestion { session, .. }
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
            | Event::ModeChanged { .. }
            | Event::ApprovalRequested { .. }
            | Event::ApprovalResolved { .. }
            | Event::PlanUsage { .. }
            | Event::Desktop { .. } => None,
        }
    }

    /// Whether every screen gets it, whatever squad it follows.
    pub fn for_everyone(&self) -> bool {
        matches!(self, Event::PlanUsage { .. } | Event::Desktop { .. })
    }
}

/// One line of a session's conversation, as kept and as replayed to a screen
/// that attaches later. Streaming fragments are joined into one `Reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    User { text: String },
    Reply { text: String },
    Tool { tool: String, target: Option<String> },
    /// A quiet line from equipe itself.
    Note { text: String },
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
    /// What follows is the PM's conversation about `project` — or, with
    /// `None`, about the whole squad. Screens show one thread at a time.
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
    /// The project the memos were left about.
    #[serde(default)]
    pub queued_project: Option<String>,
    /// The ticket it is working on, and where.
    #[serde(default)]
    pub work: Option<WorkInfo>,
    /// Its own mode, when it doesn't follow the squad's.
    #[serde(default)]
    pub mode: Option<Mode>,
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
            kind: TicketKind::Code,
            created_by: "pm".into(),
            note: None,
        }
    }

    #[test]
    fn only_the_person_accepts_and_it_travels_as_json() {
        let cmd = Command::Accept { squad: "acme".into(), ticket: 3 };
        assert_eq!(serde_json::to_value(&cmd).unwrap(), json!({ "type": "accept", "squad": "acme", "ticket": 3 }));
        assert_eq!(serde_json::to_value(TicketStatus::InProgress).unwrap(), json!("in_progress"));
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
        let cmd = Command::Submit { session: 1, text: "hi".into() };
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
                work: Some(WorkInfo { ticket: 1, project: "/repo".into(), workspace: None }),
                mode: Some(Mode::Ask),
            }],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    }

    #[test]
    fn older_messages_without_members_still_parse() {
        // A screen from before members existed sends specs without a role.
        let spec: SessionSpec = serde_json::from_str(r#"{"engine":null,"model":null,"access":"read_only","cwd":"/r"}"#).unwrap();
        assert_eq!(spec.role, None);
    }

    #[test]
    fn every_session_event_names_its_session() {
        assert_eq!(Event::NothingChanged { session: 4 }.session(), Some(4));
        assert_eq!(Event::Error { session: None, message: String::new(), next: None }.session(), None);
    }
}
