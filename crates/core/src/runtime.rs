//! The brain: runs sessions on engines, driven by [`Command`]s, reporting
//! [`Event`]s.
//!
//! Screens talk to it only through a [`Handle`] and the event receiver from
//! [`start`]. All session state — engine, model, the conversation so far, the
//! worktree an editing session works in — lives here. Every event passes through
//! one loop, which folds it into the session's transcript and, when the runtime
//! is persistent, journals it; a persistent runtime resumes its open sessions
//! when it starts again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_protocol::{
    Access, Command, Engine, Entry, Event, SessionId, SessionInfo, SessionSpec, SessionState, Snapshot, Transcript,
    TurnOutcome, Usage, Workspace,
};
use equipe_providers::claude_code::{self, Activity};
use equipe_providers::credentials::equipe_home;
use equipe_providers::openai::{codex, responses};
use equipe_providers::turn::TurnResult;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::journal::{self, Journal, SessionRecord};
use crate::worktree::{self, Sandbox};

/// How many recent lines a snapshot carries per session.
const SNAPSHOT_LINES: usize = 200;

pub struct Config {
    pub store: CredentialStore,
    /// equipe's home: worktrees and, when persistent, journals live under it.
    pub home: PathBuf,
    /// Reported to backends as equipe's version.
    pub client_version: String,
    /// The Claude Code executable to run.
    pub claude_binary: String,
    /// Journal every session and resume open ones at start (the daemon).
    pub persist: bool,
    /// Conversations older than this are pruned to the session record.
    pub retention_days: u64,
}

impl Config {
    /// The user's real setup: `~/.equipe` and the `claude` on PATH, in memory only.
    pub fn for_user(client_version: &str) -> Self {
        Self {
            store: CredentialStore::default_location(),
            home: equipe_home(),
            client_version: client_version.to_owned(),
            claude_binary: claude_code::DEFAULT_BINARY.to_owned(),
            persist: false,
            retention_days: journal::DEFAULT_RETENTION_DAYS,
        }
    }

    /// Journal sessions and resume them across restarts.
    pub fn persistent(mut self) -> Self {
        self.persist = true;
        self
    }
}

/// A new screen's view: what's open now, and every event after that.
pub type Subscription = (Snapshot, UnboundedReceiver<Event>);

enum Request {
    Command(Command),
    Snapshot(PathBuf, oneshot::Sender<Snapshot>),
    Subscribe(PathBuf, oneshot::Sender<Subscription>),
    Shutdown(oneshot::Sender<()>),
}

