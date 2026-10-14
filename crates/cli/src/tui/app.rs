//! What the terminal UI knows and how it reacts — no terminal, no engines.
//!
//! Keys and core [`Event`]s go in; the [`Command`]s to send to the core come
//! out. Everything here is plain state, so it is tested without a terminal.
//!
//! The UI is a view of one squad as the daemon has it: its members, their
//! conversations, and the board. It starts from a snapshot, follows events, and
//! leaving it changes nothing — the squad keeps working.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use equipe_core::board;
use equipe_core::protocol::{
    Access, Approval, Command, Decision, Engine, Entry, Event, Member, Mode, SessionId, SessionInfo, SessionSpec,
    SessionState, Snapshot, Ticket, TicketId, TicketStatus, Transcript, new_session_id,
};
use equipe_core::roles;

use super::commands;
use super::composer::Composer;

/// How long a first Ctrl-C waits for a second before it stops meaning "leave".
const QUIT_WINDOW: Duration = Duration::from_secs(2);

/// Lines a lone wheel event scrolls — a mouse wheel's notch.
const WHEEL_NOTCH: isize = 3;

/// Wheel events closer together than this are one gesture — a trackpad's —
/// and each moves one line, so the view follows the fingers exactly.
const WHEEL_GESTURE: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Starting,
    Ready,
    Working,
    Closing,
    Closed,
    /// It never started; the transcript says why.
    Failed,
}

/// Something on screen a click acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// The squad's name in the header: lists the other squads.
    Squads,
    /// "+N" in the header, for the projects that didn't fit: the picker.
    Projects,
    /// A member's row in the squad: talk to it.
    Member(usize),
    /// A completion in the list above the composer: take it.
    Choice(usize),
    /// A project's chip in the header — or "all" (`None`): filters to it.
    Project(Option<usize>),
    Pane(Pane),
    Ticket(TicketId),
}

/// What the right-hand pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Conversation,
    Board,
}

/// How a project is doing: members at work on it, and what in it waits for
/// the person (work to review, tickets put down until they do their part).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectStatus {
    pub working: usize,
    pub for_you: usize,
}

pub struct Agent {
    pub id: SessionId,
    pub member: Option<Member>,
    pub engine: Option<Engine>,
    pub model: Option<String>,
    /// Its role edits files (on a ticket, in a worktree).
    pub edit: bool,
    /// The ticket it is on.
    pub ticket: Option<TicketId>,
    pub branch: Option<String>,
    pub state: AgentState,
    pub transcript: Transcript,
    /// Memos waiting for its turn to end.
    pub queued: Vec<String>,
    pub tokens: u64,
    /// How full its conversation's context is, and the window.
    pub context: Option<(u64, u64)>,
    /// Of the tokens counted while this screen watched: all, and those read
    /// from or written to the prompt cache.
    pub seen: (u64, u64),
    /// Output its current turn has produced so far, in tokens.
    pub output: u64,
    /// Lines scrolled up from the bottom; 0 follows new output.
    pub scroll_back: usize,
    /// Lines that arrived while the person was scrolled up.
    pub unseen: usize,
    /// When its current turn started, for the elapsed time shown while it works.
    pub working_since: Option<Instant>,
    /// Long runs of tool use shown in full rather than folded.
    pub expanded: bool,
    /// Where it works now; tool paths are shown relative to it.
    pub base: PathBuf,
    /// Its desk, where it works between tickets.
    desk: PathBuf,
    /// Its own mode; `None` follows the squad's.
    pub mode: Option<Mode>,
    /// For the PM: the thread on screen — the project in focus, `None` for the
    /// whole squad's. Each is a conversation of its own.
    pub thread_view: Option<String>,
    /// A message to send once the session has started.
    pending: Option<String>,
}

impl Agent {
    fn new(id: SessionId, member: Option<Member>, edit: bool, base: PathBuf) -> Self {
        Self {
            id,
            member,
            engine: None,
            model: None,
            edit,
            ticket: None,
            branch: None,
            state: AgentState::Starting,
            transcript: Transcript::default(),
            queued: Vec::new(),
            tokens: 0,
            context: None,
            seen: (0, 0),
            output: 0,
            scroll_back: 0,
            unseen: 0,
            working_since: None,
            expanded: false,
            desk: base.clone(),
            base,
            pending: None,
            mode: None,
            thread_view: None,
        }
    }

    /// The conversation as it shows, through the project in focus. The PM's
    /// is its thread for that project (what came before any thread was the
    /// whole squad's). A member's is everything with the whole squad in view —
    /// each ticket headed with its project — or that project's part, with a
    /// line where other projects' parts were left out. Never the markers.
    pub fn shown(&self) -> Vec<std::borrow::Cow<'_, Entry>> {
        use std::borrow::Cow;
        let entries = self.transcript.entries();
        let pm = self.is_pm();
        let mut projects = std::collections::HashSet::new();
        for entry in entries {
            if let Entry::Thread { project: Some(p) } = entry {
                projects.insert(p.as_str());
            }
        }
        let several = projects.len() > 1;
        let mut current: Option<&str> = None;
        let mut out = Vec::new();
        // Left out in a row: whether anything was, and how many tickets.
        let mut left_out: Option<usize> = None;
        let say_left_out = |out: &mut Vec<Cow<'_, Entry>>, left: Option<usize>| {
            let text = match left {
                None => return,
                Some(0) => "· earlier talk about other projects hidden — Ctrl-P › all shows it".to_owned(),
                Some(1) => "· 1 ticket in other projects hidden — Ctrl-P › all shows it".to_owned(),
                Some(n) => format!("· {n} tickets in other projects hidden — Ctrl-P › all shows them"),
            };
            out.push(Cow::Owned(Entry::Note { text }));
        };
        for entry in entries {
            if let Entry::Thread { project } = entry {
                current = project.as_deref();
                continue;
            }
            let ticket_header = matches!(entry, Entry::Note { text } if text.starts_with("── T-"));
            let visible = if pm { current == self.thread_view.as_deref() } else { self.thread_view.is_none() || current == self.thread_view.as_deref() };
            if !visible {
                if !pm {
                    let n = left_out.get_or_insert(0);
                    *n += usize::from(ticket_header);
                }
                continue;
            }
            say_left_out(&mut out, left_out.take());
            match (entry, current) {
                (Entry::Note { text }, Some(project)) if ticket_header && !pm && self.thread_view.is_none() && several => {
                    out.push(Cow::Owned(Entry::Note { text: format!("── {project} · {}", &text["── ".len()..]) }));
                }
                _ => out.push(Cow::Borrowed(entry)),
            }
        }
        say_left_out(&mut out, left_out);
        out
    }

    fn from_info(info: SessionInfo) -> Self {
        let work = info.work.clone();
        let base = match &work {
            Some(w) => w.workspace.as_ref().map_or_else(|| w.project.clone(), |ws| ws.path.clone()),
            None => info.project.clone(),
        };
        Self {
            id: info.session,
            member: info.member,
            engine: Some(info.engine),
            model: info.model,
            edit: info.access == Access::EditFiles,
            ticket: work.as_ref().map(|w| w.ticket),
            branch: work.and_then(|w| w.workspace).map(|ws| ws.branch),
            state: match info.state {
                SessionState::Ready => AgentState::Ready,
                SessionState::Working => AgentState::Working,
            },
            transcript: Transcript::from_entries(info.recent),
            queued: info.queued,
            tokens: info.tokens,
            context: info.context,
            seen: (0, 0),
            output: 0,
            scroll_back: 0,
            unseen: 0,
            // Attached mid-turn: the clock starts now, not when the turn did.
            working_since: (info.state == SessionState::Working).then(Instant::now),
            expanded: false,
            base,
            desk: info.project,
            pending: None,
            mode: info.mode,
            thread_view: None,
        }
    }

    /// What the team calls it.
    pub fn label(&self) -> String {
        match &self.member {
            Some(member) => member.name.clone(),
            None => self.engine_label(),
        }
    }

    /// The engine, and whether it edits: `claude code · edit`.
    pub fn engine_label(&self) -> String {
        let engine = self.engine.map_or("…", Engine::label);
        if self.edit { format!("{engine} · edit") } else { engine.to_owned() }
    }

    pub fn is_pm(&self) -> bool {
        self.member.as_ref().is_some_and(|m| m.role == roles::PM)
    }

    /// The model as a short name: `opus-5-5` for `claude-opus-5-5[1m]`.
    pub fn short_model(&self) -> Option<String> {
        let model = self.model.as_deref()?;
        let model = model.strip_prefix("claude-").unwrap_or(model);
        Some(model.split('[').next().unwrap_or(model).to_owned())
    }

    pub fn is_open(&self) -> bool {
        !matches!(self.state, AgentState::Closed | AgentState::Failed)
    }

    /// It is busy now: start the clock, unless it is already running.
    fn start_working(&mut self) {
        self.state = AgentState::Working;
        self.working_since.get_or_insert_with(Instant::now);
    }

    /// Its turn ended: leave a quiet line saying how long it took.
    fn stop_working(&mut self) {
        self.output = 0;
        if let Some(since) = self.working_since.take() {
            let took = since.elapsed();
            if took >= Duration::from_secs(1) {
                let at = clock_time();
                self.transcript.push_local(Entry::Note { text: format!("─ worked {}{at}", elapsed(took)) });
            }
        }
    }
}

/// `1m 05s`, `42s`, `1h 02m` — how long something has been going.
pub fn elapsed(took: Duration) -> String {
    let s = took.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// ` · 14:32`, the local time, when the system can tell it.
fn clock_time() -> String {
    std::process::Command::new("date")
        .arg("+%H:%M")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| format!(" · {}", String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or_default()
}

/// What the person was asked after accepting work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    /// Merge this ticket's branch into main?
    Merge(TicketId),
    /// Push main after merging this ticket — merging it either way?
    Push(TicketId),
}

pub struct App {
    pub cwd: PathBuf,
    pub squad: String,
    pub projects: Vec<PathBuf>,
    pub agents: Vec<Agent>,
    /// The board: open tickets and the latest closed ones, by id.
    pub tickets: Vec<Ticket>,
    pub selected: usize,
    pub pane: Pane,
    /// Lines the board is scrolled up from its top.
    pub board_scroll: usize,
    /// The board as a tree of what waits for what (`g`), rather than by state.
    pub board_graph: bool,
    /// Words the board is narrowed to (`/find`, f), kept after Enter.
    pub board_query: Option<String>,
    /// Long sections shown whole (e), not folded past a screenful.
    pub board_expanded: bool,
    /// How many rows the board's list takes, as the person dragged it.
    pub board_split: Option<u16>,
    /// The line between the board's list and its ticket, as last drawn.
    board_rule: Option<BoardRule>,
    dragging_board: bool,
    pub composer: Composer,
    /// A one-line message in the status bar, cleared by the next key.
    pub flash: Option<String>,
    /// A yes/no the person was just asked about work they accepted, shown in
    /// the flash and answered with y or n.
    pub question: Option<Question>,
    pub should_quit: bool,
    /// Whether the mouse wheel scrolls (the terminal then can't select text).
    pub mouse: bool,
    /// Work accepted while this screen watched, for the summary on leaving:
    /// `(ticket, title, branch)`.
    pub accepted: Vec<(TicketId, String, String)>,
    /// How much members may do on their own, unless one has its own mode.
    pub mode: Mode,
    /// How many extra members of a role equipe may bring in when its work waits.
    pub scale: usize,
    /// The Claude plan's usage windows, as last reported.
    pub plan: Option<equipe_core::protocol::PlanUsage>,
    /// The usage screen is open over the conversation.
    pub usage_open: bool,
    /// Requests for permission waiting for the person, oldest first.
    pub approvals: Vec<Approval>,
    /// Lines a page key moves: half the conversation's height on screen.
    pub page: usize,
    /// How far the selected conversation and the board can scroll, as last drawn.
    conversation_max: usize,
    board_max: usize,
    /// The rows the open ticket shows in on the board.
    board_detail: Option<(u16, u16)>,
    /// The last wheel event: when, and which way.
    last_wheel: Option<(Instant, bool)>,
    /// The selected member's conversation, asked for as a file (`/export`).
    pub export: bool,
    /// Asked to open the sign-in browser (`/browser`).
    pub open_browser: bool,
    /// The help screen is open over the conversation.
    pub help_open: bool,
    /// Ctrl-V was pressed: attach the clipboard's image.
    pub paste_image: bool,
    /// iTerm2 is sending an upload as typed input: its status line, then base64
    /// lines until an empty one.
    pub upload: Option<Upload>,
    /// An upload received whole, for the loop to unpack.
    pub uploaded: Option<String>,
    /// The project the board, the review card and new requests are about;
    /// `None` shows all of the squad's projects.
    pub focus: Option<PathBuf>,
    /// Commands a click produced, for the loop to send.
    pub outbox: Vec<Command>,
    /// Another squad to show, and the roles to start it with when it is new.
    pub switch_to: Option<(String, Vec<String>)>,
    /// A folder waiting for the person's `y` before it joins a squad.
    pub confirming: Option<Adding>,
    /// A repository the person said not to add: the loop remembers it.
    pub declined: Option<PathBuf>,
    /// The key just handled picked a choice itself: keep it.
    keep_pick: bool,
    /// Which completion is picked, and the text the person closed completion on.
    pub pick: usize,
    /// The person moved the pick with ↑↓: Enter takes it even after a space.
    picked: bool,
    dismissed: Option<String>,
    /// The ticket picked on the board.
    pub board_selected: Option<TicketId>,
    /// What each ticket's branch changed (`git diff --stat`), by ticket and branch.
    pub diffs: HashMap<TicketId, (String, String)>,
    /// Memory each member's open browser takes, in MB, as last sampled.
    pub browsers: HashMap<String, u64>,
    /// A ticket whose whole diff the person asked to see.
    pub view_diff: Option<TicketId>,
    /// Where clickable things were drawn, as `(from column, to column, row, what)`.
    hits: Vec<(u16, u16, u16, Hit)>,
    /// The squad pane's width, once the person dragged its edge.
    pub squad_width: Option<u16>,
    divider: Option<Divider>,
    dragging: bool,
    text_view: Option<TextView>,
    /// Where the person pressed in the conversation, and what they dragged over.
    press: Option<Spot>,
    pub selection: Option<(Spot, Spot)>,
    /// A selection was just made: the loop copies it.
    pub copy_selection: bool,
    quit_armed: Option<Instant>,
}

/// Where the conversation's text was drawn, and which of its lines is at the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextView {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub top: usize,
}

/// A place in the laid-out conversation: line, and column on it.
pub type Spot = (usize, u16);

/// How many tickets of work not yet started a board section shows before it
/// folds the rest into one line.
pub const SECTION_ROWS: usize = 10;

/// The line between the board's list and the ticket below it, as last
/// drawn: the rows it takes, and the list's first row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardRule {
    pub rows: (u16, u16),
    pub list_top: u16,
}

/// The hairline between the squad and the conversation, as last drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub x: u16,
    pub top: u16,
    pub bottom: u16,
    /// Where the body starts, so a drag to column c makes the pane c - body_x wide.
    pub body_x: u16,
}


impl App {
    /// The squad as the daemon has it. `joining` are roles to bring in now (a
    /// new squad's starting team); a squad with nobody on it starts with its PM.
    pub fn from_snapshot(cwd: PathBuf, snapshot: Snapshot, joining: &[String]) -> (Self, Vec<Command>) {
        let mut app = Self {
            cwd,
            squad: snapshot.squad,
            projects: snapshot.projects,
            agents: snapshot.sessions.into_iter().map(Agent::from_info).collect(),
            tickets: snapshot.tickets,
            selected: 0,
            pane: Pane::Conversation,
            board_scroll: 0,
            board_graph: false,
            board_query: None,
            board_expanded: false,
            board_split: None,
            board_rule: None,
            dragging_board: false,
            composer: Composer::default(),
            flash: None,
            should_quit: false,
            mouse: true,
            accepted: Vec::new(),
            question: None,
            mode: snapshot.mode,
            scale: snapshot.scale.unwrap_or(2),
            plan: snapshot.plan,
            usage_open: false,
            approvals: snapshot.approvals,
            page: 10,
            conversation_max: usize::MAX,
            board_max: usize::MAX,
            board_detail: None,
            last_wheel: None,
            export: false,
            board_selected: None,
            diffs: HashMap::new(),
            browsers: HashMap::new(),
            view_diff: None,
            open_browser: false,
            help_open: false,
            paste_image: false,
            upload: None,
            uploaded: None,
            switch_to: None,
            confirming: None,
            declined: None,
            focus: None,
            outbox: Vec::new(),
            pick: 0,
            picked: false,
            keep_pick: false,
            dismissed: None,
            hits: Vec::new(),
            squad_width: None,
            divider: None,
            dragging: false,
            text_view: None,
            press: None,
            selection: None,
            copy_selection: false,
            quit_armed: None,
        };
        // Talk to the PM first: it is the main contact.
        app.selected = app.agents.iter().position(Agent::is_pm).unwrap_or(0);
        let mut joining: Vec<String> = joining.to_vec();
        if app.agents.is_empty() && !joining.iter().any(|r| r == roles::PM) {
            joining.insert(0, roles::PM.to_owned());
        }
        let commands = joining.iter().flat_map(|role| app.add_member(role, None, false)).collect();
        (app, commands)
    }

    /// The console opens on the whole squad, whatever folder it was run in:
    /// a filter an earlier console left in the daemon is taken off.
    pub fn unfiltered(&self) -> Command {
        Command::Focus { squad: self.squad.clone(), project: None }
    }

    pub fn selected(&self) -> Option<&Agent> {
        self.agents.get(self.selected)
    }

    pub fn total_tokens(&self) -> u64 {
        self.agents.iter().map(|a| a.tokens).sum()
    }

    pub fn working(&self) -> usize {
        self.agents.iter().filter(|a| a.state == AgentState::Working).count()
    }

