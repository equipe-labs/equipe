//! The brain: runs sessions on engines, driven by [`Command`]s, reporting
//! [`Event`]s.
//!
//! Screens talk to it only through a [`Handle`] and the event receiver from
//! [`start`]. All state lives here: the squads, their members and boards, each
//! session's engine, model and conversation, and the worktree an editing member
//! works in. Every event passes through one loop, which folds it into the
//! session's transcript and, when the runtime is persistent, journals it; a
//! persistent runtime resumes its squads when it starts again.
//!
//! A **member** is a long-lived session with a role in a squad. At its desk it
//! reads and answers; given a ticket, its turns run in that ticket's project —
//! in a worktree of its own when it edits. Who gets which ticket is decided by
//! equipe's rules ([`crate::board::assignments`]), never by a model.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_protocol::rpc::{ToolCall, ToolResult};
use equipe_protocol::{
    Access, Approval, Command, Decision, Engine, Entry, Event, Member, Mode, SessionId, SessionInfo, SessionSpec,
    SessionState, Snapshot, Ticket, TicketId, TicketStatus, Transcript, TurnOutcome, Usage, WorkInfo, Workspace,
};
use equipe_providers::claude_code::{self, Activity};
use equipe_providers::credentials::equipe_home;
use equipe_providers::openai::{codex, responses};
use equipe_providers::turn::TurnResult;
use equipe_providers::{AuthMethod, CredentialStore, Credentials};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::board::{self, Candidate};
use crate::journal::{self, Journal, SessionRecord, TicketWork};
use crate::permissions;
use crate::roles::{self, Role, Teammate};
use crate::worktree::{self, Sandbox};

/// How many recent lines a snapshot carries per session.
const SNAPSHOT_LINES: usize = 200;

/// How much of a member's answer the PM is told.
const BRIEF_REPLY_CHARS: usize = 400;

/// Closed tickets a snapshot still shows, newest first.
const CLOSED_TICKETS_SHOWN: usize = 20;

/// The MCP server equipe gives members; its tools are `mcp__equipe__*`.
const MCP_SERVER: &str = "equipe";

/// How long a member waits for the person to answer a request for permission
/// before it is declined.
const APPROVAL_WAIT: Duration = Duration::from_secs(30 * 60);

pub struct Config {
    pub store: CredentialStore,
    /// equipe's home: squads, worktrees and logs live under it.
    pub home: PathBuf,
    /// Reported to backends as equipe's version.
    pub client_version: String,
    /// The Claude Code executable to run.
    pub claude_binary: String,
    /// Journal conversations and resume squads at start (the daemon).
    pub persist: bool,
    /// Conversations older than this are pruned to the session record.
    pub retention_days: u64,
    /// The equipe executable, run as `equipe mcp` to give Claude Code members
    /// their board tools. `None`: members get no tools.
    pub equipe_binary: Option<PathBuf>,
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
            equipe_binary: std::env::current_exe().ok(),
        }
    }

    /// Journal sessions and resume them across restarts.
    pub fn persistent(mut self) -> Self {
        self.persist = true;
        self
    }
}

/// A new screen's view: what's going on now, and every event after that.
pub type Subscription = (Snapshot, UnboundedReceiver<Event>);

enum Request {
    Command(Command),
    Snapshot(String, oneshot::Sender<Snapshot>),
    Subscribe(String, oneshot::Sender<Subscription>),
    Tool(ToolCall, oneshot::Sender<ToolResult>),
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

    /// Everything `squad` has going, for a screen that attaches now.
    pub async fn snapshot(&self, squad: &str) -> Snapshot {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Snapshot(squad.to_owned(), tx));
        rx.await.unwrap_or_default()
    }

    /// Attach a screen: a snapshot of `squad` and its events from then on. Both
    /// are taken in one step of the runtime loop, so no event falls between them
    /// and none is counted twice.
    pub async fn subscribe(&self, squad: &str) -> Option<Subscription> {
        let (tx, rx) = oneshot::channel();
        self.requests.send(Request::Subscribe(squad.to_owned(), tx)).ok()?;
        rx.await.ok()
    }

    /// A member's board tool call.
    pub async fn tool(&self, call: ToolCall) -> ToolResult {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Tool(call, tx));
        rx.await.unwrap_or(ToolResult { text: "equipe is shutting down".into(), is_error: true })
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
        subscribers: vec![(None, evt_tx)],
        internal: internal_tx,
        sessions: HashMap::new(),
        journals: HashMap::new(),
        next_join: 1,
        pending: HashMap::new(),
        next_approval: 1,
    };
    if runtime.config.persist {
        runtime.restore();
        runtime.prune();
        runtime.resume_work();
    }
    tokio::spawn(run(runtime, req_rx, internal_rx));
    (Handle { requests: req_tx }, evt_rx)
}

/// The project a directory belongs to: its repository root, or itself.
pub fn project_of(dir: &Path) -> PathBuf {
    worktree::repo_root(dir).unwrap_or_else(|_| dir.canonicalize().unwrap_or_else(|_| dir.to_owned()))
}

/// A member's current ticket: where its turns run until the ticket is put down.
struct Work {
    ticket: TicketId,
    project: PathBuf,
    cwd: PathBuf,
    /// For a member that edits: its worktree on the ticket's branch.
    sandbox: Option<Sandbox>,
    /// The ticket's own conversation (Claude Code `--resume`).
    claude_session: Option<String>,
    history: Vec<responses::Message>,
    /// Set by `submit_for_review`; the work is handed in when the turn ends.
    handed_in: Option<String>,
}

struct Session {
    squad: Option<String>,
    /// For a member, the project its desk is in; otherwise where it was opened.
    project: PathBuf,
    /// Who the session is on the team; `None` for a plain agent.
    member: Option<Member>,
    /// When it joined, for a stable team order (ids are random).
    joined: u64,
    engine: Engine,
    model: Option<String>,
    access: Access,
    /// Where desk turns run — and, for a plain editing session, its worktree.
    cwd: PathBuf,
    sandbox: Option<Sandbox>,
    /// Loaded when the first Codex turn needs it.
    codex: Option<Credentials>,
    /// Codex keeps nothing server-side; the conversation travels with each turn.
    history: Vec<responses::Message>,
    /// Claude Code keeps the conversation; this id continues it.
    claude_session: Option<String>,
    work: Option<Work>,
    prompts: Vec<String>,
    title: Option<String>,
    tokens: u64,
    transcript: Transcript,
    turn: Option<JoinHandle<()>>,
    /// Memos left while a turn ran, delivered together when it ends.
    queued: Vec<String>,
    /// For the PM: what happened with the team since the PM's last turn. It
    /// travels with the PM's next message, so the PM keeps up without spending a
    /// turn on every exchange.
    briefing: Vec<String>,
    /// Where a member's engine keeps what it learns (Claude Code's auto memory).
    memory: Option<PathBuf>,
    /// Its own mode; `None` follows the squad's.
    mode: Option<Mode>,
}

impl Session {
    fn is_working(&self) -> bool {
        self.turn.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn is_pm(&self) -> bool {
        self.member.as_ref().is_some_and(|m| m.role == roles::PM)
    }

    fn name(&self) -> Option<&str> {
        self.member.as_ref().map(|m| m.name.as_str())
    }

    fn role(&self) -> Option<&'static Role> {
        self.member.as_ref().map(|m| roles::role_or_custom(&m.role))
    }

    fn edits(&self) -> bool {
        self.role().is_some_and(|r| r.edits)
    }

    /// Free for a new ticket: not on one, not mid-turn.
    fn is_free(&self) -> bool {
        self.work.is_none() && !self.is_working()
    }
}

/// What a finished turn hands back to the loop.
struct TurnDone {
    session: SessionId,
    prompt: String,
    reply: Option<String>,
    claude_session: Option<String>,
    /// The ticket the turn ran for, if any.
    ticket: Option<TicketId>,
}

enum Internal {
    Event(Event),
    Done(TurnDone),
    /// Nobody answered a request for permission in time.
    ApprovalTimedOut(u64),
}

/// A request for permission waiting for the person, and the engine waiting on it.
struct Pending {
    squad: String,
    approval: Approval,
    /// The tool use as the engine asked for it, handed back when allowed.
    input: Value,
    reply: oneshot::Sender<ToolResult>,
}