/// Where screens send commands. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    requests: UnboundedSender<Request>,
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.requests.send(Request::Command(command));
    }

    /// Everything open on `project`, for a screen that attaches now.
    pub async fn snapshot(&self, project: &Path) -> Snapshot {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Snapshot(project.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    /// Attach a screen: a snapshot of `project` and the events that follow it.
    /// Both are taken in one step of the runtime loop, so no event falls between
    /// them and none is counted twice.
    pub async fn subscribe(&self, project: &Path) -> Option<Subscription> {
        let (tx, rx) = oneshot::channel();
        self.requests.send(Request::Subscribe(project.to_owned(), tx)).ok()?;
        rx.await.ok()
    }

    /// Stop running turns and the runtime. Open sessions stay journaled.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.requests.send(Request::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

/// Start the runtime on the current Tokio runtime.
pub fn start(config: Config) -> (Handle, UnboundedReceiver<Event>) {
    let (req_tx, req_rx) = mpsc::unbounded_channel();
    let (evt_tx, evt_rx) = mpsc::unbounded_channel();
    let (internal_tx, internal_rx) = mpsc::unbounded_channel();
    let mut runtime = Runtime {
        config,
        client: reqwest::Client::new(),
        subscribers: vec![evt_tx],
        internal: internal_tx,
        sessions: HashMap::new(),
        journals: HashMap::new(),
    };
    if runtime.config.persist {
        runtime.restore();
        runtime.prune();
    }
    tokio::spawn(run(runtime, req_rx, internal_rx));
    (Handle { requests: req_tx }, evt_rx)
}

/// The project a directory belongs to: its repository root, or itself.
pub fn project_of(dir: &Path) -> PathBuf {
    worktree::repo_root(dir).unwrap_or_else(|_| dir.canonicalize().unwrap_or_else(|_| dir.to_owned()))
}

struct Session {
    project: PathBuf,
    engine: Engine,
    model: Option<String>,
    access: Access,
    cwd: PathBuf,
    sandbox: Option<Sandbox>,
    /// Loaded when the first Codex turn needs it.
    codex: Option<Credentials>,
    /// Codex keeps nothing server-side; the conversation travels with each turn.
    history: Vec<responses::Message>,
    /// Claude Code keeps the conversation; this id continues it.
    claude_session: Option<String>,
    prompts: Vec<String>,
    title: Option<String>,
    tokens: u64,
    transcript: Transcript,
    turn: Option<JoinHandle<()>>,
}

/// What a finished turn hands back to the loop.
struct TurnDone {
    session: SessionId,
    prompt: String,
    reply: Option<String>,
    claude_session: Option<String>,
}

enum Internal {
    Event(Event),
    Done(TurnDone),
}

struct Runtime {
    config: Config,
    client: reqwest::Client,
    /// Every attached screen. Closed ones drop out on the next event.
    subscribers: Vec<UnboundedSender<Event>>,
    internal: UnboundedSender<Internal>,
    sessions: HashMap<SessionId, Session>,
    journals: HashMap<PathBuf, Journal>,
}

async fn run(mut rt: Runtime, mut requests: UnboundedReceiver<Request>, mut internal: UnboundedReceiver<Internal>) {
    let mut daily = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    daily.tick().await; // the first tick is immediate; pruning already ran at start
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::Command(command)) => rt.handle(command).await,
                Some(Request::Snapshot(project, reply)) => {
                    let _ = reply.send(rt.snapshot(&project));
                }
                Some(Request::Subscribe(project, reply)) => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    rt.subscribers.push(tx);
                    let _ = reply.send((rt.snapshot(&project), rx));
                }
                Some(Request::Shutdown(reply)) => {
                    rt.stop_all_turns();
                    let _ = reply.send(());
                    return;
                }
                None => break,
            },
            Some(message) = internal.recv() => match message {
                Internal::Event(event) => rt.publish(event),
                Internal::Done(done) => rt.on_turn_done(done),
            },
            _ = daily.tick(), if rt.config.persist => rt.prune(),
        }
    }
    rt.stop_all_turns();
}

impl Runtime {
    // ── events: one way out ──