    /// A member's conversation grew by `lines`. Someone scrolled up to read keeps
    /// their place instead of being carried along, and is told what arrived.
    pub fn grew(&mut self, id: SessionId, lines: usize) {
        if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id)
            && agent.scroll_back > 0
        {
            agent.scroll_back += lines;
            agent.unseen += lines;
        }
    }

    /// Whether a member waits for the person — a request for permission.
    pub fn needs_you(&self, agent: &Agent) -> bool {
        self.approvals.iter().any(|a| a.session == agent.id)
    }

    /// The order the squad is listed in: whoever needs the person first, then
    /// who is working, then the rest — each group in the order they joined.
    pub fn display_order(&self) -> Vec<usize> {
        let rank = |agent: &Agent| match agent.state {
            _ if self.needs_you(agent) => 0,
            AgentState::Working => 1,
            AgentState::Starting | AgentState::Ready => 2,
            AgentState::Closing | AgentState::Closed | AgentState::Failed => 3,
        };
        let mut order: Vec<usize> = (0..self.agents.len()).collect();
        order.sort_by_key(|&i| rank(&self.agents[i]));
        order
    }

    /// The mode a member works in: its own, or the squad's.
    pub fn mode_of(&self, agent: &Agent) -> Mode {
        agent.mode.unwrap_or(self.mode)
    }

    /// The board's tickets in the order they are listed: the person's reviews,
    /// work in progress, to do, proposed, then the latest few finished.
    /// The project a ticket is in: its own, or the squad's first.
    pub fn ticket_project(&self, ticket: &Ticket) -> Option<PathBuf> {
        // A task or action without a project is the squad's, not the first project's.
        if ticket.kind.in_home() {
            return ticket.project.clone();
        }
        ticket.project.clone().or_else(|| self.projects.first().cloned())
    }

    /// Whether a ticket shows under the current focus.
    pub fn in_focus(&self, ticket: &Ticket) -> bool {
        self.focus.is_none() || self.ticket_project(ticket) == self.focus
    }

    /// The project a member works in: its ticket's.
    pub fn agent_project(&self, agent: &Agent) -> Option<PathBuf> {
        let ticket = self.tickets.iter().find(|t| Some(t.id) == agent.ticket)?;
        self.ticket_project(ticket)
    }

    /// How a project is doing, for its place in the header: members at work on
    /// it, and what in it waits for the person.
    pub fn project_status(&self, project: &std::path::Path) -> ProjectStatus {
        let here = |t: &Ticket| self.ticket_project(t).as_deref() == Some(project);
        ProjectStatus {
            working: self
                .agents
                .iter()
                .filter(|a| a.state == AgentState::Working && self.agent_project(a).as_deref() == Some(project))
                .count(),
            for_you: self.tickets.iter().filter(|t| matches!(t.status, TicketStatus::Review | TicketStatus::Waiting) && here(t)).count(),
        }
    }

    /// Take completion `at` (Enter, or a click): its text into the composer,
    /// and run when it is whole.
    fn take_choice(&mut self, at: usize) -> Vec<Command> {
        let Some(choice) = self.choices().into_iter().nth(at) else {
            return Vec::new();
        };
        self.composer.clear();
        self.composer.insert_str(&choice.text);
        if choice.text.ends_with(' ') { Vec::new() } else { self.submit() }
    }

    /// Ctrl-P, or "+N" in the header: the project picker, with the next
    /// project picked — so Ctrl-P, Enter walks them, and typing finds one.
    fn pick_project(&mut self) {
        if self.projects.is_empty() {
            return;
        }
        self.pane = Pane::Conversation;
        self.help_open = false;
        self.composer.clear();
        self.composer.insert_str("/project ");
        let at = self.focus.as_ref().and_then(|f| self.projects.iter().position(|p| p == f));
        // Projects, then "all": after the last project comes all, after all the first.
        self.pick = at.map_or(0, |i| i + 1);
        self.picked = true;
        self.keep_pick = true;
    }

    /// Work waiting for the person's review, under the project filter.
    pub fn to_review_here(&self) -> Vec<&Ticket> {
        self.to_review().into_iter().filter(|t| self.in_focus(t)).collect()
    }

    /// Everything waiting on the person under the project filter: work to
    /// review, then tickets a member put down until the person does their part.
    pub fn for_you_here(&self) -> Vec<&Ticket> {
        let mut out = self.to_review_here();
        out.extend(self.tickets.iter().filter(|t| t.status == TicketStatus::Waiting && self.in_focus(t)));
        out
    }

    /// Whether open work goes in a lane per project — the squad home's last:
    /// with no filter, and work in more than one of them.
    pub fn lanes(&self) -> bool {
        let home_work = self.tickets.iter().any(|t| !t.status.is_closed() && self.ticket_project(t).is_none());
        self.focus.is_none() && self.projects.len() + usize::from(home_work) > 1
    }

    /// Open work as a tree of what waits for what: each ticket with how deep
    /// it is. A ticket waiting on nothing still open is a root; one waiting on
    /// several shows under the first.
    pub fn dependency_tree(&self) -> Vec<(TicketId, usize)> {
        let open: Vec<&Ticket> = self.tickets.iter().filter(|t| !t.status.is_closed() && self.in_focus(t)).collect();
        let is_open = |id: TicketId| open.iter().any(|t| t.id == id);
        let mut out: Vec<(TicketId, usize)> = Vec::new();
        let mut stack: Vec<(TicketId, usize)> = open.iter().filter(|t| !t.depends_on.iter().any(|d| is_open(*d))).map(|t| (t.id, 0)).collect();
        stack.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        while let Some((id, depth)) = stack.pop() {
            if out.iter().any(|(seen, _)| *seen == id) {
                continue;
            }
            out.push((id, depth));
            let mut next: Vec<TicketId> = open.iter().filter(|t| t.depends_on.contains(&id)).map(|t| t.id).collect();
            next.sort_unstable_by(|a, b| b.cmp(a));
            stack.extend(next.into_iter().map(|c| (c, depth + 1)));
        }
        // Anything left in a loop of waiting: listed, not lost.
        for t in &open {
            if !out.iter().any(|(id, _)| *id == t.id) {
                out.push((t.id, 0));
            }
        }
        out
    }

    /// The words the board is narrowed to: being typed (`/find …`), or kept.
    pub fn board_query(&self) -> Option<String> {
        let typed = self.composer.text().strip_prefix("/find ").filter(|_| self.pane == Pane::Board).map(str::trim);
        typed.map(str::to_owned).or_else(|| self.board_query.clone()).map(|q| q.to_lowercase()).filter(|q| !q.is_empty())
    }

    pub fn board_order(&self) -> Vec<TicketId> {
        if let Some(query) = self.board_query() {
            // What matches, open work first, then done work, the latest first.
            let hit = |t: &&Ticket| {
                self.in_focus(t) && (t.title.to_lowercase().contains(&query) || t.body.to_lowercase().contains(&query) || format!("t-{}", t.id) == query)
            };
            let mut open: Vec<TicketId> = self.tickets.iter().filter(hit).filter(|t| !t.status.is_closed()).map(|t| t.id).collect();
            open.sort_unstable();
            let mut closed: Vec<TicketId> = self.tickets.iter().filter(hit).filter(|t| t.status.is_closed()).map(|t| t.id).collect();
            closed.sort_unstable_by(|a, b| b.cmp(a));
            open.extend(closed);
            return open;
        }
        if self.board_graph {
            return self.dependency_tree().into_iter().map(|(id, _)| id).collect();
        }
        // Work not yet started folds past a screenful, unless shown whole (e).
        let fold = |mut ids: Vec<TicketId>, status: TicketStatus| -> Vec<TicketId> {
            if !self.board_expanded && matches!(status, TicketStatus::Todo | TicketStatus::Proposed) {
                ids.truncate(SECTION_ROWS);
            }
            ids
        };
        let group = |statuses: &[TicketStatus]| -> Vec<TicketId> {
            self.tickets.iter().filter(|t| statuses.contains(&t.status) && self.in_focus(t)).map(|t| t.id).collect()
        };
        let mut order = group(&[TicketStatus::Review]);
        order.extend(group(&[TicketStatus::Waiting]));
        let open = [TicketStatus::InProgress, TicketStatus::Todo, TicketStatus::Proposed];
        if self.lanes() {
            // A lane for each project, then the squad home's, its work in the same order.
            for lane in self.projects.iter().map(Some).chain([None]) {
                for status in open {
                    let in_lane: Vec<TicketId> = group(&[status])
                        .into_iter()
                        .filter(|id| self.tickets.iter().find(|t| t.id == *id).and_then(|t| self.ticket_project(t)).as_ref() == lane)
                        .collect();
                    order.extend(fold(in_lane, status));
                }
            }
        } else {
            for status in open {
                order.extend(fold(group(&[status]), status));
            }
        }
        let mut done = group(&[TicketStatus::Done, TicketStatus::Cancelled]);
        done.reverse();
        done.truncate(5);
        order.extend(done);
        order
    }

    /// The ticket picked on the board — the first listed until one is picked.
    pub fn selected_ticket(&self) -> Option<&Ticket> {
        let order = self.board_order();
        let id = self.board_selected.filter(|id| order.contains(id)).or_else(|| order.first().copied())?;
        self.tickets.iter().find(|t| t.id == id)
    }

    fn move_ticket(&mut self, step: isize) {
        let order = self.board_order();
        if order.is_empty() {
            return;
        }
        let at = self.selected_ticket().and_then(|t| order.iter().position(|id| *id == t.id)).unwrap_or(0) as isize;
        let next = order[(at + step).clamp(0, order.len() as isize - 1) as usize];
        // Another ticket reads from its top.
        if self.board_selected != Some(next) {
            self.board_scroll = 0;
        }
        self.board_selected = Some(next);
    }

    /// Where the screen put its clickable things, as last drawn.
    pub fn set_hits(&mut self, hits: Vec<(u16, u16, u16, Hit)>, divider: Option<Divider>) {
        self.hits = hits;
        self.divider = divider;
    }

    pub fn set_text_view(&mut self, view: Option<TextView>) {
        self.text_view = view;
    }

    /// The conversation spot under a screen cell, held to the text's edges.
    fn spot(&self, column: u16, row: u16) -> Option<Spot> {
        let v = self.text_view?;
        let row = row.clamp(v.y, v.y + v.height.saturating_sub(1));
        let column = column.clamp(v.x, v.x + v.width.saturating_sub(1));
        Some((v.top + (row - v.y) as usize, column - v.x))
    }

    /// A click: on a tab, it switches panes; on a ticket, it picks it; on the
    /// squad's edge, it takes hold of it to drag.
    pub fn on_click(&mut self, column: u16, row: u16) {
        // The line under the board's list: take hold of it to resize the list.
        if self.pane == Pane::Board
            && let Some(rule) = self.board_rule
            && (rule.rows.0..=rule.rows.1).contains(&row)
        {
            self.dragging_board = true;
            return;
        }
        // A column either side of the hairline, so it isn't a pixel hunt.
        if let Some(d) = self.divider
            && (d.top..d.bottom).contains(&row)
            && column + 1 >= d.x
            && column <= d.x + 1
        {
            self.dragging = true;
            return;
        }
        let hit = self.hits.iter().find(|(from, to, y, _)| *y == row && (*from..*to).contains(&column)).map(|h| h.3);
        match hit {
            Some(Hit::Pane(pane)) => {
                self.pane = pane;
                self.board_scroll = 0;
            }
            Some(Hit::Project(at)) => {
                let name = at.and_then(|i| self.projects.get(i)).map(|p| equipe_core::board::project_name(p)).unwrap_or_default();
                let commands = self.focus_on(&name);
                self.outbox.extend(commands);
            }
            Some(Hit::Projects) => self.pick_project(),
            Some(Hit::Member(at)) if at < self.agents.len() => {
                self.selected = at;
                self.pane = Pane::Conversation;
                self.help_open = false;
                self.usage_open = false;
            }
            Some(Hit::Member(_)) => {}
            Some(Hit::Choice(at)) => {
                let commands = self.take_choice(at);
                self.outbox.extend(commands);
            }
            Some(Hit::Squads) => {
                self.pane = Pane::Conversation;
                self.help_open = false;
                self.composer.clear();
                self.composer.insert_str("/squad ");
            }
            Some(Hit::Ticket(id)) => {
                self.pane = Pane::Board;
                if self.board_selected != Some(id) {
                    self.board_scroll = 0;
                }
                self.board_selected = Some(id);
            }
            None => {
                // On the conversation's text: the start of a selection.
                let inside = self.text_view.is_some_and(|v| (v.x..v.x + v.width).contains(&column) && (v.y..v.y + v.height).contains(&row));
                self.selection = None;
                self.press = if inside { self.spot(column, row) } else { None };
            }
        }
    }

    /// The board's keys, while nothing is typed: ↑↓ pick a ticket; a accepts,
    /// r asks for changes, m merges accepted work into main, d shows the diff,
    /// Enter talks to whoever owns it.
    fn on_board_key(&mut self, code: KeyCode) -> Option<Vec<Command>> {
        let ticket = self.selected_ticket().cloned();
        match code {
            // Find: the board narrows as the words are typed.
            KeyCode::Char('f') => {
                self.composer.clear();
                self.composer.insert_str("/find ");
            }
            // Long sections whole, or folded again.
            KeyCode::Char('e') => self.board_expanded = !self.board_expanded,
            // What waits for what, as a tree; again for the board by state.
            KeyCode::Char('g') => {
                self.board_graph = !self.board_graph;
                self.board_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_ticket(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_ticket(1),
            KeyCode::Char('a') => {
                let ticket = ticket.filter(|t| !t.status.is_closed())?;
                return Some(self.accept(ticket.id));
            }
            KeyCode::Char('r') if ticket.as_ref().is_some_and(|t| t.status == TicketStatus::Waiting) => {
                let ticket = ticket?;
                self.composer.insert_str(&format!("/resume T-{} ", ticket.id));
                self.flash = Some("Done? Say what you did (or nothing), then Enter — it goes back to its member.".into());
            }
            KeyCode::Char('r') => {
                let ticket = ticket.filter(|t| matches!(t.status, TicketStatus::Review | TicketStatus::Done))?;
                self.composer.insert_str(&format!("/reopen T-{} ", ticket.id));
                self.flash = Some("Say what to change, then Enter.".into());
            }
            KeyCode::Char('m') => {
                let ticket = ticket.filter(Ticket::not_on_main)?;
                self.ask(Question::Push(ticket.id));
            }
            KeyCode::Char('d') => {
                let ticket = ticket.filter(|t| t.branch.is_some())?;
                self.view_diff = Some(ticket.id);
            }
            KeyCode::Enter => {
                let ticket = ticket?;
                let who = ticket.assignee.as_ref().or(ticket.owner.as_ref())?.clone();
                let at = self.agents.iter().position(|a| {
                    a.is_open() && a.member.as_ref().is_some_and(|m| m.name == who || m.role == who)
                })?;
                self.selected = at;
                self.pane = Pane::Conversation;
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// Tickets waiting for the person to accept or send back.
    pub fn to_review(&self) -> Vec<&Ticket> {
        self.tickets.iter().filter(|t| t.status == TicketStatus::Review).collect()
    }

    /// Who a message goes to when Enter is pressed, and whether that is someone
    /// other than the PM while a PM is on the team (so the PM hears about it).
    pub fn addressee(&self) -> Option<(String, bool)> {
        let agent = self.selected()?;
        let has_pm = self.agents.iter().any(|a| a.is_pm() && a.is_open());
        Some((agent.label(), agent.member.is_some() && !agent.is_pm() && has_pm))
    }

    /// Bring a member onto the squad. The selection stays where it is. A
    /// custom role edits a project's files only when the person says so.
    fn add_member(&mut self, role: &str, engine: Option<Engine>, custom_edits: bool) -> Vec<Command> {
        let id = new_session_id();
        let edits = roles::role_or_custom(role).edits || custom_edits;
        // Named by the core once it starts; until then, by its role.
        let member = Member { role: role.to_owned(), name: role.to_owned() };
        self.agents.push(Agent::new(id, Some(member), edits, self.cwd.clone()));
        vec![Command::Start {
            session: id,
            spec: SessionSpec {
                engine,
                model: None,
                access: if edits { Access::EditFiles } else { Access::ReadOnly },
                cwd: self.cwd.clone(),
                role: Some(role.to_owned()),
                squad: Some(self.squad.clone()),
            },
        }]
    }

    // ── events from the core ──

    pub fn on_event(&mut self, event: Event) -> Vec<Command> {
        match event {
            Event::PlanUsage { usage } => {
                self.plan = Some(usage);
                return Vec::new();
            }
            Event::TicketChanged { ticket, .. } => {
                // Its work may have moved on: count the changes again when shown.
                self.diffs.remove(&ticket.id);
                self.on_ticket(ticket);
                return Vec::new();
            }
            Event::ApprovalRequested { approval, .. } => {
                self.approvals.push(approval);
                return Vec::new();
            }
            Event::ApprovalResolved { approval, .. } => {
                self.approvals.retain(|a| a.id != approval);
                return Vec::new();
            }
            Event::ModeChanged { member: None, mode: Some(mode), .. } => {
                self.mode = mode;
                return Vec::new();
            }
            Event::ModeChanged { member: Some(name), mode, .. } => {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.member.as_ref().is_some_and(|m| m.name == name)) {
                    agent.mode = mode;
                }
                return Vec::new();
            }
            _ => {}
        }
        let Some(id) = event.session() else {
            if let Event::Error { message, .. } = event {
                self.flash = Some(message);
            }
            return Vec::new();
        };
        // A member another screen brought in joins the team here too.
        if let Event::SessionStarted { squad: Some(squad), member, .. } = &event
            && *squad == self.squad
            && !self.agents.iter().any(|a| a.id == id)
        {
            self.agents.push(Agent::new(id, member.clone(), false, self.cwd.clone()));
        }
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) else {
            return Vec::new();
        };
        agent.transcript.apply(&event);
        let mut commands = Vec::new();
        match event {
            Event::SessionStarted { engine, model, member, project, .. } => {
                if member.is_some() {
                    agent.member = member;
                }
                agent.engine = Some(engine);
                agent.model = model;
                agent.state = AgentState::Ready;
                agent.base = project.clone();
                agent.desk = project;
                if let Some(text) = agent.pending.take() {
                    agent.start_working();
                    commands.push(Command::Submit { session: id, text });
                }
            }
            Event::UserMessage { .. } => {
                // Typed on another screen, perhaps: either way it is busy now.
                agent.start_working();
                agent.scroll_back = 0;
                agent.unseen = 0;
                // Waiting memos are delivered together as one message.
                agent.queued.clear();
            }
            Event::MessageQueued { text, .. } => agent.queued.push(text),
            Event::Output { tokens, .. } => agent.output = tokens,
            Event::WorkStarted { ticket, project, workspace, .. } => {
                agent.ticket = Some(ticket);
                agent.base = workspace.as_ref().map_or(project, |ws| ws.path.clone());
                agent.branch = workspace.map(|ws| ws.branch);
                // Its first turn on the ticket starts right away.
                agent.start_working();
                agent.scroll_back = 0;
                agent.unseen = 0;
            }
            Event::WorkEnded { .. } => {
                agent.ticket = None;
                agent.branch = None;
                agent.base = agent.desk.clone();
            }
            Event::ModelKnown { model, .. } => agent.model = Some(model),
            Event::TurnEnded { usage, .. } => {
                if agent.state == AgentState::Working {
                    agent.state = AgentState::Ready;
                }
                agent.stop_working();
                if let Some(u) = usage {
                    agent.tokens += u.total();
                    agent.seen.0 += u.total();
                    agent.seen.1 += u.cache_tokens.unwrap_or(0);
                    if let (Some(used), Some(window)) = (u.context_tokens, u.context_window) {
                        agent.context = Some((used, window));
                    }
                }
            }
            Event::SessionClosed { .. } => agent.state = AgentState::Closed,
            Event::Error { message, .. } if agent.state == AgentState::Starting => {
                agent.state = AgentState::Failed;
                // Said in the footer too: the member may not be the one on screen.
                let again = agent.member.as_ref().map(|m| format!(" · /add {} tries again", m.role)).unwrap_or_default();
                self.flash = Some(format!("{} couldn't join: {message}{again}", agent.label()));
            }
            _ => {}
        }
        commands
    }

    fn on_ticket(&mut self, ticket: Ticket) {
        let was = self.tickets.iter().find(|t| t.id == ticket.id).map(|t| t.status);
        if ticket.status == TicketStatus::Done
            && was.is_some_and(|s| s != TicketStatus::Done)
            && let Some(branch) = &ticket.branch
        {
            self.accepted.push((ticket.id, ticket.title.clone(), branch.clone()));
        }
        if ticket.status == TicketStatus::Review && was != Some(TicketStatus::Review) {
            self.flash = Some(format!("T-{} is ready for your review · /accept T-{0} or /reopen T-{0} <what to change>", ticket.id));
        }
        if ticket.status == TicketStatus::Waiting && was != Some(TicketStatus::Waiting) {
            let needs = ticket.note.as_deref().map(board::waiting_needs).unwrap_or_default().join(", ");
            self.flash = Some(format!("T-{} waits for you: {needs} · /resume T-{0} when it's done", ticket.id));
        }
        match self.tickets.iter_mut().find(|t| t.id == ticket.id) {
            Some(slot) => *slot = ticket,
            None => {
                self.tickets.push(ticket);
                self.tickets.sort_by_key(|t| t.id);
            }
        }
    }

    // ── input ──

    pub fn on_paste(&mut self, text: &str) {
        self.flash = None;
        // An image file dropped on the terminal arrives as its path: attach it.
        if let Some(path) = image_path(text) {
            self.attach_image(&path);
            return;
        }
        // A screenshot typed down the terminal (⌃⇧V from the person's Mac).
        match crate::clip::pasted_image(text) {
            Some(Ok(path)) => self.attach_image(&path),
            Some(Err(why)) => self.flash = Some(why),
            None => self.composer.insert_str(text),
        }
    }

    /// Put an image in the message: the member looks at it when it reads it.
    pub fn attach_image(&mut self, path: &std::path::Path) {
        self.composer.insert_str(&format!("[image: {}] ", path.display()));
        self.flash = Some("Image attached — the member looks at it with your message".into());
    }

    /// The rows the open ticket shows in on the board, as last drawn.
    pub fn set_board_detail(&mut self, rows: Option<(u16, u16)>) {
        self.board_detail = rows;
    }

    /// The wheel (or a button) at screen row `row`: on the board, over the
    /// open ticket it reads on; anywhere else it is `on_mouse`.
    pub fn on_mouse_at(&mut self, kind: MouseEventKind, row: u16, now: Instant) {
        let up = match kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            _ => return self.on_mouse(kind, now),
        };
        if self.pane == Pane::Board && self.board_detail.is_some_and(|(top, bottom)| (top..=bottom).contains(&row)) {
            let in_gesture = self.last_wheel.is_some_and(|(at, was_up)| was_up == up && now.duration_since(at) < WHEEL_GESTURE);
            self.last_wheel = Some((now, up));
            let lines = if in_gesture { 1 } else { WHEEL_NOTCH };
            self.scroll(if up { lines } else { -lines });
            return;
        }
        self.on_mouse(kind, now);
    }

    pub fn on_mouse(&mut self, kind: MouseEventKind, now: Instant) {
        if let MouseEventKind::Up(_) = kind {
            self.dragging = false;
            self.dragging_board = false;
            // Letting go after dragging over text copies it.
            if self.press.take().is_some() && self.selection.is_some_and(|(a, b)| a != b) {
                self.copy_selection = true;
            }
            return;
        }
        let up = match kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            _ => return,
        };
        // A trackpad's stream of events moves a line each; a lone notch, three.
        let in_gesture = self.last_wheel.is_some_and(|(at, was_up)| was_up == up && now.duration_since(at) < WHEEL_GESTURE);
        self.last_wheel = Some((now, up));
        // On the board the wheel walks the tickets, a notch at a time.
        if self.pane == Pane::Board {
            if !in_gesture {
                self.move_ticket(if up { -1 } else { 1 });
            }
            return;
        }
        let lines = if in_gesture { 1 } else { WHEEL_NOTCH };
        self.scroll(if up { lines } else { -lines });
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging || self.dragging_board
    }

    pub fn set_board_rule(&mut self, rule: Option<BoardRule>) {
        self.board_rule = rule;
    }

    /// The squad's edge follows the mouse while held; the view keeps it within
    /// bounds.
    pub fn on_drag(&mut self, column: u16, row: u16) {
        if let (true, Some(rule)) = (self.dragging_board, self.board_rule) {
            self.board_split = Some(row.saturating_sub(rule.list_top).max(3));
            return;
        }
        if let (true, Some(d)) = (self.dragging, self.divider) {
            self.squad_width = Some(column.saturating_sub(d.body_x).saturating_sub(1));
            return;
        }
        // Selecting: past the top or bottom edge the conversation scrolls along.
        let (Some(anchor), Some(view)) = (self.press, self.text_view) else {
            return;
        };
        if row < view.y {
            self.scroll(1);
        } else if row >= view.y + view.height {
            self.scroll(-1);
        }
        if let Some(head) = self.spot(column, row) {
            self.selection = Some((anchor, head));
        }
    }

    /// What the screen holds, as last drawn: the page size, and how far the
    /// conversation and the board can scroll. Scrolling stops at the ends rather
    /// than running on past them (and having to come back before anything moves).
    pub fn set_extent(&mut self, page: usize, conversation_max: usize, board_max: usize) {
        self.page = page.max(3);
        self.conversation_max = conversation_max;
        self.board_max = board_max;
        if let Some(agent) = self.agents.get_mut(self.selected) {
            agent.scroll_back = agent.scroll_back.min(conversation_max);
        }
        self.board_scroll = self.board_scroll.min(board_max);
    }

    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Command> {
        let before = self.composer.text().to_owned();
        let commands = self.key(key, now);
        // A new text starts completion over — unless the key itself picked
        // a choice (Ctrl-P opens the picker on the next project).
        if self.composer.text() != before && !std::mem::take(&mut self.keep_pick) {
            self.pick = 0;
            self.picked = false;
            self.dismissed = None;
        }
        commands
    }

    /// What could complete the composer's text: commands after `/`, their
    /// arguments from the board and the squad, names after `@`.
    pub fn choices(&self) -> Vec<commands::Choice> {
        let text = self.composer.text();
        if text.is_empty() || text.contains('\n') || self.dismissed.as_deref() == Some(text) {
            return Vec::new();
        }
        let tickets = |statuses: &[TicketStatus]| -> Vec<(TicketId, &str)> {
            self.tickets.iter().filter(|t| statuses.contains(&t.status)).map(|t| (t.id, t.title.as_str())).collect()
        };
        let ctx = commands::Context {
            review: tickets(&[TicketStatus::Review]),
            reopenable: tickets(&[TicketStatus::Review, TicketStatus::Done]),
            waiting: tickets(&[TicketStatus::Waiting]),
            open: tickets(&[
                TicketStatus::Proposed,
                TicketStatus::Todo,
                TicketStatus::InProgress,
                TicketStatus::Waiting,
                TicketStatus::Review,
            ]),
            unmerged: self.tickets.iter().filter(|t| t.not_on_main()).map(|t| (t.id, t.title.as_str())).collect(),
            members: self.agents.iter().filter_map(|a| a.member.as_ref().map(|m| (m.name.as_str(), m.role.as_str()))).collect(),
            roles: roles::ROLES.iter().map(|r| r.id).collect(),
            // Read only when it's wanted: it opens every squad's journal.
            squads: if text.starts_with("/squad ") { self.other_squads() } else { Vec::new() },
            mode: Some(self.mode),
            projects: self
                .projects
                .iter()
                .map(|p| {
                    let status = self.project_status(p);
                    let mut about = Vec::new();
                    if status.for_you > 0 {
                        about.push(format!("⚑{} waiting on you", status.for_you));
                    }
                    if status.working > 0 {
                        about.push(format!("●{} working", status.working));
                    }
                    if self.focus.as_ref() == Some(p) {
                        about.push("in focus".into());
                    }
                    (equipe_core::board::project_name(p), about.join(" · "))
                })
                .collect(),
            member_modes: self.agents.iter().filter_map(|a| Some((a.member.as_ref()?.name.as_str(), a.mode?))).collect(),
            member_models: self.agents.iter().filter_map(|a| Some((a.member.as_ref()?.name.as_str(), engine_models(a)))).collect(),
            selected_models: self.agents.get(self.selected).map(engine_models).unwrap_or_default(),
        };
        commands::complete(text, &ctx)
    }

    /// What the person would likely type next, offered greyed in the empty
    /// composer — only when the moment calls for one: work waits for their
    /// review, or a member asked them to sign in somewhere.
    pub fn hint(&self) -> Option<String> {
        if !self.composer.is_empty() || !self.approvals.is_empty() || self.pane != Pane::Conversation || self.help_open {
            return None;
        }
        let asked_to_sign_in = self.selected().and_then(|a| a.transcript.entries().iter().rev().find_map(|e| match e {
            Entry::Reply { text } => Some(text.contains("equipe browser") || text.contains("/browser")),
            Entry::User { .. } => Some(false),
            _ => None,
        }));
        if asked_to_sign_in == Some(true) {
            return Some("/browser".into());
        }
        self.to_review_here().iter().map(|t| t.id).max().map(|id| format!("/accept T-{id}"))
    }

    fn key(&mut self, key: KeyEvent, now: Instant) -> Vec<Command> {
        // While iTerm2 sends an upload, its keystrokes are the upload.
        if let Some(upload) = &mut self.upload {
            match upload.take(key) {
                UploadStep::More => {}
                UploadStep::Cancelled => {
                    self.upload = None;
                    self.flash = Some("No image was sent".into());
                }
                UploadStep::Done(data) => {
                    self.upload = None;
                    self.uploaded = Some(data);
                }
            }
            return Vec::new();
        }
        // Any key puts a selection away.
        self.selection = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if let Some(question) = self.question.take()
            && self.composer.is_empty()
            && !ctrl
            && !alt
            && let Some(commands) = self.answer(question, key.code)
        {
            return commands;
        }
        if !(ctrl && key.code == KeyCode::Char('c')) {
            self.quit_armed = None;
            self.flash = None;
        }
        // A folder waiting to be added: `y` adds it, any other key leaves it out.
        // The repository equipe was run in is only offered: `n` is remembered,
        // and any other key goes on to do what it does.
        if let Some(adding) = self.confirming.take() {
            let plain = !ctrl && !alt;
            if key.code == KeyCode::Char('y') && plain {
                self.add_folder(adding);
                return Vec::new();
            }
            let path = adding.folder.path.display().to_string();
            if !adding.offered {
                self.flash = Some(format!("Not added: {path}"));
                return Vec::new();
            }
            if key.code == KeyCode::Char('n') && plain {
                self.flash = Some(format!("Not added, and not asked again · /squad add {path} adds it"));
                self.declined = Some(adding.folder.path);
                return Vec::new();
            }
        }
        // The help and usage screens close on Esc, or as soon as the person types.
        if (self.help_open || self.usage_open) && (key.code == KeyCode::Esc || matches!(key.code, KeyCode::Char(_) | KeyCode::Enter)) {
            self.help_open = false;
            self.usage_open = false;
            if key.code == KeyCode::Esc {
                return Vec::new();
            }
        }
        // Completion, while it offers something: ↑↓ pick, Tab takes, Enter takes
        // (and runs what is whole), Esc puts it away.
        let choices = if ctrl || alt { Vec::new() } else { self.choices() };
        if !choices.is_empty() {
            let picked = self.pick.min(choices.len() - 1);
            match key.code {
                KeyCode::Up => {
                    self.pick = (picked + choices.len() - 1) % choices.len();
                    self.picked = true;
                    return Vec::new();
                }
                KeyCode::Down => {
                    self.pick = (picked + 1) % choices.len();
                    self.picked = true;
                    return Vec::new();
                }
                KeyCode::Esc => {
                    self.dismissed = Some(self.composer.text().to_owned());
                    return Vec::new();
                }
                // Enter after a space sends what is typed, unless a choice was
                // picked by hand: "/accept " must never accept a ticket nobody chose.
                // So does Enter after a `/`: a folder typed whole is the folder
                // meant, not its first subfolder.
                KeyCode::Enter if self.composer.text().ends_with([' ', '/']) && !self.picked => {}
                KeyCode::Tab if !shift => {
                    let choice = &choices[picked];
                    self.composer.clear();
                    self.composer.insert_str(&choice.text);
                    return Vec::new();
                }
                KeyCode::Enter if !shift => return self.take_choice(picked),
                _ => {}
            }
        }
        // → on an empty composer takes the greyed suggestion.
        if key.code == KeyCode::Right && self.composer.is_empty()
            && let Some(hint) = self.hint()
        {
            self.composer.insert_str(&hint);
            return Vec::new();
        }
        // Esc on the board lets a search go before it leaves the board.
        if key.code == KeyCode::Esc
            && self.pane == Pane::Board
            && (self.board_query.is_some() || self.composer.text().starts_with("/find"))
        {
            self.board_query = None;
            if self.composer.text().starts_with("/find") {
                self.composer.clear();
            }
            return Vec::new();
        }
        // A request for permission answers to one key — while nothing is being
        // typed, and not on the board, where the same keys act on tickets: an
        // "a" meant to accept a ticket must never allow a command for good.
        if let (Some(approval), true, false, false, Pane::Conversation) =
            (self.approvals.first(), self.composer.is_empty(), ctrl, alt, self.pane)
        {
            let decision = match key.code {
                KeyCode::Char('y') => Some(Decision::Allow),
                KeyCode::Char('a') => Some(Decision::Always),
                KeyCode::Char('n') => Some(Decision::Deny),
                _ => None,
            };
            if let Some(decision) = decision {
                return vec![Command::Decide { squad: self.squad.clone(), approval: approval.id, decision }];
            }
            // The screen a send would go from, in Preview.
            if key.code == KeyCode::Char('o')
                && let Some(preview) = &approval.preview
            {
                let _ = std::process::Command::new("open").arg(preview).spawn();
                return Vec::new();
            }
        }
        // On the board, with nothing typed, single keys act on the picked ticket.
        if self.pane == Pane::Board && self.composer.is_empty() && !ctrl && !alt
            && let Some(commands) = self.on_board_key(key.code)
        {
            return commands;
        }
        // Ctrl-O unfolds (and folds again) long runs of tool use.
        if key.code == KeyCode::Char('o') && ctrl && self.pane == Pane::Conversation {
            if let Some(agent) = self.agents.get_mut(self.selected) {
                agent.expanded = !agent.expanded;
            }
            return Vec::new();
        }
        match key.code {
            KeyCode::Char('c') if ctrl => return self.ctrl_c(now),
            KeyCode::Char('b') if ctrl => self.toggle_board(),
            // Ctrl-P picks a project to filter to.
            KeyCode::Char('p') if ctrl => self.pick_project(),
            // Alt-1…9 filters to a project at once, Alt-0 the whole squad —
            // where the terminal sends Alt (iTerm2's Option sends Esc+).
            KeyCode::Char(c @ '0'..='9') if alt && !self.projects.is_empty() => {
                let name = match c.to_digit(10).and_then(|n| (n as usize).checked_sub(1)) {
                    Some(at) => match self.projects.get(at) {
                        Some(project) => board::project_name(project),
                        None => return Vec::new(),
                    },
                    None => "all".to_owned(),
                };
                return self.focus_on(&name);
            }
            KeyCode::Enter if alt || shift => self.composer.newline(),
            KeyCode::Char('j') if ctrl => self.composer.newline(),
            // Ctrl-V: the clipboard's image, if it holds one (the loop reads it).
            KeyCode::Char('v') if ctrl => self.paste_image = true,
            // A line ending in `\` continues: the way to a new line in terminals
            // that can't tell Shift-Enter from Enter.
            KeyCode::Enter if self.composer.before_cursor().ends_with('\\') => {
                self.composer.backspace();
                self.composer.newline();
            }
            KeyCode::Enter => return self.submit(),
            KeyCode::Esc if self.pane == Pane::Board => self.pane = Pane::Conversation,
            KeyCode::Esc => return self.interrupt(),
            KeyCode::Tab => self.cycle(1),
            KeyCode::BackTab => self.cycle(-1),
            KeyCode::PageUp => self.scroll(self.page as isize),
            KeyCode::PageDown => self.scroll(-(self.page as isize)),
            KeyCode::Up if !self.composer.recall_older() => self.scroll(1),
            KeyCode::Down if !self.composer.recall_newer() => self.scroll(-1),
            KeyCode::Up | KeyCode::Down => {}
            KeyCode::Left => self.composer.left(),
            KeyCode::Right => self.composer.right(),
            // With nothing typed, Home goes back to the beginning.
            KeyCode::Home if self.composer.is_empty() => self.scroll(isize::MAX / 2),
            KeyCode::Home => self.composer.home(),
            // With nothing typed, End jumps back to the latest line.
            KeyCode::End if self.composer.is_empty() => self.jump_to_latest(),
            KeyCode::End => self.composer.end(),
            KeyCode::Char('a') if ctrl => self.composer.home(),
            KeyCode::Char('e') if ctrl => self.composer.end(),
            KeyCode::Char('u') if ctrl => self.composer.clear(),
            KeyCode::Char('w') if ctrl => self.composer.delete_word(),
            KeyCode::Backspace => self.composer.backspace(),
            KeyCode::Delete => self.composer.delete(),
            KeyCode::Char(c) if !ctrl && !alt => self.composer.insert_char(c),
            _ => {}
        }
        Vec::new()
    }

    fn toggle_board(&mut self) {
        self.pane = if self.pane == Pane::Board { Pane::Conversation } else { Pane::Board };
        self.board_scroll = 0;
    }

    fn ctrl_c(&mut self, now: Instant) -> Vec<Command> {
        // First, Ctrl-C clears a draft — the habit from every shell.
        if !self.composer.is_empty() {
            self.composer.clear();
            return Vec::new();
        }
        match self.quit_armed {
            Some(at) if now.duration_since(at) <= QUIT_WINDOW => self.should_quit = true,
            _ => {
                self.quit_armed = Some(now);
                self.flash = Some("Press Ctrl-C again to leave. The squad keeps working.".into());
            }
        }
        Vec::new()
    }

    fn interrupt(&mut self) -> Vec<Command> {
        match self.selected() {
            Some(agent) if agent.state == AgentState::Working => vec![Command::Interrupt { session: agent.id }],
            _ => Vec::new(),
        }
    }

    fn cycle(&mut self, step: isize) {
        // Tab walks the squad in the order it is listed.
        let order = self.display_order();
        let n = order.len() as isize;
        if n > 0 {
            let at = order.iter().position(|&i| i == self.selected).unwrap_or(0) as isize;
            self.selected = order[(at + step).rem_euclid(n) as usize];
        }
        // Picking someone means talking to them.
        self.pane = Pane::Conversation;
    }

    fn jump_to_latest(&mut self) {
        if let Some(agent) = self.agents.get_mut(self.selected) {
            agent.scroll_back = 0;
            agent.unseen = 0;
        }
    }

    fn scroll(&mut self, lines: isize) {
        let moved = |at: usize, by: isize, max: usize| (at as isize).saturating_add(by).clamp(0, max.min(isize::MAX as usize) as isize) as usize;
        match self.pane {
            // The board reads top-down; the conversation bottom-up.
            Pane::Board => self.board_scroll = moved(self.board_scroll, -lines, self.board_max),
            Pane::Conversation => {
                let max = self.conversation_max;
                if let Some(agent) = self.agents.get_mut(self.selected) {
                    agent.scroll_back = moved(agent.scroll_back, lines, max);
                    if agent.scroll_back == 0 {
                        agent.unseen = 0;
                    }
                }
            }
        }
    }

    fn submit(&mut self) -> Vec<Command> {
        if self.composer.is_empty() {
            return Vec::new();
        }
        let text = self.composer.take();
        if let Some(command) = text.strip_prefix('/') {
            return self.slash(command);
        }
        let mut target = self.selected;
        let mut text = text;
        // `@frontend fix the header` goes to frontend without leaving this view.
        if let Some(mention) = text.strip_prefix('@') {
            let (name, rest) = mention.split_once(char::is_whitespace).unwrap_or((mention, ""));
            match self.agents.iter().position(|a| a.is_open() && a.member.as_ref().is_some_and(|m| m.name == name)) {
                Some(i) if rest.trim().is_empty() => {
                    // Just `@frontend`: go and talk to it.
                    self.selected = i;
                    self.pane = Pane::Conversation;
                    return Vec::new();
                }
                Some(i) => {
                    target = i;
                    text = rest.trim().to_owned();
                }
                None => {
                    self.composer.insert_str(&text);
                    self.flash = Some(format!("No one called @{name} on the squad. /add brings someone in."));
                    return Vec::new();
                }
            }
        }
        let away = target != self.selected || self.pane == Pane::Board;
        let Some(agent) = self.agents.get_mut(target) else {
            return Vec::new();
        };
        let name = agent.label();
        match agent.state {
            // The message shows once the daemon echoes it, the same on every screen.
            AgentState::Ready => {
                if away {
                    self.flash = Some(format!("Sent to {name}."));
                }
                vec![Command::Submit { session: agent.id, text }]
            }
            AgentState::Working => {
                // Left as a memo: the core holds it until the turn ends.
                self.flash = Some(format!("Left {name} a memo — it reads it when this turn ends. Esc stops it to read now."));
                vec![Command::Submit { session: agent.id, text }]
            }
            AgentState::Starting => {
                // Sent as soon as the session is up.
                agent.pending = Some(text);
                Vec::new()
            }
            AgentState::Closing | AgentState::Closed | AgentState::Failed => {
                self.composer.insert_str(&text);
                self.flash = Some(format!("{name} is no longer on the squad. /add brings in someone new."));
                Vec::new()
            }
        }
    }

    fn slash(&mut self, command: &str) -> Vec<Command> {
        let (name, rest) = command.split_once(' ').unwrap_or((command, ""));
        let rest = rest.trim();
        match name {
            "add" => self.add(rest),
            "board" => {
                self.toggle_board();
                Vec::new()
            }
            "find" => {
                let words = rest.trim();
                self.board_query = (!words.is_empty()).then(|| words.to_owned());
                self.pane = Pane::Board;
                self.board_selected = None;
                if words.is_empty() {
                    self.flash = Some("/find <words> — titles, notes and T-numbers, done work too".into());
                }
                Vec::new()
            }
            "tidy" => match self.agents.iter().find(|a| a.is_pm()) {
                Some(pm) => vec![Command::Submit {
                    session: pm.id,
                    text: "Look the board over for tickets that have gone stale, overlap or no longer fit, and propose \
what to do with each — close, merge into another, or keep, and why. Don't cancel anything yourself; I'll decide."
                        .into(),
                }],
                None => {
                    self.flash = Some("No PM on the squad to ask".into());
                    Vec::new()
                }
            },
            "accept" | "merge" | "reopen" | "resume" | "cancel" => self.decide(name, rest),
            "mode" => self.set_mode(rest),
            "close" => match self.agents.get_mut(self.selected) {
                Some(agent) if agent.is_open() => {
                    agent.state = AgentState::Closing;
                    vec![Command::Close { session: agent.id }]
                }
                _ => Vec::new(),
            },
            "export" => {
                self.export = true;
                Vec::new()
            }
            "browser" => self.browser(rest),
            "send-apps" => {
                self.flash = Some(send_apps(&self.squad, rest));
                Vec::new()
            }
            "squad" => self.squad_command(rest),
            "home" => {
                let dir = crate::home::dir(&equipe_providers::credentials::equipe_home(), &self.squad);
                let words: Vec<&str> = rest.split_whitespace().collect();
                self.flash = Some(match crate::home::run(&dir, &words) {
                    Ok(lines) if words.is_empty() => format!("{} · /home changes · /home undo [n]", lines.join(" · ")),
                    Ok(lines) => lines.join("  ·  "),
                    Err(why) => why,
                });
                Vec::new()
            }
            "memory" => {
                let dir = crate::home::dir(&equipe_providers::credentials::equipe_home(), &self.squad);
                let words: Vec<&str> = rest.split_whitespace().collect();
                self.flash = Some(match crate::home::memory(&dir, &words) {
                    // The folder is open in Finder; the line says how much is there and what changed last.
                    Ok(lines) if words.is_empty() => {
                        let latest = lines.iter().find(|l| l.starts_with("lately: ")).map(|l| format!(" · {l}")).unwrap_or_default();
                        format!("{}{latest} · /memory changes", lines[0])
                    }
                    Ok(lines) => lines.join("  ·  "),
                    Err(why) => why,
                });
                Vec::new()
            }
            "project" => self.focus_on(rest),
            "scale" => match rest.trim() {
                "" => {
                    self.flash = Some(format!(
                        "When several tickets of a role are ready at once, equipe brings in up to {} more of it, and lets them go when idle · /scale <0-6>, /scale off",
                        self.scale
                    ));
                    Vec::new()
                }
                word => match if word == "off" { Some(0) } else { word.parse::<usize>().ok().filter(|n| *n <= 6) } {
                    Some(extra) => {
                        self.scale = extra;
                        self.flash = Some(if extra == 0 {
                            "No extra members: each role's tickets wait for its members".into()
                        } else {
                            format!("Up to {extra} more of a role when its work waits")
                        });
                        vec![Command::Scale { squad: self.squad.clone(), extra }]
                    }
                    None => {
                        self.flash = Some("A number from 0 to 6: /scale 3 · /scale off".into());
                        Vec::new()
                    }
                },
            },
            "compact" | "clear" => {
                let target = match rest.trim() {
                    "" => self.agents.get(self.selected),
                    name => self.agents.iter().find(|a| a.member.as_ref().is_some_and(|m| m.name == name)),
                };
                match target.filter(|a| a.member.is_some()) {
                    Some(agent) => vec![Command::Refresh { session: agent.id, summary: name == "compact" }],
                    None => {
                        self.flash = Some(format!("Whose? /{name} <member>"));
                        Vec::new()
                    }
                }
            }
            "model" => {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let (target, model) = match words.as_slice() {
                    [name, model] => (self.agents.iter().find(|a| a.member.as_ref().is_some_and(|m| m.name == *name)), *model),
                    [model] => (self.agents.get(self.selected), *model),
                    _ => {
                        self.flash = Some("/model [member] <model> — e.g. /model mobile opus".into());
                        return Vec::new();
                    }
                };
                match target.filter(|a| a.member.is_some()) {
                    Some(agent) => {
                        self.flash = Some(format!("{} works on {model} from its next turn", agent.label()));
                        vec![Command::SetModel { session: agent.id, model: model.to_owned() }]
                    }
                    None => {
                        self.flash = Some(format!("No member called {} · /model [member] <model>", words[0]));
                        Vec::new()
                    }
                }
            }
            "mouse" => {
                self.mouse = !self.mouse;
                self.flash = Some(if self.mouse {
                    "The wheel scrolls again.".into()
                } else {
                    "Mouse released — select and copy freely. /mouse brings the wheel back.".into()
                });
                Vec::new()
            }
            "help" => {
                self.pane = Pane::Conversation;
                self.help_open = true;
                Vec::new()
            }
            "usage" => {
                self.pane = Pane::Conversation;
                self.help_open = false;
                self.usage_open = true;
                Vec::new()
            }
            "quit" | "exit" => {
                self.should_quit = true;
                Vec::new()
            }
            other => {
                self.flash = Some(format!("Unknown command /{other} — /help lists them."));
                Vec::new()
            }
        }
    }

    /// `/project <name>` filters to one project — the board, the review card
    /// and new requests are about it; `/project all` (or no name) takes the
    /// filter off: the whole squad, which is where the console starts.
    pub fn focus_on(&mut self, name: &str) -> Vec<Command> {
        let name = name.trim();
        let project = match name {
            "" | "all" => None,
            name => match self.projects.iter().find(|p| equipe_core::board::project_name(p) == name) {
                Some(p) => Some(p.clone()),
                None => {
                    let names: Vec<String> = self.projects.iter().map(|p| equipe_core::board::project_name(p)).collect();
                    self.flash = Some(format!("No project {name} in {} · {}", self.squad, names.join(", ")));
                    return Vec::new();
                }
            },
        };
        self.focus = project.clone();
        // Conversations on screen follow: the PM's thread for that project, and
        // each member's part of its work there.
        let thread = project.as_deref().map(equipe_core::board::project_name);
        for agent in self.agents.iter_mut() {
            if agent.thread_view != thread {
                agent.thread_view = thread.clone();
                agent.scroll_back = 0;
            }
        }
        self.board_selected = None;
        let label = project.as_deref().map(equipe_core::board::project_name);
        self.flash = Some(match &label {
            Some(name) => format!("Only {name}: the board and reviews show it, and new work goes there · /project all"),
            None => format!("All of {}: every project and the squad home", self.squad),
        });
        vec![Command::Focus { squad: self.squad.clone(), project: label }]
    }

    /// Every squad but this one, with what each works on.
    fn other_squads(&self) -> Vec<(String, String)> {
        let home = equipe_providers::credentials::equipe_home();
        equipe_core::squads::list(&home)
            .into_iter()
            .filter(|s| s.name != self.squad)
            .map(|s| {
                let projects: Vec<String> = s.projects.iter().map(|p| equipe_core::board::project_name(p)).collect();
                (s.name, format!("works on {}", projects.join(", ")))
            })
            .collect()
    }

    /// `/squad <name>` shows another squad; `/squad new <name> [folder]` starts
    /// one, on that folder if one is given, with the PM and the usual team.
    fn squad_command(&mut self, rest: &str) -> Vec<Command> {
        let home = equipe_providers::credentials::equipe_home();
        let words: Vec<&str> = rest.split_whitespace().collect();
        match words.as_slice() {
            [] => {
                let others: Vec<String> = self.other_squads().into_iter().map(|(name, _)| name).collect();
                self.flash = Some(match others.as_slice() {
                    [] => format!("This is {}, your only squad · /squad new <name> [folder] starts another", self.squad),
                    names => format!("On {} · also {} · /squad <name> switches, /squad new starts one", self.squad, names.join(", ")),
                });
            }
            ["add", folder] => {
                let folder = match typed_folder(folder) {
                    Ok(folder) => folder,
                    Err(err) => {
                        self.flash = Some(err);
                        return Vec::new();
                    }
                };
                if self.projects.contains(&folder.path) {
                    self.flash = Some(format!("{} already works on {}", self.squad, folder.path.display()));
                    return Vec::new();
                }
                if let Some(owner) = equipe_core::squads::find(&home, &folder.path).filter(|o| *o != self.squad) {
                    self.flash = Some(format!("{} already belongs to the squad {owner}", folder.path.display()));
                    return Vec::new();
                }
                self.ask_to_add(Adding { new_squad: None, folder, offered: false });
            }
            ["remove", name] => {
                let Some(project) = self.projects.iter().find(|p| equipe_core::board::project_name(p) == *name || p.to_string_lossy() == *name).cloned() else {
                    self.flash = Some(format!("{} doesn't work on {name}", self.squad));
                    return Vec::new();
                };
                match equipe_core::squads::remove_project(&home, &self.squad, &project) {
                    Ok(()) => {
                        self.projects.retain(|p| *p != project);
                        if self.focus.as_ref() == Some(&project) {
                            self.focus = None;
                        }
                        self.flash = Some(format!("{} no longer works on {name} — its tickets and branches stay", self.squad));
                    }
                    Err(err) => self.flash = Some(err),
                }
            }
            ["add"] | ["remove"] => self.flash = Some("/squad add <folder> · /squad remove <project>".into()),
            ["new"] => {
                self.flash = Some("A name, and the folder it works in if any: /squad new shop ~/Projects/shop".into());
            }
            ["new", name] => match equipe_core::squads::create_without_project(&home, name) {
                Ok(()) => self.switch_to = Some(((*name).to_owned(), crate::onboard::starting_team())),
                Err(err) => self.flash = Some(err),
            },
            ["new", name, folder] => {
                let folder = match typed_folder(folder) {
                    Ok(folder) => folder,
                    Err(err) => {
                        self.flash = Some(err);
                        return Vec::new();
                    }
                };
                if let Some(owner) = equipe_core::squads::find(&home, &folder.path) {
                    self.flash = Some(format!("{} is already {owner}'s · /squad {owner}", folder.name));
                    return Vec::new();
                }
                self.ask_to_add(Adding { new_squad: Some((*name).to_owned()), folder, offered: false });
            }
            [name] if *name == self.squad => self.flash = Some(format!("Already on {name}")),
            [name] => {
                if equipe_core::squads::list(&home).iter().any(|s| s.name == *name) {
                    self.switch_to = Some(((*name).to_owned(), Vec::new()));
                } else {
                    self.flash = Some(format!("No squad called {name} · /squad new {name} starts it"));
                }
            }
            _ => self.flash = Some("/squad <name> switches · /squad new <name> [folder] starts one".into()),
        }
        Vec::new()
    }

    /// Wait for `y` before the folder is added; the footer asks.
    fn ask_to_add(&mut self, adding: Adding) {
        self.flash = None;
        self.confirming = Some(adding);
    }

    /// Offer the repository equipe was run in, which no squad has: asked once,
    /// in the footer, and never in the way of typing.
    pub fn offer(&mut self, folder: crate::onboard::Folder) {
        self.confirming = Some(Adding { new_squad: None, folder, offered: true });
    }

    /// The question while a folder waits: its full path first, where it can't
    /// be cut off, then the keys, its name and its remote.
    pub fn confirm_prompt(&self) -> Option<String> {
        let adding = self.confirming.as_ref()?;
        let folder = &adding.folder;
        let question = match &adding.new_squad {
            Some(name) => format!("Start the squad {name} on {}?", folder.path.display()),
            None if adding.offered => {
                return Some(format!(
                    "{} isn't in a squad. Add {} to {}? y adds, n never asks again · {}",
                    folder.name,
                    folder.path.display(),
                    self.squad,
                    folder.describe()
                ));
            }
            None => format!("Add {} to {}?", folder.path.display(), self.squad),
        };
        Some(format!("{question} y adds, any other key cancels · {}", folder.describe()))
    }

    /// The person said yes: the folder joins this squad, or starts a new one.
    fn add_folder(&mut self, adding: Adding) {
        let home = equipe_providers::credentials::equipe_home();
        let project = adding.folder.path;
        let Some(name) = adding.new_squad else {
            match equipe_core::squads::add_project(&home, &self.squad, &project) {
                Ok(()) => {
                    if !self.projects.contains(&project) {
                        self.projects.push(project.clone());
                    }
                    let has_mobile = self.agents.iter().any(|a| a.member.as_ref().is_some_and(|m| m.role == "mobile"));
                    let tip = if looks_like_an_app(&project) && !has_mobile { " · an app: /add mobile brings in a mobile engineer" } else { "" };
                    let tip = many_projects_tip(self.projects.len()).unwrap_or(tip);
                    self.flash = Some(format!("{} works on {} too · members see it from their next turn{tip}", self.squad, project.display()));
                }
                Err(err) => self.flash = Some(err),
            }
            return;
        };
        match equipe_core::squads::create(&home, &name, &project) {
            Ok(()) => self.switch_to = Some((name, crate::onboard::starting_team())),
            Err(err) => self.flash = Some(err),
        }
    }

    /// `/browser` opens the sign-in window; `/browser show [member]` puts members'
    /// browsers on this Mac's screen, `/browser hide [member]` hides them again.
    fn browser(&mut self, rest: &str) -> Vec<Command> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (show, member) = match words.as_slice() {
            [] => {
                self.open_browser = true;
                return Vec::new();
            }
            ["max"] => {
                let home = equipe_providers::credentials::equipe_home();
                self.flash = Some(format!("Up to {} browsers open at once · /browser max <n> changes it", equipe_core::browser::at_once(&home)));
                return Vec::new();
            }
            ["max", n] => {
                let home = equipe_providers::credentials::equipe_home();
                self.flash = Some(match n.parse::<usize>() {
                    Ok(max @ 1..=16) => match equipe_core::browser::set_at_once(&home, max) {
                        Ok(()) => format!("Up to {max} browsers open at once, from the next one that opens"),
                        Err(err) => format!("Couldn't keep it: {err}"),
                    },
                    _ => "A number from 1 to 16: /browser max 6".into(),
                });
                return Vec::new();
            }
            ["show", rest @ ..] if rest.len() <= 1 => (true, rest.first()),
            ["hide", rest @ ..] if rest.len() <= 1 => (false, rest.first()),
            _ => {
                self.flash = Some("/browser signs you in · /browser show [member] puts browsers on screen · /browser hide [member]".into());
                return Vec::new();
            }
        };
        let whose = member.map_or_else(|| "Members' browsers".to_owned(), |m| format!("{m}'s browser"));
        self.flash = Some(if show {
            format!("{whose} open on this Mac's screen from the next turn · /browser hide to stop")
        } else {
            format!("{whose} run hidden from the next turn")
        });
        vec![Command::ShowBrowser { squad: self.squad.clone(), member: member.map(|m| (*m).to_owned()), show }]
    }

    /// Accept a ticket; when it is code with a branch, ask whether to merge it
    /// into main. Nothing merges unless the person says yes. A task's or action's
    /// work goes into the squad's folder with the acceptance, unasked.
    fn accept(&mut self, ticket: TicketId) -> Vec<Command> {
        let found = self.tickets.iter().find(|t| t.id == ticket);
        if found.is_some_and(|t| !t.kind.in_home() && t.branch.is_some()) {
            self.ask(Question::Merge(ticket));
        } else if found.is_some_and(|t| t.kind.in_home()) {
            self.flash = Some(format!("Accepted T-{ticket} · its work goes into the squad's folder"));
        } else {
            self.flash = Some(format!("Accepted T-{ticket} · /reopen T-{ticket} brings it back"));
        }
        vec![Command::Accept { squad: self.squad.clone(), ticket }]
    }

    fn ask(&mut self, question: Question) {
        self.flash = Some(match question {
            Question::Merge(id) => format!("Accepted T-{id} · merge it into main now? y/n"),
            Question::Push(id) => format!("Merge T-{id} into main — and push main to origin too? y/n · Esc: don't merge"),
        });
        self.question = Some(question);
    }

    /// y or n to a question; `None` for any other key, which puts it away.
    fn answer(&mut self, question: Question, key: KeyCode) -> Option<Vec<Command>> {
        let squad = self.squad.clone();
        let commands = match (question, key) {
            (Question::Merge(id), KeyCode::Char('y')) => {
                self.ask(Question::Push(id));
                Vec::new()
            }
            (Question::Push(ticket), KeyCode::Char(c @ ('y' | 'n'))) => {
                self.flash = Some(format!("Merging T-{ticket} into main…"));
                vec![Command::Merge { squad, ticket, push: c == 'y' }]
            }
            (Question::Merge(id) | Question::Push(id), KeyCode::Char('n') | KeyCode::Esc) => {
                self.flash = Some(format!("T-{id} stays on its branch, not on main · /merge T-{id} when you want it there"));
                Vec::new()
            }
            _ => return None,
        };
        Some(commands)
    }

    /// `/accept T-3`, `/reopen T-3 what to change`, `/resume T-3 what I did`,
    /// `/cancel T-3`: the person's
    /// calls, sent straight to equipe — never through a member.
    fn decide(&mut self, verb: &str, rest: &str) -> Vec<Command> {
        let (id, note) = rest.split_once(' ').unwrap_or((rest, ""));
        let Some(ticket) = board::parse_ticket_id(id).filter(|_| !id.is_empty()) else {
            let (status, what) = match verb {
                "resume" => (TicketStatus::Waiting, "Waiting on you"),
                _ => (TicketStatus::Review, "Waiting for your review"),
            };
            let waiting: Vec<String> =
                self.tickets.iter().filter(|t| t.status == status).map(|t| format!("T-{}", t.id)).collect();
            self.flash = Some(match waiting.as_slice() {
                [] => format!("Which ticket? /{verb} T-3"),
                ids => format!("Which ticket? {what}: {}", ids.join(", ")),
            });
            return Vec::new();
        };
        // Sent back with no word, the member can only guess what to change.
        if verb == "reopen" && note.trim().is_empty() {
            self.composer.insert_str(&format!("/reopen T-{ticket} "));
            self.flash = Some(format!("What should change on T-{ticket}? Write it after /reopen T-{ticket} — that's what the member works from."));
            return Vec::new();
        }
        let squad = self.squad.clone();
        match verb {
            "accept" => self.accept(ticket),
            "merge" => {
                self.ask(Question::Push(ticket));
                Vec::new()
            }
            "reopen" => vec![Command::Reopen { squad, ticket, note: note.trim().to_owned() }],
            "resume" => vec![Command::Resume { squad, ticket, note: note.trim().to_owned() }],
            _ => vec![Command::Cancel { squad, ticket }],
        }
    }

    /// `/mode auto` for the squad, `/mode frontend ask` for one member,
    /// `/mode frontend squad` to put it back on the squad's.
    fn set_mode(&mut self, rest: &str) -> Vec<Command> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let usage = || {
            let modes: Vec<String> = Mode::ALL.iter().map(|m| format!("{} ({})", m.label(), m.short())).collect();
            format!("Modes: {} · /mode <mode> for the squad, /mode <member> <mode> for one", modes.join(" · "))
        };
        let squad = self.squad.clone();
        // The mode that only reads is called that; "plan" isn't one of the names.
        if words.contains(&"plan") {
            self.flash = Some("The mode that only reads is called read-only: /mode read-only".into());
            return Vec::new();
        }
        match words.as_slice() {
            [] => {
                self.flash = Some(format!("The squad works in {} mode. {}", self.mode.label(), usage()));
                Vec::new()
            }
            [mode] => match Mode::parse(mode) {
                Some(mode) => vec![Command::SetMode { squad, member: None, mode: Some(mode) }],
                None => {
                    self.flash = Some(usage());
                    Vec::new()
                }
            },
            [name, mode, ..] => {
                if !self.agents.iter().any(|a| a.member.as_ref().is_some_and(|m| m.name == *name)) {
                    self.flash = Some(format!("No one called {name} on the squad."));
                    return Vec::new();
                }
                let mode = match *mode {
                    "squad" | "default" => None,
                    other => match Mode::parse(other) {
                        Some(mode) => Some(mode),
                        None => {
                            self.flash = Some(usage());
                            return Vec::new();
                        }
                    },
                };
                vec![Command::SetMode { squad, member: Some((*name).to_owned()), mode }]
            }
        }
    }

    /// `/add designer frontend`, `/add qa codex`, `/add custom writer`,
    /// `/add custom writer --edits`.
    fn add(&mut self, rest: &str) -> Vec<Command> {
        let mut words: Vec<&str> = rest.split_whitespace().collect();
        let custom_edits = words.contains(&"--edits");
        words.retain(|w| *w != "--edits");
        let engine = match words.last() {
            Some(&"codex") => Some(Engine::Codex),
            Some(&"claude") => Some(Engine::Claude),
            _ => None,
        };
        if engine.is_some() {
            words.pop();
        }
        let wanted: Vec<String> = match words.as_slice() {
            [] => {
                self.flash = Some(format!("Who? /add designer frontend · roles: {}, or custom <name>", role_names()));
                return Vec::new();
            }
            ["custom"] => {
                self.flash = Some("Name the role: /add custom writer".into());
                return Vec::new();
            }
            ["custom", name, ..] => vec![name.to_lowercase()],
            _ if custom_edits => {
                self.flash = Some("--edits is for a custom role: /add custom writer --edits".into());
                return Vec::new();
            }
            ids => {
                let mut wanted = Vec::new();
                for id in ids {
                    match roles::role(id) {
                        Some(role) => wanted.push(role.id.to_owned()),
                        None => {
                            self.flash = Some(format!("No role '{id}'. Roles: {} — or /add custom {id}", role_names()));
                            return Vec::new();
                        }
                    }
                }
                wanted
            }
        };
        let names = wanted.join(" and ");
        let commands = wanted.iter().flat_map(|role| self.add_member(role, engine, custom_edits)).collect();
        self.flash = Some(format!("{names} joining · Tab or @name to talk to them directly."));
        commands
    }
}