/// How a member puts a ticket down.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkEnd {
    /// It called `submit_for_review`: the person reviews it next.
    HandedIn,
    Accepted,
    Cancelled,
    /// The member left the squad; the ticket goes back on the board.
    Left,
}

struct Runtime {
    config: Config,
    client: reqwest::Client,
    /// Every attached screen, with the squad it follows (`None`: everything).
    /// Closed ones drop out on the next event.
    subscribers: Vec<(Option<String>, UnboundedSender<Event>)>,
    internal: UnboundedSender<Internal>,
    sessions: HashMap<SessionId, Session>,
    /// One journal per squad, opened on first use.
    journals: HashMap<String, Journal>,
    /// The next session's place in its team.
    next_join: u64,
    /// Requests for permission waiting for the person, by id.
    pending: HashMap<u64, Pending>,
    next_approval: u64,
}

async fn run(mut rt: Runtime, mut requests: UnboundedReceiver<Request>, mut internal: UnboundedReceiver<Internal>) {
    let mut daily = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    daily.tick().await; // the first tick is immediate; pruning already ran at start
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::Command(command)) => rt.handle(command).await,
                Some(Request::Snapshot(squad, reply)) => {
                    let _ = reply.send(rt.snapshot(&squad));
                }
                Some(Request::Subscribe(squad, reply)) => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    rt.subscribers.push((Some(squad.clone()), tx));
                    let _ = reply.send((rt.snapshot(&squad), rx));
                }
                Some(Request::Tool(call, reply)) => rt.tool(call, reply).await,
                Some(Request::Shutdown(reply)) => {
                    rt.stop_all_turns();
                    let _ = reply.send(());
                    return;
                }
                None => break,
            },
            Some(message) = internal.recv() => match message {
                Internal::Event(event) => rt.publish(event),
                Internal::Done(done) => rt.on_turn_done(done).await,
                Internal::ApprovalTimedOut(id) => rt.answer(id, Decision::Deny, "Nobody answered in time."),
            },
            _ = daily.tick(), if rt.config.persist => rt.prune(),
        }
    }
    rt.stop_all_turns();
}

impl Runtime {
    // ── events: one way out ──

    fn publish(&mut self, event: Event) {
        self.publish_in(None, event);
    }

    /// Fold an event into its session, journal what became final, and pass it to
    /// the screens following its squad. `squad` names the squad of an event that
    /// has no session to say it (a failed start, a board error).
    fn publish_in(&mut self, squad: Option<&str>, event: Event) {
        let mut squad = squad.map(str::to_owned);
        if let Some(id) = event.session()
            && let Some(session) = self.sessions.get_mut(&id)
        {
            squad = session.squad.clone();
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
            if self.config.persist
                && let Some(name) = squad.clone()
            {
                let title = session.title.clone();
                if let Some(journal) = journal_for(&mut self.journals, &self.config.home, &name) {
                    record(journal, id, &event, &finished, title.as_deref());
                }
            }
        }
        if let Some(name) = event.squad() {
            squad = Some(name.to_owned());
        }
        self.subscribers.retain(|(follows, screen)| match follows {
            Some(follows) if squad.as_ref() != Some(follows) => !screen.is_closed(),
            _ => screen.send(event.clone()).is_ok(),
        });
    }

    fn error(&mut self, session: Option<SessionId>, message: impl Into<String>, next: Option<&str>) {
        self.publish(Event::Error { session, message: message.into(), next: next.map(str::to_owned) });
    }

