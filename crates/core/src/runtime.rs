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
//! works in the squad's home and reads every project; given a ticket, its turns run in that ticket's project —
//! in a worktree of its own when it edits. Who gets which ticket is decided by
//! equipe's rules ([`crate::board::assignments`]), never by a model.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_protocol::rpc::{ToolCall, ToolResult};
use equipe_protocol::{
    Access, Approval, Command, Decision, Engine, Entry, Event, Member, MemberRef, Mode, SessionId, SessionInfo,
    SessionSpec, SessionState, Snapshot, Ticket, TicketId, TicketKind, TicketStatus, Transcript, TurnOutcome, Usage, WorkInfo, Workspace,
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
use crate::browser;
use crate::desktop_gate::{self, Act, Verdict};
use crate::desktop;
use crate::journal::{self, Journal, SessionRecord, TicketWork};
use crate::permissions;
use crate::roles::{self, Teammate};
use crate::squad_home;
use crate::squad_memory;
use crate::worktree::{self, Sandbox};

/// How many recent lines a snapshot carries per session.
const SNAPSHOT_LINES: usize = 200;

/// How much of a member's answer the PM is told.
const BRIEF_REPLY_CHARS: usize = 400;

/// Closed tickets a snapshot still shows, newest first.
const CLOSED_TICKETS_SHOWN: usize = 20;

/// The MCP server equipe gives members; its tools are `mcp__equipe__*`.
const MCP_SERVER: &str = "equipe";
/// The tool name a member's own question shows under, among approvals.
const CONFIRM: &str = "Confirm";

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
    /// Node's `npx`, which runs the members' browser. `None`: no browser.
    pub npx: Option<PathBuf>,
    /// What the Mac says about the person's session: whether they are at it,
    /// whether it is locked. Asked before every member's desktop input.
    pub mac: Box<dyn desktop::Probe>,
    /// Guess the person's next message after a member answers them, with
    /// Claude Code's small model.
    pub suggest: bool,
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
            npx: crate::browser::find_npx(),
            mac: Box::new(desktop::ThisMac),
            suggest: true,
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

/// A desktop tool call asking for the screen, or done with it.
enum DesktopRequest {
    Ask(MemberRef, desktop::Use, oneshot::Sender<Result<f64, String>>),
    Done { who: MemberRef, start: f64, input: bool },
    Release(MemberRef, oneshot::Sender<bool>),
}

enum Request {
    Command(Command),
    Snapshot(String, oneshot::Sender<Snapshot>),
    Subscribe(String, oneshot::Sender<Subscription>),
    Tool(ToolCall, oneshot::Sender<ToolResult>),
    DesktopGate(Box<desktop_gate::Check>, oneshot::Sender<ToolResult>),
    Busy(oneshot::Sender<equipe_protocol::rpc::Busy>),
    Desktop(DesktopRequest),
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

    /// Whether a member's desktop input may go ahead: at once, after the
    /// person's yes when it may send in their name, or never. `is_error` means
    /// no, with why in `text`; the input must not reach the screen.
    pub async fn desktop_gate(&self, check: desktop_gate::Check) -> ToolResult {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::DesktopGate(Box::new(check), tx));
        rx.await.unwrap_or(ToolResult { text: "equipe is shutting down".into(), is_error: true })
    }

    /// May `who` make a desktop tool call that does `what`, now? Every desktop
    /// call asks first: on `Ok(start)` it goes to the Mac, then
    /// [`Handle::desktop_done`] with `start`; `Err` is what to tell the member
    /// (another has the screen, the person is at the Mac, it is locked…).
    pub async fn desktop_ask(&self, who: MemberRef, what: desktop::Use) -> Result<f64, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Desktop(DesktopRequest::Ask(who, what, tx)));
        rx.await.unwrap_or_else(|_| Err("equipe is shutting down".into()))
    }

    /// The desktop call `desktop_ask` let through at `start` has ended;
    /// `input`: it clicked, typed or scrolled.
    pub fn desktop_done(&self, who: MemberRef, start: f64, input: bool) {
        let _ = self.requests.send(Request::Desktop(DesktopRequest::Done { who, start, input }));
    }

    /// `who` lets go of the screen, or leaves the line. Whether it had either.
    pub async fn desktop_release(&self, who: MemberRef) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Desktop(DesktopRequest::Release(who, tx)));
        rx.await.unwrap_or(false)
    }

    /// Who is mid-turn, and how many requests wait for the person.
    pub async fn busy(&self) -> equipe_protocol::rpc::Busy {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Busy(tx));
        rx.await.unwrap_or_default()
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
    let mut config = config;
    let mac = std::mem::replace(&mut config.mac, Box::new(desktop::ThisMac));
    let mut runtime = Runtime {
        desktop: desktop::Desktop::new(mac),
        config,
        client: reqwest::Client::new(),
        subscribers: vec![(None, evt_tx)],
        internal: internal_tx,
        sessions: HashMap::new(),
        journals: HashMap::new(),
        next_join: 1,
        pending: HashMap::new(),
        next_approval: 1,
        focus: HashMap::new(),
        browsers: HashMap::new(),
        watching: HashMap::new(),
        framing: std::collections::HashSet::new(),
        last_url: HashMap::new(),
        show_url: HashMap::new(),
        plan: None,
    };
    let mut memos = Vec::new();
    if runtime.config.persist {
        runtime.restore();
        runtime.prune();
        memos = runtime.resume_work();
    }
    tokio::spawn(run(runtime, req_rx, internal_rx));
    let handle = Handle { requests: req_tx };
    // Memos for a turn the restart ended go in first, as if just sent.
    for (session, text) in memos {
        handle.send(Command::Submit { session, text });
    }
    (handle, evt_rx)
}

/// The member a session is, as the screen's lease knows it.
fn session_member(session: &Session) -> Option<MemberRef> {
    Some(MemberRef { squad: session.squad.clone()?, member: session.name()?.to_owned() })
}

/// The project a directory belongs to: its repository root, or itself.
pub fn project_of(dir: &Path) -> PathBuf {
    worktree::repo_root(dir).unwrap_or_else(|_| dir.canonicalize().unwrap_or_else(|_| dir.to_owned()))
}

/// A member's current ticket: where its turns run until the ticket is put down.
struct Work {
    ticket: TicketId,
    kind: TicketKind,
    project: PathBuf,
    cwd: PathBuf,
    /// For a member that edits: its worktree on the ticket's branch.
    sandbox: Option<Sandbox>,
    /// The ticket's own conversation (Claude Code `--resume`).
    claude_session: Option<String>,
    history: Vec<responses::Message>,
    /// Set by `submit_for_review`; the work is handed in when the turn ends.
    handed_in: Option<String>,
    /// Set by `block_on_person`: what the person has to do. The ticket waits
    /// for them when the turn ends.
    waiting: Option<String>,
}

/// One of the PM's conversations at its desk, while another is the current one.
#[derive(Default)]
struct DeskThread {
    claude_session: Option<String>,
    history: Vec<responses::Message>,
    desk_context: Option<(u64, u64)>,
}

struct Session {
    squad: Option<String>,
    /// For a member, its desk: the squad's home; otherwise where it was opened.
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
    /// How full its latest conversation's context was, and the window.
    context: Option<(u64, u64)>,
    /// The same for its desk conversation — the one that lives between tickets.
    desk_context: Option<(u64, u64)>,
    /// When its latest turn ended.
    last_turn: Option<std::time::Instant>,
    /// A fresh conversation asked for at the desk: carrying a summary of the
    /// last one (`Some(true)`), or starting clean (`Some(false)`).
    refresh: Option<bool>,
    transcript: Transcript,
    turn: Option<JoinHandle<()>>,
    /// Memos left while a turn ran, delivered when it ends: those about one
    /// project together, in that project's conversation, then the next project's.
    queued: Vec<String>,
    /// For each memo, the project the person was looking at when they left it
    /// (`None`: the whole squad).
    memo_threads: Vec<Option<String>>,
    /// The conversation the memos being delivered now go into.
    memo_thread: Option<Option<String>>,
    /// For the PM: what happened with the team since the PM's last turn. It
    /// travels with the PM's next message, so the PM keeps up without spending a
    /// turn on every exchange.
    briefing: Vec<String>,
    /// Where a member's engine keeps what it learns (Claude Code's auto memory).
    memory: Option<PathBuf>,
    /// For the PM: the project its desk conversation is about now (`None`:
    /// the whole squad's), and the other threads' conversations, put aside.
    thread: Option<String>,
    threads: HashMap<Option<String>, DeskThread>,
    /// Its own mode; `None` follows the squad's.
    mode: Option<Mode>,
    /// The turn running answers the person (not the board, not the team), so
    /// a guess at what they say next may follow it.
    answers_person: bool,
}

impl Session {
    /// The memos about the first project one was left in, taken out to be sent
    /// together into its conversation; the rest wait for the next turn's end.
    fn next_memos(&mut self) -> Option<String> {
        let project = self.memo_threads.first().cloned().or_else(|| (!self.queued.is_empty()).then_some(None))?;
        let mut taken = Vec::new();
        let mut kept = (Vec::new(), Vec::new());
        let threads = std::mem::take(&mut self.memo_threads);
        for (i, text) in std::mem::take(&mut self.queued).into_iter().enumerate() {
            let about = threads.get(i).cloned().unwrap_or_else(|| project.clone());
            if about == project {
                taken.push(text);
            } else {
                kept.0.push(text);
                kept.1.push(about);
            }
        }
        (self.queued, self.memo_threads) = kept;
        self.memo_thread = Some(project);
        Some(taken.join("\n\n"))
    }