/// A member's conversation as Markdown, for a file the person can search and copy.
pub fn transcript_markdown(agent: &Agent, squad: &str) -> String {
    let mut out = format!("# {} · {squad}\n\n", agent.label());
    for entry in agent.transcript.entries() {
        match entry {
            Entry::User { text } => out.push_str(&format!("**You:** {text}\n\n")),
            Entry::Reply { text } => out.push_str(&format!("{text}\n\n")),
            Entry::Tool { tool, target } => {
                out.push_str(&format!("- {} {}\n", tool_verb(tool), target.as_deref().unwrap_or("")));
            }
            Entry::Note { text } => out.push_str(&format!("_{text}_\n\n")),
            Entry::Thread { project } => out.push_str(&format!("## {}\n\n", project.as_deref().unwrap_or("The whole squad"))),
            Entry::Error { message, next } => {
                out.push_str(&format!("> ✗ {message}{}\n\n", next.as_ref().map(|n| format!(" → `{n}`")).unwrap_or_default()))
            }
            Entry::Landed { branch, stat } => out.push_str(&format!("✓ Landed on `{branch}`\n\n```\n{stat}\n```\n\n")),
        }
    }
    out
}

/// How a tool use reads in one word.
pub fn tool_verb(name: &str) -> &str {
    match name {
        "Read" => "read",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Write" => "write",
        "Bash" => "run",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "fetch",
        // A subagent the member handed side work to.
        "Agent" | "Task" => "delegate",
        other => match other.strip_prefix("mcp__equipe__") {
            // equipe's own board tools read as what they do.
            Some("board") => "board",
            Some("create_ticket") => "ticket",
            Some("update_ticket") => "ticket",
            Some("comment") => "comment",
            Some("submit_for_review") => "hand in",
            Some("block_on_person") => "wait on you",
            Some("ask_person") => "ask you",
            // The member's browser: every step reads as browsing.
            _ if other.starts_with("mcp__browser__") => "browse",
            _ => other,
        },
    }
}

