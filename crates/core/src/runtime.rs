//! The brain: runs sessions on engines, driven by [`Command`]s, reporting
//! [`Event`]s.
//!
//! Screens talk to it only through [`Handle::send`] and the event receiver from
//! [`start`]. All session state — which engine, which model, the conversation so
//! far, the worktree an editing session works in — lives here.

use std::collections::HashMap;
use std::path::PathBuf;

use equipe_protocol::{Access, Command, Engine, Event, SessionId, SessionSpec, TurnOutcome, Usage, Workspace};
use equipe_providers::claude_code::{self, Activity};
use equipe_providers::credentials::equipe_home;
use equipe_providers::openai::{codex, responses};
use equipe_providers::turn::TurnResult;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::worktree::{self, Sandbox};

pub struct Config {
    pub store: CredentialStore,
    /// equipe's home; editing worktrees live under it.
    pub home: PathBuf,
    /// Reported to backends as equipe's version.
    pub client_version: String,
    /// The Claude Code executable to run.
    pub claude_binary: String,
}

impl Config {
    /// The user's real setup: `~/.equipe` and the `claude` on PATH.
    pub fn for_user(client_version: &str) -> Self {
        Self {
            store: CredentialStore::default_location(),
            home: equipe_home(),
            client_version: client_version.to_owned(),
            claude_binary: claude_code::DEFAULT_BINARY.to_owned(),
        }
    }
}

/// Where screens send commands. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    commands: UnboundedSender<Command>,
}

impl Handle {
    pub fn send(&self, command: Command) {
        // The runtime only stops when every handle is gone, so this can't fail
        // while a screen holds one.
        let _ = self.commands.send(command);
    }
}

/// Start the runtime on the current Tokio runtime.
pub fn start(config: Config) -> (Handle, UnboundedReceiver<Event>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (evt_tx, evt_rx) = mpsc::unbounded_channel();
    let (runtime, done_rx) = Runtime::new(config, evt_tx);
    tokio::spawn(run(runtime, cmd_rx, done_rx));
    (Handle { commands: cmd_tx }, evt_rx)
}

struct Session {
    engine: Engine,
    model: Option<String>,
    access: Access,
    cwd: PathBuf,
    sandbox: Option<Sandbox>,
    codex: Option<Credentials>,
    /// Codex keeps nothing server-side; the conversation travels with each turn.
    history: Vec<responses::Message>,
    /// Claude Code keeps the conversation; this id continues it.
    claude_session: Option<String>,
    prompts: Vec<String>,
    turn: Option<JoinHandle<()>>,
}

/// What a finished turn hands back to the runtime loop.
struct TurnDone {
    session: SessionId,
    prompt: String,
    reply: Option<String>,
    claude_session: Option<String>,
    model: Option<String>,
}

struct Runtime {
    config: Config,
    client: reqwest::Client,
    events: UnboundedSender<Event>,
    done_tx: UnboundedSender<TurnDone>,
    sessions: HashMap<SessionId, Session>,
}

impl Runtime {
    fn new(config: Config, events: UnboundedSender<Event>) -> (Self, UnboundedReceiver<TurnDone>) {
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        let runtime = Self {
            config,
            client: reqwest::Client::new(),
            events,
            done_tx,
            sessions: HashMap::new(),
        };
        (runtime, done_rx)
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn error(&self, session: Option<SessionId>, message: impl Into<String>, next: Option<&str>) {
        self.emit(Event::Error {
            session,
            message: message.into(),
            next: next.map(str::to_owned),
        });
    }
}

/// The runtime loop: one command at a time, and turn results as they finish.
async fn run(mut rt: Runtime, mut commands: UnboundedReceiver<Command>, mut done: UnboundedReceiver<TurnDone>) {
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => rt.handle(command).await,
                None => break,
            },
            Some(finished) = done.recv() => rt.on_turn_done(finished),
        }
    }
    // Every screen is gone: stop what's running. Worktrees stay on disk.
    for session in rt.sessions.values() {
        if let Some(turn) = &session.turn {
            turn.abort();
        }
    }
}