    fn is_working(&self) -> bool {
        self.turn.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn is_pm(&self) -> bool {
        self.member.as_ref().is_some_and(|m| m.role == roles::PM)
    }

    fn name(&self) -> Option<&str> {
        self.member.as_ref().map(|m| m.name.as_str())
    }

    /// Whether it changes a project's files on a code ticket: its role's say,
    /// or the person's for a custom role — as it joined.
    fn edits(&self) -> bool {
        self.member.is_some() && self.access == Access::EditFiles
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
    /// A picture of a watched member's browser, or none to be had.
    Frame(SessionId, Option<(Vec<u8>, Option<String>)>),
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
    /// It called `block_on_person`: the ticket waits for the person, its
    /// worktree kept, and the member is free for other work.
    Waiting,
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
    /// The project each squad's person is looking at, by name.
    focus: HashMap<String, String>,
    /// Members' browsers, kept between turns: by squad and member.
    browsers: HashMap<(String, String), browser::Running>,
    /// The member each squad's screen watches the browser of, and the
    /// pictures under way, one at a time each.
    watching: HashMap<String, SessionId>,
    framing: std::collections::HashSet<SessionId>,
    /// Where each member's browser was last taken, by squad and name — and a
    /// page asked for when it next comes on screen.
    last_url: HashMap<(String, String), String>,
    show_url: HashMap<(String, String), String>,
    /// The Claude plan's usage, as Claude Code last reported it.
    plan: Option<equipe_protocol::PlanUsage>,
    /// The Mac's screen, one member at a time, across squads.
    desktop: desktop::Desktop,
}

async fn run(mut rt: Runtime, mut requests: UnboundedReceiver<Request>, mut internal: UnboundedReceiver<Internal>) {
    let mut daily = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    let mut minute = tokio::time::interval(Duration::from_secs(60));
    // A screen held by a member that stopped using it goes to the next in line.
    let mut desktop_tick = tokio::time::interval(Duration::from_secs(5));
    let mut frames = tokio::time::interval(FRAME_EVERY);
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
                Some(Request::DesktopGate(check, reply)) => rt.gate_desktop(*check, reply),
                Some(Request::Busy(reply)) => {
                    let mut working: Vec<String> = rt
                        .sessions
                        .values()
                        .filter(|s| s.is_working())
                        .map(|s| format!("{}/{}", s.squad.as_deref().unwrap_or("-"), s.name().unwrap_or("?")))
                        .collect();
                    working.sort();
                    let _ = reply.send(equipe_protocol::rpc::Busy { working, approvals: rt.pending.len(), desktop: rt.desktop.state() });
                }
                Some(Request::Desktop(request)) => rt.on_desktop(request),
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
                Internal::Frame(session, frame) => {
                    rt.framing.remove(&session);
                    if let Some((png, url)) = frame {
                        use base64::Engine as _;
                        let image = base64::engine::general_purpose::STANDARD.encode(png);
                        rt.publish(Event::BrowserFrame { session, image, url });
                    }
                }
            },
            _ = daily.tick(), if rt.config.persist => rt.prune(),
            _ = desktop_tick.tick() => {
                if rt.desktop.expire() {
                    rt.publish_desktop();
                }
            }
            _ = frames.tick() => rt.take_frames(),
            _ = minute.tick() => {
                rt.close_idle_browsers();
                rt.let_helpers_go().await;
                rt.tidy_check();
            }
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
        // Where a member's browser goes, to bring it back there on screen.
        if let Event::ToolUse { session, tool, target: Some(url) } = &event
            && tool.ends_with("browser_navigate")
            && let Some(s) = self.sessions.get(session)
            && let (Some(squad), Some(name)) = (s.squad.clone(), s.name())
        {
            self.last_url.insert((squad, name.to_owned()), url.clone());
        }
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
                Event::TurnEnded { usage, .. } => {
                    session.last_turn = Some(std::time::Instant::now());
                    if let Some(u) = usage {
                        session.tokens += u.total();
                        if let (Some(used), Some(window)) = (u.context_tokens, u.context_window) {
                            session.context = Some((used, window));
                            if session.work.is_none() {
                                session.desk_context = Some((used, window));
                            }
                        }
                    }
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
        if let Event::PlanUsage { usage } = &event {
            self.plan = Some(*usage);
        }
        let everyone = event.for_everyone();
        self.subscribers.retain(|(follows, screen)| match follows {
            Some(follows) if squad.as_ref() != Some(follows) && !everyone => !screen.is_closed(),
            _ => screen.send(event.clone()).is_ok(),
        });
    }

    // ── the Mac's screen ──

    fn on_desktop(&mut self, request: DesktopRequest) {
        let before = self.desktop.state();
        match request {
            DesktopRequest::Ask(who, what, reply) => {
                let answer = self.desktop.ask(&who, what).map(|()| self.desktop.now());
                let _ = reply.send(answer.map_err(|refusal| refusal.to_string()));
            }
            DesktopRequest::Done { who, start, input } => {
                let end = self.desktop.now();
                self.desktop.done(&who, start, end, input);
            }
            DesktopRequest::Release(who, reply) => {
                let _ = reply.send(self.desktop.leave(&who));
            }
        }
        if self.desktop.state() != before {
            self.publish_desktop();
        }
    }

    fn publish_desktop(&mut self) {
        let desktop = self.desktop.state();
        self.publish(Event::Desktop { desktop });
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
            Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),
            Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,
            Command::Resume { squad, ticket, note } => self.resume(&squad, ticket, note).await,
            Command::Cancel { squad, ticket } => self.cancel(&squad, ticket).await,
            Command::SetMode { squad, member, mode } => self.set_mode(&squad, member, mode),
            Command::ShowBrowser { squad, member, show } => self.show_browser(&squad, member, show),
            Command::Desktop { open } => {
                let changed = if open { self.desktop.resume() } else { self.desktop.stop() };
                if changed {
                    self.publish_desktop();
                }
            }
            Command::WatchBrowser { squad, session } => match session {
                Some(id) => {
                    self.watching.insert(squad, id);
                }
                None => {
                    self.watching.remove(&squad);
                }
            },
            Command::Suggestions { squad, on } => {
                if let Some(journal) = self.journal(&squad) {
                    let _ = journal.set_suggestions(on);
                }
            }
            // From older screens: nobody is added on equipe's own any more.
            Command::Scale { .. } => {}
            Command::Refresh { session, summary } => {
                if let Some(s) = self.sessions.get_mut(&session) {
                    s.refresh = Some(summary);
                    let text = if summary {
                        "Its next conversation at the desk starts afresh, from a summary of this one and its memory."
                    } else {
                        "Its next conversation at the desk starts clean — its memory and the board carry over."
                    };
                    self.publish(Event::Notice { session, text: text.into() });
                }
            }
            Command::SetModel { session, model } => {
                let model = model.trim().to_owned();
                if let Some(s) = self.sessions.get_mut(&session).filter(|_| !model.is_empty()) {
                    s.model = Some(model.clone());
                    let squad = s.squad.clone();
                    if let Some(journal) = squad.as_deref().and_then(|q| journal_for(&mut self.journals, &self.config.home, q)) {
                        let _ = journal.set_model(session, &model);
                    }
                    self.publish(Event::ModelKnown { session, model: model.clone() });
                    self.publish(Event::Notice { session, text: format!("From its next turn: {model}. Its conversation carries on.") });
                }
            }
            Command::Focus { squad, project } => match project {
                Some(name) => {
                    self.focus.insert(squad, name);
                }
                None => {
                    self.focus.remove(&squad);
                }
            },
            Command::Decide { squad, approval, decision } => {
                if self.pending.get(&approval).is_some_and(|p| p.squad == squad) {
                    self.answer(approval, decision, "");
                }
            }
        }
    }

    // ── squads ──

    /// Keep what waits for `id`'s next turn in its journal.
    fn save_waiting(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (Some(squad), queued, briefing) = (session.squad.clone(), session.queued.clone(), session.briefing.clone()) else {
            return;
        };
        let threads: Vec<String> = session.memo_threads.iter().map(|p| p.clone().unwrap_or_default()).collect();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.set_waiting(id, "memo", &queued);
            let _ = journal.set_waiting(id, "memo-threads", &threads);
            let _ = journal.set_waiting(id, "briefing", &briefing);
        }
    }

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
        self.save_waiting(pm);
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
        // A built-in role edits as its role says; a custom one as the person said.
        let edits = role.map(|r| if r.id == roles::CUSTOM.id { spec.access == Access::EditFiles } else { r.edits });
        match (&squad, role) {
            (Some(squad), _) => {
                // A squad may have no project yet: its home is where it works.
                if !crate::squads::exists(&self.config.home, squad) {
                    return fail(self, format!("There is no squad called {squad}."), Some("equipe"));
                }
            }
            (None, Some(_)) => return fail(self, "A member joins a squad; name one.".into(), None),
            (None, None) => {}
        }

        let engine = match spec.engine {
            Some(engine) => engine,
            None => {
                let preference = match (role, edits) {
                    (Some(_), Some(true)) => [Engine::Claude, Engine::Codex],
                    (Some(role), _) => roles::engine_preference(role),
                    (None, _) => [Engine::Codex, Engine::Claude],
                };
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
        let (cwd, sandbox, access) = match (&member, spec.access, &squad) {
            // A member's desk is the squad's home; repositories it edits only on
            // a ticket, in a worktree.
            (Some(_), _, Some(squad)) => {
                let language = self.journal(squad).and_then(Journal::language);
                if let Err(err) = squad_home::ensure(&self.config.home, squad, language.as_deref()) {
                    return fail(self, format!("The squad's folder couldn't be made: {err}"), None);
                }
                let access = if edits == Some(true) { Access::EditFiles } else { Access::ReadOnly };
                (squad_home::path(&self.config.home, squad), None, access)
            }
            (Some(_), _, None) => unreachable!("a member joins a squad"),
            (None, Access::ReadOnly, _) => (spec.cwd.clone(), None, Access::ReadOnly),
            (None, Access::EditFiles, _) => {
                let made = worktree::repo_root(&spec.cwd).and_then(|repo| {
                    worktree::create(&repo, &self.config.home, &format!("{:x}-{id:x}", now_secs()), "try")
                });
                match made {
                    Ok(sandbox) => (sandbox.path.clone(), Some(sandbox), Access::EditFiles),
                    Err(message) => return fail(self, message, None),
                }
            }
        };
        // A member is where its desk is, the squad's home; a plain session where it was opened.
        let project = if member.is_some() { cwd.clone() } else { opened_in };
        let workspace = sandbox.as_ref().map(workspace_of);
        let memory = match (&squad, &member) {
            (Some(squad), Some(m)) => Some(journal::member_memory_dir(&self.config.home, squad, &m.role)),
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
                context: None,
                desk_context: None,
                last_turn: None,
                refresh: None,
                transcript: Transcript::default(),
                turn: None,
                queued: Vec::new(),
                memo_threads: Vec::new(),
                memo_thread: None,
                answers_person: false,
                briefing: Vec::new(),
                memory,
                mode: None,
                thread: None,
                threads: HashMap::new(),
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
            // A memo: it waits for the turn to end rather than cutting in, and
            // belongs with the project the person is looking at now.
            let looking_at = match &session.work {
                Some(work) => Some(crate::board::project_name(&work.project)),
                None => session.squad.as_ref().and_then(|s| self.focus.get(s)).cloned(),
            };
            session.queued.push(text.clone());
            session.memo_threads.push(looking_at.clone());
            let project = looking_at;
            self.save_waiting(id);
            return self.publish(Event::MessageQueued { session: id, text, project });
        }
        let memo_thread = session.memo_thread.take();
        if memo_thread.is_some() {
            self.save_waiting(id);
        }
        if !self.ensure_codex(id).await {
            return;
        }
        // The PM talks about the project the person is looking at in that
        // project's own conversation — a memo, about the one it was left in.
        self.enter_thread(id, memo_thread);
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
        let mut prompt = match std::mem::take(&mut session.briefing) {
            briefing if briefing.is_empty() => text,
            briefing => format!(
                "[equipe: since your last turn —\n{}]\n\n{text}",
                briefing.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")
            ),
        };
        // With several projects, which one the person is looking at.
        if let Some(project) = session.squad.as_ref().and_then(|s| self.focus.get(s)) {
            prompt = if session.member.is_some() && session.work.is_none() {
                format!(
                    "[equipe: this is your conversation about the project {project} — the whole squad's, and each other \
project's, are conversations of their own. New work is for {project} unless the person says otherwise; what \
should last beyond this conversation goes in the squad's memory.]\n\n{prompt}"
                )
            } else {
                format!("[equipe: the person is looking at the project {project} — new work is for it unless they say otherwise]\n\n{prompt}")
            };
        }
        self.save_waiting(id);
        self.start_turn(id, prompt);
        if let Some(session) = self.sessions.get_mut(&id)
            && session.is_working()
        {
            session.answers_person = true;
        }
    }

    /// Move a member's desk conversation to its thread for the project the person
    /// looks at (the whole squad's when none): the current one is put aside,
    /// the thread's own taken up — or started — so what's said about one
    /// project never fills another's conversation.
    fn enter_thread(&mut self, id: SessionId, wanted: Option<Option<String>>) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        if session.member.is_none() || session.work.is_some() {
            return;
        }
        let wanted = wanted.unwrap_or_else(|| session.squad.as_ref().and_then(|s| self.focus.get(s)).cloned());
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.thread == wanted {
            return;
        }
        let current = DeskThread {
            claude_session: session.claude_session.take(),
            history: std::mem::take(&mut session.history),
            desk_context: session.desk_context.take(),
        };
        let left = std::mem::replace(&mut session.thread, wanted.clone());
        session.threads.insert(left, current);
        let next = session.threads.remove(&wanted).unwrap_or_default();
        session.claude_session = next.claude_session;
        session.history = next.history;
        session.desk_context = next.desk_context;
        session.context = session.desk_context;
        let others: HashMap<String, String> = session
            .threads
            .iter()
            .filter_map(|(project, t)| Some((project.clone().unwrap_or_default(), t.claude_session.clone()?)))
            .collect();
        let resume = session.claude_session.clone();
        let squad = session.squad.clone();
        if let Some(journal) = squad.as_deref().and_then(|s| journal_for(&mut self.journals, &self.config.home, s)) {
            let _ = journal.set_desk_threads(id, wanted.as_deref(), &others);
            let _ = match &resume {
                Some(claude) => journal.set_engine_session(id, claude),
                None => journal.clear_engine_session(id),
            };
        }
        self.publish(Event::Thread { session: id, project: wanted });
    }

    /// Run a turn with `prompt` in the session's current place: its ticket's
    /// project when it has one, its desk otherwise.
    fn start_turn(&mut self, id: SessionId, prompt: String) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.answers_person = false;
        }
        // A desk conversation grown too large, or gone cold, starts afresh.
        let prompt = match self.refresh_desk(id) {
            Some(handoff) => format!("{handoff}\n\n{prompt}"),
            None => prompt,
        };
        // The squad's memory as its files say now — the person's edits in Finder
        // included, its index brought up to date before their changes are saved.
        let facts = self.squad_memory(id);
        self.save_persons_changes(id);
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let squad = session.squad.clone();
        let projects = squad.as_deref().map(|s| self.projects_cached(s)).unwrap_or_default();
        let board_tools = session.member.is_some() && session.engine == Engine::Claude && self.config.equipe_binary.is_some();
        let language = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(Journal::language);
        let edits = session.edits();
        let mode = session.mode.or_else(|| squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::mode)).unwrap_or_default();
        // A browser in any mode that may change things — signed in wherever the
        // person (or a teammate) signed in, if anywhere yet. In Auto and Full
        // it browses freely; in Ask each step comes to the person.
        let wants_browser = board_tools && mode != Mode::ReadOnly;
        let browser = match (self.config.npx.clone(), squad.clone(), session.member.as_ref().map(|m| m.name.clone())) {
            (Some(npx), Some(squad), Some(name)) if wants_browser => {
                // Page snapshots and screenshots: in the temp folder, readable without
                // asking, and never in the project.
                let _ = npx;
                let shown = self.journals.get(&squad).is_some_and(|j| j.browser_shown(&name));
                // The browser it had, if still running and showing as asked; else a new one,
                // which equipe holds open so it outlives each turn.
                self.open_browser(&squad, &name, shown)
            }
            _ => None,
        };
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let browses_freely = browser.is_some() && matches!(mode, Mode::Auto | Mode::Full);
        // Code run in the browser's own process is outside any sandbox: in auto
        // the person sees the code and says yes; in full it runs.
        let ask_tools: Vec<String> =
            if browser.is_some() && mode == Mode::Auto { vec![permissions::BROWSER_CODE.to_owned()] } else { Vec::new() };
        let mcp_config = match (&squad, &session.member, &self.config.equipe_binary) {
            (Some(squad), Some(m), Some(binary)) if board_tools => {
                let mut servers = json!({ MCP_SERVER: {
                    "type": "stdio",
                    "command": binary,
                    "args": ["mcp", "--squad", squad, "--member", m.name, "--role", m.role, "--home", self.config.home],
                }});
                if let Some(port) = browser {
                    servers[browser::SERVER] = browser::entry(port);
                }
                Some(json!({ "mcpServers": servers }).to_string())
            }
            _ => None,
        };
        let home = squad.as_deref().map(|s| squad_home::path(&self.config.home, s));
        let memory = squad_memory::for_charter(&facts);
        let charter = session.member.as_ref().map(|m| {
            let team = squad.as_deref().map(|s| self.team(s)).unwrap_or_default();
            let mates: Vec<Teammate> = team.iter().map(|(_, name, role)| Teammate { name, role }).collect();
            let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
            let home = home.as_ref().map(|h| h.to_string_lossy().into_owned());
            let ctx = roles::Context {
                team: &mates,
                projects: &names,
                board_tools,
                language: language.as_deref(),
                browser: browser.is_some(),
                home: home.as_deref(),
                memory: memory.as_deref(),
                facts: facts.len(),
                edits,
            };
            roles::charter(roles::role_or_custom(&m.role), &m.name, &ctx)
        });
        let (cwd, access, resume, history, ticket, add_dirs) = match &session.work {
            Some(work) => {
                // On a code ticket a member also reads the squad's home; on a task or
                // action, worked in a worktree of the home, it reads every project too.
                let read: Vec<PathBuf> = match &session.member {
                    Some(_) if work.kind.in_home() => projects.iter().chain(home.iter()).cloned().collect(),
                    Some(_) => home.iter().cloned().collect(),
                    None => Vec::new(),
                };
                let writes = (edits || work.kind.in_home()) && work.sandbox.is_some();
                (work.cwd.clone(), access_for(mode, writes), work.claude_session.clone(), work.history.clone(), Some(work.ticket), read)
            }
            None => {
                // At its desk a member works in the squad's home and reads every
                // project of its squad; anything more — a fetch, a write elsewhere —
                // asks the person (and in read-only is refused), never denied unseen.
                let access = if session.member.is_some() { desk_access(mode) } else { plain_access(session.access) };
                let read: Vec<PathBuf> =
                    if session.member.is_some() { projects.iter().filter(|p| **p != session.cwd).cloned().collect() } else { Vec::new() };
                (session.cwd.clone(), access, session.claude_session.clone(), session.history.clone(), None, read)
            }
        };
        // A member in Auto that may write is fenced in by the operating system; what it
        // can't do inside the fence, and anything Ask needs, goes to the person.
        let member_rules = session.member.is_some();
        let fenced = member_rules && access == claude_code::Access::EditFiles && mode == Mode::Auto;
        // Directories added for reading stay read-only in every mode, and no other
        // squad's home is read at all.
        let other_homes = squad.as_deref().map(|s| squad_home::others(&self.config.home, s)).unwrap_or_default();
        let deny: Vec<String> = if member_rules {
            let rule = |tool: &str, dir: &Path| format!("{tool}(/{}/**)", dir.display());
            add_dirs
                .iter()
                .flat_map(|d| [rule("Edit", d), rule("Write", d)])
                .chain(other_homes.iter().flat_map(|d| [rule("Read", d), rule("Edit", d)]))
                .collect()
        } else {
            Vec::new()
        };
        let read_only: Vec<String> = add_dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
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
                // Codex gets no conversation of its own: equipe sends the recent
                // part of it each turn, never more than fits.
                let history = recent_history(&history);
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
                if browses_freely {
                    allowed.push(format!("mcp__{}", browser::SERVER));
                }
                if member_rules {
                    allowed.extend(squad_rules);
                }
                let blocked: Vec<String> =
                    if member_rules { permissions::BLOCKED.iter().map(|r| (*r).to_owned()).collect() } else { Vec::new() };
                let domains: Vec<String> = permissions::ALLOWED_DOMAINS.iter().map(|d| (*d).to_owned()).collect();
                let outward: Vec<String> = permissions::OUTWARD.iter().map(|r| (*r).to_owned()).collect();
                // The person's browser sign-ins and equipe's logins: no fenced command reads them.
                // Nor other squads' homes.
                let private: Vec<String> = [self.config.home.join("browser"), self.config.home.join("credentials")]
                    .iter()
                    .chain(&other_homes)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
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
                        ask_tools: &ask_tools,
                        deny: &deny,
                        sandbox: fenced.then_some(claude_code::Sandbox {
                            allowed_domains: &domains,
                            ask: &outward,
                            deny_read: &private,
                            deny_write: &read_only,
                        }),
                        prompt_tool: prompt_tool.as_deref(),
                    };
                    // A browser just started may still be loading.
                    if let Some(port) = browser {
                        browser::ready(port, Duration::from_secs(90)).await;
                    }
                    let outcome = claude_code::run_turn(&binary, &cwd, request, |activity| match activity {
                        Activity::Text(text) => {
                            streamed = true;
                            emit(Event::TextDelta { session: id, text });
                        }
                        Activity::Tool { name, target } => emit(Event::ToolUse { session: id, tool: name, target }),
                        Activity::Model(model) => emit(Event::ModelKnown { session: id, model }),
                        Activity::Output(tokens) => emit(Event::Output { session: id, tokens }),
                        Activity::SubagentStarted { id: sub, what } => emit(Event::SubagentStarted { session: id, id: sub, what }),
                        Activity::SubagentStep { id: sub, step } => emit(Event::SubagentStep { session: id, id: sub, step }),
                        Activity::SubagentEnded { id: sub } => emit(Event::SubagentEnded { session: id, id: sub }),
                        Activity::Plan { five_hour, seven_day } => {
                            let window = |w: Option<(f64, i64)>| {
                                w.map(|(used, resets_at)| equipe_protocol::PlanWindow {
                                    used_percent: (used * 100.0).round().clamp(0.0, 100.0) as u8,
                                    resets_at,
                                })
                            };
                            emit(Event::PlanUsage {
                                usage: equipe_protocol::PlanUsage { five_hour: window(five_hour), seven_day: window(seven_day) },
                            })
                        }
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

    /// The facts in a member's squad memory, with its index kept up to date.
    fn squad_memory(&self, id: SessionId) -> Vec<squad_memory::Fact> {
        let Some(squad) = self.sessions.get(&id).filter(|s| s.member.is_some()).and_then(|s| s.squad.as_deref()) else {
            return Vec::new();
        };
        let folder = squad_home::path(&self.config.home, squad);
        let facts = squad_memory::facts(&folder);
        let language = self.journals.get(squad).and_then(Journal::language);
        squad_memory::refresh_index(&folder, &facts, language.as_deref());
        facts
    }

    /// Before a member's desk turn: whatever changed in the squad's home while no
    /// member was writing there is the person's own, and is saved as theirs.
    fn save_persons_changes(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id).filter(|s| s.member.is_some() && s.work.is_none()) else {
            return;
        };
        let Some(squad) = session.squad.clone() else {
            return;
        };
        let busy = self.sessions.values().any(|s| s.squad.as_ref() == Some(&squad) && s.work.is_none() && s.is_working());
        if !busy {
            let _ = squad_home::save_persons(&squad_home::path(&self.config.home, &squad));
        }
    }

    /// After a member's desk turn: what it changed in the squad's home is saved
    /// at once, by it and for what the person asked, and the person told — in
    /// words, never git's — that they can take it back.
    fn save_desk_changes(&mut self, id: SessionId, prompt: &str) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else {
            return;
        };
        let dir = squad_home::path(&self.config.home, &squad);
        match squad_home::save(&dir, &name, &asked_for(prompt)) {
            Ok(Some(change)) => self.publish(Event::Notice {
                session: id,
                text: format!("✎ {name} {} in the squad's folder · /home undo takes it back", change.what()),
            }),
            Ok(None) => {}
            Err(err) => self.error(Some(id), format!("The squad's folder couldn't be saved: {err}"), None),
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
        // A browser asked onto the screen during the turn comes up now.
        if let Some(session) = self.sessions.get_mut(&done.session) {
            session.turn = None;
            if let Some(squad) = session.squad.clone() {
                self.apply_browsers(&squad);
            }
        }
        let persist = self.config.persist;
        let Some(session) = self.sessions.get_mut(&done.session) else {
            return;
        };
        session.turn = None;
        let answered_person = std::mem::take(&mut session.answers_person) && done.reply.as_deref().is_some_and(|r| !r.trim().is_empty());
        let squad = session.squad.clone();
        let who = session_member(session);
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
            // What is kept in memory is bounded too; what lasts is in memory files and on the board.
            if history.len() > HISTORY_KEPT {
                history.drain(..history.len() - HISTORY_KEPT);
            }
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
        // The screen is the member's only while its turn runs.
        if let Some(who) = who
            && self.desktop.leave(&who)
        {
            self.publish_desktop();
        }
        if done.ticket.is_none() {
            self.save_desk_changes(done.session, &done.prompt);
        }
        if let Some(reply) = done.reply.filter(|r| !r.trim().is_empty()) {
            let short: String = reply.chars().take(BRIEF_REPLY_CHARS).collect();
            let more = if short.len() < reply.len() { "…" } else { "" };
            self.tell_the_pm(done.session, |name| format!("{name} answered: {short}{more}"), None);
        }
        self.after_turn(done.session).await;
        if answered_person {
            self.suggest_next(done.session);
        }
    }

    /// Guess what the person will say next to the member that just answered
    /// them, from the end of their conversation — unless the member is busy
    /// again, or the person turned guesses off.
    fn suggest_next(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let off = session.squad.as_deref().and_then(|s| self.journals.get(s)).is_some_and(Journal::suggestions_off);
        if off || !self.config.suggest || session.member.is_none() || session.engine != Engine::Claude || session.is_working() {
            return;
        }
        let Some(conversation) = suggestion_context(session.name().unwrap_or("agent"), session.transcript.entries()) else {
            return;
        };
        let binary = self.config.claude_binary.clone();
        // Somewhere with no project of the person's in it: nothing to read, nothing to load.
        let cwd = self.config.home.join("suggest");
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let _ = std::fs::create_dir_all(&cwd);
            if let Some(text) = claude_code::suggest(&binary, &cwd, &conversation).await {
                let _ = internal.send(Internal::Event(Event::Suggestion { session: id, text }));
            }
        });
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
        } else if session.work.as_ref().is_some_and(|w| w.waiting.is_some()) {
            self.end_work(id, WorkEnd::Waiting);
        }
        if let Some(text) = self.sessions.get_mut(&id).and_then(Session::next_memos) {
            self.save_waiting(id);
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
        if let Some(who) = self.sessions.remove(&id).as_ref().and_then(session_member)
            && self.desktop.leave(&who)
        {
            self.publish_desktop();
        }
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
        // Members equipe once brought in for a rush finish what they hold and take nothing new.
        let helpers = self.journals.get(squad).map(Journal::helpers).unwrap_or_default();
        let members: Vec<(String, String, bool)> = self
            .sessions
            .iter()
            .filter(|(id, s)| s.squad.as_deref() == Some(squad) && !helpers.contains(id))
            .filter_map(|(_, s)| s.member.as_ref().map(|m| (m.name.clone(), m.role.clone(), s.is_free())))
            .collect();
        let candidates: Vec<Candidate> =
            members.iter().map(|(name, role, free)| Candidate { name, role, free: *free }).collect();
        for (ticket, name) in board::assignments(&tickets, &candidates, single_project) {
            if let Some(id) = self.member_id(squad, &name) {
                self.begin_work(id, ticket);
            }
        }
    }

    /// Let go the members equipe once brought in for a rush, as each finishes
    /// what it held: nobody is added on equipe's own any more — a member splits
    /// big work across subagents, and the person adds teammates.
    async fn let_helpers_go(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let Some(helpers) = self.journals.get(&squad).map(Journal::helpers) else {
                continue;
            };
            if helpers.is_empty() {
                continue;
            }
            let idle: Vec<SessionId> = helpers
                .iter()
                .copied()
                .filter(|id| {
                    self.sessions.get(id).is_some_and(Session::is_free)
                })
                .collect();
            for id in &idle {
                let name = self.sessions.get(id).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
                Box::pin(self.close(*id)).await;
                if let Some(pm) = self.pm_of(&squad) {
                    self.publish(Event::Notice { session: pm, text: format!("{name} left, its work done: its role carries on") });
                }
            }
            // Gone or let go: no longer helpers.
            let kept: Vec<SessionId> = helpers.into_iter().filter(|id| !idle.contains(id) && self.sessions.contains_key(id)).collect();
            if let Some(journal) = self.journals.get(&squad) {
                let _ = journal.set_helpers(&kept);
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
        let role_edits = session.edits();
        let board_tools = session.engine == Engine::Claude && self.config.equipe_binary.is_some();
        let projects = self.projects(&squad);
        let home_dir = squad_home::path(&home, &squad);
        let Some(journal) = self.journal(&squad) else {
            return;
        };
        let Some(mut ticket) = journal.ticket(ticket_id).ok().flatten() else {
            return;
        };
        let saved = journal.ticket_work(ticket_id).unwrap_or_default();
        let after = work_before(&journal.tickets().unwrap_or_default(), &ticket);
        let fresh = ticket.branch.is_none() || saved.base.is_none();
        let kind = ticket.kind;
        // A task or action is worked in the squad home; what it is about, if anything, is only read.
        let Some(project) = (if kind.in_home() { ticket.project.clone().or_else(|| Some(home_dir.clone())) } else { ticket.project.clone().or_else(|| projects.first().cloned()) })
        else {
            return;
        };
        let workplace = if kind.in_home() { home_dir.clone() } else { project.clone() };
        // In the squad home every role writes, in its own worktree.
        let edits = role_edits || kind.in_home();

        let sandbox = if edits {
            let checkout = worktree::repo_root(&workplace).and_then(|repo| {
                let id_part = format!("t{ticket_id}-{:x}", now_secs());
                match (&saved.worktree, &ticket.branch, &saved.base) {
                    // In review and sent back: the worktree is still there.
                    (Some(path), Some(branch), Some(base)) if path.exists() => {
                        Ok(Sandbox { repo, path: path.clone(), branch: branch.clone(), base: base.clone() })
                    }
                    // Its worktree went away: check its branch out again.
                    (_, Some(branch), Some(base)) => worktree::resume(&repo, &home, &id_part, branch, base),
                    // New: from HEAD, with the finished work it depends on.
                    _ => worktree::create_after(&repo, &home, &id_part, &name, &after),
                }
            });
            match checkout {
                Ok(sandbox) => Some(sandbox),
                Err(err) => return self.cannot_start(&squad, ticket, &name, &err),
            }
        } else {
            None
        };
        let cwd = sandbox.as_ref().map_or_else(|| project.clone(), |s| within(s, &project));

        ticket.status = TicketStatus::InProgress;
        ticket.assignee = Some(name.clone());
        ticket.branch = sandbox.as_ref().map(|s| s.branch.clone()).or(ticket.branch);
        ticket.worktree = sandbox.as_ref().map(|s| s.path.clone());
        let reopened = ticket.note.as_ref().filter(|n| n.starts_with("Reopened") || n.starts_with("Resumed")).cloned();
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
                kind,
                project: project.clone(),
                cwd,
                sandbox: sandbox.clone(),
                claude_session: saved.engine_session,
                history: Vec::new(),
                handed_in: None,
                waiting: None,
            });
        }
        self.publish(Event::TicketChanged { squad: squad.clone(), ticket: ticket.clone() });
        // What follows in its conversation is about the ticket's project.
        self.publish(Event::Thread { session: id, project: ticket.project.as_deref().map(board::project_name) });
        self.publish(Event::WorkStarted {
            session: id,
            ticket: ticket_id,
            title: ticket.title.clone(),
            project: project.clone(),
            workspace: sandbox.as_ref().map(workspace_of),
        });
        self.brief_pm(&squad, format!("{name} picked up T-{ticket_id} ({}).", ticket.title), None);
        let language = self.journal(&squad).and_then(Journal::language);
        let mut prompt = work_prompt(&ticket, &project, sandbox.as_ref(), reopened.as_deref(), board_tools, language.as_deref());
        if sandbox.is_some() && !after.is_empty() && fresh {
            let ids: Vec<&str> = after.iter().map(|(what, _)| what.as_str()).collect();
            prompt.push_str(&format!(
                "\nYour branch starts from the accepted work of {}, which may not be on main yet — build on it; don't merge it again.",
                ids.join(", ")
            ));
        }
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
        let summary = work.handed_in.clone().or_else(|| work.waiting.clone());
        let mut kept_worktree = None;
        // A task or action accepted goes into the squad's folder now; `Err` when the folder changed under it.
        let mut landed: Option<Result<(), String>> = None;
        if let Some(sandbox) = &work.sandbox {
            let body = summary.clone().unwrap_or_else(|| ticket.body.clone());
            let message =
                format!("equipe: T-{} {}\n\n{body}\n\nRun by {name} on equipe through Claude Code.", ticket.id, ticket.title);
            if let Err(err) = sandbox.commit_all(&message) {
                let message = format!("couldn't commit T-{}: {err} — it is still in {}", ticket.id, sandbox.path.display());
                self.error(Some(id), message, None);
            }
            if matches!(how, WorkEnd::HandedIn | WorkEnd::Waiting) {
                kept_worktree = Some(sandbox.path.clone());
            } else if how == WorkEnd::Accepted && ticket.kind.in_home() {
                let result = self.land_in_home(&squad, &mut ticket, sandbox);
                if result.is_err() {
                    kept_worktree = Some(sandbox.path.clone());
                }
                landed = Some(result);
            } else {
                ticket.branch = release(sandbox);
            }
            ticket.worktree = kept_worktree.clone();
        }
        let handed_in = match (&how, &summary) {
            (WorkEnd::HandedIn, Some(summary)) if ticket.kind.in_home() => Some(self.home_hand_in(&squad, &ticket, work.sandbox.as_ref(), summary)),
            _ => summary.clone(),
        };
        let (by, event, line) = match how {
            WorkEnd::HandedIn => {
                ticket.status = TicketStatus::Review;
                ticket.note = handed_in;
                (name.as_str(), "handed in", format!("{name} handed in T-{} for your review", ticket.id))
            }
            WorkEnd::Waiting => {
                ticket.status = TicketStatus::Waiting;
                ticket.note = summary.clone();
                (name.as_str(), "waiting", format!("T-{} waits for you — {name} needs something only you can do", ticket.id))
            }
            WorkEnd::Accepted if matches!(landed, Some(Err(_))) => {
                sent_back_to_update(&mut ticket, &name);
                ("equipe", "sent back", format!("T-{} went back to {name}: the squad's folder changed under it", ticket.id))
            }
            WorkEnd::Accepted => {
                ticket.status = TicketStatus::Done;
                ticket.merged = merged_state(&ticket);
                let line = if ticket.kind.in_home() {
                    format!("You accepted T-{}; its work is in the squad's folder", ticket.id)
                } else {
                    format!("You accepted T-{}", ticket.id)
                };
                ("you", "accepted", line)
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
        // Back at its desk: the thread it was in there.
        let desk = self.sessions.get(&id).and_then(|s| s.thread.clone());
        self.publish(Event::Thread { session: id, project: desk });
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

    /// A task's or action's work, accepted, into the squad home's main: the
    /// person's own changes there are saved first, then the branch merges — no
    /// question, no git words — and its worktree and branch go. A conflict merges
    /// nothing and keeps both.
    fn land_in_home(&mut self, squad: &str, ticket: &mut Ticket, sandbox: &Sandbox) -> Result<(), String> {
        // What changed there while no member was writing at its desk is the person's.
        let desk_busy = self.sessions.values().any(|s| s.squad.as_deref() == Some(squad) && s.work.is_none() && s.is_working());
        if !desk_busy {
            let _ = squad_home::save_persons(&sandbox.repo);
        }
        let message = format!("{}: T-{} {}", ticket.assignee.as_deref().unwrap_or("equipe"), ticket.id, ticket.title);
        if let Err(err) = worktree::merge_into_main(&sandbox.repo, &sandbox.branch, &message) {
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket.id, "equipe", "not merged", &err);
            }
            return Err(err);
        }
        let _ = sandbox.discard();
        ticket.branch = None;
        ticket.worktree = None;
        Ok(())
    }

    /// What a task or action hands in for review: the member's summary, the files
    /// it leaves in the squad's folder, and — for an action — what the person
    /// allowed or declined on the way.
    fn home_hand_in(&mut self, squad: &str, ticket: &Ticket, sandbox: Option<&Sandbox>, summary: &str) -> String {
        let mut out = summary.trim().to_owned();
        let files = sandbox.and_then(|s| s.changed_files().ok()).unwrap_or_default();
        if !files.is_empty() {
            out.push_str("\n\nIn the squad's folder:");
            for file in &files {
                out.push_str(&format!("\n- {file}"));
            }
        }
        // A task's findings say where they came from: the links its files cite
        // that the summary doesn't already, or that they cite none.
        if ticket.kind == TicketKind::Task
            && !files.is_empty()
            && let Some(sandbox) = sandbox
        {
            let cited = squad_home::cited_sources(&sandbox.path, &files);
            let new: Vec<&String> = cited.iter().filter(|link| !summary.contains(link.as_str())).collect();
            if !new.is_empty() {
                out.push_str("\n\nSources its files cite:");
                for link in new {
                    out.push_str(&format!("\n- {link}"));
                }
            } else if cited.is_empty() && !summary.contains("http") {
                out.push_str("\n\nIts files cite no source — check before relying on them.");
            }
        }
        if ticket.kind == TicketKind::Action {
            let history = self.journal(squad).and_then(|j| j.ticket_history(ticket.id).ok()).unwrap_or_default();
            let steps: Vec<String> = history
                .iter()
                .filter(|(by, _, _)| by == "you")
                .filter_map(|(_, what, action)| match what.as_str() {
                    "allowed" | "always allowed" => Some(format!("- ✓ {action}")),
                    "declined" => Some(format!("- ✗ {action}")),
                    _ => None,
                })
                .collect();
            if !steps.is_empty() {
                out.push_str(&format!("\n\nWhat you approved on the way:\n{}", steps.join("\n")));
            }
        }
        out
    }

    /// Close a ticket no member holds right now (in review, or not started).
    fn close_ticket(&mut self, squad: &str, mut ticket: Ticket, status: TicketStatus) {
        let saved = self.journal(squad).and_then(|j| j.ticket_work(ticket.id).ok()).unwrap_or_default();
        let workplace = if ticket.kind.in_home() { Some(squad_home::path(&self.config.home, squad)) } else { ticket.project.clone() };
        if let (Some(path), Some(branch), Some(base), Some(workplace)) =
            (&saved.worktree, &ticket.branch, &saved.base, &workplace)
            && let Ok(repo) = worktree::repo_root(workplace)
        {
            let sandbox = Sandbox { repo, path: path.clone(), branch: branch.clone(), base: base.clone() };
            if status == TicketStatus::Done && ticket.kind.in_home() {
                let _ = sandbox.commit_all(&format!("equipe: T-{} {}", ticket.id, ticket.title));
                if self.land_in_home(squad, &mut ticket, &sandbox).is_err() {
                    let name = ticket.assignee.clone().or(ticket.owner.clone()).unwrap_or_default();
                    sent_back_to_update(&mut ticket, &name);
                    let note = ticket.note.clone().unwrap_or_default();
                    if let Some(journal) = self.journal(squad) {
                        let _ = journal.save_ticket(&ticket);
                        let _ = journal.ticket_event(ticket.id, "equipe", "sent back", &note);
                    }
                    let line = format!("T-{} went back to {name}: the squad's folder changed under it · {}.", ticket.id, ticket.title);
                    self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
                    self.brief_pm(squad, line.clone(), Some(line));
                    return;
                }
            } else {
                ticket.branch = release(&sandbox);
            }
        }
        ticket.status = status;
        ticket.worktree = None;
        if status == TicketStatus::Done {
            ticket.merged = merged_state(&ticket);
        }
        let verb = if status == TicketStatus::Done { "accepted" } else { "cancelled" };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.set_ticket_work(ticket.id, &TicketWork { worktree: None, ..saved });
            let _ = journal.ticket_event(ticket.id, "you", verb, "");
        }
        let landed = if status == TicketStatus::Done && ticket.kind.in_home() { "; its work is in the squad's folder" } else { "" };
        let line = format!("You {verb} T-{}{landed} · {}.", ticket.id, ticket.title);
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

    /// The person merges a done ticket's branch into main — their gate, never a
    /// member's — and pushes main when they said so. A conflict merges nothing.
    fn merge(&mut self, squad: &str, ticket: TicketId, push: bool) {
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status != TicketStatus::Done {
            return self.squad_error(squad, format!("T-{ticket} isn't accepted yet — /accept T-{ticket} first."));
        }
        if found.kind.in_home() {
            return self.squad_error(squad, format!("T-{ticket}'s work went into the squad's folder when you accepted it."));
        }
        let (Some(branch), Some(repo)) =
            (found.branch.clone(), found.project.as_deref().and_then(|p| worktree::repo_root(p).ok()))
        else {
            return self.squad_error(squad, format!("T-{ticket} has no branch to merge."));
        };
        let message = format!("equipe: merge T-{ticket} {}\n\nAccepted by the person; merged from {branch}.", found.title);
        let main = match worktree::merge_into_main(&repo, &branch, &message) {
            Ok(main) => main,
            Err(err) => return self.squad_error(squad, format!("T-{ticket} wasn't merged: {err}")),
        };
        let mut line = format!("You merged T-{ticket} · {} into {main}", found.title);
        if push {
            match worktree::push_main(&repo) {
                Ok(_) => line.push_str(" and pushed it"),
                Err(err) => self.squad_error(squad, format!("{main} wasn't pushed: {err}")),
            }
        }
        found.merged = Some(true);
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "merged", &main);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        // Work it was built on came along with it.
        for mut other in self.tickets(squad).into_iter().filter(|t| t.merged == Some(false)) {
            if other.status == TicketStatus::Done && merged_state(&other) == Some(true) {
                other.merged = Some(true);
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.save_ticket(&other);
                }
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: other });
            }
        }
        let line = format!("{line}.");
        self.brief_pm(squad, line.clone(), Some(line));
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

    /// The person did what a `waiting` ticket needed: it goes back to the
    /// member that put it down, in the same worktree and conversation.
    async fn resume(&mut self, squad: &str, ticket: TicketId, note: String) {
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status != TicketStatus::Waiting {
            let status = found.status.label();
            return self.squad_error(squad, format!("T-{ticket} isn't waiting on you — it is {status}."));
        }
        let found = self.resumed(squad, found, "you", &note);
        let line = format!("You resumed T-{} · {}", found.id, found.title);
        self.brief_pm(squad, line.clone(), Some(line));
        self.dispatch(squad).await;
    }

    /// `ticket`, waiting, back to `todo` for the member that put it down,
    /// saved and announced. What was done (`note`) is what the member goes on from.
    fn resumed(&mut self, squad: &str, mut ticket: Ticket, by: &str, note: &str) -> Ticket {
        let note = match note.trim() {
            "" => "Resumed: the person did what you were waiting for.".to_owned(),
            note => format!("Resumed: {note}"),
        };
        let present: Vec<String> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad))
            .filter_map(|s| s.name().map(str::to_owned))
            .collect();
        back_to_its_member(&mut ticket, &present, &note);
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.ticket_event(ticket.id, by, "resumed", &note);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: ticket.clone() });
        ticket
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
        if call.name == board::ASK_PERSON {
            return self.ask_person(call, reply);
        }
        if call.name == board::SHOW_BROWSER {
            let url = call.arguments["url"].as_str().map(str::to_owned);
            let why = call.arguments["why"].as_str().unwrap_or_default().to_owned();
            if let Some(url) = url {
                self.show_url.insert((call.squad.clone(), call.member.clone()), url);
            }
            self.show_browser(&call.squad, Some(call.member.clone()), true);
            if let Some(id) = self.member_id(&call.squad, &call.member) {
                self.publish(Event::Notice { session: id, text: format!("🖥 {}'s browser comes up on this Mac's screen: {why}", call.member) });
            }
            let _ = reply.send(ToolResult {
                text: "Your browser comes up on the person's screen when this turn ends, at that page. Tell them what to do \
there, end your turn, and carry on when they say they're done."
                    .into(),
                is_error: false,
            });
            return;
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
        if let Some(why) = self.off_limits(id, &call.squad, &tool, &action, &input) {
            let _ = reply.send(verdict(Decision::Deny, &input, why));
            self.publish(Event::Notice { session: id, text: format!("✗ {why} Refused: {action}") });
            self.audit(&call.squad, ticket, "equipe", "refused", &action);
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
            s.mode.unwrap_or(squad_mode) == Mode::Auto && s.work.as_ref().is_none_or(|w| w.sandbox.is_some())
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
        let approval = Approval { id: approval_id, session: id, member: call.member.clone(), ticket, tool, action, reason, preview: None };
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

    /// What equipe refuses a member whatever its mode or the person's rules:
    /// reading another squad's home, and writing where it only reads — the
    /// squad's projects at its desk, the squad's home on a ticket.
    fn off_limits(&mut self, id: SessionId, squad: &str, tool: &str, action: &str, input: &Value) -> Option<&'static str> {
        let session = self.sessions.get(&id)?;
        let inside = |dir: &Path| {
            let dir = dir.to_string_lossy();
            action == dir || action.contains(&format!("{dir}/"))
        };
        if squad_home::others(&self.config.home, squad).iter().any(|d| inside(d)) {
            return Some("Another squad's folder is private.");
        }
        let at_desk = session.work.is_none();
        let in_home = session.work.as_ref().is_some_and(|w| w.kind.in_home());
        let mut read_only: Vec<PathBuf> = if at_desk || in_home { self.projects_cached(squad) } else { Vec::new() };
        if !at_desk {
            read_only.push(squad_home::path(&self.config.home, squad));
        }
        let writes = match tool {
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => true,
            "Bash" => !permissions::only_reads(tool, action, input),
            _ => false,
        };
        // A ticket's own worktree may sit under a read-only directory's name; it is never inside one.
        let own = session.work.as_ref().map(|w| w.cwd.to_string_lossy().into_owned());
        let in_own = own.is_some_and(|own| action.starts_with(&format!("{own}/")) || action == own);
        if writes && !in_own && read_only.iter().any(|d| inside(d)) {
            return Some(if at_desk {
                "At the desk the squad's projects are only read: code changes on a ticket, in its worktree."
            } else if in_home {
                "On a task or action the squad's projects and its folder are only read: write in your own worktree."
            } else {
                "On a ticket the squad's folder is only read."
            });
        }
        None
    }

    /// A member asks the person's yes before acting in their name — signing up,
    /// accepting terms, paying. Put to the person in every mode, never made a rule.
    fn ask_person(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let text = |key: &str| call.arguments.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
        let (action, why) = (text("action"), text("why"));
        let Some(id) = self.member_id(&call.squad, &call.member).filter(|_| !action.is_empty()) else {
            let _ = reply.send(ToolResult { text: "Say what you'll do in `action`.".into(), is_error: true });
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval {
            id: approval_id,
            session: id,
            member: call.member.clone(),
            ticket,
            tool: CONFIRM.to_owned(),
            action,
            reason: (!why.is_empty()).then_some(why),
            preview: None,
        };
        self.pending.insert(approval_id, Pending { squad: call.squad.clone(), approval: approval.clone(), input: json!({}), reply });
        self.publish(Event::ApprovalRequested { squad: call.squad.clone(), approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// A member's input on the desktop, before the helper makes it. In a
    /// messaging app, what may send in the person's name waits for their yes —
    /// every time, never a rule; some screens take no input at all. The rules
    /// are [`desktop_gate`]'s.
    fn gate_desktop(&mut self, check: desktop_gate::Check, reply: oneshot::Sender<ToolResult>) {
        let refuse = |text: String| ToolResult { text, is_error: true };
        let Some(id) = self.member_id(&check.squad, &check.member) else {
            let _ = reply.send(refuse("Unknown member.".into()));
            return;
        };
        let act = Act::of(&check.tool, &check.input);
        let apps = self.journal(&check.squad).map(Journal::messaging_apps).unwrap_or_default();
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let action = desktop_gate::describe(&act, &check.scene);
        let why = match desktop_gate::judge(&act, &check.scene, &apps) {
            Verdict::Free => {
                let _ = reply.send(ToolResult { text: String::new(), is_error: false });
                return;
            }
            Verdict::Blocked(why) => {
                let _ = reply.send(refuse(format!("{why} Ask the person to do it.")));
                self.publish(Event::Notice { session: id, text: format!("✗ equipe blocked: {}", first_line(&action)) });
                self.audit(&check.squad, ticket, "equipe", "blocked", &action);
                return;
            }
            Verdict::Ask(why) => why,
        };
        let said = check.input.get("why").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
        if said.is_empty() {
            let _ = reply.send(refuse(format!(
                "This may send in the person's name ({why} in {}), so the person says yes first. \
                 Call it again with `why`: what you're sending, and to whom, in a line.",
                check.scene.app.name
            )));
            return;
        }
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let preview = check.screen.as_deref().and_then(|png| save_preview(approval_id, png));
        let approval = Approval {
            id: approval_id,
            session: id,
            member: check.member.clone(),
            ticket,
            tool: desktop_gate::SEND.to_owned(),
            action,
            reason: Some(said),
            preview,
        };
        self.pending.insert(approval_id, Pending { squad: check.squad.clone(), approval: approval.clone(), input: check.input, reply });
        self.publish(Event::ApprovalRequested { squad: check.squad, approval });
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
        if let Some(preview) = &approval.preview {
            let _ = std::fs::remove_file(preview);
        }
        let _ = reply.send(if approval.tool == desktop_gate::SEND {
            // The desktop tool waits on this: nothing reaches the screen on a no.
            match decision {
                Decision::Allow | Decision::Always => ToolResult { text: String::new(), is_error: false },
                Decision::Deny => ToolResult { text: format!("Not sent: {why} Leave the draft as it is and say so."), is_error: true },
            }
        } else if approval.tool == CONFIRM {
            // A question the member asked itself: an answer in words, not a permission verdict.
            let text = match decision {
                Decision::Allow | Decision::Always => format!("The person said yes. Go ahead: {}", approval.action),
                Decision::Deny => format!("The person said no ({why}). Don't do it; carry on without it, or say what you need."),
            };
            ToolResult { text, is_error: false }
        } else {
            verdict(decision, &input, why)
        });
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

    /// Members' browsers on screen or hidden, from their next turn.
    fn show_browser(&mut self, squad: &str, member: Option<String>, show: bool) {
        let team: Vec<String> = self.team(squad).into_iter().map(|(_, name, _)| name.to_owned()).collect();
        if let Some(name) = member.as_deref().filter(|name| !team.iter().any(|t| t == name)) {
            return self.squad_error(squad, format!("No one called {name} on the squad."));
        }
        if let Some(journal) = self.journal(squad) {
            // One window to touch at a time: showing one member's hides the rest.
            if show && member.is_some() {
                let _ = journal.show_browser(None, false, &team);
            }
            let _ = journal.show_browser(member.as_deref(), show, &team);
        }
        self.apply_browsers(squad);
    }

    /// Bring each of `squad`'s browsers to how it should show — on screen or
    /// not — now, for members between turns; a member mid-turn keeps its own
    /// until the turn ends (its turn is using it).
    fn apply_browsers(&mut self, squad: &str) {
        let members: Vec<(SessionId, String, bool)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .filter_map(|(id, s)| Some((*id, s.name()?.to_owned(), s.is_working())))
            .collect();
        for (_, name, working) in members {
            let shown = self.journals.get(squad).is_some_and(|j| j.browser_shown(&name));
            let key = (squad.to_owned(), name.clone());
            let fits = self.browsers.get_mut(&key).is_none_or(|b| b.fits(shown));
            if working || (fits && !(shown && !self.browsers.contains_key(&key))) {
                continue;
            }
            // Hidden: its window closes; it opens again hidden when next used.
            self.browsers.remove(&key);
            if shown {
                self.open_browser(squad, &name, true);
            }
        }
    }

    /// Start a member's browser now — on screen when `shown` — and take it to
    /// the page asked for, or where it last was.
    fn open_browser(&mut self, squad: &str, name: &str, shown: bool) -> Option<u16> {
        let npx = self.config.npx.clone()?;
        let output = std::env::temp_dir().join("equipe-browser").join(squad).join(name);
        let profile = browser::profile(&self.config.home, squad, name);
        let key = (squad.to_owned(), name.to_owned());
        if !self.browsers.get_mut(&key).is_some_and(|b| b.fits(shown)) {
            let running = browser::launch(&npx, &self.config.home, &profile, &output, shown).ok()?;
            let (client, port) = (self.client.clone(), running.port);
            let page = self.show_url.remove(&key).or_else(|| self.last_url.get(&key).cloned()).filter(|_| shown);
            tokio::spawn(async move {
                if browser::hold(&client, port).await
                    && let Some(page) = page
                {
                    browser::navigate(&client, port, &page).await;
                }
            });
            self.browsers.insert(key.clone(), running);
        }
        self.browsers.get_mut(&key).map(|b| {
            b.last_used = std::time::Instant::now();
            b.port
        })
    }

    /// A picture of each watched member's browser, one under way at a time.
    fn take_frames(&mut self) {
        let watched: Vec<(String, SessionId)> = self.watching.iter().map(|(s, id)| (s.clone(), *id)).collect();
        for (squad, id) in watched {
            if self.framing.contains(&id) {
                continue;
            }
            let Some(name) = self.sessions.get(&id).and_then(|s| s.name().map(str::to_owned)) else { continue };
            let Some(port) = self.browsers.get(&(squad.clone(), name.clone())).map(|b| b.port) else { continue };
            self.framing.insert(id);
            let (client, internal) = (self.client.clone(), self.internal.clone());
            let output = std::env::temp_dir().join("equipe-browser").join(&squad).join(&name);
            tokio::spawn(async move {
                let frame = browser::screenshot(&client, port).await;
                // Its screenshots land beside the member's: those are equipe's to clear.
                clear_frames(&output);
                let _ = internal.send(Internal::Frame(id, frame));
            });
        }
    }

    fn set_mode(&mut self, squad: &str, member: Option<String>, mode: Option<Mode>) {
        match &member {
            None => {
                let Some(mode) = mode else {
                    return self.squad_error(squad, "Name a mode: read-only, ask, auto or full.");
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
                let kind = kind_arg(&args)?;
                let named = match board::arg_str(&args, "project") {
                    Some(arg) => Some(board::resolve_project(arg, &projects).ok_or_else(|| no_project(arg))?),
                    None => None,
                };
                let project = match (kind, named) {
                    (_, Some(project)) => Some(project),
                    (Some(kind), None) if kind.in_home() => None,
                    // A code ticket needs its repository; left out, the squad's only one.
                    (Some(_), None) => Some(code_project(&projects, &names)?),
                    // No kind, no project: the only project means code there, otherwise it is a task.
                    (None, None) if projects.len() == 1 => projects.first().cloned(),
                    (None, None) => None,
                };
                let kind = kind.unwrap_or_else(|| board::default_kind(project.as_deref()));
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
                    kind,
                    owner: owner.clone(),
                    assignee: None,
                    project,
                    depends_on,
                    branch: None,
                    worktree: None,
                    merged: None,
                    created_by: member.clone(),
                    note: None,
                };
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let new_id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(new_id, &member, "created", title);
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket: Ticket { id: new_id, ..ticket } });
                let mut reply = format!("Created T-{new_id}.");
                if kind.in_home() {
                    reply = format!("Created T-{new_id} ({}).", kind.label());
                }
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
                let present: Vec<String> = self
                    .sessions
                    .values()
                    .filter(|s| s.squad.as_deref() == Some(squad.as_str()))
                    .filter_map(|s| s.name().map(str::to_owned))
                    .collect();
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let mut ticket = journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
                if !matches!(ticket.status, TicketStatus::Proposed | TicketStatus::Todo | TicketStatus::Waiting) {
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
                if let Some(kind) = kind_arg(&args)?.filter(|k| *k != ticket.kind) {
                    // Its work lives where its kind put it: a started ticket keeps its kind.
                    if ticket.branch.is_some() {
                        return Err(format!("T-{ticket_id} has work under way as {} — it keeps its kind; cancel it and make a new one.", ticket.kind.label()));
                    }
                    if !kind.in_home() && ticket.project.is_none() {
                        ticket.project = Some(code_project(&projects, &names)?);
                    }
                    ticket.kind = kind;
                }
                if let Some(ids) = board::arg_ids(&args, "depends_on") {
                    ticket.depends_on = ids;
                }
                let waiting = ticket.status == TicketStatus::Waiting;
                match board::arg_str(&args, "status") {
                    // Back to the member that put it down, where it left off.
                    Some("todo") if waiting => {
                        back_to_its_member(&mut ticket, &present, "Resumed: the PM says what you were waiting for is done.");
                    }
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
                work.waiting = None;
                Ok(format!(
                    "Handed in T-{}. Your changes are committed when this turn ends and the person reviews them. \
End your turn now with a short summary.",
                    work.ticket
                ))
            }
            board::BLOCK_ON_PERSON => {
                let needs = board::arg_strs(&args, "needs");
                if needs.is_empty() {
                    return Err("List what the person has to do, one thing per item.".into());
                }
                let note = board::waiting_note(&needs, board::arg_str(&args, "why"));
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let work = session.work.as_mut().ok_or("You're not on a ticket — there is nothing to put down.")?;
                if work.handed_in.is_some() {
                    return Err(format!("T-{} is already handed in.", work.ticket));
                }
                work.waiting = Some(note);
                Ok(format!(
                    "T-{} waits for the person when this turn ends; your changes are kept in your worktree and you get \
it back there once they have done it. End your turn now with a short line on what you need.",
                    work.ticket
                ))
            }
            board::REMEMBER | board::FORGET => {
                let folder = squad_home::path(&self.config.home, &squad);
                let language = self.journals.get(&squad).and_then(Journal::language);
                let (change, text) = if name == board::REMEMBER {
                    let fact = squad_memory::Remember {
                        name: board::arg_str(&args, "name").unwrap_or_default(),
                        description: board::arg_str(&args, "description").unwrap_or_default(),
                        kind: board::arg_str(&args, "kind").unwrap_or_default(),
                        body: board::arg_str(&args, "body").unwrap_or_default(),
                        confirmed: args.get("confirmed").and_then(Value::as_bool).unwrap_or(false),
                    };
                    let today = squad_memory::date(now_secs());
                    let (change, new) = squad_memory::remember(&folder, &member, &fact, &today, language.as_deref())?;
                    let verb = if new { "remembered" } else { "updated" };
                    let text = format!("✎ {member} {verb} for the squad: {} · /memory shows what the squad knows", clip(fact.description.trim(), 80));
                    (change, text)
                } else {
                    let fact = board::arg_str(&args, "name").ok_or("Which fact? Give its name.")?;
                    let change = squad_memory::forget(&folder, &member, fact, language.as_deref())?;
                    (change, format!("✎ {member} forgot {fact} from the squad's memory · /home undo takes it back"))
                };
                self.publish(Event::Notice { session: id, text });
                Ok(format!("Saved in the squad's memory ({}). Every member has it from their next turn.", change.what()))
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
                    context: s.context,
                    recent: entries[entries.len().saturating_sub(SNAPSHOT_LINES)..].to_vec(),
                    queued: s.queued.clone(),
                    queued_project: s.memo_threads.first().cloned().flatten(),
                    queued_in: s.memo_threads.clone(),
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
        let scale = self.journals.get(squad).map(Journal::scale);
        let suggestions_off = self.journals.get(squad).is_some_and(Journal::suggestions_off);
        Snapshot {
            squad: squad.to_owned(),
            projects,
            sessions,
            tickets,
            mode,
            approvals,
            scale,
            suggestions_off,
            plan: self.plan,
            desktop: self.desktop.state(),
        }
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
            // A squad from before squad homes gets its home now.
            let home = squad_home::path(&self.config.home, &squad);
            if let Err(err) = squad_home::ensure(&self.config.home, &squad, journal.language().as_deref()) {
                eprintln!("equipe: {squad}'s folder couldn't be made: {err}");
            }
            // Members of one role once each kept their own lessons; now they share them.
            if let Err(err) = journal::fold_member_memories(&self.config.home, &squad) {
                eprintln!("equipe: {squad}'s members' memories couldn't be gathered: {err}");
            }
            let projects = journal.projects().unwrap_or_default();
            let tickets = journal.tickets().unwrap_or_default();
            for mut record in journal.open_sessions().unwrap_or_default() {
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
                let edits = record.access == Access::EditFiles;
                let work = tickets
                    .iter()
                    .find(|t| t.status == TicketStatus::InProgress && t.assignee.as_deref() == Some(member.name.as_str()))
                    .and_then(|ticket| restore_work(&journal, ticket, &projects, &home, edits));
                // A desk opened in a project moves to the home. Claude Code resumes a
                // conversation only where it began, so the desk starts afresh there,
                // from a summary of the last one — once.
                let mut refresh = None;
                if record.cwd != home {
                    let _ = journal.set_cwd(record.id, &home);
                    record.cwd = home.clone();
                    if record.engine == Engine::Claude && record.engine_session.is_some() {
                        refresh = Some(true);
                    }
                }
                let memory = Some(journal::member_memory_dir(&self.config.home, &squad, &member.role));
                let queued = journal.waiting(record.id, "memo").unwrap_or_default();
                let mut memo_threads: Vec<Option<String>> = journal
                    .waiting(record.id, "memo-threads")
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| Some(p).filter(|p| !p.is_empty()))
                    .collect();
                // From before memos kept a project each: one for them all.
                if memo_threads.len() != queued.len() {
                    let all = journal.waiting(record.id, "memo-thread").unwrap_or_default().pop().filter(|p| !p.is_empty());
                    memo_threads = vec![all; queued.len()];
                }
                // The PM's other conversations, by project, as they were left.
                let (thread, others) = journal.desk_threads(record.id);
                let threads: HashMap<Option<String>, DeskThread> = others
                    .into_iter()
                    .map(|(project, id)| {
                        let key = (!project.is_empty()).then_some(project);
                        (key, DeskThread { claude_session: Some(id), ..DeskThread::default() })
                    })
                    .collect();
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
                        context: None,
                        desk_context: None,
                        last_turn: None,
                        refresh,
                        transcript: Transcript::from_entries(conversation),
                        turn: None,
                        queued,
                        memo_threads,
                        memo_thread: None,
                        briefing: journal.waiting(record.id, "briefing").unwrap_or_default(),
                        memory,
                        mode: record.mode,
                        thread,
                        threads,
                        answers_person: false,
                    },
                );
            }
            self.journals.insert(squad, journal);
        }
    }

    /// Members that were on a ticket when the daemon stopped carry on with it —
    /// a squad keeps working through a restart. (Codex members wait for their
    /// next message: their login loads on first use.)
    fn resume_work(&mut self) -> Vec<(SessionId, String)> {
        let on_tickets: Vec<(SessionId, TicketId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.engine == Engine::Claude)
            .filter_map(|(&id, s)| s.work.as_ref().map(|w| (id, w.ticket)))
            .collect();
        // Memos for a turn the restart ended are handed back to be sent; a
        // member back on its ticket gets them when that turn ends, as before.
        let idle: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.engine == Engine::Claude && s.work.is_none() && !s.queued.is_empty())
            .map(|(&id, _)| id)
            .collect();
        let mut memos = Vec::new();
        for id in idle {
            if let Some(text) = self.sessions.get_mut(&id).and_then(Session::next_memos) {
                memos.push((id, text));
            }
            self.save_waiting(id);
        }
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
        memos
    }

    /// A member's desk conversation (Claude Code) starts afresh when asked, when
    /// it has filled most of its context window, or when it has gone cold —
    /// unused past the prompt cache's hour, where carrying it on would make the
    /// whole of it be processed again. The fresh conversation is handed a short
    /// summary of the last one's end; the member's memory and the board carry
    /// what lasts. Returns that hand-off, to open the next prompt with.
    fn refresh_desk(&mut self, id: SessionId) -> Option<String> {
        let session = self.sessions.get_mut(&id)?;
        if session.work.is_some() || session.engine != Engine::Claude || session.claude_session.is_none() {
            return None;
        }
        let (used, window) = session.desk_context.unwrap_or((0, 0));
        let full = window > 0 && used * 100 / window >= REFRESH_FULL_PERCENT;
        let cold = used >= REFRESH_COLD_TOKENS && session.last_turn.is_some_and(|t| t.elapsed() > REFRESH_COLD_AFTER);
        let summary = match session.refresh.take() {
            Some(summary) => summary,
            None if full || cold => true,
            None => return None,
        };
        session.claude_session = None;
        session.desk_context = None;
        session.context = None;
        let tail: Vec<String> = session
            .transcript
            .entries()
            .iter()
            .filter_map(|e| match e {
                Entry::User { text } => Some(format!("- the person: {}", clip(text, 600))),
                Entry::Reply { text } => Some(format!("- you: {}", clip(text, 600))),
                _ => None,
            })
            .collect();
        let tail = tail[tail.len().saturating_sub(REFRESH_TAIL)..].join("\n");
        let why = match (full, cold) {
            _ if !summary => "asked for a clean start".to_owned(),
            (true, _) => format!("the last one had filled {}k of {}k tokens", used / 1000, window / 1000),
            (_, true) => format!("the last one ({}k tokens) had gone unused for over an hour", used / 1000),
            _ => "asked for one".to_owned(),
        };
        let squad = session.squad.clone();
        if let Some(journal) = squad.as_deref().and_then(|s| journal_for(&mut self.journals, &self.config.home, s)) {
            let _ = journal.clear_engine_session(id);
        }
        self.publish(Event::Notice { session: id, text: format!("── a fresh conversation: {why} ──") });
        Some(if summary && !tail.is_empty() {
            format!(
                "[equipe: this is a fresh conversation — {why}. Where the last one left off, latest last:\n{tail}\nYour memory and the board hold the rest.]"
            )
        } else {
            "[equipe: this is a fresh conversation. Your memory and the board hold what lasts.]".to_owned()
        })
    }

    /// Close browsers whose member is gone, or hasn't had a turn in half an
    /// hour; the next turn opens a fresh one, still signed in.
    /// Once a day, a squad whose unstarted work has sat for a week has its PM
    /// told — to propose, next time it talks with the person, what to close,
    /// merge or keep. Only the person drops work.
    fn tidy_check(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let Some(journal) = self.journal(&squad) else { continue };
            if now_secs() as i64 - journal.tidied_at() < TIDY_EVERY {
                continue;
            }
            let _ = journal.set_tidied_at();
            let stale = journal.stale_tickets(TIDY_AFTER).unwrap_or_default();
            if let Some(line) = tidy_line(&stale) {
                let shown = format!("── {} tickets haven't moved in a week: the PM will suggest tidying them ──", stale.len());
                self.brief_pm(&squad, line, Some(shown));
            }
        }
    }

    fn close_idle_browsers(&mut self) {
        let working: Vec<(String, String)> = self
            .sessions
            .values()
            .filter(|s| s.is_working())
            .filter_map(|s| Some((s.squad.clone()?, s.name()?.to_owned())))
            .collect();
        let present: Vec<(String, String)> =
            self.sessions.values().filter_map(|s| Some((s.squad.clone()?, s.name()?.to_owned()))).collect();
        self.browsers.retain(|key, b| {
            working.contains(key) || (present.contains(key) && b.last_used.elapsed() < Duration::from_secs(30 * 60))
        });
        // A window left on screen with nobody using it goes back out of sight.
        let idle_shown: Vec<(String, String)> = self
            .browsers
            .iter()
            .filter(|(key, b)| b.is_shown() && !working.contains(key) && b.last_used.elapsed() > SHOWN_IDLE)
            .map(|(key, _)| key.clone())
            .collect();
        for (squad, name) in idle_shown {
            let team: Vec<String> = self.team(&squad).into_iter().map(|(_, n, _)| n).collect();
            if let Some(journal) = self.journal(&squad) {
                let _ = journal.show_browser(Some(&name), false, &team);
            }
            self.browsers.remove(&(squad, name));
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
/// Where in a worktree `project` is: the worktree itself for a repository,
/// the same folder in it for a folder of a repository.
fn within(sandbox: &Sandbox, project: &Path) -> PathBuf {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    match canon(project).strip_prefix(canon(&sandbox.repo)) {
        Ok(inside) if !inside.as_os_str().is_empty() => sandbox.path.join(inside),
        _ => sandbox.path.clone(),
    }
}

fn restore_work(journal: &Journal, ticket: &Ticket, projects: &[PathBuf], home: &Path, edits: bool) -> Option<Work> {
    let in_home = ticket.kind.in_home();
    let project = if in_home {
        ticket.project.clone().unwrap_or_else(|| home.to_owned())
    } else {
        ticket.project.clone().or_else(|| projects.first().cloned())?
    };
    let workplace = if in_home { home } else { project.as_path() };
    let saved = journal.ticket_work(ticket.id).unwrap_or_default();
    let sandbox = match (edits || in_home, &saved.worktree, &ticket.branch, &saved.base) {
        (false, ..) => None,
        (true, Some(path), Some(branch), Some(base)) if path.exists() => Some(Sandbox {
            repo: worktree::repo_root(workplace).ok()?,
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
        kind: ticket.kind,
        cwd: sandbox.as_ref().map_or_else(|| project.clone(), |s| within(s, &project)),
        project,
        sandbox,
        claude_session: saved.engine_session,
        history: Vec::new(),
        handed_in: None,
        waiting: None,
    })
}

/// A `waiting` ticket back to `todo`, for the member that put it down while it
/// is still on the squad (`present`); otherwise for whoever owns it.
fn back_to_its_member(ticket: &mut Ticket, present: &[String], note: &str) {
    if let Some(member) = ticket.assignee.take().filter(|a| present.contains(a)) {
        ticket.owner = Some(member);
    }
    ticket.status = TicketStatus::Todo;
    ticket.note = Some(note.to_owned());
}

/// The `kind` a tool call names, if any.
fn kind_arg(args: &Value) -> Result<Option<TicketKind>, String> {
    match board::arg_str(args, "kind") {
        None => Ok(None),
        Some(text) => TicketKind::parse(text).map(Some).ok_or_else(|| format!("A ticket's kind is code, task or action, not {text}.")),
    }
}

/// The project a `code` ticket that names none works in: the squad's only one.
fn code_project(projects: &[PathBuf], names: &[String]) -> Result<PathBuf, String> {
    match projects {
        [one] => Ok(one.clone()),
        [] => Err("A code ticket needs a project, and the squad has none: make it a task, or the person adds one with /squad add.".into()),
        _ => Err(format!("Name the project — the squad has several: {}.", names.join(", "))),
    }
}

/// A task or action whose work couldn't go into the squad's folder — it changed
/// under it — back to `member`, same worktree and conversation, to bring it up
/// to date and hand it in again. Nothing is closed.
fn sent_back_to_update(ticket: &mut Ticket, member: &str) {
    ticket.status = TicketStatus::Todo;
    ticket.owner = Some(member.to_owned());
    ticket.assignee = None;
    ticket.note = Some(
        "Reopened: the squad's folder changed under this work, so it couldn't go in. Merge main into your branch \
in your worktree (git merge main), keep both sides' changes, and hand it in again."
            .to_owned(),
    );
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
    if ticket.kind.in_home() {
        out.push_str(&format!("Kind: {} — work for the squad's folder, not a repository.", ticket.kind.label()));
        if let Some(about) = &ticket.project {
            out.push_str(&format!(" It is about {} ({}), which you only read.", board::project_name(about), about.display()));
        }
        if let Some(sandbox) = sandbox {
            out.push_str(&format!(
                " You work in a copy of the squad's folder of your own, {}: leave what you produce under \
`outputs/T-{} …/` (or `notes/` for findings worth keeping). When the person accepts, it goes into the squad's folder.",
                sandbox.path.display(),
                ticket.id
            ));
        }
        if ticket.kind == TicketKind::Task {
            out.push_str(
                " The person reviews what you leave: in your files and your summary, give each fact its source — a \
link, a document, a page — and mark what you couldn't confirm as unconfirmed.",
            );
            if board_tools {
                out.push_str(" A date, an amount or a rule that recurs goes into the squad's memory with `remember`.");
            }
        }
        if ticket.kind == TicketKind::Action {
            out.push_str(
                " This ticket is done by acting — in an app or the browser. Anything that reaches someone else or can't \
be taken back (sending, posting, submitting, paying, signing up) asks the person first: wait for their yes. \
Keep a short log of what you did, step by step, and hand it in as your summary.",
            );
        }
    } else {
        out.push_str(&format!("Project: {} ({}).", board::project_name(project), project.display()));
        if let Some(sandbox) = sandbox {
            out.push_str(&format!(
                " You work in a worktree of your own on branch {}; the person's checkout is untouched.",
                sandbox.branch
            ));
        }
    }
    out.push('\n');
    if let Some(note) = reopened.and_then(|n| n.strip_prefix("Resumed: ")) {
        out.push_str(&format!(
            "You put this ticket down to wait for the person; it is back. {note}\nCarry on from where you left off — \
check what is already done in your worktree first.\n"
        ));
    } else if let Some(note) = reopened {
        out.push_str(&format!(
            "The person sent it back. {note}\nWhat they asked for says something about how they want work done: save that \
lesson in your memory before you start, so the next ticket begins with it.\n"
        ));
    }
    if !ticket.body.is_empty() {
        out.push_str(&format!("\n{}\n", ticket.body));
    }
    out.push_str(if board_tools {
        "\nWhen it meets the acceptance criteria, call submit_for_review with a short summary, then end your turn. \
If you can't go on without something only the person can do (a token, an account, a setting), call block_on_person \
with what they need to do and end your turn — you are free for other work until they have. If you need a decision, \
say so and end your turn — the person answers here."
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
        (_, false) | (Mode::ReadOnly, _) => claude_code::Access::ReadOnly,
        (Mode::Ask, true) => claude_code::Access::Ask,
        (Mode::Auto, true) => claude_code::Access::EditFiles,
        (Mode::Full, true) => claude_code::Access::Full,
    }
}

/// A member at its desk, between tickets, in the squad's home: it looks around
/// freely and, in auto, writes the home on its own, fenced in like a ticket;
/// beyond that, the mode decides — read-only refuses, ask puts every change to
/// the person, full lets it.
fn desk_access(mode: Mode) -> claude_code::Access {
    match mode {
        Mode::ReadOnly => claude_code::Access::ReadOnly,
        Mode::Ask => claude_code::Access::Ask,
        Mode::Auto => claude_code::Access::EditFiles,
        Mode::Full => claude_code::Access::Full,
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
/// The screen before a send, where the person can open it while asked: in
/// the temp folder, readable by them alone, deleted once answered.
fn save_preview(id: u64, png: &[u8]) -> Option<String> {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("equipe-send-{}-{id}.png", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&path).and_then(|mut f| f.write_all(png)).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

fn verdict(decision: Decision, input: &Value, why: &str) -> ToolResult {
    let text = match decision {
        Decision::Allow | Decision::Always => json!({ "behavior": "allow", "updatedInput": input }),
        Decision::Deny => json!({ "behavior": "deny", "message": why }),
    };
    ToolResult { text: text.to_string(), is_error: false }
}


/// What a turn was for, in a line: the person's words, without what equipe
/// put before them (`[equipe: …]`).
fn asked_for(prompt: &str) -> String {
    let mut rest = prompt.trim();
    while rest.starts_with('[')
        && let Some(end) = rest.find("]\n\n")
    {
        rest = rest[end + 3..].trim_start();
    }
    let line = rest.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("on its own");
    clip(line, 80)
}

/// A desk conversation this full of its context window starts afresh.
const REFRESH_FULL_PERCENT: u64 = 60;
/// One this big, unused this long, starts afresh rather than being processed
/// again whole once the prompt cache has expired.
const REFRESH_COLD_TOKENS: u64 = 60_000;
const REFRESH_COLD_AFTER: Duration = Duration::from_secs(60 * 60);
/// How many of the last messages a fresh conversation is handed.
const REFRESH_TAIL: usize = 12;

/// `text` cut to about `max` characters, on a character boundary.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// Messages of a Codex conversation kept in memory.
const HISTORY_KEPT: usize = 200;
/// What a Codex turn is sent of its conversation: the latest messages, up to
/// this many characters (about 30k tokens) and this many messages.
const HISTORY_CHARS: usize = 120_000;
const HISTORY_SENT: usize = 40;

/// The recent part of a conversation, newest kept; when older messages are
/// left out, a note says so.
fn recent_history(history: &[responses::Message]) -> Vec<responses::Message> {
    let mut chars = 0;
    let mut kept: Vec<responses::Message> = history
        .iter()
        .rev()
        .take(HISTORY_SENT)
        .take_while(|m| {
            chars += m.text.len();
            chars <= HISTORY_CHARS
        })
        .cloned()
        .collect();
    kept.reverse();
    // Start on the person's turn, not mid-exchange.
    while kept.first().is_some_and(|m| m.role == responses::Role::Assistant) {
        kept.remove(0);
    }
    if kept.len() < history.len() {
        kept.insert(0, responses::Message {
            role: responses::Role::User,
            text: "[equipe: the earlier part of this conversation is left out to fit; your memory and the board hold what lasts]".into(),
        });
    }
    kept
}

/// Remove a ticket's worktree; keep its branch if it has commits, drop it if not.
/// Returns the branch that remains.
/// Whether a ticket's branch is in its project's main branch; `None` when that
/// can't be told (no branch, no repository).
fn merged_state(ticket: &Ticket) -> Option<bool> {
    if ticket.kind.in_home() {
        return None;
    }
    let repo = worktree::repo_root(ticket.project.as_deref()?).ok()?;
    worktree::is_merged(&repo, ticket.branch.as_deref()?).ok()
}

/// The finished work a ticket builds on — what it depends on, and what that
/// depended on — as `(T-n, branch)`, oldest first: its worktree starts with
/// all of it, merged or not.
fn work_before(tickets: &[Ticket], ticket: &Ticket) -> Vec<(String, String)> {
    let mut seen: Vec<TicketId> = Vec::new();
    let mut next: Vec<TicketId> = ticket.depends_on.clone();
    while let Some(id) = next.pop() {
        if seen.contains(&id) || id == ticket.id {
            continue;
        }
        seen.push(id);
        if let Some(t) = tickets.iter().find(|t| t.id == id) {
            next.extend(&t.depends_on);
        }
    }
    seen.sort_unstable();
    seen.iter()
        // Only work from the same place: a task's is in the home's main already, a code ticket's on its branch.
        .filter_map(|id| tickets.iter().find(|t| t.id == *id && t.status == TicketStatus::Done && t.kind.in_home() == ticket.kind.in_home()))
        .filter_map(|t| Some((format!("T-{}", t.id), t.branch.clone()?)))
        .collect()
}

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
            let _ = journal.add_tokens(id, u.total());
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
/// The end of a conversation, as the guess at the person's next message reads
/// it: the last few exchanges, each cut short, the member's last answer most
/// fully. `None` when it doesn't end with the member answering.
fn suggestion_context(member: &str, entries: &[Entry]) -> Option<String> {
    let said: Vec<(bool, &str)> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::User { text } => Some((true, text.as_str())),
            Entry::Reply { text } if !text.trim().is_empty() => Some((false, text.as_str())),
            _ => None,
        })
        .collect();
    if said.last().is_none_or(|(person, _)| *person) {
        return None;
    }
    let from = said.len().saturating_sub(6);
    let last = said.len() - 1;
    let lines: Vec<String> = said[from..]
        .iter()
        .enumerate()
        .map(|(i, (person, text))| {
            let keep = if from + i == last { 4000 } else { 1200 };
            let mut text: String = text.trim().chars().take(keep).collect();
            if text.chars().count() == keep {
                text.push('…');
            }
            format!("[{}] {text}", if *person { "person" } else { member })
        })
        .collect();
    Some(lines.join("\n\n"))
}

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
            let usage = result.usage.map(|u| Usage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_tokens: u.cache_tokens,
                context_tokens: u.context_tokens,
                context_window: u.context_window,
            });
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
    } else if lower.contains("failed to authenticate") || lower.contains("oauth") {
        // The sign-in Claude Code has expired. Signed in again over SSH, it
        // lands in a file; a daemon run as a login service reads the Keychain,
        // which only a sign-in at the Mac's own screen updates.
        Some("claude auth login — in Terminal on this Mac's own screen (Screen Sharing is fine), not over SSH")
    } else if lower.contains("login") || lower.contains("logged in") {
        Some("equipe login claude")
    } else {
        None
    };
    (message, next)
}

/// How often a squad's stale work is looked at, and how long work sits
/// before it counts as stale.
const TIDY_EVERY: i64 = 24 * 60 * 60;
const TIDY_AFTER: i64 = 7 * 24 * 60 * 60;

/// What the PM is told about work that has sat: each ticket and for how long,
/// and what to do about it.
fn tidy_line(stale: &[(TicketId, String, i64)]) -> Option<String> {
    if stale.is_empty() {
        return None;
    }
    let listed: Vec<String> = stale.iter().take(20).map(|(id, title, age)| format!("T-{id} {title} ({} days)", age / 86400)).collect();
    let more = if stale.len() > 20 { format!(" and {} more", stale.len() - 20) } else { String::new() };
    Some(format!(
        "Tidy-up: these haven't moved in a week or more — {}{more}. Next time you talk with the person, propose what to do \
with each: close it, merge it into another, or keep it (and why). Don't cancel them on your own; the person decides.",
        listed.join("; ")
    ))
}

/// How long a browser stays on screen unused before it goes out of sight.
const SHOWN_IDLE: Duration = Duration::from_secs(10 * 60);

/// How often a watched browser is pictured.
const FRAME_EVERY: Duration = Duration::from_millis(1500);

/// Screenshots older than a moment in a member's browser folder: the pictures
/// equipe took to show the person, not the member's own (it reads them at once).
fn clear_frames(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(30));
        if name.starts_with("page-") && name.ends_with(".png") && old {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pm_hears_about_stale_tickets_and_proposes_rather_than_cancels() {
        assert_eq!(tidy_line(&[]), None);
        let line = tidy_line(&[(12, "Dark mode".into(), 9 * 86400), (15, "Old CSV export".into(), 21 * 86400)]).unwrap();
        assert!(line.contains("T-12 Dark mode (9 days)") && line.contains("T-15 Old CSV export (21 days)"), "{line}");
        assert!(line.contains("propose") && line.contains("Don't cancel"), "{line}");
    }

    #[test]
    fn an_expired_claude_sign_in_says_where_to_sign_in_again() {
        let err = equipe_providers::Error::Backend { message: "Failed to authenticate: OAuth session expired and could not be refreshed".into(), code: None };
        let (_, next) = claude_failure(err);
        assert!(next.is_some_and(|n| n.starts_with("claude auth login")), "{next:?}");
    }

    fn exchange(n: usize, size: usize) -> Vec<responses::Message> {
        (0..n)
            .flat_map(|i| {
                [
                    responses::Message { role: responses::Role::User, text: format!("q{i} {}", "x".repeat(size)) },
                    responses::Message { role: responses::Role::Assistant, text: format!("a{i}") },
                ]
            })
            .collect()
    }

    #[test]
    fn a_short_conversation_goes_whole() {
        let history = exchange(3, 10);
        assert_eq!(recent_history(&history), history);
    }

    #[test]
    fn a_long_one_sends_its_latest_part_and_says_so() {
        let sent = recent_history(&exchange(100, 10));
        assert!(sent.len() <= HISTORY_SENT + 1);
        assert!(sent[0].text.contains("left out to fit"));
        assert_eq!(sent[1].role, responses::Role::User, "starts on the person's turn");
        assert_eq!(sent.last().unwrap().text, "a99", "the latest is kept");
        // Big messages: bounded by size too.
        let sent = recent_history(&exchange(10, 50_000));
        let chars: usize = sent.iter().skip(1).map(|m| m.text.len()).sum();
        assert!(chars <= HISTORY_CHARS, "{chars}");
    }
}
