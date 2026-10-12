//! What the terminal UI knows and how it reacts — no terminal, no engines.
//!
//! Keys and core [`Event`]s go in; the [`Command`]s to send to the core come
//! out. Everything here is plain state, so it is tested without a terminal.
//!
//! The UI is a view of one squad as the daemon has it: its members, their
//! conversations, and the board. It starts from a snapshot, follows events, and
//! leaving it changes nothing — the squad keeps working.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use equipe_core::board;
use equipe_core::protocol::{
    Access, Approval, Command, Decision, Engine, Entry, Event, Member, Mode, SessionId, SessionInfo, SessionSpec,
    SessionState, Snapshot, Ticket, TicketId, TicketStatus, Transcript, new_session_id,
};
use equipe_core::roles;

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

/// What the right-hand pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Conversation,
    Board,
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
            scroll_back: 0,
            unseen: 0,
            working_since: None,
            expanded: false,
            desk: base.clone(),
            base,
            pending: None,
            mode: None,
        }
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
            scroll_back: 0,
            unseen: 0,
            // Attached mid-turn: the clock starts now, not when the turn did.
            working_since: (info.state == SessionState::Working).then(Instant::now),
            expanded: false,
            base,
            desk: info.project,
            pending: None,
            mode: info.mode,
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
    pub composer: Composer,
    /// A one-line message in the status bar, cleared by the next key.
    pub flash: Option<String>,
    pub should_quit: bool,
    /// Whether the mouse wheel scrolls (the terminal then can't select text).
    pub mouse: bool,
    /// Work accepted while this screen watched, for the summary on leaving:
    /// `(ticket, title, branch)`.
    pub accepted: Vec<(TicketId, String, String)>,
    /// How much members may do on their own, unless one has its own mode.
    pub mode: Mode,
    /// Requests for permission waiting for the person, oldest first.
    pub approvals: Vec<Approval>,
    /// Lines a page key moves: half the conversation's height on screen.
    pub page: usize,
    /// How far the selected conversation and the board can scroll, as last drawn.
    conversation_max: usize,
    board_max: usize,
    /// The last wheel event: when, and which way.
    last_wheel: Option<(Instant, bool)>,
    /// The selected member's conversation, asked for as a file (`/export`).
    pub export: bool,
    quit_armed: Option<Instant>,
}