    /// Fold an event into its session, journal what became final, pass it on.
    fn publish(&mut self, event: Event) {
        if let Some(id) = event.session()
            && let Some(session) = self.sessions.get_mut(&id)
        {
            let finished = session.transcript.apply(&event);
            match &event {
                Event::UserMessage { text, .. } if session.title.is_none() => {
                    session.title = text.lines().next().map(str::to_owned);
                }
                Event::ModelKnown { model, .. } => session.model = Some(model.clone()),
                Event::TurnEnded { usage: Some(u), .. } => {
                    session.tokens += u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0);
                }
                _ => {}
            }
            if self.config.persist {
                let project = session.project.clone();
                let title = session.title.clone();
                if let Some(journal) = journal_for(&mut self.journals, &self.config.home, &project) {
                    record(journal, id, &event, &finished, title.as_deref());
                }
            }
        }
        self.subscribers.retain(|screen| screen.send(event.clone()).is_ok());
    }

    fn error(&mut self, session: Option<SessionId>, message: impl Into<String>, next: Option<&str>) {
        self.publish(Event::Error { session, message: message.into(), next: next.map(str::to_owned) });
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Start { session, spec } => self.start(session, spec).await,
            Command::Submit { session, text } => self.submit(session, text).await,
            Command::Interrupt { session } => self.interrupt(session),
            Command::Close { session } => self.close(session),
        }
    }

    // ── sessions ──

    async fn start(&mut self, id: SessionId, spec: SessionSpec) {
        if self.sessions.contains_key(&id) {
            return self.error(Some(id), format!("session {id} is already open"), None);
        }
        let engine = match spec.engine {
            Some(engine) => engine,
            None => match first_ready_engine(&self.config.store, &self.config.claude_binary).await {
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
                let cred = match codex_credentials(&self.client, &self.config.store).await {
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

        let project = project_of(&spec.cwd);
        let (cwd, sandbox) = match spec.access {
            Access::ReadOnly => (spec.cwd.clone(), None),
            Access::EditFiles => {
                let made = worktree::repo_root(&spec.cwd)
                    .and_then(|repo| worktree::create(&repo, &self.config.home, &format!("{:x}-{id:x}", now_secs())));
                match made {
                    Ok(sandbox) => (sandbox.path.clone(), Some(sandbox)),
                    Err(message) => return self.error(Some(id), message, None),
                }
            }
        };
        let workspace = sandbox.as_ref().map(workspace_of);

        if self.config.persist
            && let Some(journal) = journal_for(&mut self.journals, &self.config.home, &project)
        {
            let _ = journal.record_session(&SessionRecord {
                id,
                engine,
                model: model.clone(),
                access: spec.access,
                cwd: cwd.clone(),
                repo: sandbox.as_ref().map(|s| s.repo.clone()),
                branch: sandbox.as_ref().map(|s| s.branch.clone()),
                base: sandbox.as_ref().map(|s| s.base.clone()),
                engine_session: None,
                title: None,
                tokens: 0,
                open: true,
            });
        }
        self.sessions.insert(
            id,
            Session {
                project: project.clone(),
                engine,
                model: model.clone(),
                access: spec.access,
                cwd,
                sandbox,
                codex: codex_cred,
                history: Vec::new(),
                claude_session: None,
                prompts: Vec::new(),
                title: None,
                tokens: 0,
                transcript: Transcript::default(),
                turn: None,
            },
        );
        self.publish(Event::SessionStarted { session: id, project, engine, model, workspace });
    }

    async fn submit(&mut self, id: SessionId, text: String) {
        let Some(session) = self.sessions.get(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        if session.turn.as_ref().is_some_and(|t| !t.is_finished()) {
            return self.error(Some(id), "A turn is already running. Esc stops it.", None);
        }
        // A session resumed after a restart loads its Codex login on first use.
        if session.engine == Engine::Codex && session.codex.is_none() {
            match codex_credentials(&self.client, &self.config.store).await {
                Ok(cred) => {
                    if let Some(session) = self.sessions.get_mut(&id) {
                        session.codex = Some(cred);
                    }
                }
                Err((message, next)) => return self.error(Some(id), message, Some(next)),
            }
        }

        self.publish(Event::UserMessage { session: id, text: text.clone() });
        let internal = self.internal.clone();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.prompts.push(text.clone());
        let turn = match session.engine {
            Engine::Codex => {
                let client = self.client.clone();
                let cred = session.codex.clone().expect("loaded above");
                let model = session.model.clone().unwrap_or_default();
                let history = session.history.clone();
                let version = self.config.client_version.clone();
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(event));
                    };
                    let mut streamed = false;
                    let request = responses::TurnRequest { prompt: &text, model: &model, instructions: None, history: &history };
                    let result = responses::run_turn(&client, &cred, &version, request, |delta| {
                        streamed = true;
                        emit(Event::TextDelta { session: id, text: delta.to_owned() });
                    })
                    .await;
                    let reply = finish_turn(&emit, id, result.map_err(codex_failure), streamed);
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt: text, reply, claude_session: None }));
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
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(event));
                    };
                    let mut streamed = false;
                    let request =
                        claude_code::TurnRequest { prompt: &text, model: model.as_deref(), access, resume: resume.as_deref() };
                    let outcome = claude_code::run_turn(&binary, &cwd, request, |activity| match activity {
                        Activity::Text(text) => {
                            streamed = true;
                            emit(Event::TextDelta { session: id, text });
                        }
                        Activity::Tool { name, target } => emit(Event::ToolUse { session: id, tool: name, target }),
                        Activity::Model(model) => emit(Event::ModelKnown { session: id, model }),
                    })
                    .await;
                    let claude_session = outcome.as_ref().ok().and_then(|turn| turn.session_id.clone());
                    let result = outcome.map(|turn| turn.result).map_err(claude_failure);
                    let reply = finish_turn(&emit, id, result, streamed);
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt: text, reply, claude_session }));
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
            self.publish(Event::TurnEnded { session: id, outcome: TurnOutcome::Interrupted, usage: None });
        }
    }

    fn stop_all_turns(&mut self) {
        let running: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in running {
            self.interrupt(id);
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
        if let Some(engine_session) = done.claude_session {
            if self.config.persist {
                let project = session.project.clone();
                if let Some(journal) = journal_for(&mut self.journals, &self.config.home, &project) {
                    let _ = journal.set_engine_session(done.session, &engine_session);
                }
            }
            if let Some(session) = self.sessions.get_mut(&done.session) {
                session.claude_session = Some(engine_session);
            }
        }
    }

    fn close(&mut self, id: SessionId) {
        self.interrupt(id);
        let Some(session) = self.sessions.get(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        if let Some(sandbox) = session.sandbox.clone() {
            let subject: String = session
                .prompts
                .first()
                .or(session.title.as_ref())
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
                    self.publish(Event::BranchLanded { session: id, branch: sandbox.branch.clone(), base: sandbox.base.clone(), stat });
                }
                Ok(None) => {
                    if let Err(err) = sandbox.discard() {
                        self.error(Some(id), err, None);
                    }
                    self.publish(Event::NothingChanged { session: id });
                }
                Err(err) => self.error(
                    Some(id),
                    format!("couldn't commit the changes: {err} — they are still in {}", sandbox.path.display()),
                    None,
                ),
            }
        }
        self.publish(Event::SessionClosed { session: id });
        self.sessions.remove(&id);
    }

    // ── attach and restart ──

    fn snapshot(&self, project: &Path) -> Snapshot {
        let project = project_of(project);
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.project == project)
            .map(|(&id, s)| {
                let entries = s.transcript.entries();
                SessionInfo {
                    session: id,
                    project: s.project.clone(),
                    engine: s.engine,
                    model: s.model.clone(),
                    access: s.access,
                    workspace: s.sandbox.as_ref().map(workspace_of),
                    state: if s.turn.as_ref().is_some_and(|t| !t.is_finished()) {
                        SessionState::Working
                    } else {
                        SessionState::Ready
                    },
                    title: s.title.clone(),
                    tokens: s.tokens,
                    recent: entries[entries.len().saturating_sub(SNAPSHOT_LINES)..].to_vec(),
                }
            })
            .collect();
        sessions.sort_by_key(|s| s.session);
        Snapshot { sessions }
    }

    /// Bring back every open session from every project's journal.
    fn restore(&mut self) {
        let Ok(dirs) = std::fs::read_dir(self.config.home.join("projects")) else {
            return;
        };
        for dir in dirs.flatten() {
            let Ok(journal) = Journal::open(&dir.path().join("journal.db")) else {
                continue;
            };
            let Some(project) = journal.project() else {
                continue;
            };
            for record in journal.open_sessions().unwrap_or_default() {
                let conversation = journal.conversation(record.id).unwrap_or_default();
                let sandbox = match (&record.repo, &record.branch, &record.base) {
                    (Some(repo), Some(branch), Some(base)) if record.access == Access::EditFiles => {
                        if !record.cwd.exists() {
                            // The worktree is gone (removed by hand): nothing left to resume.
                            let _ = journal.append(record.id, &Entry::Note { text: "Its worktree was removed; closed.".into() });
                            let _ = journal.close_session(record.id);
                            continue;
                        }
                        Some(Sandbox { repo: repo.clone(), path: record.cwd.clone(), branch: branch.clone(), base: base.clone() })
                    }
                    _ => None,
                };
                let mut history = Vec::new();
                let mut prompts = Vec::new();
                for entry in &conversation {
                    match entry {
                        Entry::User { text } => {
                            prompts.push(text.clone());
                            history.push(responses::Message { role: responses::Role::User, text: text.clone() });
                        }
                        Entry::Reply { text } => {
                            history.push(responses::Message { role: responses::Role::Assistant, text: text.clone() })
                        }
                        _ => {}
                    }
                }
                self.sessions.insert(
                    record.id,
                    Session {
                        project: project.clone(),
                        engine: record.engine,
                        model: record.model,
                        access: record.access,
                        cwd: record.cwd,
                        sandbox,
                        codex: None,
                        history,
                        claude_session: record.engine_session,
                        prompts,
                        title: record.title,
                        tokens: record.tokens,
                        transcript: Transcript::from_entries(conversation),
                        turn: None,
                    },
                );
            }
            self.journals.insert(project, journal);
        }
    }

    /// Prune conversations past the retention window in every journal.
    fn prune(&mut self) {
        let cutoff = now_secs() as i64 - (self.config.retention_days as i64) * 86_400;
        for journal in self.journals.values() {
            let _ = journal.prune(cutoff);
        }
    }
}

