//! What the terminal UI knows and how it reacts — no terminal, no engines.
//!
//! Keys and core [`Event`]s go in; the [`Command`]s to send to the core come
//! out. Everything here is plain state, so it is tested without a terminal.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use equipe_core::protocol::{Access, Command, Engine, Event, SessionId, SessionSpec, TurnOutcome};

use super::composer::Composer;

/// How long a first Ctrl-C waits for a second before it stops meaning "quit".
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// What the person asked.
    User(String),
    /// The agent's answer, as it streams.
    Reply(String),
    Tool { verb: String, target: String },
    /// A quiet line from equipe itself.
    Note(String),
    Error { message: String, next: Option<String> },
    Landed { branch: String, stat: String },
}

pub struct Agent {
    pub id: SessionId,
    pub engine: Option<Engine>,
    pub model: Option<String>,
    pub edit: bool,
    pub branch: Option<String>,
    pub state: AgentState,
    pub transcript: Vec<Entry>,
    pub tokens: u64,
    /// Lines scrolled up from the bottom; 0 follows new output.
    pub scroll_back: usize,
    /// Where the agent works; tool paths are shown relative to it.
    base: PathBuf,
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
            transcript: Vec::new(),
            tokens: 0,
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
    pub agents: Vec<Agent>,
    pub selected: usize,
    pub composer: Composer,
    /// A one-line message in the status bar, cleared by the next key.
    pub flash: Option<String>,
    pub should_quit: bool,
    /// Branches landed this session, for the summary printed on exit.
    pub landed: Vec<(String, String)>,
    next_id: SessionId,
    quit_armed: Option<Instant>,
}

const HELP: &str = "Enter sends · Alt-Enter or Ctrl-J adds a line · Esc stops the agent · \
Tab switches agents · PgUp/PgDn scroll · Ctrl-C twice quits. \
Commands: /new [codex|claude] opens another agent · /edit [task] opens one that may edit files on its own branch · \
/close lands its branch · /quit.";

impl App {
    /// A fresh UI with its first agent starting. Returns the command that starts it.
    pub fn new(cwd: PathBuf) -> (Self, Vec<Command>) {
        let mut app = Self {
            cwd,
            agents: Vec::new(),
            selected: 0,
            composer: Composer::default(),
            flash: None,
            should_quit: false,
            landed: Vec::new(),
            next_id: 1,
            quit_armed: None,
        };
        let start = app.open_agent(None, false, None);
        (app, start)
    }

    pub fn selected(&self) -> Option<&Agent> {
        self.agents.get(self.selected)
    }

    fn agent_mut(&mut self, id: SessionId) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    pub fn total_tokens(&self) -> u64 {
        self.agents.iter().map(|a| a.tokens).sum()
    }