/// An upload arriving from iTerm2 as typed input: a status line (`ok` or
/// `abort`), then base64 lines, then an empty line.
#[derive(Debug, Default)]
pub struct Upload {
    status: Option<String>,
    line: String,
    data: String,
}

pub enum UploadStep {
    More,
    Cancelled,
    Done(String),
}

impl Upload {
    fn take(&mut self, key: KeyEvent) -> UploadStep {
        let end_of_line = matches!(key.code, KeyCode::Enter)
            || (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('j' | 'm')));
        match key.code {
            KeyCode::Esc => return UploadStep::Cancelled,
            KeyCode::Char(c) if !end_of_line => self.line.push(c),
            _ if end_of_line => {
                let line = std::mem::take(&mut self.line);
                match &self.status {
                    None if line.trim() == "ok" => self.status = Some(line),
                    None => return UploadStep::Cancelled,
                    Some(_) if line.is_empty() => return UploadStep::Done(std::mem::take(&mut self.data)),
                    Some(_) => self.data.push_str(line.trim()),
                }
            }
            _ => {}
        }
        UploadStep::More
    }
}

/// The image file a paste names — a path, as terminals paste a dropped file,
/// spaces escaped or the whole of it quoted — when it is one that exists.
pub fn image_path(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    if text.contains('\n') {
        return None;
    }
    let unquoted = text.trim_matches(|c| c == '\'' || c == '"');
    let path = PathBuf::from(unquoted.replace("\\ ", " "));
    (is_image(&path) && path.is_absolute() && path.is_file()).then_some(path)
}

