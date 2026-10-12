//! What the terminal UI knows and how it reacts — no terminal, no engines.
//!
//! Keys and core [`Event`]s go in; the [`Command`]s to send to the core come
//! out. Everything here is plain state, so it is tested without a terminal.
//!
//! The UI is a view of the daemon's team: it starts from a snapshot, follows
//! events, and leaving it changes nothing — the agents keep working.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use equipe_core::protocol::{
    Access, Command, Engine, Entry, Event, Member, SessionId, SessionInfo, SessionSpec, SessionState, Snapshot,
    Transcript, new_session_id,
};
use equipe_core::roles;

use super::composer::Composer;

/// How long a first Ctrl-C waits for a second before it stops meaning "leave".
const QUIT_WINDOW: Duration = Duration::from_secs(2);

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

pub struct Agent {
    pub id: SessionId,
    /// Who it is on the team; `None` for a plain agent.
    pub member: Option<Member>,
    pub engine: Option<Engine>,
    pub model: Option<String>,
    pub edit: bool,
    pub branch: Option<String>,
    pub state: AgentState,
    pub transcript: Transcript,
    /// Memos waiting for its turn to end.
    pub queued: Vec<String>,
    pub tokens: u64,
    /// Lines scrolled up from the bottom; 0 follows new output.
    pub scroll_back: usize,
    /// Where the agent works; tool paths are shown relative to it.
    pub base: PathBuf,
    /// A message to send once the session has started.
    pending: Option<String>,
}

impl Agent {
    fn new(id: SessionId, edit: bool, base: PathBuf) -> Self {
        Self {
            id,
            member: None,
            engine: None,
            model: None,
            edit,
            branch: None,
            state: AgentState::Starting,
            transcript: Transcript::default(),
            queued: Vec::new(),
            tokens: 0,
            scroll_back: 0,
            base,
            pending: None,
        }
    }

    fn from_info(info: SessionInfo) -> Self {
        let base = info.workspace.as_ref().map_or_else(|| info.project.clone(), |w| w.path.clone());
        Self {
            id: info.session,
            member: info.member,
            engine: Some(info.engine),
            model: info.model,
            edit: info.access == Access::EditFiles,
            branch: info.workspace.map(|w| w.branch),
            state: match info.state {
                SessionState::Ready => AgentState::Ready,
                SessionState::Working => AgentState::Working,
            },
            transcript: Transcript::from_entries(info.recent),
            queued: info.queued,
            tokens: info.tokens,
            scroll_back: 0,
            base,
            pending: None,
        }
    }

    /// What the team calls it: its name, or for a plain agent its engine.
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
}

pub struct App {
    pub cwd: PathBuf,
    /// The project this screen follows; sessions others open here appear too.
    pub project: PathBuf,
    pub agents: Vec<Agent>,
    pub selected: usize,
    pub composer: Composer,
    /// A one-line message in the status bar, cleared by the next key.
    pub flash: Option<String>,
    pub should_quit: bool,
    /// Branches landed while this screen watched, for the summary on leaving.
    pub landed: Vec<(String, String)>,
    quit_armed: Option<Instant>,
}

const HELP: &str = "The PM is your main contact: tell it what you want and it plans the work. \
You can talk to anyone directly — Tab or @name — and the PM hears about it.\n\
Enter sends; while a member works, a message waits as a memo until its turn ends · Esc stops the member \
(a waiting memo goes in at once) · Alt-Enter or Ctrl-J adds a line · PgUp/PgDn scroll · Ctrl-C twice leaves — \
the team keeps working.\n\
/add <role…> [codex|claude] brings in teammates: pm, designer, frontend, backend, qa, data, or custom <name> · \
/close ends the selected member and lands its branch · /new and /edit [task] open a plain agent · /quit leaves.";