    fn open_agent(&mut self, engine: Option<Engine>, edit: bool, first: Option<String>) -> Vec<Command> {
        let id = self.next_id;
        self.next_id += 1;
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

    /// Close every open agent, landing editing branches. Sent on quit.
    pub fn closing_commands(&mut self) -> Vec<Command> {
        self.agents
            .iter_mut()
            .filter(|a| a.is_open())
            .map(|a| {
                a.state = AgentState::Closing;
                Command::Close { session: a.id }
            })
            .collect()
    }

    pub fn all_closed(&self) -> bool {
        self.agents.iter().all(|a| !a.is_open())
    }

    // ── events from the core ──

    pub fn on_event(&mut self, event: Event) -> Vec<Command> {
        let Some(id) = event.session() else {
            if let Event::Error { message, .. } = event {
                self.flash = Some(message);
            }
            return Vec::new();
        };
        let Some(agent) = self.agent_mut(id) else {
            return Vec::new();
        };
        let mut commands = Vec::new();
        match event {
            Event::SessionStarted { engine, model, workspace, .. } => {
                agent.engine = Some(engine);
                agent.model = model;
                agent.state = AgentState::Ready;
                if let Some(ws) = workspace {
                    agent.base = ws.path.clone();
                    agent.transcript.push(Entry::Note(format!(
                        "Working on branch {} in its own worktree. Your checkout stays untouched; \
/close lands the branch.",
                        ws.branch
                    )));
                    agent.branch = Some(ws.branch);
                }
                if let Some(text) = agent.pending.take() {
                    agent.transcript.push(Entry::User(text.clone()));
                    agent.state = AgentState::Working;
                    commands.push(Command::Submit { session: id, text });
                }
            }
            Event::TextDelta { text, .. } => match agent.transcript.last_mut() {
                Some(Entry::Reply(reply)) => reply.push_str(&text),
                _ => agent.transcript.push(Entry::Reply(text)),
            },
            Event::ToolUse { tool, target, .. } => agent.transcript.push(Entry::Tool {
                verb: tool_verb(&tool).to_owned(),
                target: target.map(|t| relative(&t, &agent.base)).unwrap_or_default(),
            }),
            Event::ModelKnown { model, .. } => agent.model = Some(model),
            Event::TurnEnded { outcome, usage, .. } => {
                if agent.state == AgentState::Working {
                    agent.state = AgentState::Ready;
                }
                if let Some(u) = usage {
                    agent.tokens += u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0);
                }
                match outcome {
                    TurnOutcome::Interrupted => agent.transcript.push(Entry::Note("Stopped.".into())),
                    TurnOutcome::Incomplete => agent.transcript.push(Entry::Note("The engine ended early.".into())),
                    TurnOutcome::Completed | TurnOutcome::Failed => {}
                }
            }
            Event::BranchLanded { branch, stat, .. } => {
                agent.transcript.push(Entry::Landed { branch: branch.clone(), stat: stat.clone() });
                self.landed.push((branch, stat));
            }
            Event::NothingChanged { .. } => {
                agent.transcript.push(Entry::Note("No files changed, so no branch was kept.".into()));
            }
            Event::SessionClosed { .. } => agent.state = AgentState::Closed,
            Event::Error { message, next, .. } => {
                if agent.state == AgentState::Starting {
                    agent.state = AgentState::Failed;
                }
                agent.transcript.push(Entry::Error { message, next });
            }
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
                self.flash = Some("Press Ctrl-C again to quit.".into());
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
            AgentState::Ready => {
                agent.transcript.push(Entry::User(text.clone()));
                agent.state = AgentState::Working;
                agent.scroll_back = 0;
                vec![Command::Submit { session: agent.id, text }]
            }
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
                    agent.transcript.push(Entry::Note(HELP.into()));
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
fn relative(target: &str, base: &std::path::Path) -> String {
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
    use equipe_core::protocol::{Usage, Workspace};

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

    /// An app whose first agent has started on Claude Code.
    fn ready() -> App {
        let (mut app, _) = App::new("/repo".into());
        app.on_event(Event::SessionStarted { session: 1, engine: Engine::Claude, model: None, workspace: None });
        app
    }

    #[test]
    fn starts_its_first_agent_on_whatever_engine_is_ready() {
        let (app, commands) = App::new("/repo".into());
        assert!(matches!(
            commands.as_slice(),
            [Command::Start { session: 1, spec }] if spec.engine.is_none() && spec.access == Access::ReadOnly
        ));
        assert_eq!(app.agents[0].state, AgentState::Starting);
    }

    #[test]
    fn enter_sends_the_message() {
        let mut app = ready();
        type_text(&mut app, "hello");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: 1, text: "hello".into() }]);
        assert_eq!(app.agents[0].state, AgentState::Working);
        assert_eq!(app.agents[0].transcript, vec![Entry::User("hello".into())]);
    }

    #[test]
    fn a_pasted_newline_never_sends() {
        let mut app = ready();
        app.on_paste("fix this\nTypeError: x is undefined\n");
        // The paste alone sent nothing; the person's Enter sends all of it.
        assert_eq!(app.composer.line_count(), 3);
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(sent, vec![Command::Submit { session: 1, text: "fix this\nTypeError: x is undefined".into() }]);
    }

