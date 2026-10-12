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
    Access, Command, Engine, Event, SessionId, SessionInfo, SessionSpec, SessionState, Snapshot, Transcript,
    new_session_id,
};

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
    pub engine: Option<Engine>,
    pub model: Option<String>,
    pub edit: bool,
    pub branch: Option<String>,
    pub state: AgentState,
    pub transcript: Transcript,
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
            engine: None,
            model: None,
            edit,
            branch: None,
            state: AgentState::Starting,
            transcript: Transcript::default(),
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
            engine: Some(info.engine),
            model: info.model,
            edit: info.access == Access::EditFiles,
            branch: info.workspace.map(|w| w.branch),
            state: match info.state {
                SessionState::Ready => AgentState::Ready,
                SessionState::Working => AgentState::Working,
            },
            transcript: Transcript::from_entries(info.recent),
            tokens: info.tokens,
            scroll_back: 0,
            base,
            pending: None,
        }
    }

    pub fn label(&self) -> String {
        let engine = self.engine.map_or("…", Engine::label);
        if self.edit { format!("{engine} · edit") } else { engine.to_owned() }
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

const HELP: &str = "Enter sends · Alt-Enter or Ctrl-J adds a line · Esc stops the agent · \
Tab switches agents · PgUp/PgDn scroll · Ctrl-C twice leaves — the team keeps working. \
Commands: /new [codex|claude] opens another agent · /edit [task] opens one that may edit files on its own branch · \
/close ends the selected agent and lands its branch · /quit leaves.";

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
        let start = if app.agents.is_empty() { app.open_agent(None, false, None) } else { Vec::new() };
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
            Event::SessionStarted { engine, model, workspace, .. } => {
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
            }
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
        let Some(agent) = self.agents.get_mut(self.selected) else {
            return Vec::new();
        };
        match agent.state {
            // The message shows once the daemon echoes it, the same on every screen.
            AgentState::Ready => vec![Command::Submit { session: agent.id, text }],
            AgentState::Starting => {
                // Sent as soon as the session is up.
                agent.pending = Some(text);
                Vec::new()
            }
            AgentState::Working => {
                self.composer.insert_str(&text);
                self.flash = Some("The agent is still working — Esc stops it.".into());
                Vec::new()
            }
            AgentState::Closing | AgentState::Closed | AgentState::Failed => {
                self.composer.insert_str(&text);
                self.flash = Some("This agent is closed. /new opens another.".into());
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
            "close" => match self.agents.get_mut(self.selected) {
                Some(agent) if agent.is_open() => {
                    agent.state = AgentState::Closing;
                    vec![Command::Close { session: agent.id }]
                }
                _ => Vec::new(),
            },
            "help" => {
                if let Some(agent) = self.agents.get_mut(self.selected) {
                    agent.transcript.push_local(equipe_core::protocol::Entry::Note { text: HELP.into() });
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

    fn started(id: SessionId) -> Event {
        Event::SessionStarted { session: id, project: "/repo".into(), engine: Engine::Claude, model: None, workspace: None }
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
    fn an_empty_team_starts_its_first_agent() {
        let (app, commands) = App::from_snapshot("/repo".into(), "/repo".into(), Snapshot::default());
        assert!(matches!(
            commands.as_slice(),
            [Command::Start { spec, .. }] if spec.engine.is_none() && spec.access == Access::ReadOnly
        ));
        assert_eq!(app.agents[0].state, AgentState::Starting);
    }

    #[test]
    fn attaching_shows_the_team_as_it_is() {
        let snapshot = Snapshot {
            sessions: vec![SessionInfo {
                session: 9,
                project: "/repo".into(),
                engine: Engine::Codex,
                model: Some("gpt-6-luna".into()),
                access: Access::EditFiles,
                workspace: Some(Workspace { branch: "equipe/try-9".into(), base: "abc".into(), path: "/w/9".into() }),
                state: SessionState::Working,
                title: Some("fix it".into()),
                tokens: 40,
                recent: vec![Entry::User { text: "fix it".into() }, Entry::Reply { text: "on it".into() }],
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
        app.on_event(Event::SessionStarted { session: 77, project: "/repo".into(), engine: Engine::Codex, model: None, workspace: None });
        assert_eq!(app.agents.len(), 2);
        // Another project's sessions stay out.
        app.on_event(Event::SessionStarted { session: 78, project: "/other".into(), engine: Engine::Codex, model: None, workspace: None });
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

    #[test]
    fn model_names_shorten_for_the_team_list() {
        let (mut app, id) = ready();
        app.on_event(Event::ModelKnown { session: id, model: "claude-opus-5-5[1m]".into() });
        assert_eq!(app.agents[0].short_model().as_deref(), Some("opus-5-5"));
    }
}