impl Runtime {
    async fn handle(&mut self, command: Command) {
        match command {
            Command::Start { session, spec } => self.start(session, spec).await,
            Command::Submit { session, text } => self.submit(session, text),
            Command::Interrupt { session } => self.interrupt(session),
            Command::Close { session } => self.close(session),
        }
    }

    async fn start(&mut self, id: SessionId, spec: SessionSpec) {
        if self.sessions.contains_key(&id) {
            return self.error(Some(id), format!("session {id} is already open"), None);
        }
        let engine = match spec.engine {
            Some(engine) => engine,
            None => match self.first_ready_engine().await {
                Some(engine) => engine,
                None => {
                    return self.error(
                        Some(id),
                        "No engine is ready. Sign in to Codex or Claude Code first.",
                        Some("equipe login"),
                    );
                }
            },
        };
        if spec.access == Access::EditFiles && engine == Engine::Codex {
            return self.error(
                Some(id),
                "Editing runs on Claude Code for now; Codex edits arrive with its ACP engine.",
                Some("equipe try --edit --engine claude \"…\""),
            );
        }

        let mut model = spec.model.clone();
        let mut codex_cred = None;
        match engine {
            Engine::Codex => {
                let cred = match self.codex_credentials().await {
                    Ok(cred) => cred,
                    Err((message, next)) => return self.error(Some(id), message, Some(next)),
                };
                if model.is_none() {
                    // Ask the backend what this account has rather than hardcoding an id.
                    match codex::list_models(&self.client, &cred, &self.config.client_version).await {
                        Ok(models) => model = models.into_iter().next(),
                        Err(err) => return self.error(Some(id), err.to_string(), Some("equipe status")),
                    }
                    if model.is_none() {
                        return self.error(Some(id), "The backend offered no models.", Some("equipe status"));
                    }
                }
                codex_cred = Some(cred);
            }
            Engine::Claude => match claude_code::auth_status(&self.config.claude_binary).await {
                Ok(Some(status)) if status.logged_in => {}
                Ok(Some(_)) => return self.error(Some(id), "Claude Code isn't signed in.", Some("equipe login claude")),
                Ok(None) => {
                    return self.error(
                        Some(id),
                        "Claude Code isn't installed (no `claude` on PATH).",
                        Some("see https://code.claude.com/docs/en/setup"),
                    );
                }
                Err(err) => return self.error(Some(id), err.to_string(), None),
            },
        }

        let (cwd, sandbox, workspace) = match spec.access {
            Access::ReadOnly => (spec.cwd.clone(), None, None),
            Access::EditFiles => {
                let made = worktree::repo_root(&spec.cwd)
                    .and_then(|repo| worktree::create(&repo, &self.config.home, &format!("{:x}-{id}", now_secs())));
                match made {
                    Ok(sandbox) => {
                        let workspace = Workspace {
                            branch: sandbox.branch.clone(),
                            base: sandbox.base.clone(),
                            path: sandbox.path.clone(),
                        };
                        (sandbox.path.clone(), Some(sandbox), Some(workspace))
                    }
                    Err(message) => return self.error(Some(id), message, None),
                }
            }
        };

        self.sessions.insert(
            id,
            Session {
                engine,
                model: model.clone(),
                access: spec.access,
                cwd,
                sandbox,
                codex: codex_cred,
                history: Vec::new(),
                claude_session: None,
                prompts: Vec::new(),
                turn: None,
            },
        );
        self.emit(Event::SessionStarted { session: id, engine, model, workspace });
    }

    fn submit(&mut self, id: SessionId, text: String) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        if session.turn.as_ref().is_some_and(|t| !t.is_finished()) {
            return self.error(Some(id), "A turn is already running. Esc stops it.", None);
        }
        session.prompts.push(text.clone());