/// The models a member's engine takes, for completion: Claude Code's aliases;
/// for other engines, what it runs now (any name the engine knows works).
fn engine_models(agent: &Agent) -> Vec<String> {
    let mut models: Vec<String> = match agent.engine {
        Some(Engine::Claude) => ["opus", "sonnet", "haiku"].iter().map(|m| (*m).to_owned()).collect(),
        _ => Vec::new(),
    };
    if let Some(current) = agent.model.as_ref().filter(|m| !models.contains(m)) {
        models.push(current.clone());
    }
    models
}

/// Past six projects, a word that unrelated ones may do better in a squad of
/// their own: the PM's priorities and the members' memory stay sharp.
pub fn many_projects_tip(projects: usize) -> Option<&'static str> {
    (projects > 6).then_some(" · many projects now: unrelated ones may do better in a squad of their own — /squad new <name> <folder>")
}

/// The built-in roles, as `/add` takes them.
/// `/send-apps`: the apps where a member's send waits for the person's yes —
/// shown, or changed — kept in the squad's journal, which the daemon reads at
/// each input.
fn send_apps(squad: &str, words: &str) -> String {
    let journal = match equipe_core::journal::Journal::for_squad(&equipe_providers::credentials::equipe_home(), squad) {
        Ok(journal) => journal,
        Err(err) => return format!("Couldn't open the squad's settings: {err}"),
    };
    let apps = journal.messaging_apps();
    if words.trim().is_empty() {
        let list = if apps.is_empty() { "no app".to_owned() } else { apps.join(", ") };
        return format!("Sends from {list} wait for your yes · /send-apps add|remove <app> · reset");
    }
    match equipe_core::desktop_gate::edit_apps(&apps, words) {
        Ok(apps) => match journal.set_messaging_apps(&apps) {
            Ok(()) if apps.is_empty() => "No app asks before a send now · /send-apps reset brings the usual ones back".into(),
            Ok(()) => format!("Sends wait for your yes in: {}", apps.join(", ")),
            Err(err) => format!("Couldn't keep it: {err}"),
        },
        Err(why) => why,
    }
}

fn role_names() -> String {
    roles::ROLES.iter().map(|r| r.id).collect::<Vec<_>>().join(" ")
}

/// Whether a folder holds a mobile app: an Xcode project, Flutter, React
/// Native or Expo, or an Android Gradle project.
pub fn looks_like_an_app(folder: &std::path::Path) -> bool {
    let named = |name: &str| folder.join(name).exists();
    let xcode = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "xcodeproj" || x == "xcworkspace"));
    xcode || named("pubspec.yaml") || named("app.json") || (named("ios") && named("android")) || named("app/src/main/AndroidManifest.xml")
}

/// Whether a file is an image a member can look at, by its extension.
pub fn is_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
}

/// The folder the person typed, read exactly — see [`crate::onboard::resolve_folder`].
fn typed_folder(typed: &str) -> Result<crate::onboard::Folder, String> {
    let home = dirs::home_dir().ok_or("No home folder to read ~ from")?;
    crate::onboard::resolve_folder(typed, &home, &std::env::current_dir().unwrap_or_default())
}

/// A folder the person is asked about before it joins a squad.
#[derive(Debug, Clone)]
pub struct Adding {
    /// The squad it would start, for `/squad new`; `None` adds to this one.
    pub new_squad: Option<String>,
    pub folder: crate::onboard::Folder,
    /// The repository equipe was run in, offered rather than typed.
    pub offered: bool,
}

/// The memory each member's browser takes, from `ps -axo pid=,ppid=,rss=,command=`:
/// a browser server is known by its output folder (`equipe-browser/<squad>/<member>`),
/// and counts with everything it started — Chrome and its helpers.
pub fn browser_memory(ps: &str, squad: &str) -> HashMap<String, u64> {
    let rows: Vec<(u32, u32, u64, &str)> = ps
        .lines()
        .filter_map(|line| {
            // Three numbers, then the command with its own spaces.
            let mut rest = line.trim_start();
            let mut number = || -> Option<&str> {
                let end = rest.find(char::is_whitespace)?;
                let field = &rest[..end];
                rest = rest[end..].trim_start();
                Some(field)
            };
            let pid = number()?.parse().ok()?;
            let ppid = number()?.parse().ok()?;
            let rss = number()?.parse().ok()?;
            Some((pid, ppid, rss, rest))
        })
        .collect();
    let marker = format!("equipe-browser/{squad}/");
    let mut memory = HashMap::new();
    for &(pid, _, _, command) in &rows {
        // The server itself, not the `npx` that started it.
        if !command.contains("@playwright/mcp") || command.contains("npm exec") {
            continue;
        }
        let Some(member) = command.split(&marker).nth(1).and_then(|rest| rest.split(['/', ' ']).next()) else {
            continue;
        };
        let mut family = vec![pid];
        let mut kb = 0;
        while let Some(parent) = family.pop() {
            for &(child, ppid, rss, _) in &rows {
                if child == parent {
                    kb += rss;
                }
                if ppid == parent {
                    family.push(child);
                }
            }
        }
        *memory.entry(member.to_owned()).or_insert(0) += kb / 1024;
    }
    memory
}