// ── engines ──
//
// Free functions over the parts they need: a future that held `&Runtime` across an
// await would require the journals' SQLite connections to be shareable across
// threads, which they are not.

/// Codex if signed in, else Claude Code if signed in.
async fn first_ready_engine(store: &CredentialStore, claude_binary: &str) -> Option<Engine> {
    if matches!(store.load("openai"), Ok(Some(_))) {
        return Some(Engine::Codex);
    }
    match claude_code::auth_status(claude_binary).await {
        Ok(Some(status)) if status.logged_in => Some(Engine::Claude),
        _ => None,
    }
}

/// The Codex login, refreshed if stale — or why there isn't one.
async fn codex_credentials(client: &reqwest::Client, store: &CredentialStore) -> Result<Credentials, (String, &'static str)> {
    let cred = store
        .load("openai")
        .map_err(|err| (format!("couldn't read {}: {err}", store.path("openai").display()), "equipe login codex"))?
        .filter(|c| c.method == AuthMethod::OauthSubscription)
        .ok_or(("Codex isn't signed in.".to_owned(), "equipe login codex"))?;
    codex::ensure_fresh(client, store, cred)
        .await
        .map_err(|err| (format!("couldn't refresh the Codex login: {err}"), "equipe login codex"))
}

/// The journal for `project`, opened on first use.
fn journal_for<'a>(journals: &'a mut HashMap<PathBuf, Journal>, home: &Path, project: &Path) -> Option<&'a Journal> {
    if !journals.contains_key(project) {
        let journal = Journal::for_project(home, project).ok()?;
        journals.insert(project.to_owned(), journal);
    }
    journals.get(project)
}