impl App {
    /// The team as the daemon has it. With nobody on the team yet, the first
    /// agent starts — that command is returned.
    pub fn from_snapshot(cwd: PathBuf, project: PathBuf, snapshot: Snapshot) -> (Self, Vec<Command>) {
        let mut app = Self {
            cwd,
            project,
            agents: snapshot.sessions.into_iter().map(Agent::from_info).collect(),
            selected: 0,
            composer: Composer::default(),
            flash: None,
            should_quit: false,
            landed: Vec::new(),
            quit_armed: None,
        };
        // Talk to the PM first: it is the main contact.
        app.selected = app.agents.iter().position(Agent::is_pm).unwrap_or(0);
        let start = if app.agents.is_empty() { app.add_member(roles::PM, None) } else { Vec::new() };
        (app, start)
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

    /// Who a message goes to when Enter is pressed, and whether that is someone
    /// other than the PM while a PM is on the team (so the PM hears about it).
    pub fn addressee(&self) -> Option<(String, bool)> {
        let agent = self.selected()?;
        let has_pm = self.agents.iter().any(|a| a.is_pm() && a.is_open());
        Some((agent.label(), agent.member.is_some() && !agent.is_pm() && has_pm))
    }

    fn open_agent(&mut self, engine: Option<Engine>, edit: bool, first: Option<String>) -> Vec<Command> {
        let id = new_session_id();
        let mut agent = Agent::new(id, edit, self.cwd.clone());
        agent.pending = first;
        self.agents.push(agent);
        self.selected = self.agents.len() - 1;
        vec![Command::Start {
            session: id,
            spec: SessionSpec {
                // Editing runs on Claude Code for now.
                engine: if edit { Some(Engine::Claude) } else { engine },
                model: None,
                access: if edit { Access::EditFiles } else { Access::ReadOnly },
                cwd: self.cwd.clone(),
                role: None,
            },
        }]
    }

    /// Bring a member onto the team. The selection stays where it is.
    fn add_member(&mut self, role: &str, engine: Option<Engine>) -> Vec<Command> {
        let id = new_session_id();
        let edits = roles::role_or_custom(role).edits;
        let mut agent = Agent::new(id, edits, self.cwd.clone());
        // Named by the core once it starts; until then, by its role.
        agent.member = Some(Member { role: role.to_owned(), name: role.to_owned() });
        self.agents.push(agent);
        vec![Command::Start {
            session: id,
            spec: SessionSpec {
                engine,
                model: None,
                access: if edits { Access::EditFiles } else { Access::ReadOnly },
                cwd: self.cwd.clone(),
                role: Some(role.to_owned()),
            },
        }]
    }

    // ── events from the core ──

    pub fn on_event(&mut self, event: Event) -> Vec<Command> {
        let Some(id) = event.session() else {
            if let Event::Error { message, .. } = event {
                self.flash = Some(message);
            }
            return Vec::new();
        };
        // A session another screen opened on this project joins the team here too.
        if let Event::SessionStarted { project, .. } = &event
            && *project == self.project
            && !self.agents.iter().any(|a| a.id == id)
        {
            self.agents.push(Agent::new(id, false, self.cwd.clone()));
        }
        let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) else {
            return Vec::new();
        };
        agent.transcript.apply(&event);
        let mut commands = Vec::new();
        match event {
            Event::SessionStarted { engine, model, workspace, member, .. } => {
                if member.is_some() {
                    agent.member = member;
                }
                agent.engine = Some(engine);
                agent.model = model;
                agent.state = AgentState::Ready;
                if let Some(ws) = workspace {
                    agent.edit = true;
                    agent.base = ws.path;
                    agent.branch = Some(ws.branch);
                }
                if let Some(text) = agent.pending.take() {
                    agent.state = AgentState::Working;
                    commands.push(Command::Submit { session: id, text });
                }
            }
            Event::UserMessage { .. } => {
                // Typed on another screen, perhaps: either way the agent is busy now.
                agent.state = AgentState::Working;
                agent.scroll_back = 0;
                // Waiting memos are delivered together as one message.
                agent.queued.clear();
            }
            Event::MessageQueued { text, .. } => agent.queued.push(text),
            Event::ModelKnown { model, .. } => agent.model = Some(model),
            Event::TurnEnded { usage, .. } => {
                if agent.state == AgentState::Working {
                    agent.state = AgentState::Ready;
                }
                if let Some(u) = usage {
                    agent.tokens += u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0);
                }
            }
            Event::BranchLanded { branch, stat, .. } => self.landed.push((branch, stat)),
            Event::SessionClosed { .. } => agent.state = AgentState::Closed,
            Event::Error { .. } if agent.state == AgentState::Starting => agent.state = AgentState::Failed,
            _ => {}
        }
        commands
    }

    // ── input ──

    pub fn on_paste(&mut self, text: &str) {
        self.flash = None;
        self.composer.insert_str(text);
    }

    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Command> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if !(ctrl && key.code == KeyCode::Char('c')) {
            self.quit_armed = None;
            self.flash = None;
        }
        match key.code {
            KeyCode::Char('c') if ctrl => return self.ctrl_c(now),
            KeyCode::Enter if alt || shift => self.composer.newline(),
            KeyCode::Char('j') if ctrl => self.composer.newline(),
            KeyCode::Enter => return self.submit(),
            KeyCode::Esc => return self.interrupt(),
            KeyCode::Tab => self.cycle(1),
            KeyCode::BackTab => self.cycle(-1),
            KeyCode::PageUp => self.scroll(10),
            KeyCode::PageDown => self.scroll(-10),
            KeyCode::Up if !self.composer.recall_older() => self.scroll(1),
            KeyCode::Down if !self.composer.recall_newer() => self.scroll(-1),
            KeyCode::Up | KeyCode::Down => {}
            KeyCode::Left => self.composer.left(),
            KeyCode::Right => self.composer.right(),
            KeyCode::Home => self.composer.home(),
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
                self.flash = Some("Press Ctrl-C again to leave. The team keeps working.".into());
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
        let n = self.agents.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + step).rem_euclid(n) as usize;
        }
    }

    fn scroll(&mut self, lines: isize) {
        if let Some(agent) = self.agents.get_mut(self.selected) {
            agent.scroll_back = (agent.scroll_back as isize + lines).max(0) as usize;
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
                    return Vec::new();
                }
                Some(i) => {
                    target = i;
                    text = rest.trim().to_owned();
                }
                None => {
                    self.composer.insert_str(&text);
                    self.flash = Some(format!("No one called @{name} on the team. /add brings someone in."));
                    return Vec::new();
                }
            }
        }
        let Some(agent) = self.agents.get_mut(target) else {
            return Vec::new();
        };
        let away = target != self.selected;
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
                self.flash = Some(format!("{name} is closed. /add brings in a new one."));
                Vec::new()
            }
        }
    }

    fn slash(&mut self, command: &str) -> Vec<Command> {
        let (name, rest) = command.split_once(' ').unwrap_or((command, ""));
        let rest = rest.trim();
        match name {
            "new" => {
                let engine = match rest {
                    "" => None,
                    "codex" => Some(Engine::Codex),
                    "claude" => Some(Engine::Claude),
                    other => {
                        self.flash = Some(format!("Unknown engine '{other}' — try /new codex or /new claude."));
                        return Vec::new();
                    }
                };
                self.open_agent(engine, false, None)
            }
            "edit" => self.open_agent(None, true, (!rest.is_empty()).then(|| rest.to_owned())),
            "add" => self.add(rest),
            "close" => match self.agents.get_mut(self.selected) {
                Some(agent) if agent.is_open() => {
                    agent.state = AgentState::Closing;
                    vec![Command::Close { session: agent.id }]
                }
                _ => Vec::new(),
            },
            "help" => {
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
}

impl App {
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

/// How a tool use reads in one word.
pub fn tool_verb(name: &str) -> &str {
    match name {
        "Read" => "read",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Write" => "write",
        "Bash" => "run",
        "Glob" | "Grep" => "search",
        "WebFetch" | "WebSearch" => "fetch",
        other => other,
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
    use equipe_core::protocol::{Entry, TurnOutcome, Usage, Workspace};

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

    fn pm() -> Option<Member> {
        Some(Member { role: "pm".into(), name: "pm".into() })
    }

    fn started(id: SessionId) -> Event {
        Event::SessionStarted { session: id, project: "/repo".into(), engine: Engine::Claude, model: None, workspace: None, member: pm() }
    }

    /// A member that has started, as the core announces it.
    fn joined(app: &mut App, id: SessionId, name: &str) {
        app.on_event(Event::SessionStarted {
            session: id,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: None,
            workspace: None,
            member: Some(Member { role: name.into(), name: name.into() }),
        });
    }

    /// A fresh UI whose first agent has started. Returns it with that agent's id.
    fn ready() -> (App, SessionId) {
        let (mut app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        let Some(Command::Start { session, .. }) = commands.first() else { panic!("no start") };
        let id = *session;
        app.on_event(started(id));
        (app, id)
    }

    #[test]
    fn an_empty_team_starts_with_the_pm() {
        let (app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        assert!(matches!(
            commands.as_slice(),
            [Command::Start { spec, .. }]
                if spec.role.as_deref() == Some("pm") && spec.engine.is_none() && spec.access == Access::ReadOnly
        ));
        assert_eq!(app.agents[0].state, AgentState::Starting);
        assert_eq!(app.addressee(), Some(("pm".into(), false)));
    }

    #[test]
    fn attaching_shows_the_team_as_it_is() {
        let snapshot = Snapshot {
            sessions: vec![SessionInfo {
                session: 9,
                project: "/repo".into(),
                member: None,
                engine: Engine::Codex,
                model: Some("gpt-6-luna".into()),
                access: Access::EditFiles,
                workspace: Some(Workspace { branch: "equipe/try-9".into(), base: "abc".into(), path: "/w/9".into() }),
                state: SessionState::Working,
                title: Some("fix it".into()),
                tokens: 40,
                recent: vec![Entry::User { text: "fix it".into() }, Entry::Reply { text: "on it".into() }],
                queued: Vec::new(),
            }],
        };
        let (app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), snapshot);
        assert!(commands.is_empty(), "nothing new starts when a team exists");
        let agent = &app.agents[0];
        assert_eq!(agent.state, AgentState::Working);
        assert_eq!(agent.branch.as_deref(), Some("equipe/try-9"));
        assert_eq!(agent.transcript.entries().len(), 2);
        assert_eq!(app.total_tokens(), 40);
    }

    #[test]
    fn enter_sends_and_the_echo_shows_the_message() {
        let (mut app, id) = ready();
        type_text(&mut app, "hello");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: id, text: "hello".into() }]);
        app.on_event(Event::UserMessage { session: id, text: "hello".into() });
        assert_eq!(app.agents[0].state, AgentState::Working);
        assert_eq!(app.agents[0].transcript.entries(), &[Entry::User { text: "hello".into() }]);
    }

    #[test]
    fn a_pasted_newline_never_sends() {
        let (mut app, id) = ready();
        app.on_paste("fix this\nTypeError: x is undefined\n");
        assert_eq!(app.composer.line_count(), 3);
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: id, text: "fix this\nTypeError: x is undefined".into() }]);
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
        let (mut app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        let id = *id;
        type_text(&mut app, "early");
        assert!(app.on_key(key(KeyCode::Enter), Instant::now()).is_empty());
        let sent = app.on_event(started(id));
        assert_eq!(sent, vec![Command::Submit { session: id, text: "early".into() }]);
    }

    #[test]
    fn a_session_opened_on_another_screen_joins_this_team() {
        let (mut app, _) = ready();
        app.on_event(Event::SessionStarted { session: 77, project: "/repo".into(), engine: Engine::Codex, model: None, workspace: None, member: None });
        assert_eq!(app.agents.len(), 2);
        // Another project's sessions stay out.
        app.on_event(Event::SessionStarted { session: 78, project: "/other".into(), engine: Engine::Codex, model: None, workspace: None, member: None });
        assert_eq!(app.agents.len(), 2);
    }

    #[test]
    fn esc_stops_only_a_working_agent() {
        let (mut app, id) = ready();
        assert!(app.on_key(key(KeyCode::Esc), Instant::now()).is_empty());
        app.on_event(Event::UserMessage { session: id, text: "go".into() });
        assert_eq!(app.on_key(key(KeyCode::Esc), Instant::now()), vec![Command::Interrupt { session: id }]);
    }

    #[test]
    fn a_finished_turn_frees_the_agent_and_counts_tokens() {
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
    fn leaving_sends_nothing_to_the_team() {
        let (mut app, _) = ready();
        type_text(&mut app, "/quit");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(sent.is_empty(), "leaving never closes agents: {sent:?}");
        assert!(app.should_quit);
    }

    #[test]
    fn slash_new_opens_another_agent_and_tab_switches() {
        let (mut app, _) = ready();
        type_text(&mut app, "/new codex");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(matches!(sent.as_slice(), [Command::Start { spec, .. }] if spec.engine == Some(Engine::Codex)));
        assert_eq!(app.selected, 1);
        app.on_key(key(KeyCode::Tab), Instant::now());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn slash_edit_opens_an_editing_agent_and_sends_its_task() {
        let (mut app, _) = ready();
        type_text(&mut app, "/edit add a docstring");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        let Some(Command::Start { session: id, spec }) = sent.first() else { panic!("{sent:?}") };
        assert_eq!((spec.access, spec.engine), (Access::EditFiles, Some(Engine::Claude)));
        let id = *id;
        let sent = app.on_event(Event::SessionStarted {
            session: id,
            project: "/repo".into(),
            engine: Engine::Claude,
            model: None,
            workspace: Some(Workspace { branch: "equipe/try-1".into(), base: "abc".into(), path: "/w".into() }),
            member: None,
        });
        assert_eq!(sent, vec![Command::Submit { session: id, text: "add a docstring".into() }]);
        assert_eq!(app.agents[1].branch.as_deref(), Some("equipe/try-1"));
    }

    #[test]
    fn a_start_failure_marks_the_agent_failed_with_the_fix() {
        let (mut app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        let Some(Command::Start { session: id, .. }) = commands.first() else { panic!() };
        app.on_event(Event::Error { session: Some(*id), message: "No engine is ready.".into(), next: Some("equipe login".into()) });
        assert_eq!(app.agents[0].state, AgentState::Failed);
        assert!(matches!(app.agents[0].transcript.entries().last(), Some(Entry::Error { next: Some(n), .. }) if n == "equipe login"));
    }

    #[test]
    fn tool_paths_show_relative_to_where_the_agent_works() {
        assert_eq!(relative("/repo/src/x.rs", std::path::Path::new("/repo")), "src/x.rs");
        assert_eq!(relative("/elsewhere/x.rs", std::path::Path::new("/repo")), "/elsewhere/x.rs");
        assert_eq!(relative("*.rs", std::path::Path::new("/repo")), "*.rs");
    }

    // ── the team ──

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
        }
    }

    /// The PM (selected) and a frontend engineer, both ready.
    fn team() -> (App, SessionId, SessionId) {
        let frontend = Some(Member { role: "frontend".into(), name: "frontend".into() });
        let snapshot = Snapshot { sessions: vec![info(1, pm()), info(2, frontend)] };
        let (app, _) = App::from_snapshot("/repo".into(), "/repo".into(), snapshot);
        (app, 1, 2)
    }

    #[test]
    fn attaching_talks_to_the_pm_first() {
        let frontend = Some(Member { role: "frontend".into(), name: "frontend".into() });
        let snapshot = Snapshot { sessions: vec![info(2, frontend), info(1, pm())] };
        let (app, _) = App::from_snapshot("/repo".into(), "/repo".into(), snapshot);
        assert_eq!(app.selected().unwrap().label(), "pm");
    }

    #[test]
    fn add_brings_in_members_with_their_access_and_stays_put() {
        let (mut app, _, _) = team();
        type_text(&mut app, "/add designer backend");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        let specs: Vec<_> = sent
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
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(matches!(sent.as_slice(), [Command::Start { spec, .. }] if spec.engine == Some(Engine::Codex) && spec.access == Access::ReadOnly));

        type_text(&mut app, "/add custom writer");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(matches!(sent.as_slice(), [Command::Start { spec, .. }] if spec.role.as_deref() == Some("writer")));

        type_text(&mut app, "/add fronend");
        assert!(app.on_key(key(KeyCode::Enter), Instant::now()).is_empty());
        assert!(app.flash.as_deref().unwrap().contains("/add custom fronend"), "{:?}", app.flash);
    }

    #[test]
    fn a_mention_sends_to_that_member_without_leaving_the_pm() {
        let (mut app, _, frontend) = team();
        type_text(&mut app, "@frontend make the header sticky");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: frontend, text: "make the header sticky".into() }]);
        assert_eq!(app.selected().unwrap().label(), "pm");
    }

    #[test]
    fn a_bare_mention_goes_to_talk_to_that_member() {
        let (mut app, _, _) = team();
        type_text(&mut app, "@frontend");
        assert!(app.on_key(key(KeyCode::Enter), Instant::now()).is_empty());
        assert_eq!(app.selected().unwrap().label(), "frontend");
        assert_eq!(app.addressee(), Some(("frontend".into(), true)), "the PM hears about it");
    }

    #[test]
    fn a_mention_of_nobody_keeps_the_draft() {
        let (mut app, _, _) = team();
        type_text(&mut app, "@ceo hi");
        assert!(app.on_key(key(KeyCode::Enter), Instant::now()).is_empty());
        assert_eq!(app.composer.text(), "@ceo hi");
        assert!(app.flash.as_deref().unwrap().contains("@ceo"));
    }

    #[test]
    fn a_message_to_a_working_member_is_left_as_a_memo() {
        let (mut app, pm, _) = team();
        app.on_event(Event::UserMessage { session: pm, text: "plan it".into() });
        type_text(&mut app, "also the pricing page");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: pm, text: "also the pricing page".into() }]);
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
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        let Some(Command::Start { session, .. }) = sent.first() else { panic!() };
        joined(&mut app, *session, "frontend-2");
        assert_eq!(app.agents[2].label(), "frontend-2");
    }

    #[test]
    fn model_names_shorten_for_the_team_list() {
        let (mut app, id) = ready();
        app.on_event(Event::ModelKnown { session: id, model: "claude-opus-5-5[1m]".into() });
        assert_eq!(app.agents[0].short_model().as_deref(), Some("opus-5-5"));
    }
}