/// A tool's path, relative to where the agent works when it's inside it.
pub fn relative(target: &str, base: &std::path::Path) -> String {
    let path = std::path::Path::new(target);
    // macOS reports temp dirs under /private; compare canonical forms.
    let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    canon(path)
        .strip_prefix(canon(base))
        .ok()
        .filter(|rel| !rel.as_os_str().is_empty())
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|| target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use equipe_core::protocol::{TurnOutcome, Usage, WorkInfo, Workspace};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.on_key(key(KeyCode::Char(c)), Instant::now());
        }
    }

    fn enter(app: &mut App) -> Vec<Command> {
        app.on_key(key(KeyCode::Enter), Instant::now())
    }

    fn member(name: &str, role: &str) -> Option<Member> {
        Some(Member { role: role.into(), name: name.into() })
    }

    fn started(id: SessionId, member: Option<Member>) -> Event {
        Event::SessionStarted {
            session: id,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member,
            squad: Some("acme".into()),
        }
    }

    fn empty() -> Snapshot {
        Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], ..Snapshot::default() }
    }

    fn info(id: SessionId, member: Option<Member>) -> SessionInfo {
        SessionInfo {
            session: id,
            project: "/repo".into(),
            member,
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: Vec::new(),
            queued: Vec::new(),
            work: None,
            mode: None,
        }
    }

    fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
        Ticket {
            id,
            title: format!("ticket {id}"),
            body: String::new(),
            status,
            owner: Some("frontend".into()),
            assignee: None,
            project: Some("/repo".into()),
            depends_on: vec![],
            branch: None,
            worktree: None,
            merged: None,
            kind: equipe_core::protocol::TicketKind::Code,
            created_by: "pm".into(),
            note: None,
        }
    }

    /// The PM (selected) and a frontend engineer, both ready.
    fn team() -> (App, SessionId, SessionId) {
        let snapshot = Snapshot { sessions: vec![info(1, member("pm", "pm")), info(2, member("frontend", "frontend"))], ..empty() };
        let (app, _) = App::from_snapshot("/repo".into(), snapshot, &[]);
        (app, 1, 2)
    }

    /// A fresh squad whose PM has started.
    fn ready() -> (App, SessionId) {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session, .. }) = commands.first() else { panic!("no start") };
        let id = *session;
        app.on_event(started(id, member("pm", "pm")));
        (app, id)
    }

    #[test]
    fn an_empty_squad_starts_with_its_pm() {
        let (app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        assert!(matches!(
            commands.as_slice(),
            [Command::Start { spec, .. }]
                if spec.role.as_deref() == Some("pm") && spec.squad.as_deref() == Some("acme") && spec.access == Access::ReadOnly
        ));
        assert_eq!(app.agents[0].state, AgentState::Starting);
        assert_eq!(app.addressee(), Some(("pm".into(), false)));
    }

    #[test]
    fn a_new_squad_brings_in_its_starting_team() {
        let joining = ["pm".to_owned(), "designer".to_owned(), "frontend".to_owned()];
        let (app, commands) = App::from_snapshot("/repo".into(), empty(), &joining);
        let roles: Vec<_> = commands
            .iter()
            .map(|c| match c {
                Command::Start { spec, .. } => (spec.role.clone().unwrap(), spec.access),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(roles, [("pm".into(), Access::ReadOnly), ("designer".into(), Access::EditFiles), ("frontend".into(), Access::EditFiles)]);
        assert_eq!(app.selected().unwrap().label(), "pm");
    }

    #[test]
    fn attaching_shows_the_squad_as_it_is_and_talks_to_the_pm_first() {
        let mut busy = info(2, member("frontend", "frontend"));
        busy.state = SessionState::Working;
        busy.work = Some(WorkInfo {
            ticket: 3,
            project: "/repo".into(),
            workspace: Some(Workspace { branch: "equipe/frontend-t3".into(), base: "abc".into(), path: "/w/3".into() }),
        });
        busy.recent = vec![Entry::User { text: "fix it".into() }];
        busy.tokens = 40;
        let snapshot = Snapshot { sessions: vec![busy, info(1, member("pm", "pm"))], tickets: vec![ticket(3, TicketStatus::InProgress)], ..empty() };
        let (app, commands) = App::from_snapshot("/repo".into(), snapshot, &[]);
        assert!(commands.is_empty(), "nothing new starts when a team exists");
        assert_eq!(app.selected().unwrap().label(), "pm");
        let frontend = &app.agents[0];
        assert_eq!((frontend.state, frontend.ticket), (AgentState::Working, Some(3)));
        assert_eq!(frontend.base, PathBuf::from("/w/3"));
        assert_eq!(app.total_tokens(), 40);
        assert_eq!(app.tickets.len(), 1);
    }

    #[test]
    fn enter_sends_and_the_echo_shows_the_message() {
        let (mut app, id) = ready();
        type_text(&mut app, "hello");
        assert_eq!(enter(&mut app), vec![Command::Submit { session: id, text: "hello".into() }]);
        app.on_event(Event::UserMessage { session: id, text: "hello".into() });
        assert_eq!(app.agents[0].state, AgentState::Working);
        assert_eq!(app.agents[0].transcript.entries(), &[Entry::User { text: "hello".into() }]);
    }

    #[test]
    fn a_pasted_newline_never_sends() {
        let (mut app, id) = ready();
        app.on_paste("fix this\nTypeError: x is undefined\n");
        assert_eq!(app.composer.line_count(), 3);
        assert_eq!(enter(&mut app), vec![Command::Submit { session: id, text: "fix this\nTypeError: x is undefined".into() }]);
    }

    #[test]
    fn alt_enter_and_ctrl_j_add_a_line() {
        let (mut app, _) = ready();
        type_text(&mut app, "a");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), Instant::now());
        type_text(&mut app, "b");
        app.on_key(ctrl('j'), Instant::now());
        type_text(&mut app, "c");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), Instant::now());
        type_text(&mut app, "d\\");
        assert!(enter(&mut app).is_empty(), "a line ending in a backslash continues");
        type_text(&mut app, "e");
        assert_eq!(app.composer.text(), "a\nb\nc\nd\ne");
    }

    #[test]
    fn a_message_typed_while_starting_is_sent_once_ready() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        let id = *id;
        type_text(&mut app, "early");
        assert!(enter(&mut app).is_empty());
        assert_eq!(app.on_event(started(id, member("pm", "pm"))), vec![Command::Submit { session: id, text: "early".into() }]);
    }

    #[test]
    fn a_member_another_screen_brings_in_joins_this_team() {
        let (mut app, _) = ready();
        app.on_event(started(77, member("qa", "qa")));
        assert_eq!(app.agents.len(), 2);
        let mut elsewhere = started(78, member("qa", "qa"));
        if let Event::SessionStarted { squad, .. } = &mut elsewhere {
            *squad = Some("beta".into());
        }
        app.on_event(elsewhere);
        assert_eq!(app.agents.len(), 2, "another squad's members stay out");
    }

    #[test]
    fn esc_stops_only_a_working_member() {
        let (mut app, id) = ready();
        assert!(app.on_key(key(KeyCode::Esc), Instant::now()).is_empty());
        app.on_event(Event::UserMessage { session: id, text: "go".into() });
        assert_eq!(app.on_key(key(KeyCode::Esc), Instant::now()), vec![Command::Interrupt { session: id }]);
    }

    #[test]
    fn a_finished_turn_frees_the_member_and_counts_tokens() {
        let (mut app, id) = ready();
        app.on_event(Event::UserMessage { session: id, text: "go".into() });
        app.on_event(Event::TurnEnded {
            session: id,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(10), output_tokens: Some(5), ..Default::default() }),
        });
        assert_eq!(app.agents[0].state, AgentState::Ready);
        assert_eq!(app.total_tokens(), 15);
    }

    #[test]
    fn ctrl_c_clears_a_draft_then_leaves_on_a_second_press() {
        let (mut app, _) = ready();
        let t0 = Instant::now();
        type_text(&mut app, "draft");
        app.on_key(ctrl('c'), t0);
        assert!(app.composer.is_empty() && !app.should_quit);
        app.on_key(ctrl('c'), t0);
        assert!(!app.should_quit, "one press only warns");
        assert!(app.flash.as_deref().unwrap().contains("keeps working"));
        app.on_key(ctrl('c'), t0 + Duration::from_millis(500));
        assert!(app.should_quit);
    }

    #[test]
    fn leaving_sends_nothing_to_the_squad() {
        let (mut app, _) = ready();
        type_text(&mut app, "/quit");
        assert!(enter(&mut app).is_empty(), "leaving never closes members");
        assert!(app.should_quit);
    }

    #[test]
    fn a_start_failure_marks_the_member_failed_with_the_fix() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), empty(), &[]);
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        app.on_event(Event::Error { session: Some(*id), message: "No engine is ready.".into(), next: Some("equipe login".into()) });
        assert_eq!(app.agents[0].state, AgentState::Failed);
        assert!(matches!(app.agents[0].transcript.entries().last(), Some(Entry::Error { next: Some(n), .. }) if n == "equipe login"));
        assert_eq!(app.flash.as_deref(), Some("pm couldn't join: No engine is ready. · /add pm tries again"));
    }

    #[test]
    fn tool_paths_show_relative_to_where_the_member_works() {
        assert_eq!(relative("/repo/src/x.rs", std::path::Path::new("/repo")), "src/x.rs");
        assert_eq!(relative("/elsewhere/x.rs", std::path::Path::new("/repo")), "/elsewhere/x.rs");
        assert_eq!(relative("*.rs", std::path::Path::new("/repo")), "*.rs");
        assert_eq!(tool_verb("mcp__equipe__submit_for_review"), "hand in");
    }

    #[test]
    fn model_names_shorten_for_the_team_list() {
        let (mut app, id) = ready();
        app.on_event(Event::ModelKnown { session: id, model: "claude-opus-5-5[1m]".into() });
        assert_eq!(app.agents[0].short_model().as_deref(), Some("opus-5-5"));
    }

    // ── the team ──

    #[test]
    fn add_brings_in_members_with_their_access_and_stays_put() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/add designer backend");
        let specs: Vec<_> = enter(&mut app)
            .iter()
            .map(|c| match c {
                Command::Start { spec, .. } => (spec.role.clone().unwrap(), spec.access),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(specs, [("designer".into(), Access::EditFiles), ("backend".into(), Access::EditFiles)]);
        assert_eq!(app.selected().unwrap().label(), "pm", "the conversation stays where it was");
        assert_eq!(app.agents.len(), 4);
    }

    #[test]
    fn add_takes_an_engine_a_custom_role_and_refuses_typos() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/add qa codex");
        assert!(matches!(enter(&mut app).as_slice(), [Command::Start { spec, .. }] if spec.engine == Some(Engine::Codex)));
        type_text(&mut app, "/add custom writer");
        assert!(matches!(enter(&mut app).as_slice(), [Command::Start { spec, .. }]
            if spec.role.as_deref() == Some("writer") && spec.access == Access::ReadOnly));
        // The person decides whether a custom role changes a project's files.
        type_text(&mut app, "/add custom editor --edits");
        assert!(matches!(enter(&mut app).as_slice(), [Command::Start { spec, .. }]
            if spec.role.as_deref() == Some("editor") && spec.access == Access::EditFiles));
        assert!(app.agents.last().unwrap().edit);
        type_text(&mut app, "/add qa --edits");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("for a custom role"), "{:?}", app.flash);
        type_text(&mut app, "/add fronend");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("/add custom fronend"), "{:?}", app.flash);
    }

    #[test]
    fn a_mention_sends_to_that_member_without_leaving_the_pm() {
        let (mut app, _, frontend) = team();
        type_text(&mut app, "@frontend make the header sticky");
        assert_eq!(enter(&mut app), vec![Command::Submit { session: frontend, text: "make the header sticky".into() }]);
        assert_eq!(app.selected().unwrap().label(), "pm");
    }

    #[test]
    fn a_bare_mention_goes_to_talk_to_that_member() {
        let (mut app, _, _) = team();
        type_text(&mut app, "@frontend");
        assert!(enter(&mut app).is_empty());
        assert_eq!(app.selected().unwrap().label(), "frontend");
        assert_eq!(app.addressee(), Some(("frontend".into(), true)), "the PM hears about it");
    }

    #[test]
    fn a_mention_of_nobody_keeps_the_draft() {
        let (mut app, _, _) = team();
        type_text(&mut app, "@ceo hi");
        assert!(enter(&mut app).is_empty());
        assert_eq!(app.composer.text(), "@ceo hi");
        assert!(app.flash.as_deref().unwrap().contains("@ceo"));
    }

    #[test]
    fn a_message_to_a_working_member_is_left_as_a_memo() {
        let (mut app, pm, _) = team();
        app.on_event(Event::UserMessage { session: pm, text: "plan it".into() });
        type_text(&mut app, "also the pricing page");
        assert_eq!(enter(&mut app), vec![Command::Submit { session: pm, text: "also the pricing page".into() }]);
        assert!(app.flash.as_deref().unwrap().contains("memo"));
        app.on_event(Event::MessageQueued { session: pm, text: "also the pricing page".into() });
        assert_eq!(app.agents[0].queued, ["also the pricing page"]);
        // Delivered when the turn ends: the memo becomes the next message.
        app.on_event(Event::UserMessage { session: pm, text: "also the pricing page".into() });
        assert!(app.agents[0].queued.is_empty());
    }

    #[test]
    fn a_member_is_named_by_the_core() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/add frontend");
        let sent = enter(&mut app);
        let Some(Command::Start { session, .. }) = sent.first() else { panic!() };
        app.on_event(started(*session, member("frontend-2", "frontend")));
        assert_eq!(app.agents[2].label(), "frontend-2");
    }

    // ── the board ──

    #[test]
    fn the_board_follows_ticket_changes_and_ctrl_b_shows_it() {
        let (mut app, _, _) = team();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(2, TicketStatus::Todo) });
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(1, TicketStatus::Todo) });
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(2, TicketStatus::InProgress) });
        let board: Vec<_> = app.tickets.iter().map(|t| (t.id, t.status)).collect();
        assert_eq!(board, [(1, TicketStatus::Todo), (2, TicketStatus::InProgress)]);
        app.on_key(ctrl('b'), Instant::now());
        assert_eq!(app.pane, Pane::Board);
        app.on_key(key(KeyCode::Esc), Instant::now());
        assert_eq!(app.pane, Pane::Conversation, "Esc closes the board before it stops anyone");
    }

    #[test]
    fn work_handed_in_asks_for_the_persons_review() {
        let (mut app, _, _) = team();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(4, TicketStatus::InProgress) });
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(4, TicketStatus::Review) });
        assert!(app.flash.as_deref().unwrap().contains("/accept T-4"), "{:?}", app.flash);
        assert_eq!(app.to_review().len(), 1);
    }

    #[test]
    fn a_members_browser_counts_with_everything_it_started() {
        let ps = "\
  10     1  50000 node /x/npm exec @playwright/mcp@0.0.83 --output-dir /T/equipe-browser/acme/frontend
  11    10 100000 node /x/node_modules/.bin/mcp-server-playwright @playwright/mcp --output-dir /T/equipe-browser/acme/frontend --init-page k.ts
  12    11 409600 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --headless
  13    12 204800 Google Chrome Helper (Renderer)
  20     1 100000 node @playwright/mcp --output-dir /T/equipe-browser/other/pm
  30     1 999999 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        let memory = browser_memory(ps, "acme");
        assert_eq!(memory.get("frontend"), Some(&((100000 + 409600 + 204800) / 1024)));
        assert_eq!(memory.len(), 1, "another squad's browser and the person's own Chrome don't count: {memory:?}");
    }

    #[test]
    fn completion_offers_commands_their_arguments_and_names() {
        let (mut app, _, _) = team();
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(8, TicketStatus::Review) });
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(10, TicketStatus::Review) });
        // A command name: Tab takes it.
        type_text(&mut app, "/acc");
        assert_eq!(app.choices()[0].label, "/accept T-n");
        app.on_key(key(KeyCode::Tab), Instant::now());
        assert_eq!(app.composer.text(), "/accept ");
        // After the space, Enter sends what's typed — nobody picked a ticket.
        assert_eq!(app.choices().len(), 2);
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("Which ticket"), "{:?}", app.flash);
        // Picked by hand, Enter takes it and runs it.
        type_text(&mut app, "/accept ");
        app.on_key(key(KeyCode::Down), Instant::now());
        assert_eq!(enter(&mut app), vec![Command::Accept { squad: "acme".into(), ticket: 10 }]);
        // A whole command runs on Enter; Esc puts the list away.
        type_text(&mut app, "/bo");
        assert!(enter(&mut app).is_empty());
        assert_eq!(app.pane, Pane::Board);
        app.pane = Pane::Conversation;
        type_text(&mut app, "@fr");
        app.on_key(key(KeyCode::Tab), Instant::now());
        assert_eq!(app.composer.text(), "@frontend ");
        app.composer.clear();
        type_text(&mut app, "/mo");
        app.on_key(key(KeyCode::Esc), Instant::now());
        assert!(app.choices().is_empty());
    }

    #[test]
    fn the_empty_composer_suggests_what_the_moment_calls_for() {
        let (mut app, _, _) = team();
        assert_eq!(app.hint(), None, "nothing waiting, nothing suggested");
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(8, TicketStatus::Review) });
        assert_eq!(app.hint().as_deref(), Some("/accept T-8"));
        app.on_key(key(KeyCode::Right), Instant::now());
        assert_eq!(app.composer.text(), "/accept T-8");
        assert_eq!(app.hint(), None, "not while typing");
    }

    #[test]
    fn help_opens_over_the_conversation_and_esc_closes_it() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/help");
        assert!(enter(&mut app).is_empty());
        assert!(app.help_open);
        app.on_key(key(KeyCode::Esc), Instant::now());
        assert!(!app.help_open);
    }

    #[test]
    fn a_squad_is_switched_to_by_name_and_started_on_a_folder() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/squad acme");
        enter(&mut app);
        assert!(app.flash.as_deref().unwrap().contains("Already on acme"), "{:?}", app.flash);
        assert!(app.switch_to.is_none());
        type_text(&mut app, "/squad new");
        enter(&mut app);
        assert!(app.flash.as_deref().unwrap().contains("the folder it works in if any"), "{:?}", app.flash);
        type_text(&mut app, "/squad new Bad!");
        enter(&mut app);
        assert!(app.flash.as_deref().unwrap().contains("isn't a squad name"), "{:?}", app.flash);
        type_text(&mut app, "/squad new shop /no/such/folder");
        enter(&mut app);
        assert!(app.flash.as_deref().unwrap().contains("No folder"), "{:?}", app.flash);
        assert!(app.switch_to.is_none());
    }

    /// `equipe` beside `equipe-labs-profile`, both with a subfolder.
    fn siblings(tag: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("equipe-add-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for dir in ["equipe/crates", "equipe-labs-profile/a"] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
        }
        tmp.canonicalize().unwrap()
    }

    #[test]
    fn squad_add_asks_about_exactly_the_folder_typed() {
        let (mut app, _, _) = team();
        let tmp = siblings("exact");
        type_text(&mut app, &format!("/squad add {}/equipe", tmp.display()));
        assert!(!app.choices().is_empty(), "completion offers the neighbours too");
        assert!(enter(&mut app).is_empty());
        let adding = app.confirming.as_ref().expect("asks first");
        assert_eq!(adding.folder.path, tmp.join("equipe"));
        let prompt = app.confirm_prompt().unwrap();
        assert!(prompt.starts_with(&format!("Add {} to acme?", tmp.join("equipe").display())), "{prompt}");
        assert!(prompt.contains("y adds"), "{prompt}");
        // Anything but `y` leaves it out, and nothing is added.
        app.on_key(key(KeyCode::Char('n')), Instant::now());
        assert!(app.confirming.is_none());
        assert!(app.flash.as_deref().unwrap().starts_with("Not added"), "{:?}", app.flash);
        assert!(!app.projects.contains(&tmp.join("equipe")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    fn offered(app: &mut App, tmp: &Path) {
        let folder = crate::onboard::resolve_folder(&tmp.join("equipe").display().to_string(), Path::new("/nowhere"), tmp).unwrap();
        app.offer(folder);
    }

    #[test]
    fn the_repository_equipe_was_run_in_is_offered_and_no_is_remembered() {
        let (mut app, _, _) = team();
        let tmp = siblings("offer");
        offered(&mut app, &tmp);
        let prompt = app.confirm_prompt().unwrap();
        assert!(prompt.starts_with(&format!("equipe isn't in a squad. Add {} to acme? y adds, n never asks again", tmp.join("equipe").display())), "{prompt}");
        app.on_key(key(KeyCode::Char('n')), Instant::now());
        assert!(app.confirming.is_none());
        assert_eq!(app.declined.as_deref(), Some(tmp.join("equipe").as_path()), "the loop remembers the no");
        assert!(!app.projects.contains(&tmp.join("equipe")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn typing_past_an_offer_leaves_it_out_and_keeps_the_key() {
        let (mut app, _, _) = team();
        let tmp = siblings("past");
        offered(&mut app, &tmp);
        app.on_key(key(KeyCode::Char('h')), Instant::now());
        assert!(app.confirming.is_none());
        assert!(app.declined.is_none(), "only n is remembered");
        assert_eq!(app.composer.text(), "h", "the key goes on to the composer");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn enter_after_a_slash_sends_the_folder_not_its_first_subfolder() {
        let (mut app, _, _) = team();
        let tmp = siblings("slash");
        type_text(&mut app, &format!("/squad add {}/equipe-labs-profile/", tmp.display()));
        assert!(!app.choices().is_empty(), "its subfolder is offered");
        enter(&mut app);
        assert_eq!(app.confirming.as_ref().unwrap().folder.path, tmp.join("equipe-labs-profile"));
        app.on_key(key(KeyCode::Esc), Instant::now());
        assert!(app.confirming.is_none());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn squad_add_refuses_home_and_missing_folders_without_asking() {
        let (mut app, _, _) = team();
        for typed in ["/squad add ~", "/squad add ~/", "/squad add /", "/squad new shop ~", "/squad add /no/such/folder"] {
            type_text(&mut app, typed);
            enter(&mut app);
            assert!(app.confirming.is_none(), "{typed}");
            let flash = app.flash.clone().unwrap_or_default();
            assert!(flash.contains("home folder") || flash.contains("No folder"), "{typed}: {flash}");
            app.composer.clear();
        }
        assert!(app.switch_to.is_none());
    }

    #[test]
    fn the_squads_name_in_the_header_lists_the_others() {
        let (mut app, _, _) = team();
        app.set_hits(vec![(10, 20, 0, Hit::Squads)], None);
        app.on_click(12, 0);
        assert_eq!(app.composer.text(), "/squad ");
    }

    #[test]
    fn the_mode_that_only_reads_is_read_only_not_plan() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/mode read-only");
        assert_eq!(enter(&mut app), vec![Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::ReadOnly) }]);
        type_text(&mut app, "/mode plan");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("/mode read-only"), "{:?}", app.flash);
        assert_eq!(Mode::ReadOnly.label(), "read-only");
        // A squad kept from before the rename still reads as read-only.
        assert_eq!(Mode::stored("plan"), Some(Mode::ReadOnly));
        assert_eq!(Mode::parse("plan"), None);
    }

    #[test]
    fn a_project_in_focus_narrows_the_board_and_tells_the_squad() {
        let (mut app, _, _) = team();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut site = ticket(1, TicketStatus::Review);
        site.project = Some("/code/site".into());
        let mut app_ticket = ticket(2, TicketStatus::Review);
        app_ticket.project = Some("/code/app".into());
        for t in [site, app_ticket] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        assert_eq!(app.board_order(), vec![1, 2]);
        type_text(&mut app, "/project app");
        assert_eq!(enter(&mut app), vec![Command::Focus { squad: "acme".into(), project: Some("app".into()) }]);
        assert_eq!(app.board_order(), vec![2]);
        assert_eq!(app.to_review_here().len(), 1);
        assert_eq!(app.hint().as_deref(), Some("/accept T-2"));
        // A chip click does the same, and "all" shows everything again.
        app.set_hits(vec![(0, 3, 0, Hit::Project(None))], None);
        app.on_click(1, 0);
        assert_eq!(app.outbox, vec![Command::Focus { squad: "acme".into(), project: None }]);
        assert_eq!(app.board_order(), vec![1, 2]);
    }

    /// A squad on two projects, site and app: frontend works on an app ticket;
    /// a site ticket waits for review.
    fn two_projects() -> App {
        let (mut app, _, frontend) = team();
        app.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut review = ticket(1, TicketStatus::Review);
        review.project = Some("/code/site".into());
        let mut working = ticket(2, TicketStatus::InProgress);
        working.project = Some("/code/app".into());
        for t in [review, working] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        let agent = app.agents.iter_mut().find(|a| a.id == frontend).unwrap();
        agent.ticket = Some(2);
        agent.state = AgentState::Working;
        app
    }

    #[test]
    fn each_project_says_who_works_on_it_and_what_waits_for_the_person() {
        let app = two_projects();
        assert_eq!(app.project_status(std::path::Path::new("/code/site")), ProjectStatus { working: 0, for_you: 1 });
        assert_eq!(app.project_status(std::path::Path::new("/code/app")), ProjectStatus { working: 1, for_you: 0 });
        let frontend = app.agents.iter().find(|a| a.label() == "frontend").unwrap();
        assert_eq!(app.agent_project(frontend).as_deref(), Some(std::path::Path::new("/code/app")));
    }

    #[test]
    fn ctrl_p_opens_a_project_picker_with_the_next_one_picked() {
        let mut app = two_projects();
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let enter = KeyEvent::from(KeyCode::Enter);
        // The picker: every project and all, how each is doing, the next one picked.
        assert!(app.on_key(ctrl_p, Instant::now()).is_empty());
        assert_eq!(app.composer.text(), "/project ");
        let choices = app.choices();
        let texts: Vec<&str> = choices.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["/project site", "/project app", "/project all"]);
        assert!(choices[0].about.contains("⚑1"), "{:?}", choices[0].about);
        assert!(choices[1].about.contains("●1"), "{:?}", choices[1].about);
        // So Ctrl-P, Enter walks them as before.
        assert_eq!(app.on_key(enter, Instant::now()), vec![Command::Focus { squad: "acme".into(), project: Some("site".into()) }]);
        app.on_key(ctrl_p, Instant::now());
        assert_eq!(app.on_key(enter, Instant::now()), vec![Command::Focus { squad: "acme".into(), project: Some("app".into()) }]);
        app.on_key(ctrl_p, Instant::now());
        assert_eq!(app.on_key(enter, Instant::now()), vec![Command::Focus { squad: "acme".into(), project: None }]);
        // Typing narrows it — any part of the name.
        app.on_key(ctrl_p, Instant::now());
        type_text(&mut app, "pp");
        let texts: Vec<String> = app.choices().into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["/project app"]);
        app.composer.clear();
        // Alt and a number goes straight to one, where the terminal sends it.
        let alt_2 = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT);
        assert_eq!(app.on_key(alt_2, Instant::now()), vec![Command::Focus { squad: "acme".into(), project: Some("app".into()) }]);
        // The header's "+N" opens the same picker.
        app.set_hits(vec![(0, 3, 0, Hit::Projects)], None);
        app.on_click(1, 0);
        assert_eq!(app.composer.text(), "/project ");
    }

    #[test]
    fn model_sets_a_members_model_from_its_next_turn_on_any_engine() {
        let (mut app, pm, frontend) = team();
        type_text(&mut app, "/model frontend opus");
        assert_eq!(enter(&mut app), vec![Command::SetModel { session: frontend, model: "opus".into() }]);
        // Without a name: the member selected.
        type_text(&mut app, "/model gpt-5-codex");
        assert_eq!(enter(&mut app), vec![Command::SetModel { session: pm, model: "gpt-5-codex".into() }]);
        // Completion offers members, then the models of that member's engine.
        app.composer.clear();
        app.composer.insert_str("/model frontend ");
        let models: Vec<String> = app.choices().into_iter().map(|c| c.label).collect();
        assert!(models.contains(&"opus".to_owned()) && models.contains(&"sonnet".to_owned()), "{models:?}");
        app.composer.clear();
        type_text(&mut app, "/model nobody opus");
        assert!(enter(&mut app).is_empty());
    }

    #[test]
    fn the_pm_shows_the_thread_of_the_project_in_focus() {
        let mut app = two_projects();
        let pm = app.agents.iter().find(|a| a.is_pm()).unwrap().id;
        for event in [
            Event::UserMessage { session: pm, text: "about the squad".into() },
            Event::Thread { session: pm, project: Some("app".into()) },
            Event::UserMessage { session: pm, text: "about the app".into() },
            Event::Thread { session: pm, project: Some("site".into()) },
            Event::UserMessage { session: pm, text: "about the site".into() },
        ] {
            app.on_event(event);
        }
        let said = |app: &App| -> Vec<String> {
            app.agents.iter().find(|a| a.id == pm).unwrap().shown().iter().filter_map(|e| match e.as_ref() {
                Entry::User { text } => Some(text.clone()),
                _ => None,
            }).collect()
        };
        assert_eq!(said(&app), ["about the squad"]);
        app.focus_on("app");
        assert_eq!(said(&app), ["about the app"]);
        app.focus_on("site");
        assert_eq!(said(&app), ["about the site"]);
    }

    #[test]
    fn a_members_conversation_shows_the_project_in_focus_and_says_what_it_hides() {
        let mut app = two_projects();
        let frontend = app.agents.iter().find(|a| !a.is_pm()).unwrap().id;
        for event in [
            Event::Thread { session: frontend, project: Some("site".into()) },
            Event::Notice { session: frontend, text: "── T-4 · Hero — on branch b".into() },
            Event::UserMessage { session: frontend, text: "site work".into() },
            Event::Thread { session: frontend, project: Some("app".into()) },
            Event::Notice { session: frontend, text: "── T-2 · Onboarding — on branch c".into() },
            Event::UserMessage { session: frontend, text: "app work".into() },
        ] {
            app.on_event(event);
        }
        let texts = |app: &App| -> Vec<String> {
            app.agents.iter().find(|a| a.id == frontend).unwrap().shown().iter().filter_map(|e| match e.as_ref() {
                Entry::User { text } | Entry::Note { text } => Some(text.clone()),
                _ => None,
            }).collect()
        };
        // All: everything, each ticket with its project.
        let all = texts(&app);
        assert!(all.contains(&"── site · T-4 · Hero — on branch b".to_owned()) && all.contains(&"app work".to_owned()), "{all:?}");
        // One project: its part, and a line for what's hidden.
        app.focus_on("app");
        let app_only = texts(&app);
        assert!(!app_only.iter().any(|t| t.contains("site work")), "{app_only:?}");
        assert!(app_only.iter().any(|t| t.contains("1 ticket in other projects hidden")), "{app_only:?}");
        assert!(app_only.contains(&"app work".to_owned()));
    }

    #[test]
    fn g_on_the_board_shows_open_work_as_a_tree_of_what_waits_for_what() {
        let (mut app, _, _) = team();
        let mut add = |id: TicketId, status: TicketStatus, after: Vec<TicketId>| {
            let mut t = ticket(id, status);
            t.depends_on = after;
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        };
        add(1, TicketStatus::Done, vec![]);
        add(2, TicketStatus::InProgress, vec![1]);
        add(3, TicketStatus::Todo, vec![2]);
        add(4, TicketStatus::Todo, vec![3]);
        add(5, TicketStatus::Todo, vec![]);
        add(6, TicketStatus::Todo, vec![2]);
        app.pane = Pane::Board;
        app.on_key(KeyEvent::from(KeyCode::Char('g')), Instant::now());
        assert!(app.board_graph);
        assert_eq!(app.dependency_tree(), vec![(2, 0), (3, 1), (4, 2), (6, 1), (5, 0)]);
        assert_eq!(app.board_order(), vec![2, 3, 4, 6, 5]);
        app.on_key(KeyEvent::from(KeyCode::Char('g')), Instant::now());
        assert!(!app.board_graph);
    }

    #[test]
    fn the_line_between_the_board_list_and_the_ticket_drags_up_and_down() {
        let (mut app, _, _) = team();
        app.pane = Pane::Board;
        app.set_board_rule(Some(BoardRule { rows: (10, 11), list_top: 3 }));
        app.on_click(5, 10);
        assert!(app.is_dragging());
        app.on_drag(5, 15);
        assert_eq!(app.board_split, Some(12));
        app.on_mouse(MouseEventKind::Up(crossterm::event::MouseButton::Left), Instant::now());
        assert!(!app.is_dragging());
        assert_eq!(app.board_split, Some(12));
    }

    #[test]
    fn find_narrows_the_board_as_it_is_typed_and_reaches_done_work() {
        let (mut app, _, _) = team();
        for (id, title, status) in [(1, "Hero section", TicketStatus::Done), (2, "Pricing table", TicketStatus::Todo), (3, "Hero copy", TicketStatus::Todo)] {
            let mut t = ticket(id, status);
            t.title = title.into();
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        app.pane = Pane::Board;
        // f on the board starts a search; it narrows as it is typed.
        app.on_key(KeyEvent::from(KeyCode::Char('f')), Instant::now());
        assert_eq!(app.composer.text(), "/find ");
        type_text(&mut app, "hero");
        assert_eq!(app.board_order(), vec![3, 1]);
        // Enter keeps it; Esc on the board lets it go.
        assert!(enter(&mut app).is_empty());
        assert_eq!(app.pane, Pane::Board);
        assert_eq!(app.board_order(), vec![3, 1]);
        app.on_key(KeyEvent::from(KeyCode::Esc), Instant::now());
        assert_eq!(app.pane, Pane::Board);
        assert_eq!(app.board_order(), vec![2, 3, 1]);
    }

    #[test]
    fn a_long_section_folds_and_e_shows_it_whole() {
        let (mut app, _, _) = team();
        for id in 1..=30 {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Todo) });
        }
        app.pane = Pane::Board;
        assert_eq!(app.board_order().len(), SECTION_ROWS);
        app.on_key(KeyEvent::from(KeyCode::Char('e')), Instant::now());
        assert_eq!(app.board_order().len(), 30);
    }

    #[test]
    fn tidy_asks_the_pm_to_look_the_board_over() {
        let (mut app, pm, _) = team();
        type_text(&mut app, "/tidy");
        let sent = enter(&mut app);
        assert!(matches!(&sent[..], [Command::Submit { session, text }] if *session == pm && text.contains("stale") && text.contains("Don't cancel")), "{sent:?}");
    }

    #[test]
    fn clicking_a_completion_takes_it() {
        let mut app = two_projects();
        app.composer.insert_str("/project ");
        app.set_hits(vec![(0, 40, 7, Hit::Choice(1))], None);
        app.on_click(5, 7);
        assert_eq!(app.outbox, vec![Command::Focus { squad: "acme".into(), project: Some("app".into()) }]);
    }

    #[test]
    fn clicking_a_member_in_the_squad_opens_its_conversation() {
        let (mut app, _, frontend) = team();
        app.pane = Pane::Board;
        let at = app.agents.iter().position(|a| a.id == frontend).unwrap();
        app.set_hits(vec![(0, 20, 5, Hit::Member(at))], None);
        app.on_click(3, 5);
        assert_eq!(app.selected, at);
        assert_eq!(app.pane, Pane::Conversation);
    }

    #[test]
    fn a_squad_growing_past_six_projects_is_told_it_could_split() {
        assert!(many_projects_tip(6).is_none());
        assert!(many_projects_tip(7).is_some_and(|t| t.contains("/squad new")));
    }

    #[test]
    fn the_board_of_several_projects_has_a_lane_for_each_below_what_waits_for_the_person() {
        let mut app = two_projects();
        let mut more = Vec::new();
        for (id, status, project) in [(3, TicketStatus::Todo, "/code/site"), (4, TicketStatus::Waiting, "/code/app"), (5, TicketStatus::Done, "/code/site"), (6, TicketStatus::Todo, "/code/app")] {
            let mut t = ticket(id, status);
            t.project = Some(project.into());
            more.push(t);
        }
        for t in more {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: t });
        }
        // What waits for the person, from every project; then site's lane, app's
        // lane; then what's done.
        assert_eq!(app.board_order(), vec![1, 4, 3, 2, 6, 5]);
        app.focus = Some("/code/app".into());
        assert_eq!(app.board_order(), vec![4, 2, 6]);
    }

    #[test]
    fn adding_an_app_suggests_a_mobile_engineer() {
        let dir = std::env::temp_dir().join(format!("equipe-app-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Acme.xcodeproj")).unwrap();
        assert!(looks_like_an_app(&dir));
        std::fs::remove_dir_all(dir.join("Acme.xcodeproj")).unwrap();
        assert!(!looks_like_an_app(&dir));
        std::fs::write(dir.join("pubspec.yaml"), "name: acme").unwrap();
        assert!(looks_like_an_app(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(role_names().contains("mobile"));
    }

    #[test]
    fn on_the_board_the_wheel_walks_tickets_over_the_list_and_reads_the_ticket_below() {
        let (mut app, _, _) = team();
        for id in [1, 2] {
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(id, TicketStatus::Todo) });
        }
        app.pane = Pane::Board;
        app.set_extent(5, 0, 40);
        app.set_board_detail(Some((10, 20)));
        let t0 = Instant::now();
        // Over the open ticket: it scrolls.
        app.on_mouse_at(MouseEventKind::ScrollDown, 12, t0);
        assert_eq!(app.board_scroll, 3);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(1));
        // Over the list: the next ticket, read from its top.
        app.on_mouse_at(MouseEventKind::ScrollDown, 3, t0 + Duration::from_secs(1));
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(2));
        assert_eq!(app.board_scroll, 0);
        // PgDn reads on too.
        app.on_key(KeyEvent::from(KeyCode::PageDown), t0);
        assert_eq!(app.board_scroll, 5);
    }

    #[test]
    fn compact_and_clear_start_a_members_next_conversation_afresh() {
        let (mut app, pm, frontend) = team();
        type_text(&mut app, "/compact");
        assert_eq!(enter(&mut app), vec![Command::Refresh { session: pm, summary: true }]);
        type_text(&mut app, "/clear frontend");
        assert_eq!(enter(&mut app), vec![Command::Refresh { session: frontend, summary: false }]);
        type_text(&mut app, "/clear nobody");
        assert!(enter(&mut app).is_empty());
    }

    #[test]
    fn a_dropped_image_file_is_attached_and_ctrl_v_asks_for_the_clipboards() {
        let dir = std::env::temp_dir().join(format!("equipe drop {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shot = dir.join("Screen Shot 1.png");
        std::fs::write(&shot, b"png").unwrap();
        let (mut app, _, _) = team();
        // As iTerm2 pastes a dropped file: spaces escaped.
        app.on_paste(&shot.display().to_string().replace(' ', "\\ "));
        assert_eq!(app.composer.text(), format!("[image: {}] ", shot.display()));
        // Text that only looks like a path stays text.
        app.composer.clear();
        app.on_paste("/no/such/file.png");
        assert_eq!(app.composer.text(), "/no/such/file.png");
        app.composer.clear();
        app.on_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL), Instant::now());
        assert!(app.paste_image);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_image_typed_down_the_terminal_is_attached() {
        let (mut app, _, _) = team();
        // ⌃⇧V on the person's Mac types the screenshot as a data: URL.
        app.on_paste("data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==");
        let text = app.composer.text();
        let path = text.strip_prefix("[image: ").and_then(|t| t.strip_suffix("] ")).expect("attached");
        assert!(path.ends_with(".png"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_upload_from_iterm2_arrives_as_keystrokes_and_is_taken_whole() {
        let (mut app, _, _) = team();
        app.upload = Some(Upload::default());
        let mut send = |text: &str| {
            for c in text.chars() {
                let key = if c == '\n' { KeyEvent::from(KeyCode::Enter) } else { KeyEvent::from(KeyCode::Char(c)) };
                app.on_key(key, Instant::now());
            }
        };
        send("ok\nQUJD\nREVG\n\n");
        assert_eq!(app.uploaded.as_deref(), Some("QUJDREVG"), "the lines, joined");
        assert!(app.upload.is_none() && app.composer.is_empty(), "nothing typed into the message");
        // Cancelled in iTerm2's window: nothing comes, and the console says so.
        app.upload = Some(Upload::default());
        for c in "abort".chars() {
            app.on_key(KeyEvent::from(KeyCode::Char(c)), Instant::now());
        }
        app.on_key(KeyEvent::from(KeyCode::Enter), Instant::now());
        assert!(app.upload.is_none());
        assert!(app.flash.as_deref().unwrap().contains("No image"));
    }

    #[test]
    fn browser_opens_the_sign_in_window() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/browser");
        assert!(enter(&mut app).is_empty());
        assert!(app.open_browser);
        type_text(&mut app, "/browser show frontend");
        assert_eq!(enter(&mut app), vec![Command::ShowBrowser { squad: "acme".into(), member: Some("frontend".into()), show: true }]);
        type_text(&mut app, "/browser hide");
        assert_eq!(enter(&mut app), vec![Command::ShowBrowser { squad: "acme".into(), member: None, show: false }]);
        assert!(app.flash.as_deref().unwrap().contains("hidden"));
    }

    #[test]
    fn the_person_accepts_reopens_and_cancels_by_ticket() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/accept T-4");
        assert_eq!(enter(&mut app), vec![Command::Accept { squad: "acme".into(), ticket: 4 }]);
        type_text(&mut app, "/reopen 4 make the button blue");
        assert_eq!(enter(&mut app), vec![Command::Reopen { squad: "acme".into(), ticket: 4, note: "make the button blue".into() }]);
        // Without a word on what to change it stays in the composer, asking.
        type_text(&mut app, "/reopen T-4");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("What should change on T-4"), "{:?}", app.flash);
        assert_eq!(app.composer.text(), "/reopen T-4 ");
        app.composer.clear();
        type_text(&mut app, "/cancel t4");
        assert_eq!(enter(&mut app), vec![Command::Cancel { squad: "acme".into(), ticket: 4 }]);
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(5, TicketStatus::Review) });
        type_text(&mut app, "/accept");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("T-5"), "{:?}", app.flash);
    }

    #[test]
    fn accepting_a_task_asks_nothing_its_work_goes_into_the_squads_folder() {
        let (mut app, _, _) = team();
        let mut task = ticket(5, TicketStatus::Review);
        task.kind = equipe_core::protocol::TicketKind::Task;
        task.branch = Some("equipe/frontend-t5".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: task });
        type_text(&mut app, "/accept T-5");
        assert_eq!(enter(&mut app), vec![Command::Accept { squad: "acme".into(), ticket: 5 }]);
        assert_eq!(app.question, None);
        assert!(app.flash.as_deref().unwrap().contains("squad's folder"), "{:?}", app.flash);
    }

    #[test]
    fn accepting_asks_before_merging_and_only_a_yes_merges() {
        let (mut app, _, _) = team();
        for id in [6, 7, 8, 9] {
            let mut review = ticket(id, TicketStatus::Review);
            review.branch = Some(format!("equipe/frontend-t{id}"));
            app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: review });
        }
        let accept = |id: TicketId| vec![Command::Accept { squad: "acme".into(), ticket: id }];
        let merge = |id: TicketId, push: bool| vec![Command::Merge { squad: "acme".into(), ticket: id, push }];

        // n: accepted, nothing merged.
        type_text(&mut app, "/accept T-6");
        assert_eq!(enter(&mut app), accept(6));
        assert!(app.flash.as_deref().unwrap().contains("merge it into main now? y/n"), "{:?}", app.flash);
        assert!(app.on_key(key(KeyCode::Char('n')), Instant::now()).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("/merge T-6"), "{:?}", app.flash);
        assert_eq!(app.question, None);

        // y, then whether to push.
        type_text(&mut app, "/accept T-7");
        enter(&mut app);
        assert!(app.on_key(key(KeyCode::Char('y')), Instant::now()).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("push main to origin too? y/n"), "{:?}", app.flash);
        assert_eq!(app.on_key(key(KeyCode::Char('n')), Instant::now()), merge(7, false));
        type_text(&mut app, "/accept T-8");
        enter(&mut app);
        app.on_key(key(KeyCode::Char('y')), Instant::now());
        assert_eq!(app.on_key(key(KeyCode::Char('y')), Instant::now()), merge(8, true));

        // Esc at the push question merges nothing; typing something else puts it away.
        type_text(&mut app, "/accept T-9");
        enter(&mut app);
        app.on_key(key(KeyCode::Char('y')), Instant::now());
        assert!(app.on_key(key(KeyCode::Esc), Instant::now()).is_empty());
        assert_eq!(app.question, None);
        type_text(&mut app, "/merge T-9");
        assert!(enter(&mut app).is_empty(), "/merge asks about pushing first");
        type_text(&mut app, "hi");
        assert_eq!((app.question, app.composer.text()), (None, "hi"));
    }

    #[test]
    fn work_not_on_main_merges_from_the_board() {
        let (mut app, _, _) = team();
        let mut done = ticket(3, TicketStatus::Done);
        done.branch = Some("equipe/frontend-t3".into());
        done.merged = Some(false);
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: done.clone() });
        app.pane = Pane::Board;
        assert!(app.on_key(key(KeyCode::Char('m')), Instant::now()).is_empty());
        assert_eq!(app.on_key(key(KeyCode::Char('n')), Instant::now()), vec![Command::Merge { squad: "acme".into(), ticket: 3, push: false }]);
        // Once on main there is nothing to merge.
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: Ticket { merged: Some(true), ..done } });
        app.on_key(key(KeyCode::Char('m')), Instant::now());
        assert_eq!(app.question, None);
    }

    #[test]
    fn a_ticket_waiting_on_the_person_says_what_it_needs_and_resumes() {
        let (mut app, _, _) = team();
        let mut waiting = ticket(6, TicketStatus::Waiting);
        waiting.note = Some("Waiting on you:\n- [ ] GitHub token\n- [ ] PostHog secrets".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: waiting });
        let flash = app.flash.clone().unwrap();
        assert!(flash.contains("T-6 waits for you: GitHub token, PostHog secrets") && flash.contains("/resume T-6"), "{flash}");
        type_text(&mut app, "/resume");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("Waiting on you: T-6"), "{:?}", app.flash);
        // A word on what was done is optional.
        type_text(&mut app, "/resume T-6");
        assert_eq!(enter(&mut app), vec![Command::Resume { squad: "acme".into(), ticket: 6, note: String::new() }]);
        type_text(&mut app, "/resume 6 both in .env.local");
        assert_eq!(enter(&mut app), vec![Command::Resume { squad: "acme".into(), ticket: 6, note: "both in .env.local".into() }]);
        // On the board, r on it starts the same.
        app.pane = Pane::Board;
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(6), "it sits at the top, with what waits for the person");
        app.on_key(KeyEvent::from(KeyCode::Char('r')), Instant::now());
        assert_eq!(app.composer.text(), "/resume T-6 ");
    }

    #[test]
    fn accepted_work_is_remembered_for_the_summary() {
        let (mut app, _, _) = team();
        let mut review = ticket(6, TicketStatus::Review);
        review.branch = Some("equipe/frontend-t6".into());
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: review.clone() });
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: Ticket { status: TicketStatus::Done, ..review } });
        assert_eq!(app.accepted, [(6, "ticket 6".to_owned(), "equipe/frontend-t6".to_owned())]);
    }

    #[test]
    fn a_member_on_a_ticket_works_in_its_worktree_then_returns_to_its_desk() {
        let (mut app, _, frontend) = team();
        app.on_event(Event::WorkStarted {
            session: frontend,
            ticket: 3,
            title: "Hero".into(),
            project: "/repo".into(),
            workspace: Some(Workspace { branch: "equipe/frontend-t3".into(), base: "abc".into(), path: "/w/3".into() }),
        });
        let agent = &app.agents[1];
        assert_eq!((agent.ticket, agent.state, agent.base.clone()), (Some(3), AgentState::Working, PathBuf::from("/w/3")));
        app.on_event(Event::WorkEnded { session: frontend, ticket: 3 });
        assert_eq!((app.agents[1].ticket, app.agents[1].base.clone()), (None, PathBuf::from("/repo")));
    }

    fn approval(id: u64) -> Approval {
        Approval {
            id,
            session: 2,
            member: "frontend".into(),
            ticket: Some(3),
            tool: "Bash".into(),
            action: "npm install".into(),
            reason: None,
            preview: None,
        }
    }

    #[test]
    fn a_request_for_permission_answers_to_y_a_and_n_unless_the_person_is_typing() {
        let (mut app, _, _) = team();
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(7) });
        assert_eq!(
            app.on_key(key(KeyCode::Char('a')), Instant::now()),
            vec![Command::Decide { squad: "acme".into(), approval: 7, decision: Decision::Always }]
        );
        assert_eq!(
            app.on_key(key(KeyCode::Char('n')), Instant::now()),
            vec![Command::Decide { squad: "acme".into(), approval: 7, decision: Decision::Deny }]
        );
        // Mid-sentence, letters are letters.
        type_text(&mut app, "hey");
        assert_eq!(app.composer.text(), "hey");
        app.on_event(Event::ApprovalResolved { squad: "acme".into(), approval: 7, decision: Decision::Deny });
        assert!(app.approvals.is_empty());
    }

    #[test]
    fn modes_are_set_for_the_squad_or_one_member() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/mode ask");
        assert_eq!(enter(&mut app), vec![Command::SetMode { squad: "acme".into(), member: None, mode: Some(Mode::Ask) }]);
        type_text(&mut app, "/mode frontend full");
        assert_eq!(
            enter(&mut app),
            vec![Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::Full) }]
        );
        type_text(&mut app, "/mode frontend squad");
        assert_eq!(enter(&mut app), vec![Command::SetMode { squad: "acme".into(), member: Some("frontend".into()), mode: None }]);
        app.on_event(Event::ModeChanged { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::ReadOnly) });
        app.on_event(Event::ModeChanged { squad: "acme".into(), member: None, mode: Some(Mode::Ask) });
        assert_eq!(app.mode_of(&app.agents[1]), Mode::ReadOnly);
        assert_eq!(app.mode_of(&app.agents[0]), Mode::Ask);
    }

    #[test]
    fn a_turn_leaves_how_long_it_took_and_elapsed_reads_like_a_clock() {
        assert_eq!(elapsed(Duration::from_secs(42)), "42s");
        assert_eq!(elapsed(Duration::from_secs(65)), "1m 05s");
        assert_eq!(elapsed(Duration::from_secs(3720)), "1h 02m");
        let (mut app, id) = ready();
        app.on_event(Event::UserMessage { session: id, text: "go".into() });
        assert!(app.agents[0].working_since.is_some());
        app.agents[0].working_since = Some(Instant::now() - Duration::from_secs(125));
        app.on_event(Event::TurnEnded { session: id, outcome: TurnOutcome::Completed, usage: None });
        assert!(app.agents[0].working_since.is_none());
        assert!(matches!(app.agents[0].transcript.entries().last(), Some(Entry::Note { text }) if text.starts_with("─ worked 2m 05s")));
    }

    #[test]
    fn whoever_needs_the_person_is_listed_first_and_tab_follows_the_list() {
        let (mut app, pm, frontend) = team();
        app.on_event(Event::UserMessage { session: frontend, text: "go".into() });
        assert_eq!(app.display_order(), [1, 0], "working before ready");
        app.on_event(Event::ApprovalRequested { squad: "acme".into(), approval: approval(1) });
        assert_eq!(app.display_order(), [1, 0]);
        assert!(app.needs_you(&app.agents[1]));
        assert_eq!(app.selected().unwrap().id, pm);
        app.on_key(key(KeyCode::Tab), Instant::now());
        assert_eq!(app.selected().unwrap().id, frontend, "Tab wraps round the listed order");
    }

    #[test]
    fn scrolled_up_new_lines_are_counted_and_end_jumps_back() {
        let (mut app, pm, _) = team();
        app.on_mouse(MouseEventKind::ScrollUp, Instant::now());
        app.grew(pm, 4);
        assert_eq!(app.agents[0].unseen, 4);
        app.on_key(key(KeyCode::End), Instant::now());
        assert_eq!((app.agents[0].scroll_back, app.agents[0].unseen), (0, 0));
    }

    #[test]
    fn a_wheel_notch_moves_three_lines_and_a_trackpad_gesture_one_per_event() {
        let (mut app, _, _) = team();
        let t0 = Instant::now();
        app.on_mouse(MouseEventKind::ScrollUp, t0);
        assert_eq!(app.agents[0].scroll_back, 3, "a lone notch");
        for ms in [10, 20, 30, 40] {
            app.on_mouse(MouseEventKind::ScrollUp, t0 + Duration::from_millis(ms));
        }
        assert_eq!(app.agents[0].scroll_back, 7, "a gesture follows the fingers");
        app.on_mouse(MouseEventKind::ScrollDown, t0 + Duration::from_millis(45));
        assert_eq!(app.agents[0].scroll_back, 4, "turning round is a new gesture");
    }

    #[test]
    fn scrolling_stops_at_the_ends_and_pages_are_half_a_screen() {
        let (mut app, _, _) = team();
        app.set_extent(12, 30, 0);
        app.on_key(key(KeyCode::PageUp), Instant::now());
        assert_eq!(app.agents[0].scroll_back, 12);
        for _ in 0..5 {
            app.on_key(key(KeyCode::PageUp), Instant::now());
        }
        assert_eq!(app.agents[0].scroll_back, 30, "the top is the top");
        app.on_key(key(KeyCode::PageDown), Instant::now());
        assert_eq!(app.agents[0].scroll_back, 18, "and coming back moves at once");
        app.on_key(key(KeyCode::End), Instant::now());
        app.on_key(key(KeyCode::Home), Instant::now());
        assert_eq!(app.agents[0].scroll_back, 30, "Home goes to the beginning");
    }

    #[test]
    fn ctrl_o_folds_and_unfolds_tool_runs_and_o_stays_a_letter() {
        let (mut app, _, _) = team();
        app.on_key(ctrl('o'), Instant::now());
        assert!(app.agents[0].expanded);
        app.on_key(ctrl('o'), Instant::now());
        assert!(!app.agents[0].expanded);
        type_text(&mut app, "ok");
        assert_eq!(app.composer.text(), "ok", "a message may start with o");
    }

    #[test]
    fn export_asks_for_the_conversation_as_markdown() {
        let (mut app, pm, _) = team();
        app.on_event(Event::UserMessage { session: pm, text: "plan it".into() });
        app.on_event(Event::TextDelta { session: pm, text: "**Done.**".into() });
        app.on_event(Event::ToolUse { session: pm, tool: "Read".into(), target: Some("a.rs".into()) });
        type_text(&mut app, "/export");
        enter(&mut app);
        assert!(app.export);
        let text = transcript_markdown(&app.agents[0], "acme");
        assert!(text.starts_with("# pm · acme"), "{text}");
        assert!(text.contains("**You:** plan it") && text.contains("**Done.**") && text.contains("- read a.rs"), "{text}");
    }

    #[test]
    fn the_wheel_scrolls_the_conversation_and_mouse_can_be_released() {
        let (mut app, _, _) = team();
        let t0 = Instant::now();
        app.on_mouse(MouseEventKind::ScrollUp, t0);
        assert_eq!(app.agents[0].scroll_back, 3);
        app.grew(1, 5);
        assert_eq!(app.agents[0].scroll_back, 8, "scrolled up, it keeps its place as lines arrive");
        app.on_mouse(MouseEventKind::ScrollDown, t0 + Duration::from_secs(1));
        app.on_mouse(MouseEventKind::ScrollDown, t0 + Duration::from_secs(2));
        app.on_mouse(MouseEventKind::ScrollDown, t0 + Duration::from_secs(3));
        assert_eq!(app.agents[0].scroll_back, 0);
        type_text(&mut app, "/mouse");
        enter(&mut app);
        assert!(!app.mouse);
    }
}