        let events = self.events.clone();
        let done = self.done_tx.clone();
        let turn = match session.engine {
            Engine::Codex => {
                let client = self.client.clone();
                let cred = session.codex.clone().expect("codex sessions hold credentials");
                let model = session.model.clone().unwrap_or_default();
                let history = session.history.clone();
                let version = self.config.client_version.clone();
                tokio::spawn(async move {
                    let mut streamed = false;
                    let request = responses::TurnRequest {
                        prompt: &text,
                        model: &model,
                        instructions: None,
                        history: &history,
                    };
                    let result = responses::run_turn(&client, &cred, &version, request, |delta| {
                        streamed = true;
                        let _ = events.send(Event::TextDelta { session: id, text: delta.to_owned() });
                    })
                    .await;
                    let reply = finish_turn(&events, id, result.map_err(codex_failure), streamed);
                    let _ = done.send(TurnDone { session: id, prompt: text, reply, claude_session: None, model: None });
                })
            }
            Engine::Claude => {
                let binary = self.config.claude_binary.clone();
                let cwd = session.cwd.clone();
                let model = session.model.clone();
                let resume = session.claude_session.clone();
                let access = match session.access {
                    Access::ReadOnly => claude_code::Access::ReadOnly,
                    Access::EditFiles => claude_code::Access::EditFiles,
                };
                tokio::spawn(async move {
                    let mut streamed = false;
                    let mut reported_model = None;
                    let request = claude_code::TurnRequest {
                        prompt: &text,
                        model: model.as_deref(),
                        access,
                        resume: resume.as_deref(),
                    };
                    let outcome = claude_code::run_turn(&binary, &cwd, request, |activity| match activity {
                        Activity::Text(text) => {
                            streamed = true;
                            let _ = events.send(Event::TextDelta { session: id, text });
                        }
                        Activity::Tool { name, target } => {
                            let _ = events.send(Event::ToolUse { session: id, tool: name, target });
                        }
                        Activity::Model(model) => {
                            reported_model = Some(model.clone());
                            let _ = events.send(Event::ModelKnown { session: id, model });
                        }
                    })
                    .await;
                    let claude_session = outcome.as_ref().ok().and_then(|turn| turn.session_id.clone());
                    let result = outcome.map(|turn| turn.result).map_err(claude_failure);
                    let reply = finish_turn(&events, id, result, streamed);
                    let _ = done.send(TurnDone { session: id, prompt: text, reply, claude_session, model: reported_model });
                })
            }
        };
        session.turn = Some(turn);
    }

    fn interrupt(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // A turn that already finished has its result on the way; let it land.
        if let Some(turn) = session.turn.take_if(|t| !t.is_finished()) {
            // Dropping the turn kills the engine process with it.
            turn.abort();
            self.emit(Event::TurnEnded { session: id, outcome: TurnOutcome::Interrupted, usage: None });
        }
    }

    fn on_turn_done(&mut self, done: TurnDone) {
        let Some(session) = self.sessions.get_mut(&done.session) else {
            return;
        };
        session.turn = None;
        if let Some(reply) = done.reply
            && session.engine == Engine::Codex
        {
            session.history.push(responses::Message { role: responses::Role::User, text: done.prompt });
            session.history.push(responses::Message { role: responses::Role::Assistant, text: reply });
        }
        if done.claude_session.is_some() {
            session.claude_session = done.claude_session;
        }
        if done.model.is_some() {
            session.model = done.model;
        }
    }

    fn close(&mut self, id: SessionId) {
        self.interrupt(id);
        let Some(session) = self.sessions.remove(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        if let Some(sandbox) = &session.sandbox {
            let subject: String = session
                .prompts
                .first()
                .and_then(|p| p.lines().next())
                .unwrap_or("agent changes")
                .chars()
                .take(60)
                .collect();
            let prompts = session.prompts.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n");
            let message = format!("equipe: {subject}\n\nPrompts:\n{prompts}\n\nRun by equipe through Claude Code.");
            match sandbox.commit_all(&message) {
                Ok(Some(_)) => {
                    let stat = sandbox.diff_stat().unwrap_or_default();
                    if let Err(err) = sandbox.remove_worktree() {
                        self.error(Some(id), err, None);
                    }
                    self.emit(Event::BranchLanded {
                        session: id,
                        branch: sandbox.branch.clone(),
                        base: sandbox.base.clone(),
                        stat,
                    });
                }
                Ok(None) => {
                    if let Err(err) = sandbox.discard() {
                        self.error(Some(id), err, None);
                    }
                    self.emit(Event::NothingChanged { session: id });
                }
                Err(err) => self.error(
                    Some(id),
                    format!("couldn't commit the changes: {err} — they are still in {}", sandbox.path.display()),
                    None,
                ),
            }
        }
        self.emit(Event::SessionClosed { session: id });
    }

    /// Codex if signed in, else Claude Code if signed in.
    async fn first_ready_engine(&self) -> Option<Engine> {
        if matches!(self.config.store.load("openai"), Ok(Some(_))) {
            return Some(Engine::Codex);
        }
        match claude_code::auth_status(&self.config.claude_binary).await {
            Ok(Some(status)) if status.logged_in => Some(Engine::Claude),
            _ => None,
        }
    }

    /// The Codex login, refreshed if stale — or why there isn't one.
    async fn codex_credentials(&self) -> Result<Credentials, (String, &'static str)> {
        let store = &self.config.store;
        let cred = store
            .load("openai")
            .map_err(|err| (format!("couldn't read {}: {err}", store.path("openai").display()), "equipe login codex"))?
            .filter(|c| c.method == AuthMethod::OauthSubscription)
            .ok_or(("Codex isn't signed in.".to_owned(), "equipe login codex"))?;
        codex::ensure_fresh(&self.client, store, cred)
            .await
            .map_err(|err| (format!("couldn't refresh the Codex login: {err}"), "equipe login codex"))
    }
}