    /// An error for the screens of `squad`, about something with no session of its own.
    fn squad_error(&mut self, squad: &str, message: impl Into<String>) {
        self.publish_in(Some(squad), Event::Error { session: None, message: message.into(), next: None });
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Start { session, spec } => self.start(session, spec).await,
            Command::Submit { session, text } => self.submit(session, text).await,
            Command::Interrupt { session } => self.interrupt(session).await,
            Command::Close { session } => self.close(session).await,
            Command::Accept { squad, ticket } => self.accept(&squad, ticket).await,
            Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,
            Command::Cancel { squad, ticket } => self.cancel(&squad, ticket).await,
            Command::SetMode { squad, member, mode } => self.set_mode(&squad, member, mode),
            Command::Decide { squad, approval, decision } => {
                if self.pending.get(&approval).is_some_and(|p| p.squad == squad) {
                    self.answer(approval, decision, "");
                }
            }
        }
    }

    // ── squads ──

    fn journal(&mut self, squad: &str) -> Option<&Journal> {
        journal_for(&mut self.journals, &self.config.home, squad)
    }

    fn projects(&mut self, squad: &str) -> Vec<PathBuf> {
        self.journal(squad).and_then(|j| j.projects().ok()).unwrap_or_default()
    }

    /// The squad's projects from a journal already open.
    fn projects_cached(&self, squad: &str) -> Vec<PathBuf> {
        self.journals.get(squad).and_then(|j| j.projects().ok()).unwrap_or_default()
    }

    fn tickets(&mut self, squad: &str) -> Vec<Ticket> {
        self.journal(squad).and_then(|j| j.tickets().ok()).unwrap_or_default()
    }

    /// The member of `squad` called `name`.
    fn member_id(&self, squad: &str, name: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.name() == Some(name))
            .map(|(&id, _)| id)
    }

    /// `role`, or `role-2`, `role-3`… — whichever no member of `squad` has.
    fn free_name(&self, squad: &str, role: &str) -> String {
        (1..)
            .map(|n| if n == 1 { role.to_owned() } else { format!("{role}-{n}") })
            .find(|name| self.member_id(squad, name).is_none())
            .expect("some name is free")
    }

    /// The members of `squad`, in the order they joined: `(joined, name, role)`.
    fn team(&self, squad: &str) -> Vec<(u64, String, String)> {
        let mut members: Vec<(u64, String, String)> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad))
            .filter_map(|s| s.member.as_ref().map(|m| (s.joined, m.name.clone(), m.role.clone())))
            .collect();
        members.sort();
        members
    }

    fn pm_of(&self, squad: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.is_pm())
            .map(|(&id, _)| id)
    }

    /// Tell `squad`'s PM `line` with its next message; with `shown`, its
    /// conversation also shows what happened, so a screen on the PM sees it.
    fn brief_pm(&mut self, squad: &str, line: String, shown: Option<String>) {
        let Some(pm) = self.pm_of(squad) else {
            return;
        };
        if let Some(session) = self.sessions.get_mut(&pm) {
            session.briefing.push(line);
        }
        if let Some(text) = shown {
            self.publish(Event::Notice { session: pm, text });
        }
    }

    /// When `id` is a member other than the PM, brief its PM with `line(name)`.
    fn tell_the_pm(&mut self, id: SessionId, line: impl FnOnce(&str) -> String, shown: Option<&str>) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (Some(name), Some(squad), false) = (session.name().map(str::to_owned), session.squad.clone(), session.is_pm())
        else {
            return;
        };
        let shown = shown.map(|text| {
            let first = text.lines().next().unwrap_or_default();
            let short: String = first.chars().take(120).collect();
            let more = if short.len() < text.len() { "…" } else { "" };
            format!("↪ You → {name}: {short}{more}")
        });
        self.brief_pm(&squad, line(&name), shown);
    }

    // ── sessions ──

    async fn start(&mut self, id: SessionId, spec: SessionSpec) {
        if self.sessions.contains_key(&id) {
            return self.error(Some(id), format!("session {id} is already open"), None);
        }
        let squad = spec.squad.clone();
        let fail = |rt: &mut Self, message: String, next: Option<&str>| {
            rt.publish_in(squad.as_deref(), Event::Error { session: Some(id), message, next: next.map(str::to_owned) });
        };
        let role = spec.role.as_deref().map(roles::role_or_custom);
        let projects = match (&squad, role) {
            (Some(squad), _) => {
                let projects = self.projects(squad);
                if projects.is_empty() {
                    return fail(self, format!("There is no squad called {squad}."), Some("equipe"));
                }
                projects
            }
            (None, Some(_)) => return fail(self, "A member joins a squad; name one.".into(), None),
            (None, None) => Vec::new(),
        };

        let engine = match spec.engine {
            Some(engine) => engine,
            None => {
                let preference = role.map_or([Engine::Codex, Engine::Claude], roles::engine_preference);
                match first_ready_engine(&preference, &self.config.store, &self.config.claude_binary).await {
                    Some(engine) => engine,
                    None => {
                        let message = "No engine is ready. Sign in to Codex or Claude Code first.".to_owned();
                        return fail(self, message, Some("equipe login"));
                    }
                }
            }
        };
        if spec.access == Access::EditFiles && engine == Engine::Codex {
            return fail(
                self,
                "Editing runs on Claude Code for now; Codex edits arrive with its ACP engine.".into(),
                Some("equipe try --edit --engine claude \"…\""),
            );
        }

        let mut model = spec.model.clone();
        let mut codex_cred = None;
        match engine {
            Engine::Codex => {
                let cred = match codex_credentials(&self.client, &self.config.store).await {
                    Ok(cred) => cred,
                    Err((message, next)) => return fail(self, message, Some(next)),
                };
                if model.is_none() {
                    // Ask the backend what this account has rather than hardcoding an id.
                    let catalog = match codex::list_models(&self.client, &cred, &self.config.client_version).await {
                        Ok(models) => models,
                        Err(err) => return fail(self, err.to_string(), Some("equipe status")),
                    };
                    model = match role {
                        Some(role) => roles::recommended_model(role.tier, engine, &catalog),
                        None => catalog.into_iter().next(),
                    };
                    if model.is_none() {
                        return fail(self, "The backend offered no models.".into(), Some("equipe status"));
                    }
                }
                codex_cred = Some(cred);
            }
            Engine::Claude => {
                match claude_code::auth_status(&self.config.claude_binary).await {
                    Ok(Some(status)) if status.logged_in => {}
                    Ok(Some(_)) => return fail(self, "Claude Code isn't signed in.".into(), Some("equipe login claude")),
                    Ok(None) => {
                        return fail(
                            self,
                            "Claude Code isn't installed (no `claude` on PATH).".into(),
                            Some("see https://code.claude.com/docs/en/setup"),
                        );
                    }
                    Err(err) => return fail(self, err.to_string(), None),
                }
                if model.is_none()
                    && let Some(role) = role
                {
                    model = roles::recommended_model(role.tier, engine, &[]);
                }
            }
        }

        let opened_in = project_of(&spec.cwd);
        let member = match (&squad, spec.role.as_deref()) {
            (Some(squad), Some(role)) => {
                let role = if role == "po" { roles::PM } else { role };
                Some(Member { role: role.to_owned(), name: self.free_name(squad, role) })
            }
            _ => None,
        };
        // A member's desk is the project it was added from, when that is the squad's.
        let project = if member.is_some() && !projects.contains(&opened_in) { projects[0].clone() } else { opened_in };
        let (cwd, sandbox, access) = match (&member, spec.access) {
            // At its desk a member reads; it edits only on a ticket, in a worktree.
            (Some(_), _) => {
                let access = if role.is_some_and(|r| r.edits) { Access::EditFiles } else { Access::ReadOnly };
                (project.clone(), None, access)
            }
            (None, Access::ReadOnly) => (spec.cwd.clone(), None, Access::ReadOnly),
            (None, Access::EditFiles) => {
                let made = worktree::repo_root(&spec.cwd).and_then(|repo| {
                    worktree::create(&repo, &self.config.home, &format!("{:x}-{id:x}", now_secs()), "try")
                });
                match made {
                    Ok(sandbox) => (sandbox.path.clone(), Some(sandbox), Access::EditFiles),
                    Err(message) => return fail(self, message, None),
                }
            }
        };
        let workspace = sandbox.as_ref().map(workspace_of);
        let memory = match (&squad, &member) {
            (Some(squad), Some(m)) => Some(journal::member_memory_dir(&self.config.home, squad, &m.name)),
            _ => None,
        };

        if self.config.persist
            && let Some(squad) = &squad
            && let Some(journal) = journal_for(&mut self.journals, &self.config.home, squad)
        {
            let _ = journal.record_session(&SessionRecord {
                id,
                member: member.clone(),
                mode: None,
                engine,
                model: model.clone(),
                access,
                cwd: cwd.clone(),
                repo: None,
                branch: None,
                base: None,
                engine_session: None,
                title: None,
                tokens: 0,
                open: true,
            });
        }
        let joined = self.next_join;
        self.next_join += 1;
        self.sessions.insert(
            id,
            Session {
                squad: squad.clone(),
                project: project.clone(),
                member: member.clone(),
                joined,
                engine,
                model: model.clone(),
                access,
                cwd,
                sandbox,
                codex: codex_cred,
                history: Vec::new(),
                claude_session: None,
                work: None,
                prompts: Vec::new(),
                title: None,
                tokens: 0,
                transcript: Transcript::default(),
                turn: None,
                queued: Vec::new(),
                briefing: Vec::new(),
                memory,
                mode: None,
            },
        );
        self.publish(Event::SessionStarted { session: id, project, engine, model, workspace, member, squad: squad.clone() });
        // A new member may be just who a waiting ticket needs.
        if let Some(squad) = squad {
            self.dispatch(&squad).await;
        }
    }

    /// Load a Codex session's login on first use (after a restart, say).
    async fn ensure_codex(&mut self, id: SessionId) -> bool {
        let Some(session) = self.sessions.get(&id) else {
            return false;
        };
        if session.engine != Engine::Codex || session.codex.is_some() {
            return true;
        }
        match codex_credentials(&self.client, &self.config.store).await {
            Ok(cred) => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.codex = Some(cred);
                }
                true
            }
            Err((message, next)) => {
                self.error(Some(id), message, Some(next));
                false
            }
        }
    }

    async fn submit(&mut self, id: SessionId, text: String) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        if session.is_working() {
            // A memo: it waits for the turn to end rather than cutting in.
            session.queued.push(text.clone());
            return self.publish(Event::MessageQueued { session: id, text });
        }
        if !self.ensure_codex(id).await {
            return;
        }
        self.publish(Event::UserMessage { session: id, text: text.clone() });
        // What the person writes in is what the squad writes for them.
        if let (Some(squad), Some(language)) = (self.sessions.get(&id).and_then(|s| s.squad.clone()), crate::language::detect(&text))
            && let Some(journal) = self.journal(&squad)
            && journal.language().as_deref() != Some(language)
        {
            let _ = journal.set_language(language);
        }
        self.tell_the_pm(id, |name| format!("The person told {name}: {text}"), Some(&text));
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.prompts.push(text.clone());
        // What the PM missed travels with its next message; the person sees only theirs.
        let prompt = match std::mem::take(&mut session.briefing) {
            briefing if briefing.is_empty() => text,
            briefing => format!(
                "[equipe: since your last turn —\n{}]\n\n{text}",
                briefing.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")
            ),
        };
        self.start_turn(id, prompt);
    }

    /// Run a turn with `prompt` in the session's current place: its ticket's
    /// project when it has one, its desk otherwise.
    fn start_turn(&mut self, id: SessionId, prompt: String) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let squad = session.squad.clone();
        let projects = squad.as_deref().map(|s| self.projects_cached(s)).unwrap_or_default();
        let board_tools = session.member.is_some() && session.engine == Engine::Claude && self.config.equipe_binary.is_some();
        let language = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(Journal::language);
        let charter = session.member.as_ref().map(|m| {
            let team = squad.as_deref().map(|s| self.team(s)).unwrap_or_default();
            let mates: Vec<Teammate> = team.iter().map(|(_, name, role)| Teammate { name, role }).collect();
            let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
            let ctx = roles::Context { team: &mates, projects: &names, board_tools, language: language.as_deref() };
            roles::charter(roles::role_or_custom(&m.role), &m.name, &ctx)
        });
        let mcp_config = match (&squad, &session.member, &self.config.equipe_binary) {
            (Some(squad), Some(m), Some(binary)) if board_tools => Some(
                json!({ "mcpServers": { MCP_SERVER: {
                    "type": "stdio",
                    "command": binary,
                    "args": ["mcp", "--squad", squad, "--member", m.name, "--role", m.role, "--home", self.config.home],
                }}})
                .to_string(),
            ),
            _ => None,
        };
        let edits = session.edits();
        let mode = session.mode.or_else(|| squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::mode)).unwrap_or_default();
        let (cwd, access, resume, history, ticket, add_dirs) = match &session.work {
            Some(work) => {
                (work.cwd.clone(), access_for(mode, edits && work.sandbox.is_some()), work.claude_session.clone(), work.history.clone(), Some(work.ticket), Vec::new())
            }
            None => {
                // At its desk a member reads every project of its squad; it writes nowhere.
                let access = if session.member.is_some() { claude_code::Access::ReadOnly } else { plain_access(session.access) };
                let others: Vec<PathBuf> = projects.iter().filter(|p| **p != session.cwd).cloned().collect();
                (session.cwd.clone(), access, session.claude_session.clone(), session.history.clone(), None, others)
            }
        };
        // A member on a ticket, in Auto, is fenced in by the operating system; what it
        // can't do inside the fence, and anything Ask needs, goes to the person.
        let member_rules = session.member.is_some();
        let fenced = member_rules && access == claude_code::Access::EditFiles && mode == Mode::Auto;
        let prompt_tool = (mcp_config.is_some() && matches!(access, claude_code::Access::EditFiles | claude_code::Access::Ask))
            .then(|| format!("mcp__{MCP_SERVER}__{}", permissions::APPROVE_TOOL));
        let squad_rules = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(|j| j.allowed().ok()).unwrap_or_default();
        let internal = self.internal.clone();
        let turn = match session.engine {
            Engine::Codex => {
                let client = self.client.clone();
                let Some(cred) = session.codex.clone() else {
                    return self.error(Some(id), "Codex isn't signed in.", Some("equipe login codex"));
                };
                let model = session.model.clone().unwrap_or_default();
                let version = self.config.client_version.clone();
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(event));
                    };
                    let mut streamed = false;
                    let request = responses::TurnRequest {
                        prompt: &prompt,
                        model: &model,
                        instructions: charter.as_deref(),
                        history: &history,
                    };
                    let result = responses::run_turn(&client, &cred, &version, request, |delta| {
                        streamed = true;
                        emit(Event::TextDelta { session: id, text: delta.to_owned() });
                    })
                    .await;
                    let reply = finish_turn(&emit, id, result.map_err(codex_failure), streamed);
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply, claude_session: None, ticket }));
                })
            }
            Engine::Claude => {
                let binary = self.config.claude_binary.clone();
                let model = session.model.clone();
                let memory = session.memory.clone();
                let mut allowed: Vec<String> = mcp_config.iter().map(|_| format!("mcp__{MCP_SERVER}")).collect();
                if member_rules {
                    allowed.extend(squad_rules);
                }
                let blocked: Vec<String> =
                    if member_rules { permissions::BLOCKED.iter().map(|r| (*r).to_owned()).collect() } else { Vec::new() };
                let domains: Vec<String> = permissions::ALLOWED_DOMAINS.iter().map(|d| (*d).to_owned()).collect();
                let outward: Vec<String> = permissions::OUTWARD.iter().map(|r| (*r).to_owned()).collect();
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(event));
                    };
                    let mut streamed = false;
                    let request = claude_code::TurnRequest {
                        prompt: &prompt,
                        model: model.as_deref(),
                        access,
                        resume: resume.as_deref(),
                        charter: charter.as_deref(),
                        memory: memory.as_deref(),
                        add_dirs: &add_dirs,
                        mcp_config: mcp_config.as_deref(),
                        allowed_tools: &allowed,
                        disallowed_tools: &blocked,
                        sandbox: fenced.then_some(claude_code::Sandbox { allowed_domains: &domains, ask: &outward }),
                        prompt_tool: prompt_tool.as_deref(),
                    };
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
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply, claude_session, ticket }));
                })
            }
        };
        if let Some(session) = self.sessions.get_mut(&id) {
            session.turn = Some(turn);
        }
    }

    /// Stop the turn in progress (Esc). Memos left during it are delivered at once.
    async fn interrupt(&mut self, id: SessionId) {
        if self.stop_turn(id) {
            self.after_turn(id).await;
        }
    }

    /// Stop the turn in progress, if any. Returns whether one was stopped.
    fn stop_turn(&mut self, id: SessionId) -> bool {
        let Some(session) = self.sessions.get_mut(&id) else {
            return false;
        };
        // A turn that already finished has its result on the way; let it land.
        let Some(turn) = session.turn.take_if(|t| !t.is_finished()) else {
            return false;
        };
        // Dropping the turn kills the engine process with it.
        turn.abort();
        self.publish(Event::TurnEnded { session: id, outcome: TurnOutcome::Interrupted, usage: None });
        // Nobody is waiting for those answers any more.
        let asked: Vec<u64> = self.pending.iter().filter(|(_, p)| p.approval.session == id).map(|(&a, _)| a).collect();
        for approval in asked {
            self.answer(approval, Decision::Deny, "The turn was stopped.");
        }
        true
    }

    fn stop_all_turns(&mut self) {
        let running: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in running {
            self.stop_turn(id);
        }
    }

    async fn on_turn_done(&mut self, done: TurnDone) {
        let persist = self.config.persist;
        let Some(session) = self.sessions.get_mut(&done.session) else {
            return;
        };
        session.turn = None;
        let squad = session.squad.clone();
        let in_work = session.work.as_ref().is_some_and(|w| Some(w.ticket) == done.ticket);
        if let Some(reply) = &done.reply
            && session.engine == Engine::Codex
        {
            let history = match &mut session.work {
                Some(work) if in_work => &mut work.history,
                _ => &mut session.history,
            };
            history.push(responses::Message { role: responses::Role::User, text: done.prompt.clone() });
            history.push(responses::Message { role: responses::Role::Assistant, text: reply.clone() });
        }
        if let Some(engine_session) = done.claude_session {
            match &mut session.work {
                Some(work) if in_work => work.claude_session = Some(engine_session.clone()),
                _ => session.claude_session = Some(engine_session.clone()),
            }
            if let Some(squad) = &squad
                && let Some(journal) = journal_for(&mut self.journals, &self.config.home, squad)
            {
                match done.ticket.filter(|_| in_work) {
                    Some(ticket) => {
                        let mut work = journal.ticket_work(ticket).unwrap_or_default();
                        work.engine_session = Some(engine_session);
                        let _ = journal.set_ticket_work(ticket, &work);
                    }
                    None if persist => {
                        let _ = journal.set_engine_session(done.session, &engine_session);
                    }
                    None => {}
                }
            }
        }
        if let Some(reply) = done.reply.filter(|r| !r.trim().is_empty()) {
            let short: String = reply.chars().take(BRIEF_REPLY_CHARS).collect();
            let more = if short.len() < reply.len() { "…" } else { "" };
            self.tell_the_pm(done.session, |name| format!("{name} answered: {short}{more}"), None);
        }
        self.after_turn(done.session).await;
    }

    /// After a turn, finished or stopped: hand in work that was submitted,
    /// deliver memos, and let the rules hand out what is ready.
    async fn after_turn(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let squad = session.squad.clone();
        if session.work.as_ref().is_some_and(|w| w.handed_in.is_some()) {
            self.end_work(id, WorkEnd::HandedIn);
        }
        if let Some(session) = self.sessions.get_mut(&id)
            && !session.queued.is_empty()
        {
            let text = std::mem::take(&mut session.queued).join("\n\n");
            Box::pin(self.submit(id, text)).await;
        }
        if let Some(squad) = squad {
            Box::pin(self.dispatch(&squad)).await;
        }
    }

    async fn close(&mut self, id: SessionId) {
        self.stop_turn(id);
        let Some(session) = self.sessions.get(&id) else {
            return self.error(Some(id), format!("no open session {id}"), None);
        };
        let squad = session.squad.clone();
        if session.work.is_some() {
            self.end_work(id, WorkEnd::Left);
        }
        if let Some(sandbox) = self.sessions.get(&id).and_then(|s| s.sandbox.clone()) {
            self.land_plain_session(id, &sandbox);
        }
        self.publish(Event::SessionClosed { session: id });
        self.sessions.remove(&id);
        // Someone else may take over what it left.
        if let Some(squad) = squad {
            self.dispatch(&squad).await;
        }
    }

    /// A plain editing session (`equipe try --edit`) commits to its branch as it closes.
    fn land_plain_session(&mut self, id: SessionId, sandbox: &Sandbox) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
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

    // ── the board: equipe's rules ──

    /// Hand every ready ticket to a free member who owns it.
    async fn dispatch(&mut self, squad: &str) {
        let tickets = self.tickets(squad);
        let single_project = self.projects(squad).len() == 1;
        let members: Vec<(String, String, bool)> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad))
            .filter_map(|s| s.member.as_ref().map(|m| (m.name.clone(), m.role.clone(), s.is_free())))
            .collect();
        let candidates: Vec<Candidate> =
            members.iter().map(|(name, role, free)| Candidate { name, role, free: *free }).collect();
        for (ticket, name) in board::assignments(&tickets, &candidates, single_project) {
            if let Some(id) = self.member_id(squad, &name) {
                self.begin_work(id, ticket);
            }
        }
    }

    /// Put member `id` on `ticket`: a worktree on the ticket's branch when it
    /// edits, the ticket's project otherwise; then its first turn.
    fn begin_work(&mut self, id: SessionId, ticket_id: TicketId) {
        let home = self.config.home.clone();
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else {
            return;
        };
        let edits = session.edits();
        let board_tools = session.engine == Engine::Claude && self.config.equipe_binary.is_some();
        let projects = self.projects(&squad);
        let Some(journal) = self.journal(&squad) else {
            return;
        };
        let Some(mut ticket) = journal.ticket(ticket_id).ok().flatten() else {
            return;
        };
        let saved = journal.ticket_work(ticket_id).unwrap_or_default();
        let Some(project) = ticket.project.clone().or_else(|| projects.first().cloned()) else {
            return;
        };

        let sandbox = if edits {
            let checkout = worktree::repo_root(&project).and_then(|repo| {
                let id_part = format!("t{ticket_id}-{:x}", now_secs());
                match (&saved.worktree, &ticket.branch, &saved.base) {
                    // In review and sent back: the worktree is still there.
                    (Some(path), Some(branch), Some(base)) if path.exists() => {
                        Ok(Sandbox { repo, path: path.clone(), branch: branch.clone(), base: base.clone() })
                    }
                    // Its worktree went away: check its branch out again.
                    (_, Some(branch), Some(base)) => worktree::resume(&repo, &home, &id_part, branch, base),
                    _ => worktree::create(&repo, &home, &id_part, &name),
                }
            });
            match checkout {
                Ok(sandbox) => Some(sandbox),
                Err(err) => return self.cannot_start(&squad, ticket, &name, &err),
            }
        } else {
            None
        };
        let cwd = sandbox.as_ref().map_or_else(|| project.clone(), |s| s.path.clone());

        ticket.status = TicketStatus::InProgress;
        ticket.assignee = Some(name.clone());
        ticket.branch = sandbox.as_ref().map(|s| s.branch.clone()).or(ticket.branch);
        ticket.worktree = sandbox.as_ref().map(|s| s.path.clone());
        let reopened = ticket.note.as_ref().filter(|n| n.starts_with("Reopened")).cloned();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.set_ticket_work(
                ticket_id,
                &TicketWork {
                    base: sandbox.as_ref().map(|s| s.base.clone()),
                    worktree: sandbox.as_ref().map(|s| s.path.clone()),
                    engine_session: saved.engine_session.clone(),
                },
            );
            let _ = journal.ticket_event(ticket_id, "equipe", "assigned", &name);
        }
        if let Some(session) = self.sessions.get_mut(&id) {
            session.work = Some(Work {
                ticket: ticket_id,
                project: project.clone(),
                cwd,
                sandbox: sandbox.clone(),
                claude_session: saved.engine_session,
                history: Vec::new(),
                handed_in: None,
            });
        }
        self.publish(Event::TicketChanged { squad: squad.clone(), ticket: ticket.clone() });
        self.publish(Event::WorkStarted {
            session: id,
            ticket: ticket_id,
            title: ticket.title.clone(),
            project: project.clone(),
            workspace: sandbox.as_ref().map(workspace_of),
        });
        self.brief_pm(&squad, format!("{name} picked up T-{ticket_id} ({}).", ticket.title), None);
        let language = self.journal(&squad).and_then(Journal::language);
        let prompt = work_prompt(&ticket, &project, sandbox.as_ref(), reopened.as_deref(), board_tools, language.as_deref());
        self.start_turn(id, prompt);
    }

    /// A ticket that can't start (its project isn't a repository, say) goes back
    /// to a proposal with the reason, for the PM or the person to sort out.
    fn cannot_start(&mut self, squad: &str, mut ticket: Ticket, name: &str, reason: &str) {
        ticket.status = TicketStatus::Proposed;
        ticket.note = Some(format!("Couldn't start: {reason}"));
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.ticket_event(ticket.id, "equipe", "blocked", reason);
        }
        let line = format!("T-{} couldn't start for {name}: {reason}", ticket.id);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line.clone(), Some(format!("✗ {line}")));
    }

    /// Member `id` puts its ticket down. Changes are committed to the ticket's
    /// branch; the worktree stays while the work is in review, so it can be sent
    /// back, and goes otherwise — the branch stays to merge.
    fn end_work(&mut self, id: SessionId, how: WorkEnd) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(work) = session.work.take() else {
            return;
        };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else {
            return;
        };
        let Some(mut ticket) = self.journal(&squad).and_then(|j| j.ticket(work.ticket).ok().flatten()) else {
            return;
        };
        let summary = work.handed_in.clone();
        let mut kept_worktree = None;
        if let Some(sandbox) = &work.sandbox {
            let body = summary.clone().unwrap_or_else(|| ticket.body.clone());
            let message =
                format!("equipe: T-{} {}\n\n{body}\n\nRun by {name} on equipe through Claude Code.", ticket.id, ticket.title);
            if let Err(err) = sandbox.commit_all(&message) {
                let message = format!("couldn't commit T-{}: {err} — it is still in {}", ticket.id, sandbox.path.display());
                self.error(Some(id), message, None);
            }
            if how == WorkEnd::HandedIn {
                kept_worktree = Some(sandbox.path.clone());
            } else {
                ticket.branch = release(sandbox);
            }
            ticket.worktree = kept_worktree.clone();
        }
        let (by, event, line) = match how {
            WorkEnd::HandedIn => {
                ticket.status = TicketStatus::Review;
                ticket.note = summary.clone();
                (name.as_str(), "handed in", format!("{name} handed in T-{} for your review", ticket.id))
            }
            WorkEnd::Accepted => {
                ticket.status = TicketStatus::Done;
                ("you", "accepted", format!("You accepted T-{}", ticket.id))
            }
            WorkEnd::Cancelled => {
                ticket.status = TicketStatus::Cancelled;
                ("you", "cancelled", format!("You cancelled T-{}", ticket.id))
            }
            WorkEnd::Left => {
                ticket.status = TicketStatus::Todo;
                ticket.assignee = None;
                ticket.note = Some(match &ticket.branch {
                    Some(branch) => format!("{name} left the squad; the work so far is on {branch}."),
                    None => format!("{name} left the squad."),
                });
                ("equipe", "released", format!("{name} left T-{}; it is back on the board", ticket.id))
            }
        };
        let by = by.to_owned();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.save_ticket(&ticket);
            let mut saved = journal.ticket_work(ticket.id).unwrap_or_default();
            saved.worktree = kept_worktree;
            let _ = journal.set_ticket_work(ticket.id, &saved);
            let _ = journal.ticket_event(ticket.id, &by, event, summary.as_deref().unwrap_or(""));
        }
        let ticket_id = ticket.id;
        let shown = format!("{line} · {}.", ticket.title);
        self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
        self.publish(Event::WorkEnded { session: id, ticket: ticket_id });
        self.publish(Event::Notice { session: id, text: shown.clone() });
        let briefing = match &summary {
            Some(summary) => format!("{line}: {summary}"),
            None => format!("{line}."),
        };
        if self.pm_of(&squad) != Some(id) {
            self.brief_pm(&squad, briefing, Some(shown));
        }
    }

    /// The ticket `ticket` of `squad`, or an error saying there is none.
    fn find_ticket(&mut self, squad: &str, ticket: TicketId) -> Option<Ticket> {
        let found = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten());
        if found.is_none() {
            self.squad_error(squad, format!("There is no T-{ticket} on the board."));
        }
        found
    }

    /// The member of `squad` working on `ticket` right now.
    fn worker_on(&self, squad: &str, ticket: TicketId) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.work.as_ref().is_some_and(|w| w.ticket == ticket))
            .map(|(&id, _)| id)
    }

    /// Close a ticket no member holds right now (in review, or not started).
    fn close_ticket(&mut self, squad: &str, mut ticket: Ticket, status: TicketStatus) {
        let saved = self.journal(squad).and_then(|j| j.ticket_work(ticket.id).ok()).unwrap_or_default();
        if let (Some(path), Some(branch), Some(base), Some(project)) =
            (&saved.worktree, &ticket.branch, &saved.base, &ticket.project)
            && let Ok(repo) = worktree::repo_root(project)
        {
            ticket.branch = release(&Sandbox { repo, path: path.clone(), branch: branch.clone(), base: base.clone() });
        }
        ticket.status = status;
        ticket.worktree = None;
        let verb = if status == TicketStatus::Done { "accepted" } else { "cancelled" };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.set_ticket_work(ticket.id, &TicketWork { worktree: None, ..saved });
            let _ = journal.ticket_event(ticket.id, "you", verb, "");
        }
        let line = format!("You {verb} T-{} · {}.", ticket.id, ticket.title);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line.clone(), Some(line));
    }

    /// The person accepts a ticket: it is done, and its branch stays to merge.
    async fn accept(&mut self, squad: &str, ticket: TicketId) {
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status.is_closed() {
            return self.squad_error(squad, format!("T-{ticket} is already {}.", found.status.label()));
        }
        match self.worker_on(squad, ticket) {
            Some(id) => {
                self.stop_turn(id);
                self.end_work(id, WorkEnd::Accepted);
            }
            None => self.close_ticket(squad, found, TicketStatus::Done),
        }
        self.dispatch(squad).await;
    }

    /// The person sends work back: the ticket goes to its member again, on the
    /// same branch and conversation, with what to change.
    async fn reopen(&mut self, squad: &str, ticket: TicketId, note: String) {
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if self.worker_on(squad, ticket).is_some() {
            let who = found.assignee.clone().unwrap_or_default();
            return self.squad_error(squad, format!("T-{ticket} is still being worked on — talk to {who} directly."));
        }
        let note = if note.trim().is_empty() { "Reopened.".to_owned() } else { format!("Reopened: {}", note.trim()) };
        found.owner = found.assignee.take().or(found.owner);
        found.status = TicketStatus::Todo;
        found.note = Some(note.clone());
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "reopened", &note);
        }
        let line = format!("You reopened T-{} · {} — {note}", found.id, found.title);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        self.brief_pm(squad, line.clone(), Some(line));
        self.dispatch(squad).await;
    }

    async fn cancel(&mut self, squad: &str, ticket: TicketId) {
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status.is_closed() {
            return self.squad_error(squad, format!("T-{ticket} is already {}.", found.status.label()));
        }
        match self.worker_on(squad, ticket) {
            Some(id) => {
                self.stop_turn(id);
                self.end_work(id, WorkEnd::Cancelled);
            }
            None => self.close_ticket(squad, found, TicketStatus::Cancelled),
        }
        self.dispatch(squad).await;
    }

    // ── board tools ──

    /// A member's tool call. A request for permission waits for the person; the
    /// rest are answered at once.
    async fn tool(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        if call.name == permissions::APPROVE_TOOL {
            return self.ask_permission(call, reply);
        }
        let squad = call.squad.clone();
        let result = self.run_tool(call);
        // Whatever changed, the rules look again.
        self.dispatch(&squad).await;
        let _ = reply.send(match result {
            Ok(text) => ToolResult { text, is_error: false },
            Err(text) => ToolResult { text, is_error: true },
        });
    }

    // ── permission ──

    /// The engine asks whether a member may use a tool its mode doesn't allow on
    /// its own. Blocked things are refused, "always allow" rules answered, and
    /// everything else put to the person.
    fn ask_permission(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let tool = call.arguments.get("tool_name").and_then(Value::as_str).unwrap_or("?").to_owned();
        let input = call.arguments.get("input").cloned().unwrap_or_else(|| json!({}));
        let (action, reason) = permissions::describe(&tool, &input);
        let Some(id) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(verdict(Decision::Deny, &input, "Unknown member."));
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        if permissions::blocked(&tool, &action) {
            let _ = reply.send(verdict(Decision::Deny, &input, "equipe never allows this, in any mode."));
            self.publish(Event::Notice { session: id, text: format!("✗ equipe blocked: {action}") });
            self.audit(&call.squad, ticket, "equipe", "blocked", &action);
            return;
        }
        let rules = self.journal(&call.squad).and_then(|j| j.allowed().ok()).unwrap_or_default();
        if rules.iter().any(|rule| permissions::matches(rule, &tool, &action)) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        // In auto, a member on a ticket works inside the fence on its own: a shell
        // command that stays in the sandbox and reaches nowhere outward is allowed —
        // the operating system still holds it in.
        let squad_mode = self.journal(&call.squad).map(Journal::mode).unwrap_or_default();
        let fenced = self.sessions.get(&id).is_some_and(|s| {
            s.mode.unwrap_or(squad_mode) == Mode::Auto && s.work.as_ref().is_some_and(|w| w.sandbox.is_some())
        });
        // Looking changes nothing: in auto a member reads, lists and searches
        // anywhere — its own screenshots in the temp folder, another project —
        // without asking. Credentials still ask.
        let auto = self.sessions.get(&id).is_some_and(|s| s.mode.unwrap_or(squad_mode) == Mode::Auto);
        if auto && permissions::only_reads(&tool, &action, &input) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        if fenced && permissions::stays_in_sandbox(&tool, &input) && !permissions::outward(&tool, &action) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval { id: approval_id, session: id, member: call.member.clone(), ticket, tool, action, reason };
        self.pending.insert(
            approval_id,
            Pending { squad: call.squad.clone(), approval: approval.clone(), input, reply },
        );
        self.publish(Event::ApprovalRequested { squad: call.squad.clone(), approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// Answer a waiting request for permission. "Always" also becomes one of the
    /// squad's rules, so the same kind of thing doesn't ask again.
    fn answer(&mut self, approval_id: u64, decision: Decision, why: &str) {
        let Some(pending) = self.pending.remove(&approval_id) else {
            return;
        };
        let Pending { squad, approval, input, reply } = pending;
        let mut decision = decision;
        let rule = (decision == Decision::Always).then(|| permissions::rule_for(&approval.tool, &approval.action)).flatten();
        if decision == Decision::Always {
            match &rule {
                Some(rule) => {
                    if let Some(journal) = self.journal(&squad) {
                        let _ = journal.allow(rule, "you");
                    }
                }
                // Nothing sound to generalise to: allow it this once.
                None => decision = Decision::Allow,
            }
        }
        let why = if why.is_empty() && decision == Decision::Deny { "The person declined." } else { why };
        let _ = reply.send(verdict(decision, &input, why));
        let line = match (decision, &rule) {
            (Decision::Always, Some(rule)) => format!("✓ You always allow {rule} for the squad · {}", approval.action),
            (Decision::Allow | Decision::Always, _) => format!("✓ You allowed: {}", approval.action),
            (Decision::Deny, _) if why == "The person declined." => format!("✗ You declined: {}", approval.action),
            (Decision::Deny, _) => format!("✗ {why} Declined: {}", approval.action),
        };
        self.publish(Event::Notice { session: approval.session, text: line });
        let verb = match decision {
            Decision::Allow => "allowed",
            Decision::Always => "always allowed",
            Decision::Deny => "declined",
        };
        self.audit(&squad, approval.ticket, "you", verb, &approval.action);
        self.publish(Event::ApprovalResolved { squad, approval: approval_id, decision });
    }

    /// Note a permission decision on the ticket's record.
    fn audit(&mut self, squad: &str, ticket: Option<TicketId>, by: &str, what: &str, action: &str) {
        if let (Some(ticket), Some(journal)) = (ticket, self.journal(squad)) {
            let _ = journal.ticket_event(ticket, by, what, action);
        }
    }

    fn set_mode(&mut self, squad: &str, member: Option<String>, mode: Option<Mode>) {
        match &member {
            None => {
                let Some(mode) = mode else {
                    return self.squad_error(squad, "Name a mode: plan, ask, auto or full.");
                };
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.set_mode(mode);
                }
            }
            Some(name) => {
                let Some(id) = self.member_id(squad, name) else {
                    return self.squad_error(squad, format!("No one called {name} on the squad."));
                };
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.mode = mode;
                }
                if self.config.persist
                    && let Some(journal) = self.journal(squad)
                {
                    let _ = journal.set_member_mode(id, mode);
                }
            }
        }
        self.publish(Event::ModeChanged { squad: squad.to_owned(), member, mode });
    }

    fn run_tool(&mut self, call: ToolCall) -> Result<String, String> {
        let ToolCall { squad, member, name, arguments: args } = call;
        let id = self.member_id(&squad, &member).ok_or_else(|| format!("{member} isn't on the squad {squad}."))?;
        let is_pm = self.sessions.get(&id).is_some_and(Session::is_pm);
        let projects = self.projects(&squad);
        let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
        let no_project = |arg: &str| format!("No project called {arg}. The squad's projects: {}.", names.join(", "));
        match name.as_str() {
            board::BOARD => Ok(board::render(&self.tickets(&squad))),
            board::CREATE_TICKET => {
                let title = board::arg_str(&args, "title").ok_or("A ticket needs a title.")?;
                let body = board::arg_str(&args, "body").unwrap_or_default();
                let project = match board::arg_str(&args, "project") {
                    Some(arg) => Some(board::resolve_project(arg, &projects).ok_or_else(|| no_project(arg))?),
                    None if projects.len() > 1 => {
                        return Err(format!("Name the project — the squad has several: {}.", names.join(", ")));
                    }
                    None => projects.first().cloned(),
                };
                let depends_on = board::arg_ids(&args, "depends_on").unwrap_or_default();
                let tickets = self.tickets(&squad);
                if let Some(missing) = depends_on.iter().find(|d| !tickets.iter().any(|t| t.id == **d)) {
                    return Err(format!("There is no T-{missing} to depend on."));
                }
                let owner = board::arg_str(&args, "owner").map(str::to_owned);
                let ticket = Ticket {
                    id: 0,
                    title: title.to_owned(),
                    body: body.to_owned(),
                    status: if is_pm { TicketStatus::Todo } else { TicketStatus::Proposed },
                    owner: owner.clone(),
                    assignee: None,
                    project,
                    depends_on,
                    branch: None,
                    worktree: None,
                    created_by: member.clone(),
                    note: None,
                };
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let new_id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(new_id, &member, "created", title);
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket: Ticket { id: new_id, ..ticket } });
                let mut reply = format!("Created T-{new_id}.");
                if !is_pm {
                    reply = format!("Proposed T-{new_id} — it waits for the PM or the person to accept it.");
                    let line = format!("{member} proposed T-{new_id}: {title}");
                    self.brief_pm(&squad, line.clone(), Some(line));
                }
                let staffed = |o: &str| self.member_id(&squad, o).is_some() || self.team(&squad).iter().any(|(_, _, r)| r == o);
                if let Some(owner) = owner.filter(|o| !staffed(o)) {
                    reply.push_str(&format!(" Nobody on the squad is {owner} yet — the person can add one with /add {owner}."));
                }
                Ok(reply)
            }
            board::UPDATE_TICKET => {
                if !is_pm {
                    return Err("Only the PM changes tickets; propose a new one, or comment.".into());
                }
                let ticket_id = board::arg_id(&args, "id").ok_or("Which ticket? Give its id.")?;
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let mut ticket = journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
                if !matches!(ticket.status, TicketStatus::Proposed | TicketStatus::Todo) {
                    let status = ticket.status.label();
                    return Err(format!("T-{ticket_id} is {status} — from here it's the person's to change."));
                }
                if let Some(title) = board::arg_str(&args, "title") {
                    ticket.title = title.to_owned();
                }
                if let Some(body) = board::arg_str(&args, "body") {
                    ticket.body = body.to_owned();
                }
                if let Some(owner) = board::arg_str(&args, "owner") {
                    ticket.owner = Some(owner.to_owned());
                }
                if let Some(arg) = board::arg_str(&args, "project") {
                    ticket.project = Some(board::resolve_project(arg, &projects).ok_or_else(|| no_project(arg))?);
                }
                if let Some(ids) = board::arg_ids(&args, "depends_on") {
                    ticket.depends_on = ids;
                }
                match board::arg_str(&args, "status") {
                    Some("todo") => {
                        ticket.status = TicketStatus::Todo;
                        ticket.note = None;
                    }
                    Some("cancelled") => ticket.status = TicketStatus::Cancelled,
                    Some(other) => return Err(format!("A ticket's status can be set to todo or cancelled, not {other}.")),
                    None => {}
                }
                journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(ticket_id, &member, "updated", &args.to_string());
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
                Ok(format!("Updated T-{ticket_id}."))
            }
            board::COMMENT => {
                let ticket_id = board::arg_id(&args, "id").ok_or("Which ticket? Give its id.")?;
                let text = board::arg_str(&args, "text").ok_or("Say something.")?;
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
                journal.ticket_event(ticket_id, &member, "comment", text).map_err(|e| e.to_string())?;
                Ok(format!("Noted on T-{ticket_id}."))
            }
            board::SUBMIT_FOR_REVIEW => {
                let summary = board::arg_str(&args, "summary").ok_or("Summarize what you did.")?.to_owned();
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let work = session.work.as_mut().ok_or("You're not on a ticket — there is nothing to hand in.")?;
                work.handed_in = Some(summary);
                Ok(format!(
                    "Handed in T-{}. Your changes are committed when this turn ends and the person reviews them. \
End your turn now with a short summary.",
                    work.ticket
                ))
            }
            other => Err(format!("equipe has no tool called {other}.")),
        }
    }

    // ── attach and restart ──

    fn snapshot(&mut self, squad: &str) -> Snapshot {
        let projects = self.projects(squad);
        let mut tickets = self.tickets(squad);
        // Open tickets, and the latest closed ones.
        let recent_closed: Vec<TicketId> =
            tickets.iter().rev().filter(|t| t.status.is_closed()).map(|t| t.id).take(CLOSED_TICKETS_SHOWN).collect();
        tickets.retain(|t| !t.status.is_closed() || recent_closed.contains(&t.id));
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .map(|(&id, s)| {
                let entries = s.transcript.entries();
                SessionInfo {
                    session: id,
                    project: s.project.clone(),
                    member: s.member.clone(),
                    engine: s.engine,
                    model: s.model.clone(),
                    access: s.access,
                    workspace: s.sandbox.as_ref().map(workspace_of),
                    state: if s.is_working() { SessionState::Working } else { SessionState::Ready },
                    title: s.title.clone(),
                    tokens: s.tokens,
                    recent: entries[entries.len().saturating_sub(SNAPSHOT_LINES)..].to_vec(),
                    queued: s.queued.clone(),
                    work: s.work.as_ref().map(|w| WorkInfo {
                        ticket: w.ticket,
                        project: w.project.clone(),
                        workspace: w.sandbox.as_ref().map(workspace_of),
                    }),
                    mode: s.mode,
                }
            })
            .collect();
        // The PM first, then everyone in the order they joined.
        sessions.sort_by_key(|info| {
            let s = &self.sessions[&info.session];
            (!s.is_pm(), s.joined)
        });
        let mode = self.journal(squad).map(Journal::mode).unwrap_or_default();
        let mut approvals: Vec<Approval> =
            self.pending.values().filter(|p| p.squad == squad).map(|p| p.approval.clone()).collect();
        approvals.sort_by_key(|a| a.id);
        Snapshot { squad: squad.to_owned(), projects, sessions, tickets, mode, approvals }
    }

    /// Bring back every squad's members, and the tickets they were on.
    fn restore(&mut self) {
        let Ok(dirs) = std::fs::read_dir(self.config.home.join("squads")) else {
            return;
        };
        for dir in dirs.flatten() {
            let Ok(journal) = Journal::open(&dir.path().join("squad.db")) else {
                continue;
            };
            let Some(squad) = journal.squad() else {
                continue;
            };
            let projects = journal.projects().unwrap_or_default();
            let tickets = journal.tickets().unwrap_or_default();
            for record in journal.open_sessions().unwrap_or_default() {
                let Some(member) = record.member.clone() else {
                    // Only members live in a squad; anything else can't be resumed.
                    let _ = journal.close_session(record.id);
                    continue;
                };
                let conversation = journal.conversation(record.id).unwrap_or_default();
                let history = conversation
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::User { text } => Some(responses::Message { role: responses::Role::User, text: text.clone() }),
                        Entry::Reply { text } => {
                            Some(responses::Message { role: responses::Role::Assistant, text: text.clone() })
                        }
                        _ => None,
                    })
                    .collect();
                let edits = roles::role_or_custom(&member.role).edits;
                let work = tickets
                    .iter()
                    .find(|t| t.status == TicketStatus::InProgress && t.assignee.as_deref() == Some(member.name.as_str()))
                    .and_then(|ticket| restore_work(&journal, ticket, &projects, edits));
                let memory = Some(journal::member_memory_dir(&self.config.home, &squad, &member.name));
                let joined = self.next_join;
                self.next_join += 1;
                self.sessions.insert(
                    record.id,
                    Session {
                        squad: Some(squad.clone()),
                        project: record.cwd.clone(),
                        member: Some(member),
                        joined,
                        engine: record.engine,
                        model: record.model,
                        access: record.access,
                        cwd: record.cwd,
                        sandbox: None,
                        codex: None,
                        history,
                        claude_session: record.engine_session,
                        work,
                        prompts: Vec::new(),
                        title: record.title,
                        tokens: record.tokens,
                        transcript: Transcript::from_entries(conversation),
                        turn: None,
                        queued: Vec::new(),
                        briefing: Vec::new(),
                        memory,
                        mode: record.mode,
                    },
                );
            }
            self.journals.insert(squad, journal);
        }
    }

    /// Members that were on a ticket when the daemon stopped carry on with it —
    /// a squad keeps working through a restart. (Codex members wait for their
    /// next message: their login loads on first use.)
    fn resume_work(&mut self) {
        let on_tickets: Vec<(SessionId, TicketId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.engine == Engine::Claude)
            .filter_map(|(&id, s)| s.work.as_ref().map(|w| (id, w.ticket)))
            .collect();
        for (id, ticket) in on_tickets {
            self.publish(Event::Notice { session: id, text: format!("equipe restarted; picking T-{ticket} up again.") });
            self.start_turn(
                id,
                format!(
                    "[equipe] equipe restarted while you worked on T-{ticket}. Carry on from where you left off — \
check what is already done in your worktree first."
                ),
            );
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

/// A member's ticket as it was when the daemon stopped. A ticket whose worktree
/// was removed in the meantime goes back on the board instead.
fn restore_work(journal: &Journal, ticket: &Ticket, projects: &[PathBuf], edits: bool) -> Option<Work> {
    let project = ticket.project.clone().or_else(|| projects.first().cloned())?;
    let saved = journal.ticket_work(ticket.id).unwrap_or_default();
    let sandbox = match (edits, &saved.worktree, &ticket.branch, &saved.base) {
        (false, ..) => None,
        (true, Some(path), Some(branch), Some(base)) if path.exists() => Some(Sandbox {
            repo: worktree::repo_root(&project).ok()?,
            path: path.clone(),
            branch: branch.clone(),
            base: base.clone(),
        }),
        _ => {
            let mut back = ticket.clone();
            back.status = TicketStatus::Todo;
            back.owner = back.assignee.take().or(back.owner);
            back.note = Some("Its worktree was removed; it goes back on the board.".into());
            let _ = journal.save_ticket(&back);
            return None;
        }
    };
    Some(Work {
        ticket: ticket.id,
        cwd: sandbox.as_ref().map_or_else(|| project.clone(), |s| s.path.clone()),
        project,
        sandbox,
        claude_session: saved.engine_session,
        history: Vec::new(),
        handed_in: None,
    })
}

/// What a member is told when it picks up a ticket.
fn work_prompt(
    ticket: &Ticket,
    project: &Path,
    sandbox: Option<&Sandbox>,
    reopened: Option<&str>,
    board_tools: bool,
    language: Option<&str>,
) -> String {
    let mut out = format!("[equipe] You're on T-{}: {}\n", ticket.id, ticket.title);
    out.push_str(&format!("Project: {} ({}).", board::project_name(project), project.display()));
    if let Some(sandbox) = sandbox {
        out.push_str(&format!(
            " You work in a worktree of your own on branch {}; the person's checkout is untouched.",
            sandbox.branch
        ));
    }
    out.push('\n');
    if let Some(note) = reopened {
        out.push_str(&format!("The person sent it back. {note}\n"));
    }
    if !ticket.body.is_empty() {
        out.push_str(&format!("\n{}\n", ticket.body));
    }
    out.push_str(if board_tools {
        "\nWhen it meets the acceptance criteria, call submit_for_review with a short summary, then end your turn. \
If you are blocked or need a decision, say so and end your turn — the person answers here."
    } else {
        "\nWhen it's done, say so plainly; the person accepts it. If you are blocked or need a decision, say so."
    });
    // This prompt is in English; what the person reads is not.
    if let Some(language) = language {
        out.push_str(&format!("\nWrite your summary and anything else for the person in {language}."));
    }
    out
}

/// How a member's turn on a ticket runs in `mode`. Only a member that edits, in
/// its own worktree, ever writes.
fn access_for(mode: Mode, may_edit: bool) -> claude_code::Access {
    match (mode, may_edit) {
        (_, false) | (Mode::Plan, _) => claude_code::Access::ReadOnly,
        (Mode::Ask, true) => claude_code::Access::Ask,
        (Mode::Auto, true) => claude_code::Access::EditFiles,
        (Mode::Full, true) => claude_code::Access::Full,
    }
}

/// A plain session (`equipe try`) reads, or edits in its worktree.
fn plain_access(access: Access) -> claude_code::Access {
    match access {
        Access::ReadOnly => claude_code::Access::ReadOnly,
        Access::EditFiles => claude_code::Access::EditFiles,
    }
}

/// What the engine's permission prompt tool answers: Claude Code reads it as JSON.
fn verdict(decision: Decision, input: &Value, why: &str) -> ToolResult {
    let text = match decision {
        Decision::Allow | Decision::Always => json!({ "behavior": "allow", "updatedInput": input }),
        Decision::Deny => json!({ "behavior": "deny", "message": why }),
    };
    ToolResult { text: text.to_string(), is_error: false }
}

/// Remove a ticket's worktree; keep its branch if it has commits, drop it if not.
/// Returns the branch that remains.
fn release(sandbox: &Sandbox) -> Option<String> {
    if sandbox.has_commits() {
        let _ = sandbox.remove_worktree();
        Some(sandbox.branch.clone())
    } else {
        let _ = sandbox.discard();
        None
    }
}

// ── engines ──
//
// Free functions over the parts they need: a future that held `&Runtime` across an
// await would require the journals' SQLite connections to be shareable across
// threads, which they are not.

/// The first engine in `preference` that is signed in.
async fn first_ready_engine(preference: &[Engine], store: &CredentialStore, claude_binary: &str) -> Option<Engine> {
    for &engine in preference {
        let ready = match engine {
            Engine::Codex => matches!(store.load("openai"), Ok(Some(_))),
            Engine::Claude => {
                matches!(claude_code::auth_status(claude_binary).await, Ok(Some(status)) if status.logged_in)
            }
        };
        if ready {
            return Some(engine);
        }
    }
    None
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

/// The journal of `squad`, opened on first use.
fn journal_for<'a>(journals: &'a mut HashMap<String, Journal>, home: &Path, squad: &str) -> Option<&'a Journal> {
    if !journals.contains_key(squad) {
        let journal = Journal::for_squad(home, squad).ok()?;
        journals.insert(squad.to_owned(), journal);
    }
    journals.get(squad)
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