const HELP: &str = "The PM is your main contact: tell it what you want; it puts the work on the board and \
equipe hands each ticket to its owner. Talk to anyone directly — Tab or @name — and the PM hears about it.\n\
Enter sends; while a member works, a message waits as a memo until its turn ends · Esc stops it (a waiting \
memo goes in at once) · Alt-Enter or Ctrl-J adds a line · PgUp/PgDn or the wheel scroll · Ctrl-B shows the board · \
End jumps to the latest line · Ctrl-C twice leaves — the squad keeps working.\n\
/add <role…> [codex|claude] brings in teammates: pm, designer, frontend, backend, qa, data, or custom <name> · \
/accept T-3 approves handed-in work · /reopen T-3 <what to change> sends it back · /cancel T-3 drops it · \
/board · /mode [member] plan|ask|auto|full sets how much members do on their own — y/a/n answers a request \
for permission (allow, always allow, decline) · Ctrl-O unfolds long runs of tool use · /export opens the conversation in your editor · /mouse lets the terminal select text (or hold Shift — Option in iTerm2 — while dragging) · /close removes the selected \
member · /quit leaves.";

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
            composer: Composer::default(),
            flash: None,
            should_quit: false,
            mouse: true,
            accepted: Vec::new(),
            mode: snapshot.mode,
            approvals: snapshot.approvals,
            page: 10,
            conversation_max: usize::MAX,
            board_max: usize::MAX,
            last_wheel: None,
            export: false,
            quit_armed: None,
        };
        // Talk to the PM first: it is the main contact.
        app.selected = app.agents.iter().position(Agent::is_pm).unwrap_or(0);
        let mut joining: Vec<String> = joining.to_vec();
        if app.agents.is_empty() && !joining.iter().any(|r| r == roles::PM) {
            joining.insert(0, roles::PM.to_owned());
        }
        let commands = joining.iter().flat_map(|role| app.add_member(role, None)).collect();
        (app, commands)
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

    /// Bring a member onto the squad. The selection stays where it is.
    fn add_member(&mut self, role: &str, engine: Option<Engine>) -> Vec<Command> {
        let id = new_session_id();
        let edits = roles::role_or_custom(role).edits;
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
            Event::TicketChanged { ticket, .. } => {
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
                    agent.tokens += u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0);
                }
            }
            Event::SessionClosed { .. } => agent.state = AgentState::Closed,
            Event::Error { .. } if agent.state == AgentState::Starting => agent.state = AgentState::Failed,
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
        self.composer.insert_str(text);
    }

    pub fn on_mouse(&mut self, kind: MouseEventKind, now: Instant) {
        let up = match kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            _ => return,
        };
        // A trackpad's stream of events moves a line each; a lone notch, three.
        let in_gesture = self.last_wheel.is_some_and(|(at, was_up)| was_up == up && now.duration_since(at) < WHEEL_GESTURE);
        self.last_wheel = Some((now, up));
        let lines = if in_gesture { 1 } else { WHEEL_NOTCH };
        self.scroll(if up { lines } else { -lines });
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if !(ctrl && key.code == KeyCode::Char('c')) {
            self.quit_armed = None;
            self.flash = None;
        }
        // A request for permission answers to one key — while nothing is being typed.
        if let (Some(approval), true, false, false) = (self.approvals.first(), self.composer.is_empty(), ctrl, alt) {
            let decision = match key.code {
                KeyCode::Char('y') => Some(Decision::Allow),
                KeyCode::Char('a') => Some(Decision::Always),
                KeyCode::Char('n') => Some(Decision::Deny),
                _ => None,
            };
            if let Some(decision) = decision {
                return vec![Command::Decide { squad: self.squad.clone(), approval: approval.id, decision }];
            }
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
            KeyCode::Enter if alt || shift => self.composer.newline(),
            KeyCode::Char('j') if ctrl => self.composer.newline(),
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
            "accept" | "reopen" | "cancel" => self.decide(name, rest),
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
                if let Some(agent) = self.agents.get_mut(self.selected) {
                    agent.transcript.push_local(Entry::Note { text: HELP.into() });
                }
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

    /// `/accept T-3`, `/reopen T-3 what to change`, `/cancel T-3`: the person's
    /// calls, sent straight to equipe — never through a member.
    fn decide(&mut self, verb: &str, rest: &str) -> Vec<Command> {
        let (id, note) = rest.split_once(' ').unwrap_or((rest, ""));
        let Some(ticket) = board::parse_ticket_id(id).filter(|_| !id.is_empty()) else {
            let waiting: Vec<String> = self.to_review().iter().map(|t| format!("T-{}", t.id)).collect();
            self.flash = Some(match waiting.as_slice() {
                [] => format!("Which ticket? /{verb} T-3"),
                ids => format!("Which ticket? Waiting for your review: {}", ids.join(", ")),
            });
            return Vec::new();
        };
        let squad = self.squad.clone();
        match verb {
            "accept" => vec![Command::Accept { squad, ticket }],
            "reopen" => vec![Command::Reopen { squad, ticket, note: note.trim().to_owned() }],
            _ => vec![Command::Cancel { squad, ticket }],
        }
    }

    /// `/mode auto` for the squad, `/mode frontend ask` for one member,
    /// `/mode frontend squad` to put it back on the squad's.
    fn set_mode(&mut self, rest: &str) -> Vec<Command> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let usage = || {
            let modes: Vec<String> = Mode::ALL.iter().map(|m| format!("{} ({})", m.label(), m.summary())).collect();
            format!("Modes: {} · /mode <mode> for the squad, /mode <member> <mode> for one", modes.join(" · "))
        };
        let squad = self.squad.clone();
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

    /// `/add designer frontend`, `/add qa codex`, `/add custom writer`.
    fn add(&mut self, rest: &str) -> Vec<Command> {
        let mut words: Vec<&str> = rest.split_whitespace().collect();
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
                self.flash = Some("Who? /add designer frontend · roles: pm designer frontend backend qa data, or custom <name>".into());
                return Vec::new();
            }
            ["custom"] => {
                self.flash = Some("Name the role: /add custom writer".into());
                return Vec::new();
            }
            ["custom", name, ..] => vec![name.to_lowercase()],
            ids => {
                let mut wanted = Vec::new();
                for id in ids {
                    match roles::role(id) {
                        Some(role) => wanted.push(role.id.to_owned()),
                        None => {
                            self.flash = Some(format!(
                                "No role '{id}'. Roles: pm designer frontend backend qa data — or /add custom {id}"
                            ));
                            return Vec::new();
                        }
                    }
                }
                wanted
            }
        };
        let names = wanted.join(" and ");
        let commands = wanted.iter().flat_map(|role| self.add_member(role, engine)).collect();
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
        other => match other.strip_prefix("mcp__equipe__") {
            // equipe's own board tools read as what they do.
            Some("board") => "board",
            Some("create_ticket") => "ticket",
            Some("update_ticket") => "ticket",
            Some("comment") => "comment",
            Some("submit_for_review") => "hand in",
            _ => other,
        },
    }
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
        assert_eq!(app.composer.text(), "a\nb\nc");
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
            usage: Some(Usage { input_tokens: Some(10), output_tokens: Some(5) }),
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
        assert!(matches!(enter(&mut app).as_slice(), [Command::Start { spec, .. }] if spec.role.as_deref() == Some("writer")));
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
    fn the_person_accepts_reopens_and_cancels_by_ticket() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/accept T-4");
        assert_eq!(enter(&mut app), vec![Command::Accept { squad: "acme".into(), ticket: 4 }]);
        type_text(&mut app, "/reopen 4 make the button blue");
        assert_eq!(enter(&mut app), vec![Command::Reopen { squad: "acme".into(), ticket: 4, note: "make the button blue".into() }]);
        type_text(&mut app, "/cancel t4");
        assert_eq!(enter(&mut app), vec![Command::Cancel { squad: "acme".into(), ticket: 4 }]);
        app.on_event(Event::TicketChanged { squad: "acme".into(), ticket: ticket(5, TicketStatus::Review) });
        type_text(&mut app, "/accept");
        assert!(enter(&mut app).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("T-5"), "{:?}", app.flash);
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
        app.on_event(Event::ModeChanged { squad: "acme".into(), member: Some("frontend".into()), mode: Some(Mode::Plan) });
        app.on_event(Event::ModeChanged { squad: "acme".into(), member: None, mode: Some(Mode::Ask) });
        assert_eq!(app.mode_of(&app.agents[1]), Mode::Plan);
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