/// Report a finished turn. Returns the reply text for the conversation history.
fn finish_turn(
    events: &UnboundedSender<Event>,
    id: SessionId,
    result: Result<TurnResult, (String, Option<&'static str>)>,
    streamed: bool,
) -> Option<String> {
    match result {
        Ok(result) => {
            // A turn that sent only a terminal event streamed nothing; send it whole
            // so screens never have to special-case it.
            if !streamed && !result.text.is_empty() {
                let _ = events.send(Event::TextDelta { session: id, text: result.text.clone() });
            }
            let outcome = if result.status == "completed" { TurnOutcome::Completed } else { TurnOutcome::Incomplete };
            let usage = result.usage.map(|u| Usage { input_tokens: u.input_tokens, output_tokens: u.output_tokens });
            let _ = events.send(Event::TurnEnded { session: id, outcome, usage });
            Some(result.text)
        }
        Err((message, next)) => {
            let _ = events.send(Event::Error { session: Some(id), message, next: next.map(str::to_owned) });
            let _ = events.send(Event::TurnEnded { session: id, outcome: TurnOutcome::Failed, usage: None });
            None
        }
    }
}

fn codex_failure(err: equipe_providers::Error) -> (String, Option<&'static str>) {
    let next = matches!(err, equipe_providers::Error::Unauthorized { .. }).then_some("equipe login codex");
    (err.to_string(), next)
}

fn claude_failure(err: equipe_providers::Error) -> (String, Option<&'static str>) {
    let message = err.to_string();
    let lower = message.to_lowercase();
    let next = if lower.contains("not installed") {
        Some("see https://code.claude.com/docs/en/setup")
    } else if lower.contains("login") || lower.contains("logged in") {
        Some("equipe login claude")
    } else {
        None
    };
    (message, next)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