/// Write what an event made final, and what it changed about the session.
fn record(journal: &Journal, id: SessionId, event: &Event, finished: &[Entry], title: Option<&str>) {
    for entry in finished {
        let _ = journal.append(id, entry);
    }
    match event {
        Event::UserMessage { .. } => {
            if let Some(title) = title {
                let _ = journal.set_title_if_empty(id, title);
            }
        }
        Event::ModelKnown { model, .. } => {
            let _ = journal.set_model(id, model);
        }
        Event::TurnEnded { usage: Some(u), .. } => {
            let _ = journal.add_tokens(id, u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0));
        }
        Event::BranchLanded { branch, .. } => {
            let _ = journal.set_outcome(id, &format!("landed {branch}"));
        }
        Event::NothingChanged { .. } => {
            let _ = journal.set_outcome(id, "no changes");
        }
        Event::SessionClosed { .. } => {
            let _ = journal.close_session(id);
        }
        _ => {}
    }
}

fn workspace_of(sandbox: &Sandbox) -> Workspace {
    Workspace { branch: sandbox.branch.clone(), base: sandbox.base.clone(), path: sandbox.path.clone() }
}

/// Report a finished turn. Returns the reply text for the conversation history.
fn finish_turn(
    emit: &impl Fn(Event),
    id: SessionId,
    result: Result<TurnResult, (String, Option<&'static str>)>,
    streamed: bool,
) -> Option<String> {
    match result {
        Ok(result) => {
            // A turn that sent only a terminal event streamed nothing; send it whole
            // so screens never have to special-case it.
            if !streamed && !result.text.is_empty() {
                emit(Event::TextDelta { session: id, text: result.text.clone() });
            }
            let outcome = if result.status == "completed" { TurnOutcome::Completed } else { TurnOutcome::Incomplete };
            let usage = result.usage.map(|u| Usage { input_tokens: u.input_tokens, output_tokens: u.output_tokens });
            emit(Event::TurnEnded { session: id, outcome, usage });
            Some(result.text)
        }
        Err((message, next)) => {
            emit(Event::Error { session: Some(id), message, next: next.map(str::to_owned) });
            emit(Event::TurnEnded { session: id, outcome: TurnOutcome::Failed, usage: None });
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