    #[test]
    fn alt_enter_and_ctrl_j_add_a_line() {
        let mut app = ready();
        type_text(&mut app, "a");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), Instant::now());
        type_text(&mut app, "b");
        app.on_key(ctrl('j'), Instant::now());
        type_text(&mut app, "c");
        assert_eq!(app.composer.text(), "a\nb\nc");
    }

    #[test]
    fn a_message_typed_while_starting_is_sent_once_ready() {
        let (mut app, _) = App::new("/repo".into());
        type_text(&mut app, "early");
        assert!(app.on_key(key(KeyCode::Enter), Instant::now()).is_empty());
        let sent = app.on_event(Event::SessionStarted { session: 1, engine: Engine::Codex, model: None, workspace: None });
        assert_eq!(sent, vec![Command::Submit { session: 1, text: "early".into() }]);
    }

    #[test]
    fn streamed_text_joins_one_reply_until_a_tool_runs() {
        let mut app = ready();
        for e in [
            Event::TextDelta { session: 1, text: "Let me ".into() },
            Event::TextDelta { session: 1, text: "look.".into() },
            Event::ToolUse { session: 1, tool: "Read".into(), target: Some("a.rs".into()) },
            Event::TextDelta { session: 1, text: "Done.".into() },
        ] {
            app.on_event(e);
        }
        assert_eq!(
            app.agents[0].transcript,
            vec![
                Entry::Reply("Let me look.".into()),
                Entry::Tool { verb: "read".into(), target: "a.rs".into() },
                Entry::Reply("Done.".into()),
            ]
        );
    }

    #[test]
    fn esc_stops_only_a_working_agent() {
        let mut app = ready();
        assert!(app.on_key(key(KeyCode::Esc), Instant::now()).is_empty());
        type_text(&mut app, "go");
        app.on_key(key(KeyCode::Enter), Instant::now());
        assert_eq!(app.on_key(key(KeyCode::Esc), Instant::now()), vec![Command::Interrupt { session: 1 }]);
    }

    #[test]
    fn a_finished_turn_frees_the_agent_and_counts_tokens() {
        let mut app = ready();
        type_text(&mut app, "go");
        app.on_key(key(KeyCode::Enter), Instant::now());
        app.on_event(Event::TurnEnded {
            session: 1,
            outcome: TurnOutcome::Completed,
            usage: Some(Usage { input_tokens: Some(10), output_tokens: Some(5) }),
        });
        assert_eq!(app.agents[0].state, AgentState::Ready);
        assert_eq!(app.total_tokens(), 15);
    }

    #[test]
    fn ctrl_c_clears_a_draft_then_quits_on_a_second_press() {
        let mut app = ready();
        let t0 = Instant::now();
        type_text(&mut app, "draft");
        app.on_key(ctrl('c'), t0);
        assert!(app.composer.is_empty() && !app.should_quit);
        app.on_key(ctrl('c'), t0);
        assert!(!app.should_quit, "one press only warns");
        app.on_key(ctrl('c'), t0 + Duration::from_millis(500));
        assert!(app.should_quit);
    }

    #[test]
    fn a_slow_second_ctrl_c_only_warns_again() {
        let mut app = ready();
        let t0 = Instant::now();
        app.on_key(ctrl('c'), t0);
        app.on_key(ctrl('c'), t0 + Duration::from_secs(5));
        assert!(!app.should_quit);
    }

    #[test]
    fn slash_new_opens_another_agent_and_tab_switches() {
        let mut app = ready();
        type_text(&mut app, "/new codex");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(matches!(sent.as_slice(), [Command::Start { session: 2, spec }] if spec.engine == Some(Engine::Codex)));
        assert_eq!(app.selected, 1);
        app.on_key(key(KeyCode::Tab), Instant::now());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn slash_edit_opens_an_editing_agent_and_sends_its_task() {
        let mut app = ready();
        type_text(&mut app, "/edit add a docstring");
        let sent = app.on_key(key(KeyCode::Enter), Instant::now());
        assert!(matches!(
            sent.as_slice(),
            [Command::Start { session: 2, spec }] if spec.access == Access::EditFiles && spec.engine == Some(Engine::Claude)
        ));
        let sent = app.on_event(Event::SessionStarted {
            session: 2,
            engine: Engine::Claude,
            model: None,
            workspace: Some(Workspace { branch: "equipe/try-1".into(), base: "abc".into(), path: "/w".into() }),
        });
        assert_eq!(sent, vec![Command::Submit { session: 2, text: "add a docstring".into() }]);
        assert_eq!(app.agents[1].branch.as_deref(), Some("equipe/try-1"));
    }

    #[test]
    fn a_start_failure_marks_the_agent_failed_with_the_fix() {
        let (mut app, _) = App::new("/repo".into());
        app.on_event(Event::Error { session: Some(1), message: "No engine is ready.".into(), next: Some("equipe login".into()) });
        assert_eq!(app.agents[0].state, AgentState::Failed);
        assert!(matches!(app.agents[0].transcript.last(), Some(Entry::Error { next: Some(n), .. }) if n == "equipe login"));
    }

    #[test]
    fn quitting_closes_every_open_agent() {
        let mut app = ready();
        type_text(&mut app, "/edit");
        app.on_key(key(KeyCode::Enter), Instant::now());
        let closing = app.closing_commands();
        assert_eq!(closing, vec![Command::Close { session: 1 }, Command::Close { session: 2 }]);
        app.on_event(Event::BranchLanded { session: 2, branch: "equipe/try-2".into(), base: "a".into(), stat: "1 file".into() });
        app.on_event(Event::SessionClosed { session: 1 });
        app.on_event(Event::SessionClosed { session: 2 });
        assert!(app.all_closed());
        assert_eq!(app.landed, vec![("equipe/try-2".to_owned(), "1 file".to_owned())]);
    }

    #[test]
    fn tool_paths_show_relative_to_where_the_agent_works() {
        assert_eq!(relative("/repo/src/x.rs", std::path::Path::new("/repo")), "src/x.rs");
        assert_eq!(relative("/elsewhere/x.rs", std::path::Path::new("/repo")), "/elsewhere/x.rs");
        assert_eq!(relative("*.rs", std::path::Path::new("/repo")), "*.rs");
    }

    #[test]
    fn model_names_shorten_for_the_team_list() {
        let mut app = ready();
        app.on_event(Event::ModelKnown { session: 1, model: "claude-opus-5-5[1m]".into() });
        assert_eq!(app.agents[0].short_model().as_deref(), Some("opus-5-5"));
    }
}
