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
use equipe_protocol::transcript::scope;
use equipe_protocol::{
    Access, Approval, BrowserNavAction, Command, Decision, Engine, Entry, Event, Member, MemberRef, Mode, SessionId, SessionInfo,
    Estimate, OffMain, SessionSpec, SessionState, Snapshot, Ticket, TicketId, TicketKind, TicketStatus, Transcript, TurnOutcome, Usage, WorkInfo, Workspace,
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
use crate::eta;
use crate::journal::{self, Journal, SessionRecord, TicketWork};
use crate::permissions;
use crate::roles::{self, Teammate};
use crate::squad_home;
use crate::squad_memory;
use crate::worktree::{self, Sandbox};

/// What equipe says to the person, in their language: `say!(self, "notice.model", model = m)`.
/// What it tells the models is written out in English instead.
macro_rules! say {
    ($rt:expr, $key:literal) => {
        crate::i18n::core($rt.config.lang).get($key).to_owned()
    };
    ($rt:expr, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        crate::i18n::core($rt.config.lang).fill($key, &[$((stringify!($name), ($value).to_string())),+])
    };
}

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
    /// Members' browsers as lanes of one shared Chrome (the browser host),
    /// rather than a Chrome each: the person signs in once, Google included.
    pub browser_host: bool,
    /// The language equipe speaks to the person in until a screen says
    /// otherwise: notices, errors. What it tells the models stays English.
    pub lang: crate::i18n::Lang,
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
            browser_host: cfg!(target_os = "macos"),
            lang: crate::i18n::Lang::resolve(crate::i18n::Front::Terminal),
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
    PersonWindow(String, oneshot::Sender<bool>),
    BrowserHub(oneshot::Sender<std::sync::Arc<browser::Hub>>),
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

    /// A window for the person in the members' shared Chrome, at `url`.
    /// `false` when members use a Chrome each.
    pub async fn person_window(&self, url: String) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::PersonWindow(url, tx));
        rx.await.unwrap_or(false)
    }

    /// The shared Chrome's pictures and input, for a browser panel's stream.
    pub async fn browser_hub(&self) -> Option<std::sync::Arc<browser::Hub>> {
        let (tx, rx) = oneshot::channel();
        self.requests.send(Request::BrowserHub(tx)).ok()?;
        rx.await.ok()
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
    let (host_tx, host_rx) = mpsc::unbounded_channel();
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
        builds_trimmed: None,
        browsers: HashMap::new(),
        host: None,
        hub: browser::Hub::new(),
        host_events: host_tx,
        tabs: HashMap::new(),
        asked_in: HashMap::new(),
        held: HashMap::new(),
        watching: HashMap::new(),
        framing: std::collections::HashSet::new(),
        frame_watchers: HashMap::new(),
        frame_sent: HashMap::new(),
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
    tokio::spawn(run(runtime, req_rx, internal_rx, host_rx));
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
    /// Into the turn running now: what the person says while it works, read
    /// at its next step (steering). `None` when the engine can't take it.
    steer: Option<mpsc::UnboundedSender<String>>,
    /// What was sent that way and not read yet; at the turn's end, memos.
    steering: Vec<String>,
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
    /// For a lane: the member's own session, and the project its conversation
    /// is about (`None`: the whole squad's). A lane runs that conversation's
    /// turns beside the member's others; it is never a member of its own.
    lane: Option<(SessionId, Option<String>)>,
    /// When it expects its work done, and what it ships live, as it said.
    estimate: Option<Estimate>,
    /// When its turn running now began (unix seconds).
    turn_began: Option<u64>,
    /// Its own mode; `None` follows the squad's.
    mode: Option<Mode>,
    /// The turn running answers the person (not the board, not the team), so
    /// a guess at what they say next may follow it.
    answers_person: bool,
    /// How hard it thinks (`--effort` for Claude Code, reasoning effort for
    /// Codex); `None`: the engine's default.
    effort: Option<String>,
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
    Event(Box<Event>),
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
    /// When members' build caches were last looked at.
    builds_trimmed: Option<std::time::Instant>,
    /// Members' browsers, kept between turns: by squad and member.
    browsers: HashMap<(String, String), browser::Running>,
    /// The one Chrome members' browsers are lanes of, once started.
    host: Option<browser::SharedHost>,
    /// Its pages' pictures, for browser panels; and where it speaks to us.
    hub: std::sync::Arc<browser::Hub>,
    host_events: UnboundedSender<browser::HostEvent>,
    /// Each lane's tabs as last told, for a screen that attaches later.
    tabs: HashMap<String, Event>,
    /// Lanes whose member asked the person in (`show_my_browser` with a browser
    /// panel attached): handing back tells the member the person is done.
    asked_in: HashMap<String, String>,
    /// Lanes in the person's hands, and what their member asked them for.
    held: HashMap<String, Option<String>>,
    /// The member each squad's screen watches the browser of, and the
    /// pictures under way, one at a time each.
    watching: HashMap<String, SessionId>,
    framing: std::collections::HashSet<SessionId>,
    /// For each squad watching a lane, its watcher in the hub; and the frame
    /// each watched conversation was sent last.
    frame_watchers: HashMap<String, u64>,
    frame_sent: HashMap<SessionId, (u64, u64)>,
    /// Where each member's browser was last taken, by squad and name — and a
    /// page asked for when it next comes on screen.
    last_url: HashMap<(String, String), String>,
    show_url: HashMap<(String, String), String>,
    /// The Claude plan's usage, as Claude Code last reported it.
    plan: Option<equipe_protocol::PlanUsage>,
    /// The Mac's screen, one member at a time, across squads.
    desktop: desktop::Desktop,
}

async fn run(mut rt: Runtime, mut requests: UnboundedReceiver<Request>, mut internal: UnboundedReceiver<Internal>, mut host: UnboundedReceiver<browser::HostEvent>) {
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
                    // The shared Chrome's tabs as they stand, then what changes.
                    for tabs in rt.tabs.values() {
                        if tabs.for_everyone() || tabs.squad() == Some(squad.as_str()) {
                            let _ = tx.send(tabs.clone());
                        }
                    }
                    // And the members' browsers in the person's hands.
                    for (lane, why) in &rt.held {
                        if lane.split_once('/').is_some_and(|(q, _)| q == squad) {
                            let _ = tx.send(Event::BrowserControl { squad: Some(squad.clone()), lane: lane.clone(), by: Some("person".into()), why: why.clone() });
                        }
                    }
                    rt.subscribers.push((Some(squad.clone()), tx));
                    let _ = reply.send((rt.snapshot(&squad), rx));
                }
                Some(Request::Tool(call, reply)) => rt.tool(call, reply).await,
                Some(Request::PersonWindow(url, reply)) => {
                    let _ = reply.send(rt.person_window(&url));
                }
                Some(Request::BrowserHub(reply)) => {
                    let _ = reply.send(rt.hub.clone());
                }
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
                Internal::Event(event) => rt.publish(*event),
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
            Some(said) = host.recv() => rt.on_host(said).await,
            _ = daily.tick(), if rt.config.persist => rt.prune(),
            _ = desktop_tick.tick() => {
                if rt.desktop.expire() {
                    rt.publish_desktop();
                }
            }
            _ = frames.tick() => rt.take_frames(),
            _ = minute.tick() => {
                rt.fold_idle_lanes();
                rt.trim_builds();
                rt.expire_estimates();
                rt.close_idle_browsers();
                rt.let_helpers_go().await;
                rt.tidy_check();
                rt.settle_all_merged();
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
    fn publish_in(&mut self, squad: Option<&str>, mut event: Event) {
        // A ticket waiting for the person carries what it asks, item by item
        // (kept in its note); any other, nothing.
        if let Event::TicketChanged { ticket, .. } = &mut event {
            ticket.asks = Vec::new();
            ticket.asks = ticket.asks();
        }
        // Where a member's browser goes, to bring it back there on screen.
        if let Event::ToolUse { session, tool, target: Some(url) } = &event
            && tool.ends_with("browser_navigate")
            && let Some(key) = self.browser_key(*session)
        {
            self.last_url.insert(key, url.clone());
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
                Event::Steered { text, .. } => {
                    if let Some(at) = session.steering.iter().position(|t| t == text) {
                        session.steering.remove(at);
                    }
                }
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

    /// equipe's words for the person under `key`, in their language, filled in.
    fn said(&self, key: &str, args: &[(&str, String)]) -> String {
        crate::i18n::core(self.config.lang).fill(key, args)
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
            Command::Submit { session, text } => {
                let session = self.route(session);
                self.submit(session, text).await
            }
            Command::SubmitLater { session, text } => {
                let session = self.route(session);
                match self.sessions.get(&session) {
                    Some(s) if s.is_working() => self.leave_memo(session, text),
                    _ => self.submit(session, text).await,
                }
            }
            Command::Interrupt { session } => self.interrupt(session).await,
            Command::Close { session } => self.close(session).await,
            Command::Accept { squad, ticket } => self.accept(&squad, ticket).await,
            Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),
            Command::KeepOnBranch { squad, ticket } => self.keep_on_branch(&squad, ticket),
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
            Command::BrowserNav { page, action, url } => {
                let action = match action {
                    BrowserNavAction::Back => "back",
                    BrowserNavAction::Forward => "forward",
                    BrowserNavAction::Reload => "reload",
                    BrowserNavAction::Stop => "stop",
                    BrowserNavAction::Go => "go",
                };
                self.hub.op(&json!({ "op": "nav", "page": page, "action": action, "url": url }));
            }
            Command::BrowserNewTab { url } => {
                // The person's own tab: the shared Chrome starts for it if it must.
                if let Some(npx) = self.config.npx.clone()
                    && self.browser_host(&npx).is_some()
                {
                    self.hub.op(&json!({ "op": "newtab", "url": url }));
                }
            }
            Command::BrowserCloseTab { page } => {
                self.hub.op(&json!({ "op": "closetab", "page": page }));
            }
            Command::BrowserControl { lane, take } => {
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": take.then_some("person") }));
            }
            Command::BrowserDialog { page, accept, text } => {
                self.hub.op(&json!({ "op": "dialog", "page": page, "accept": accept, "text": text }));
            }
            Command::BrowserPopOut { page, bounds } => {
                self.hub.op(&json!({ "op": "popout", "page": page, "bounds": bounds }));
            }
            Command::Language { lang } => {
                if let Some(lang) = crate::i18n::Lang::parse(&lang) {
                    self.config.lang = lang;
                }
            }
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
                    let text = if summary { say!(self, "notice.refresh_summary") } else { say!(self, "notice.refresh_clean") };
                    self.publish(Event::Notice { session, text });
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
                    let text = say!(self, "notice.model", model = model);
                    self.publish(Event::Notice { session, text });
                }
            }
            Command::Unqueue { session, text } => self.unqueue(session, &text),
            Command::SetEffort { session, effort } => {
                let effort = effort.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty());
                if let Some(s) = self.sessions.get_mut(&session) {
                    s.effort = effort.clone();
                    let text = match &effort {
                        Some(effort) => say!(self, "notice.effort", effort = effort),
                        None => say!(self, "notice.effort_default"),
                    };
                    self.publish(Event::Notice { session, text });
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
            Command::Disallow { squad, rule } => {
                let taken = self.journals.get(&squad).and_then(|j| j.disallow(&rule).ok()).unwrap_or(false);
                if taken {
                    self.publish_allowed(&squad);
                }
            }
            // Phones are paired by the gateway, per screen, with the daemon's
            // phone link (`crate::pairing`); they never reach the runtime.
            Command::PairStart | Command::PairAnswer { .. } | Command::PairCancel | Command::Devices | Command::Unpair { .. } => {}
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
        let steering = session.steering.clone();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.set_waiting(id, "steer", &steering);
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
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.name() == Some(name))
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
            .filter(|s| s.squad.as_deref() == Some(squad) && s.lane.is_none())
            .filter_map(|s| s.member.as_ref().map(|m| (s.joined, m.name.clone(), m.role.clone())))
            .collect();
        members.sort();
        members
    }

    fn pm_of(&self, squad: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.is_pm())
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
            self.show_news(pm, text);
        }
    }

    /// Team news on the PM's screen: in its conversation — beside its own
    /// ticket's work, when it runs there.
    /// It is about the project of the ticket it names (none named: the whole
    /// squad); a turn under way about another goes on about its own after it.
    fn show_news(&mut self, pm: SessionId, text: String) {
        let at = self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == pm)).map_or(pm, |(&lane, _)| lane);
        let desk = self.sessions.get(&at).filter(|s| s.work.is_none()).map(|s| (scope::current(s.transcript.entries()).flatten(), s.is_working(), s.squad.clone()));
        let Some((now, working, Some(squad))) = desk else {
            return self.publish(Event::Notice { session: at, text });
        };
        let about = self.news_project(&squad, &text);
        if about == now {
            return self.publish(Event::Notice { session: at, text });
        }
        self.publish(Event::Thread { session: at, project: about });
        self.publish(Event::Notice { session: at, text });
        if working {
            self.publish(Event::Thread { session: at, project: now });
        }
    }

    /// The project news is about: that of the first ticket it names (`T-12`).
    fn news_project(&mut self, squad: &str, text: &str) -> Option<String> {
        let id: TicketId = text.split("T-").skip(1).find_map(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })?;
        let ticket = self.find_ticket(squad, id)?;
        self.ticket_project(squad, &ticket)
    }

    /// The project a ticket is about, by name: its own, else the squad's
    /// first for work in code; none for work in the squad's home.
    fn ticket_project(&mut self, squad: &str, ticket: &Ticket) -> Option<String> {
        match &ticket.project {
            Some(project) => Some(board::project_name(project)),
            None if ticket.kind.in_home() => None,
            None => self.projects(squad).first().map(|p| board::project_name(p)),
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
            say!(self, "news.you_told", name = name, text = format!("{short}{more}"))
        });
        self.brief_pm(&squad, line(&name), shown);
    }

    // ── sessions ──

    async fn start(&mut self, id: SessionId, spec: SessionSpec) {
        if self.sessions.contains_key(&id) {
            return self.error(Some(id), say!(self, "error.already_open", id = id), None);
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
                    return fail(self, say!(self, "error.no_squad", squad = squad), Some("equipe"));
                }
            }
            (None, Some(_)) => return fail(self, say!(self, "error.join_a_squad"), None),
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
                        let message = say!(self, "error.no_engine");
                        return fail(self, message, Some("equipe login"));
                    }
                }
            }
        };
        if spec.access == Access::EditFiles && engine == Engine::Codex {
            return fail(
                self,
                say!(self, "error.codex_edits"),
                Some("equipe try --edit --engine claude \"…\""),
            );
        }

        let mut model = spec.model.clone();
        let mut codex_cred = None;
        match engine {
            Engine::Codex => {
                let cred = match codex_credentials(&self.client, &self.config.store, self.config.lang).await {
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
                        return fail(self, say!(self, "error.no_models"), Some("equipe status"));
                    }
                }
                codex_cred = Some(cred);
            }
            Engine::Claude => {
                match claude_code::auth_status(&self.config.claude_binary).await {
                    Ok(Some(status)) if status.logged_in => {}
                    Ok(Some(_)) => return fail(self, say!(self, "error.claude_signed_out"), Some("equipe login claude")),
                    Ok(None) => {
                        return fail(
                            self,
                            say!(self, "error.claude_missing"),
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
                    return fail(self, say!(self, "error.folder_not_made", err = err), None);
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
                steer: None,
                steering: Vec::new(),
                answers_person: false,
                effort: None,
                briefing: Vec::new(),
                memory,
                mode: None,
                thread: None,
                threads: HashMap::new(),
                lane: None,
                estimate: None,
                turn_began: None,
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
        match codex_credentials(&self.client, &self.config.store, self.config.lang).await {
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
            return self.error(Some(id), say!(self, "error.no_session", id = id), None);
        };
        if session.is_working() {
            // Said while it works: it reads it at its next step and carries on,
            // steered (what is said mid-work is mostly a correction); an engine
            // that can't take it mid-turn gets it as a memo.
            if let Some(steer) = session.steer.clone()
                && steer.send(text.clone()).is_ok()
            {
                session.steering.push(text.clone());
                self.save_waiting(id);
                self.tell_the_pm(id, |name| format!("The person told {name}, mid-work: {text}"), Some(&text));
                return self.publish(Event::SteerQueued { session: id, text });
            }
            return self.leave_memo(id, text);
        }
        self.submit_now(id, text).await;
    }

    /// A memo: it waits for the turn to end rather than cutting in, and belongs
    /// with the project the person is looking at now.
    fn leave_memo(&mut self, id: SessionId, text: String) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        {
            let looking_at = match &session.work {
                Some(work) => Some(crate::board::project_name(&work.project)),
                None => session.squad.as_ref().and_then(|s| self.focus.get(s)).cloned(),
            };
            session.queued.push(text.clone());
            session.memo_threads.push(looking_at.clone());
            let project = looking_at;
            self.save_waiting(id);
            self.publish(Event::MessageQueued { session: id, text, project });
        }
    }

    /// Take back a memo not delivered yet: the first one with `text`, in the
    /// session it was left in (a lane's, when the member had one).
    fn unqueue(&mut self, id: SessionId, text: &str) {
        let held = |s: &Session| s.queued.iter().any(|q| q == text);
        let id = match self.sessions.get(&id) {
            Some(s) if held(s) => id,
            _ => match self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && held(s)) {
                Some((lane, _)) => *lane,
                None => return,
            },
        };
        let Some(session) = self.sessions.get_mut(&id) else { return };
        let Some(at) = session.queued.iter().position(|q| q == text) else { return };
        session.queued.remove(at);
        if at < session.memo_threads.len() {
            session.memo_threads.remove(at);
        }
        self.save_waiting(id);
        self.publish(Event::MemoWithdrawn { session: id, text: text.to_owned() });
    }

    /// Send `text` to a member between turns: it starts one.
    async fn submit_now(&mut self, id: SessionId, text: String) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let memo_thread = session.memo_thread.take();
        if memo_thread.is_some() {
            self.save_waiting(id);
        }
        if !self.ensure_codex(id).await {
            return;
        }
        // The project the message is about: the one the person looks at — a
        // memo's, the one it was left in. None: the whole squad.
        let about = memo_thread.flatten().or_else(|| {
            let squad = self.sessions.get(&id)?.squad.clone()?;
            self.focus.get(&squad).cloned()
        });
        // In the PM's conversation the message, its reply and its steps are
        // about that project: a marker says so where it changes.
        if let Some(session) = self.sessions.get(&id)
            && session.is_pm()
            && session.work.is_none()
            && scope::current(session.transcript.entries()).flatten() != about
        {
            self.publish(Event::Thread { session: id, project: about.clone() });
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
        // About the whole squad, the PM oversees every project: it is told
        // where each stands with every message.
        let overview = self
            .sessions
            .get(&id)
            .filter(|s| s.is_pm() && s.work.is_none() && about.is_none())
            .and_then(|s| s.squad.clone())
            .and_then(|squad| self.squad_overview(&squad));
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
        if let Some(project) = about {
            prompt = format!("[equipe: the person is looking at the project {project} — new work is for it unless they say otherwise]\n\n{prompt}");
        } else if let Some(overview) = overview {
            prompt = format!(
                "[equipe: the person is looking at the whole squad — you oversee every project. Where each stands \
now:\n{overview}\nAnswer for the whole: priorities, what waits on the person, what goes next across projects. Work \
for one project goes to it as tickets for its owners, with that project — and you say where it went.]\n\n{prompt}"
            );
        }
        self.save_waiting(id);
        self.start_turn(id, prompt);
        if let Some(session) = self.sessions.get_mut(&id)
            && session.is_working()
        {
            session.answers_person = true;
        }
    }

    /// Where each of `squad`'s projects stands, a line a project: who is at it
    /// and what in it waits. `None` with a single project — the squad's
    /// conversation is about it anyway.
    fn squad_overview(&mut self, squad: &str) -> Option<String> {
        let projects = self.projects(squad);
        if projects.len() < 2 {
            return None;
        }
        let tickets = self.tickets(squad);
        let short = |text: &str| {
            let line = text.lines().next().unwrap_or_default().trim();
            let cut: String = line.chars().take(80).collect();
            if cut.len() < line.len() { format!("{cut}…") } else { cut }
        };
        let mut lines = Vec::new();
        for project in &projects {
            let name = board::project_name(project);
            let here: Vec<&Ticket> = tickets
                .iter()
                .filter(|t| t.project.as_ref().or(if t.kind.in_home() { None } else { projects.first() }) == Some(project))
                .collect();
            let mut at = Vec::new();
            let mut members: Vec<&Session> = self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad)).collect();
            members.sort_by_key(|s| s.joined);
            for s in members {
                let Some(who) = s.name() else {
                    continue;
                };
                // What it said it expects: done in, live in.
                let expects = s.estimate.as_ref().map_or_else(String::new, |e| {
                    let left = |at: u64| at.saturating_sub(now_secs()).div_ceil(60);
                    let mut out = String::new();
                    if let Some(at) = e.done_at {
                        let whose = if e.guessed { "as such work usually takes" } else { "by its word" };
                        out.push_str(&format!(", done in ~{}m {whose}", left(at)));
                    }
                    if let Some(at) = e.live_at {
                        out.push_str(&format!(", {} live in ~{}m", e.live_what.as_deref().unwrap_or("what it ships"), left(at)));
                    }
                    out
                });
                if let Some(work) = s.work.as_ref().filter(|w| &w.project == project) {
                    let title = here.iter().find(|t| t.id == work.ticket).map(|t| t.title.as_str()).unwrap_or_default();
                    let working = if s.is_working() { ", at work" } else { "" };
                    at.push(format!("{who} on T-{} ({title}){working}{expects}", work.ticket));
                    continue;
                }
                let about = match &s.lane {
                    Some((_, about)) => about.as_deref(),
                    None if s.work.is_none() => s.thread.as_deref(),
                    None => None,
                };
                if s.is_working() && about == Some(name.as_str()) {
                    let asked = s.prompts.last().map(|p| short(p)).unwrap_or_default();
                    at.push(format!("{who} talking with the person about it (“{asked}”){expects}"));
                }
            }
            let mut line = format!("- {name}: {}", if at.is_empty() { "no one on it now".to_owned() } else { at.join("; ") });
            for t in here.iter().filter(|t| t.status == TicketStatus::Review) {
                line.push_str(&format!("; T-{} ({}) waits for the person's review", t.id, t.title));
            }
            for t in here.iter().filter(|t| t.status == TicketStatus::Waiting) {
                let needs = t.note.as_deref().map(board::waiting_needs).unwrap_or_default();
                let needs = if needs.is_empty() { String::new() } else { format!(": needs {}", needs.join("; ")) };
                line.push_str(&format!("; T-{} ({}) waits for the person{needs}", t.id, t.title));
            }
            let open = here.iter().filter(|t| matches!(t.status, TicketStatus::Todo | TicketStatus::InProgress)).count();
            if open > 0 {
                line.push_str(&format!("; {open} to do or in progress"));
            }
            lines.push(line);
        }
        Some(lines.join("\n"))
    }

    /// Where a message to member `id` goes. A member has one conversation
    /// with the person; to anyone on a ticket, what is said is about that
    /// work. The PM alone is never out of reach: while it works a ticket of
    /// its own, its conversation runs beside the work — a lane.
    fn route(&mut self, id: SessionId) -> SessionId {
        let Some(session) = self.sessions.get(&id) else {
            return id;
        };
        if session.lane.is_some() || !session.is_pm() {
            return id;
        }
        if let Some((&lane, _)) = self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id)) {
            return lane;
        }
        if session.work.is_none() {
            return id;
        }
        self.open_lane(id, None).unwrap_or(id)
    }

    /// A lane of member `main` for its conversation about `project`, taking
    /// that conversation up where the member left it.
    fn open_lane(&mut self, main: SessionId, project: Option<String>) -> Option<SessionId> {
        let m = self.sessions.get_mut(&main)?;
        // On a ticket, the member's conversation waits in its own place.
        let thread = if m.work.is_some() {
            DeskThread {
                claude_session: m.claude_session.take(),
                history: std::mem::take(&mut m.history),
                desk_context: m.desk_context.take(),
            }
        } else {
            m.threads.remove(&project).unwrap_or_default()
        };
        let id = equipe_protocol::new_session_id();
        let lane = Session {
            squad: m.squad.clone(),
            project: m.project.clone(),
            member: m.member.clone(),
            joined: m.joined,
            engine: m.engine,
            model: m.model.clone(),
            access: m.access,
            cwd: m.cwd.clone(),
            sandbox: None,
            codex: m.codex.clone(),
            history: thread.history,
            claude_session: thread.claude_session,
            work: None,
            prompts: Vec::new(),
            title: None,
            tokens: 0,
            context: thread.desk_context,
            desk_context: thread.desk_context,
            last_turn: None,
            refresh: None,
            transcript: Transcript::default(),
            turn: None,
            queued: Vec::new(),
            memo_threads: Vec::new(),
            memo_thread: None,
            steer: None,
            steering: Vec::new(),
            answers_person: false,
            effort: None,
            briefing: Vec::new(),
            memory: m.memory.clone(),
            mode: m.mode,
            thread: project.clone(),
            threads: HashMap::new(),
            estimate: None,
            turn_began: None,
            lane: Some((main, project.clone())),
        };
        let started = Event::SessionStarted {
            session: id,
            project: lane.project.clone(),
            engine: lane.engine,
            model: lane.model.clone(),
            workspace: None,
            member: lane.member.clone(),
            squad: lane.squad.clone(),
        };
        self.sessions.insert(id, lane);
        self.publish(started);
        self.publish(Event::LaneOpened { session: id, of: main, project });
        Some(id)
    }

    /// Lanes left with nothing to do for a while go back into their members.
    fn fold_idle_lanes(&mut self) {
        let idle: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.lane.is_some() && !s.is_working() && s.queued.is_empty())
            .filter(|(_, s)| s.last_turn.is_none_or(|t| t.elapsed() > LANE_IDLE))
            .map(|(&id, _)| id)
            .collect();
        for lane in idle {
            self.fold_lane(lane);
        }
    }

    /// A lane goes back into its member: its conversation is the member's for
    /// that project again, taken up there next time — its browser closes.
    fn fold_lane(&mut self, lane: SessionId) {
        let Some((main, project)) = self.sessions.get(&lane).and_then(|s| s.lane.clone()) else {
            return;
        };
        self.publish(Event::SessionClosed { session: lane });
        let Some(s) = self.sessions.remove(&lane) else {
            return;
        };
        if let Some(m) = self.sessions.get_mut(&main) {
            let thread = DeskThread { claude_session: s.claude_session, history: s.history, desk_context: s.desk_context };
            // The conversation taken from it while it was on a ticket: back in
            // place — unless it began another meanwhile.
            if m.claude_session.is_none() && m.history.is_empty() {
                (m.claude_session, m.history, m.desk_context) = (thread.claude_session, thread.history, thread.desk_context);
            } else {
                m.threads.entry(project).or_insert(thread);
            }
        }
        self.persist_threads(main);
    }

    /// Keep where each of member `main`'s conversations stands — its own
    /// threads and its lanes' — so a restart picks each one up again.
    fn persist_threads(&mut self, main: SessionId) {
        let Some(m) = self.sessions.get(&main) else {
            return;
        };
        let mut others: HashMap<String, String> = m
            .threads
            .iter()
            .filter_map(|(project, t)| Some((project.clone().unwrap_or_default(), t.claude_session.clone()?)))
            .collect();
        for s in self.sessions.values().filter(|s| s.lane.as_ref().is_some_and(|(of, _)| *of == main)) {
            if let (Some((_, project)), Some(claude)) = (&s.lane, &s.claude_session) {
                others.insert(project.clone().unwrap_or_default(), claude.clone());
            }
        }
        let (thread, squad) = (m.thread.clone(), m.squad.clone());
        if self.config.persist
            && let Some(journal) = squad.as_deref().and_then(|s| journal_for(&mut self.journals, &self.config.home, s))
        {
            let _ = journal.set_desk_threads(main, thread.as_deref(), &others);
        }
    }

    /// Run a turn with `prompt` in the session's current place: its ticket's
    /// project when it has one, its desk otherwise.
    fn start_turn(&mut self, id: SessionId, prompt: String) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.answers_person = false;
            session.turn_began = Some(now_secs());
        }
        self.guess_estimate(id);
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
        let browser = match (self.config.npx.clone(), self.browser_key(id), session.member.as_ref().map(|m| m.name.clone())) {
            (Some(npx), Some((squad, key)), Some(name)) if wants_browser => {
                // Page snapshots and screenshots: in the temp folder, readable without
                // asking, and never in the project.
                let _ = npx;
                let shown = self.journals.get(&squad).is_some_and(|j| j.browser_shown(&name));
                // The browser it had, if still running and showing as asked; else a new one,
                // which equipe holds open so it outlives each turn.
                self.open_browser(&squad, &key, shown)
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
                screen_language: Some(self.config.lang.english_name()),
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
        // A member builds in one place of its own, whichever ticket it is on: a
        // Rust build is gigabytes, and one in every ticket's worktree filled the disk.
        // Its own, not the squad's: two builds at once in one place overwrite each
        // other's binaries.
        let builds = session.squad.as_deref().zip(session.name()).map(|(squad, name)| journal::build_dir(&self.config.home, squad, name));
        if let Some(dir) = &builds {
            let _ = std::fs::create_dir_all(dir);
        }
        let env: Vec<(String, String)> = builds.iter().map(|d| ("CARGO_TARGET_DIR".to_owned(), d.to_string_lossy().into_owned())).collect();
        let writable: Vec<String> = builds.iter().map(|d| d.to_string_lossy().into_owned()).collect();
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
        // What the person says while this turn runs goes into it (Claude Code only).
        let mut steer = None;
        let turn = match session.engine {
            Engine::Codex => {
                let client = self.client.clone();
                let Some(cred) = session.codex.clone() else {
                    return self.error(Some(id), say!(self, "error.codex_signed_out"), Some("equipe login codex"));
                };
                let model = session.model.clone().unwrap_or_default();
                let effort = session.effort.clone();
                let version = self.config.client_version.clone();
                // Codex gets no conversation of its own: equipe sends the recent
                // part of it each turn, never more than fits.
                let history = recent_history(&history);
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(Box::new(event)));
                    };
                    let mut streamed = false;
                    let request = responses::TurnRequest {
                        prompt: &prompt,
                        model: &model,
                        instructions: charter.as_deref(),
                        history: &history,
                        effort: effort.as_deref(),
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
                let effort = session.effort.clone();
                let memory = session.memory.clone();
                let mut allowed: Vec<String> = mcp_config.iter().map(|_| format!("mcp__{MCP_SERVER}")).collect();
                if browses_freely {
                    allowed.push(format!("mcp__{}", browser::SERVER));
                }
                // What the person always allows no longer asks, and reaches its service from inside the fence.
                let outward = permissions::still_asks(&squad_rules);
                let mut domains: Vec<String> = permissions::ALLOWED_DOMAINS.iter().map(|d| (*d).to_owned()).collect();
                domains.extend(permissions::domains_allowed(&squad_rules));
                if member_rules {
                    allowed.extend(squad_rules);
                }
                let blocked: Vec<String> =
                    if member_rules { permissions::BLOCKED.iter().map(|r| (*r).to_owned()).collect() } else { Vec::new() };
                // The person's browser sign-ins and equipe's logins: no fenced command reads them.
                // Nor other squads' homes.
                let (steer_tx, steer_rx) = mpsc::unbounded_channel::<String>();
                steer = Some(steer_tx);
                let private: Vec<String> = [self.config.home.join("browser"), self.config.home.join("credentials")]
                    .iter()
                    .chain(&other_homes)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(Box::new(event)));
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
                            allow_write: &writable,
                        }),
                        prompt_tool: prompt_tool.as_deref(),
                        env: &env,
                        effort: effort.as_deref(),
                    };
                    // A browser just started may still be loading.
                    if let Some(port) = browser {
                        browser::ready(port, Duration::from_secs(90)).await;
                    }
                    let outcome = claude_code::run_turn(&binary, &cwd, request, Some(steer_rx), |activity| match activity {
                        Activity::Text(text) => {
                            streamed = true;
                            emit(Event::TextDelta { session: id, text });
                        }
                        Activity::Tool { name, target } => emit(Event::ToolUse { session: id, tool: name, target }),
                        Activity::ToolDone(done) => {
                            let (output, skipped) = equipe_protocol::ToolOutcome::tail(&done.output);
                            let outcome = equipe_protocol::ToolOutcome { ok: done.ok, exit: done.exit, ms: Some(done.ms), output, skipped, diff: done.diff };
                            emit(Event::ToolResult { session: id, tool: done.name, target: done.target, outcome })
                        }
                        Activity::Thinking(text) => emit(Event::Thinking { session: id, text }),
                        Activity::Model(model) => emit(Event::ModelKnown { session: id, model }),
                        Activity::Output(tokens) => emit(Event::Output { session: id, tokens }),
                        Activity::SubagentStarted { id: sub, what } => emit(Event::SubagentStarted { session: id, id: sub, what }),
                        Activity::SubagentStep { id: sub, step } => emit(Event::SubagentStep { session: id, id: sub, step }),
                        Activity::SubagentEnded { id: sub } => emit(Event::SubagentEnded { session: id, id: sub }),
                        Activity::Steered(text) => emit(Event::Steered { session: id, text }),
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
            session.steer = steer;
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
            Ok(Some(change)) => {
                let text = say!(self, "news.desk_saved", name = name, what = change.said(self.config.lang));
                self.publish(Event::Notice { session: id, text });
            }
            Ok(None) => {}
            Err(err) => self.error(Some(id), say!(self, "error.folder_not_saved", err = err), None),
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
        // A lane's conversation is kept with the member's others.
        if let Some(main) = self.sessions.get(&done.session).and_then(|s| s.lane.as_ref()).map(|(of, _)| *of) {
            self.persist_threads(main);
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
                let _ = internal.send(Internal::Event(Box::new(Event::Suggestion { session: id, text })));
            }
        });
    }

    /// After a turn, finished or stopped: hand in work that was submitted,
    /// deliver memos, and let the rules hand out what is ready.
    async fn after_turn(&mut self, id: SessionId) {
        // How long a turn at the desk took, for guessing the next ones'.
        if let Some(s) = self.sessions.get_mut(&id)
            && let (Some(began), None, Some(squad), Some(name)) = (s.turn_began.take(), &s.work, s.squad.clone(), s.name())
        {
            let (took, role) = (now_secs().saturating_sub(began), eta::role_of(name).to_owned());
            if took >= GUESSED_AFTER.as_secs()
                && let Some(journal) = self.journal(&squad)
            {
                let _ = journal.record_turn_time(&role, took);
            }
        }
        // Said while it worked but not read before the turn ended: delivered now, with the memos.
        let unread = self.sessions.get_mut(&id).map(|s| {
            s.steer = None;
            std::mem::take(&mut s.steering)
        });
        for text in unread.unwrap_or_default() {
            self.leave_memo(id, text);
        }
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let squad = session.squad.clone();
        if session.work.as_ref().is_some_and(|w| w.handed_in.is_some()) {
            self.end_work(id, WorkEnd::HandedIn);
        } else if session.work.as_ref().is_some_and(|w| w.waiting.is_some()) {
            self.end_work(id, WorkEnd::Waiting);
        }
        // Off a ticket, the turn's end is the work's: what was expected is done.
        if self.sessions.get(&id).is_some_and(|s| s.work.is_none()) {
            self.estimate_done(id);
        }
        if let Some(text) = self.sessions.get_mut(&id).and_then(Session::next_memos) {
            self.save_waiting(id);
            Box::pin(self.submit(id, text)).await;
        }
        if let Some(squad) = squad {
            Box::pin(self.dispatch(&squad)).await;
        }
    }

    /// Without a member's word on when its work is done, equipe's guess: how
    /// long a ticket of the kind took its role before, or a turn at its desk.
    fn guess_estimate(&mut self, id: SessionId) {
        let Some(s) = self.sessions.get(&id) else {
            return;
        };
        if s.estimate.as_ref().is_some_and(|e| e.done_at.is_some()) {
            return;
        }
        let (Some(squad), Some(name)) = (s.squad.clone(), s.name().map(str::to_owned)) else {
            return;
        };
        let ticket = s.work.as_ref().map(|w| (w.ticket, w.kind));
        let Some(journal) = self.journal(&squad) else {
            return;
        };
        let typical = match ticket {
            Some((_, kind)) => eta::ticket_time(&journal.ticket_times().unwrap_or_default(), &name, kind),
            None => eta::typical(&journal.turn_times(eta::role_of(&name)).unwrap_or_default()),
        };
        let (Some(took), Some(session)) = (typical, self.sessions.get_mut(&id)) else {
            return;
        };
        let estimate = Estimate {
            done_at: Some(now_secs() + took),
            what: ticket.map(|(t, _)| format!("T-{t}")),
            guessed: true,
            ..session.estimate.clone().unwrap_or_default()
        };
        session.estimate = Some(estimate.clone());
        self.publish(Event::Estimated { session: id, estimate: Some(estimate) });
    }

    /// The work a member expected done is: only what it ships, if anything, is
    /// still to come.
    fn estimate_done(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(estimate) = session.estimate.as_mut().filter(|e| e.done_at.is_some()) else {
            return;
        };
        (estimate.done_at, estimate.what, estimate.guessed) = (None, None, false);
        let estimate = session.estimate.take().filter(|e| !e.is_empty());
        session.estimate = estimate.clone();
        self.publish(Event::Estimated { session: id, estimate });
    }

    /// What was to be live long ago is no longer waited for.
    fn expire_estimates(&mut self) {
        let stale: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.estimate.as_ref().and_then(|e| e.live_at).is_some_and(|at| at + LIVE_STALE.as_secs() < now_secs()))
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            let estimate = session.estimate.take().map(|e| Estimate { live_at: None, live_what: None, ..e }).filter(|e| !e.is_empty());
            session.estimate = estimate.clone();
            self.publish(Event::Estimated { session: id, estimate });
        }
    }

    async fn close(&mut self, id: SessionId) {
        // A member leaving takes its lanes with it.
        let lanes: Vec<SessionId> = self.sessions.iter().filter(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id)).map(|(&l, _)| l).collect();
        for lane in lanes {
            Box::pin(self.close(lane)).await;
        }
        self.stop_turn(id);
        let Some(session) = self.sessions.get(&id) else {
            return self.error(Some(id), say!(self, "error.no_session", id = id), None);
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
            Err(err) => {
                let message = say!(self, "error.not_committed", err = err, path = sandbox.path.display());
                self.error(Some(id), message, None);
            }
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
            .filter(|(id, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && !helpers.contains(id))
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
                    let text = say!(self, "news.helper_left", name = name);
                    self.publish(Event::Notice { session: pm, text });
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
        let shown = say!(self, "news.cannot_start", id = ticket.id, name = name, reason = reason);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line, Some(shown));
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
                let message = say!(self, "error.ticket_not_committed", id = ticket.id, err = err, path = sandbox.path.display());
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
        // What the PM is told, in English, and what the person reads (`news`).
        let (by, event, line, news) = match how {
            WorkEnd::HandedIn => {
                ticket.status = TicketStatus::Review;
                ticket.note = handed_in;
                (name.as_str(), "handed in", format!("{name} handed in T-{} for your review", ticket.id), "news.handed_in")
            }
            WorkEnd::Waiting => {
                ticket.status = TicketStatus::Waiting;
                ticket.note = summary.clone();
                (name.as_str(), "waiting", format!("T-{} waits for you — {name} needs something only you can do", ticket.id), "news.waiting")
            }
            WorkEnd::Accepted if matches!(landed, Some(Err(_))) => {
                sent_back_to_update(&mut ticket, &name);
                ("equipe", "sent back", format!("T-{} went back to {name}: the squad's folder changed under it", ticket.id), "news.sent_back")
            }
            WorkEnd::Accepted => {
                ticket.status = TicketStatus::Done;
                ticket.merged = merged_state(&ticket);
                ticket.off_main = None;
                let (line, news) = if ticket.kind.in_home() {
                    (format!("You accepted T-{}; its work is in the squad's folder", ticket.id), "news.accepted_home")
                } else {
                    (format!("You accepted T-{}", ticket.id), "news.accepted")
                };
                ("you", "accepted", line, news)
            }
            WorkEnd::Cancelled => {
                ticket.status = TicketStatus::Cancelled;
                ("you", "cancelled", format!("You cancelled T-{}", ticket.id), "news.cancelled")
            }
            WorkEnd::Left => {
                ticket.status = TicketStatus::Todo;
                ticket.assignee = None;
                ticket.note = Some(match &ticket.branch {
                    Some(branch) => format!("{name} left the squad; the work so far is on {branch}."),
                    None => format!("{name} left the squad."),
                });
                ("equipe", "released", format!("{name} left T-{}; it is back on the board", ticket.id), "news.left")
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
        let shown = self.said(news, &[("id", ticket.id.to_string()), ("name", name.clone()), ("title", ticket.title.clone())]);
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
            let message = say!(self, "error.no_ticket", id = ticket);
            self.squad_error(squad, message);
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
                    let shown = say!(self, "news.sent_back", id = ticket.id, name = name, title = ticket.title);
                    self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
                    self.brief_pm(squad, line, Some(shown));
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
            ticket.off_main = None;
        }
        let verb = if status == TicketStatus::Done { "accepted" } else { "cancelled" };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.set_ticket_work(ticket.id, &TicketWork { worktree: None, ..saved });
            let _ = journal.ticket_event(ticket.id, "you", verb, "");
        }
        let in_home = status == TicketStatus::Done && ticket.kind.in_home();
        let landed = if in_home { "; its work is in the squad's folder" } else { "" };
        let line = format!("You {verb} T-{}{landed} · {}.", ticket.id, ticket.title);
        let news = match status {
            TicketStatus::Done if in_home => "news.accepted_home",
            TicketStatus::Done => "news.accepted",
            _ => "news.cancelled",
        };
        let shown = self.said(news, &[("id", ticket.id.to_string()), ("title", ticket.title.clone())]);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line, Some(shown));
    }

    /// The person accepts a ticket: it is done, and its branch stays to merge.
    async fn accept(&mut self, squad: &str, ticket: TicketId) {
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status.is_closed() {
            let message = say!(self, "error.already_closed", id = ticket, status = crate::i18n::label::status(self.config.lang, found.status));
            return self.squad_error(squad, message);
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
            let message = say!(self, "error.not_accepted", id = ticket);
            return self.squad_error(squad, message);
        }
        if found.kind.in_home() {
            let message = say!(self, "error.in_folder", id = ticket);
            return self.squad_error(squad, message);
        }
        let (Some(branch), Some(repo)) =
            (found.branch.clone(), found.project.as_deref().and_then(|p| worktree::repo_root(p).ok()))
        else {
            let message = say!(self, "error.no_branch", id = ticket);
            return self.squad_error(squad, message);
        };
        let message = format!("equipe: merge T-{ticket} {}\n\nAccepted by the person; merged from {branch}.", found.title);
        let main = match worktree::merge_into_main(&repo, &branch, &message) {
            Ok(main) => main,
            Err(err) => {
                // Why it stays off main, for the screens to say.
                found.off_main = Some(if err.contains(" conflicts with ") {
                    OffMain::Conflict { detail: err.clone() }
                } else {
                    OffMain::Failed { detail: err.clone() }
                });
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.save_ticket(&found);
                    let _ = journal.ticket_event(ticket, "equipe", "not merged", &err);
                }
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
                let message = say!(self, "error.not_merged", id = ticket, err = err);
                return self.squad_error(squad, message);
            }
        };
        let mut line = format!("You merged T-{ticket} · {} into {main}", found.title);
        let mut news = "news.merged";
        if push {
            match worktree::push_main(&repo) {
                Ok(_) => {
                    line.push_str(" and pushed it");
                    news = "news.merged_pushed";
                }
                Err(err) => {
                    let message = say!(self, "error.not_pushed", branch = main, err = err);
                    self.squad_error(squad, message);
                }
            }
        }
        let shown = self.said(news, &[("id", ticket.to_string()), ("title", found.title.clone()), ("branch", main.clone())]);
        found.merged = Some(true);
        found.off_main = None;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "merged", &main);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        // Work it was built on came along with it.
        self.settle_merged(squad);
        self.brief_pm(squad, format!("{line}."), Some(shown));
    }

    /// Done tickets said not to be on main whose branch has nothing left to
    /// bring there (it got in another way, or it changes nothing: `+0 −0`)
    /// are merged: nothing to ask the person. Their worktree went when they
    /// were accepted; their branch stays, as after any merge.
    fn settle_merged(&mut self, squad: &str) {
        for mut t in self.tickets(squad).into_iter().filter(Ticket::not_on_main) {
            if merged_state(&t) != Some(true) {
                continue;
            }
            t.merged = Some(true);
            t.off_main = None;
            if let Some(journal) = self.journal(squad) {
                let _ = journal.save_ticket(&t);
                let _ = journal.ticket_event(t.id, "equipe", "merged", "nothing left to merge: its work is in main");
            }
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
        }
    }

    /// Every squad open, settled: work that reached main outside equipe stops
    /// asking to be merged.
    fn settle_all_merged(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.settle_merged(&squad);
        }
    }

    /// The person leaves a done ticket's work on its branch: it stops asking
    /// to be merged, and a merge later still lands it.
    fn keep_on_branch(&mut self, squad: &str, ticket: TicketId) {
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status != TicketStatus::Done {
            let message = say!(self, "error.not_accepted", id = ticket);
            return self.squad_error(squad, message);
        }
        if !found.not_on_main() {
            let message = say!(self, "error.no_branch", id = ticket);
            return self.squad_error(squad, message);
        }
        found.off_main = Some(OffMain::Kept);
        let branch = found.branch.clone().unwrap_or_default();
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "kept on branch", &branch);
        }
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
    }

    /// The person sends work back: the ticket goes to its member again, on the
    /// same branch and conversation, with what to change.
    async fn reopen(&mut self, squad: &str, ticket: TicketId, note: String) {
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if self.worker_on(squad, ticket).is_some() {
            let who = found.assignee.clone().unwrap_or_default();
            let message = say!(self, "error.still_worked_on", id = ticket, name = who);
            return self.squad_error(squad, message);
        }
        let said = note.clone();
        let note = if note.trim().is_empty() { "Reopened.".to_owned() } else { format!("Reopened: {}", note.trim()) };
        found.owner = found.assignee.take().or(found.owner);
        found.status = TicketStatus::Todo;
        found.note = Some(note.clone());
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "reopened", &note);
        }
        let line = format!("You reopened T-{} · {} — {note}", found.id, found.title);
        let shown = match said.trim() {
            "" => say!(self, "news.reopened", id = found.id, title = found.title),
            said => say!(self, "news.reopened_note", id = found.id, title = found.title, note = said),
        };
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        self.brief_pm(squad, line, Some(shown));
        self.dispatch(squad).await;
    }

    /// The person did what a `waiting` ticket needed: it goes back to the
    /// member that put it down, in the same worktree and conversation.
    async fn resume(&mut self, squad: &str, ticket: TicketId, note: String) {
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if found.status != TicketStatus::Waiting {
            let status = crate::i18n::label::status(self.config.lang, found.status);
            let message = say!(self, "error.not_waiting", id = ticket, status = status);
            return self.squad_error(squad, message);
        }
        let found = self.resumed(squad, found, "you", &note);
        let line = format!("You resumed T-{} · {}", found.id, found.title);
        let shown = say!(self, "news.resumed", id = found.id, title = found.title);
        self.brief_pm(squad, line, Some(shown));
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
            let message = say!(self, "error.already_closed", id = ticket, status = crate::i18n::label::status(self.config.lang, found.status));
            return self.squad_error(squad, message);
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
            if self.ask_in_panel(&call.squad, &call.member, url.as_deref(), &why) {
                if let Some(id) = self.member_id(&call.squad, &call.member) {
                    let text = say!(self, "notice.browser_in_panel", name = call.member, why = why);
                    self.publish(Event::Notice { session: id, text });
                }
                let _ = reply.send(ToolResult {
                    text: "Your browser is open in the person's browser panel, at that page, in their hands. Tell them what \
to do there and end your turn: you'll be told when they hand it back."
                        .into(),
                    is_error: false,
                });
                return;
            }
            if let Some(url) = url {
                self.show_url.insert((call.squad.clone(), call.member.clone()), url);
            }
            self.show_browser(&call.squad, Some(call.member.clone()), true);
            if let Some(id) = self.member_id(&call.squad, &call.member) {
                let text = say!(self, "notice.browser_on_screen", name = call.member, why = why);
                self.publish(Event::Notice { session: id, text });
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
        let (action, reason) = permissions::describe_in(self.config.lang, &tool, &input);
        let Some(id) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(verdict(Decision::Deny, &input, "Unknown member."));
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        if permissions::blocked(&tool, &action) {
            let _ = reply.send(verdict(Decision::Deny, &input, "equipe never allows this, in any mode."));
            let text = say!(self, "notice.blocked", action = action);
            self.publish(Event::Notice { session: id, text });
            self.audit(&call.squad, ticket, "equipe", "blocked", &action);
            return;
        }
        if let Some(why) = self.off_limits(id, &call.squad, &tool, &action, &input) {
            let _ = reply.send(verdict(Decision::Deny, &input, why));
            let text = say!(self, "notice.refused", why = why, action = action);
            self.publish(Event::Notice { session: id, text });
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
        let action = desktop_gate::describe_in(self.config.lang, &act, &check.scene);
        let why = match desktop_gate::judge(&act, &check.scene, &apps) {
            Verdict::Free => {
                let _ = reply.send(ToolResult { text: String::new(), is_error: false });
                return;
            }
            Verdict::Blocked(why) => {
                let _ = reply.send(refuse(format!("{why} Ask the person to do it.")));
                let text = say!(self, "notice.blocked", action = first_line(&action));
                self.publish(Event::Notice { session: id, text });
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
                    self.publish_allowed(&squad);
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
            (Decision::Always, Some(rule)) => say!(self, "notice.always_allowed", rule = rule, action = approval.action),
            (Decision::Allow | Decision::Always, _) => say!(self, "notice.allowed", action = approval.action),
            (Decision::Deny, _) if why == "The person declined." => say!(self, "notice.declined", action = approval.action),
            (Decision::Deny, _) => say!(self, "notice.declined_why", why = why, action = approval.action),
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

    /// Tell the screens the squad's "always allow" rules, all of them.
    fn publish_allowed(&mut self, squad: &str) {
        let rules = self.journal(squad).and_then(|j| j.allowed().ok()).unwrap_or_default();
        self.publish(Event::Allowed { squad: squad.to_owned(), rules });
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
            return self.squad_error(squad, say!(self, "error.no_member", name = name));
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
        let members: Vec<(String, bool)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .filter_map(|(id, s)| Some((self.browser_key(*id)?.1, s.is_working())))
            .collect();
        for (name, working) in members {
            let shown = self.journals.get(squad).is_some_and(|j| j.browser_shown(browser::member_of(&name)));
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

    /// The browser a conversation of session `id` uses: `(squad, key)`. Its
    /// member's own for the member's conversation; one of its own for a lane,
    /// so conversations side by side never share a tab.
    fn browser_key(&self, id: SessionId) -> Option<(String, String)> {
        let s = self.sessions.get(&id)?;
        let name = s.name()?;
        let key = match &s.lane {
            Some((_, project)) => browser::lane_key(name, project.as_deref()),
            None => name.to_owned(),
        };
        Some((s.squad.clone()?, key))
    }

    /// Start a conversation's browser now — on screen when `shown` — and take
    /// it to the page asked for, or where it last was. `name` is its key.
    fn open_browser(&mut self, squad: &str, name: &str, shown: bool) -> Option<u16> {
        let npx = self.config.npx.clone()?;
        let output = std::env::temp_dir().join("equipe-browser").join(squad).join(name);
        let profile = browser::profile(&self.config.home, squad, name);
        let key = (squad.to_owned(), name.to_owned());
        let member = (squad.to_owned(), browser::member_of(name).to_owned());
        if !self.browsers.get_mut(&key).is_some_and(|b| b.fits(shown)) {
            // A lane of the shared Chrome; a Chrome of its own if that can't be had.
            let running = match self.browser_host(&npx) {
                Some(host) => browser::lane(&host, squad, name, &output, shown).ok(),
                None => None,
            };
            let running = match running {
                Some(running) => running,
                None => browser::launch(&npx, &self.config.home, &profile, &output, shown).ok()?,
            };
            let (client, port) = (self.client.clone(), running.port);
            let page = self
                .show_url
                .remove(&key)
                .or_else(|| self.show_url.remove(&member))
                .or_else(|| self.last_url.get(&key).cloned())
                .filter(|_| shown);
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

    /// The browser host, started — or started again, after its Chrome went
    /// away — when it is wanted. `None` when members use a Chrome each.
    fn browser_host(&mut self, npx: &Path) -> Option<browser::SharedHost> {
        if !self.config.browser_host {
            return None;
        }
        let alive = self.host.as_ref().is_some_and(|h| h.lock().is_ok_and(|mut h| h.alive()));
        if !alive {
            // Its lanes went with it.
            self.browsers.retain(|_, b| !b.is_lane());
            self.host = browser::Host::start(npx, &self.config.home, &self.hub, self.host_events.clone()).ok();
            self.hub.set_host(self.host.clone());
        }
        self.host.clone()
    }

    /// A window in the shared Chrome for the person — to sign in, once, where
    /// every member works. `false` when members use a Chrome each.
    fn person_window(&mut self, url: &str) -> bool {
        let Some(npx) = self.config.npx.clone() else {
            return false;
        };
        let Some(host) = self.browser_host(&npx) else {
            return false;
        };
        host.lock().map(|mut h| h.person(url)).is_ok()
    }

    /// A picture of each watched member's browser, one under way at a time.
    /// A lane of the shared Chrome is watched as a browser panel watches it —
    /// its member's tab, a frame now and then; a Chrome of its own is asked
    /// for a screenshot.
    fn take_frames(&mut self) {
        let watched: Vec<(String, SessionId)> = self.watching.iter().map(|(s, id)| (s.clone(), *id)).collect();
        let mut watching_lanes = Vec::new();
        for (squad, id) in watched {
            if self.framing.contains(&id) {
                continue;
            }
            let Some(key) = self.browser_key(id) else { continue };
            if self.browsers.get(&key).is_some_and(|b| b.is_lane()) {
                let watcher = match self.frame_watchers.get(&squad) {
                    Some(w) => *w,
                    None => {
                        let w = self.hub.watcher();
                        self.frame_watchers.insert(squad.clone(), w);
                        w
                    }
                };
                watching_lanes.push(watcher);
                let lane = format!("{}/{}", key.0, key.1);
                let (page, url) = match self.tabs.get(&lane) {
                    Some(Event::BrowserTabs { pages, agent_page, .. }) => {
                        let page = agent_page.and_then(|a| pages.iter().find(|p| p.id == a)).or(pages.last());
                        (page.map(|p| p.id), page.map(|p| p.url.clone()))
                    }
                    _ => (None, None),
                };
                let Some(page) = page else { continue };
                self.hub.watch(watcher, browser::Want { page, fps: 2, max_w: 1280, max_h: 1280, quality: 60 });
                if let Some(frame) = self.hub.latest(page)
                    && self.frame_sent.get(&id) != Some(&(page, frame.header.seq))
                {
                    use base64::Engine as _;
                    self.frame_sent.insert(id, (page, frame.header.seq));
                    let image = base64::engine::general_purpose::STANDARD.encode(&frame.jpeg);
                    self.publish(Event::BrowserFrame { session: id, image, url });
                }
                continue;
            }
            let Some(port) = self.browsers.get(&key).map(|b| b.port) else { continue };
            let name = key.1;
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
        // Squads that stopped watching a lane stop its frames.
        let idle: Vec<String> = self.frame_watchers.iter().filter(|(_, w)| !watching_lanes.contains(w)).map(|(s, _)| s.clone()).collect();
        for squad in idle {
            if let Some(watcher) = self.frame_watchers.remove(&squad) {
                self.hub.unwatch(watcher);
            }
        }
        let watching = &self.watching;
        self.frame_sent.retain(|id, _| watching.values().any(|w| w == id));
    }

    // ── the shared Chrome, for browser panels ──

    /// What the browser host said.
    async fn on_host(&mut self, said: browser::HostEvent) {
        use browser::HostEvent;
        match said {
            HostEvent::Ready => {}
            HostEvent::Tabs { lane, member, color, pages, agent, .. } => {
                let (squad, session) = self.lane_session(&lane);
                let event = Event::BrowserTabs { squad, lane: lane.clone(), session, member, color, pages, agent_page: agent };
                self.tabs.insert(lane, event.clone());
                self.publish(event);
            }
            HostEvent::Control { lane, by } => {
                let (squad, session) = self.lane_session(&lane);
                let asked = if by.is_none() { self.asked_in.remove(&lane) } else { None };
                match &by {
                    Some(_) => {
                        let why = self.asked_in.get(&lane).cloned();
                        self.held.insert(lane.clone(), why);
                    }
                    None => {
                        self.held.remove(&lane);
                    }
                }
                self.publish(Event::BrowserControl { squad, lane: lane.clone(), by: by.clone(), why: None });
                let Some(session) = session else { return };
                let working = self.sessions.get(&session).is_some_and(Session::is_working);
                match (by.is_some(), asked.is_some()) {
                    // Only a turn under way needs telling: between turns nothing waits on it.
                    (true, _) if working => {
                        let text = say!(self, "notice.browser_taken");
                        self.publish(Event::Notice { session, text });
                    }
                    // Done with what it asked of them: it carries on.
                    (false, true) => self.submit(session, "The person finished in the browser.".into()).await,
                    (false, false) if working => {
                        let text = say!(self, "notice.browser_given_back");
                        self.publish(Event::Notice { session, text });
                    }
                    _ => {}
                }
            }
            HostEvent::Dialog { page, lane, kind, message, default } => {
                let (squad, _) = self.lane_session(&lane);
                self.publish(Event::BrowserDialog { squad, lane, page, kind, message, default });
            }
            HostEvent::DialogClosed { page } => self.publish(Event::BrowserDialogClosed { page }),
            HostEvent::Exited => {
                // Its tabs went with it.
                for (lane, tabs) in std::mem::take(&mut self.tabs) {
                    if let Event::BrowserTabs { squad, session, member, color, .. } = tabs {
                        self.publish(Event::BrowserTabs { squad, lane, session, member, color, pages: Vec::new(), agent_page: None });
                    }
                }
                self.asked_in.clear();
                self.held.clear();
            }
        }
    }

    /// A lane's squad and the conversation it is the browser of: `squad/key`;
    /// `""` is the person's own.
    fn lane_session(&self, lane: &str) -> (Option<String>, Option<SessionId>) {
        let Some((squad, key)) = lane.split_once('/') else {
            return (None, None);
        };
        let session = self.sessions.keys().copied().find(|id| self.browser_key(*id).is_some_and(|(q, k)| q == squad && k == key));
        (Some(squad.to_owned()), session)
    }

    /// A member asks the person into its browser. With a browser panel
    /// attached, the panel shows it, in their hands, at `url` — rather than a
    /// window over whatever they are doing. `false`: show the window as before.
    fn ask_in_panel(&mut self, squad: &str, member: &str, url: Option<&str>, why: &str) -> bool {
        if !self.hub.has_panel() {
            return false;
        }
        let Some(id) = self.member_id(squad, member) else { return false };
        let Some((_, key)) = self.browser_key(id) else { return false };
        if self.open_browser(squad, &key, false).is_none() || !self.browsers.get(&(squad.to_owned(), key.clone())).is_some_and(|b| b.is_lane()) {
            return false;
        }
        let lane = format!("{squad}/{key}");
        if let Some(url) = url {
            self.hub.op(&json!({ "op": "goto", "lane": lane, "url": url }));
        }
        self.hub.op(&json!({ "op": "control", "lane": lane, "by": "person" }));
        self.asked_in.insert(lane.clone(), why.to_owned());
        self.held.insert(lane.clone(), Some(why.to_owned()));
        let (squad, _) = self.lane_session(&lane);
        self.publish(Event::BrowserControl { squad, lane, by: Some("person".into()), why: Some(why.to_owned()) });
        true
    }

    fn set_mode(&mut self, squad: &str, member: Option<String>, mode: Option<Mode>) {
        match &member {
            None => {
                let Some(mode) = mode else {
                    return self.squad_error(squad, say!(self, "error.name_a_mode"));
                };
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.set_mode(mode);
                }
            }
            Some(name) => {
                let Some(id) = self.member_id(squad, name) else {
                    return self.squad_error(squad, say!(self, "error.no_member", name = name));
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
        let ToolCall { squad, member, name, arguments: args, .. } = call;
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
                    off_main: None,
                    created_by: member.clone(),
                    note: None,
                    asks: vec![],
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
                    let shown = say!(self, "news.proposed", name = member, id = new_id, title = title);
                    self.brief_pm(&squad, line, Some(shown));
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
            board::ESTIMATE => {
                let minutes = |key: &str| args.get(key).and_then(Value::as_f64).map(|m| m.max(0.0));
                let (done, live) = (minutes("done_in_minutes"), minutes("live_in_minutes"));
                if done.is_none() && live.is_none() {
                    return Err("Say in how many minutes it is done (done_in_minutes), or live (live_in_minutes).".into());
                }
                let at = |m: f64| now_secs() + (m * 60.0).round() as u64;
                // Said in a conversation beside the member's own: that one's.
                let id = if self.sessions.get(&id).is_some_and(Session::is_working) {
                    id
                } else {
                    self.sessions
                        .iter()
                        .find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && s.is_working())
                        .map_or(id, |(&lane, _)| lane)
                };
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let mut estimate = session.estimate.clone().unwrap_or_default();
                if let Some(m) = done {
                    estimate.done_at = Some(at(m));
                    estimate.guessed = false;
                    estimate.what = board::arg_str(&args, "what").map(|w| clip(w.trim(), 60));
                }
                if let Some(m) = live {
                    // Live now: nothing left to wait for.
                    estimate.live_at = (m > 0.0).then(|| at(m));
                    estimate.live_what = board::arg_str(&args, "live_what").map(|w| clip(w.trim(), 60)).filter(|_| m > 0.0);
                }
                let estimate = Some(estimate).filter(|e| !e.is_empty());
                session.estimate = estimate.clone();
                self.publish(Event::Estimated { session: id, estimate });
                Ok("Shown to the person.".into())
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
                    let what = clip(fact.description.trim(), 80);
                    let text = if new {
                        say!(self, "notice.remembered", name = member, what = what)
                    } else {
                        say!(self, "notice.memory_updated", name = member, what = what)
                    };
                    (change, text)
                } else {
                    let fact = board::arg_str(&args, "name").ok_or("Which fact? Give its name.")?;
                    let change = squad_memory::forget(&folder, &member, fact, language.as_deref())?;
                    (change, say!(self, "notice.forgot", name = member, fact = fact))
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
                    recent: scope::from(entries, entries.len().saturating_sub(SNAPSHOT_LINES)),
                    queued: s.queued.clone(),
                    queued_project: s.memo_threads.first().cloned().flatten(),
                    queued_in: s.memo_threads.clone(),
                    steering: s.steering.clone(),
                    lane: s.lane.clone().map(|(of, project)| equipe_protocol::Lane { of, project }),
                    estimate: s.estimate.clone().filter(|e| !e.is_empty()),
                    work: s.work.as_ref().map(|w| WorkInfo {
                        ticket: w.ticket,
                        project: w.project.clone(),
                        workspace: w.sandbox.as_ref().map(workspace_of),
                    }),
                    mode: s.mode,
                    effort: s.effort.clone(),
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
        let allowed = self.journals.get(squad).and_then(|j| j.allowed().ok()).unwrap_or_default();
        let browsers_shown = self.journals.get(squad).is_some_and(Journal::browsers_all_shown);
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
            allowed,
            browsers_shown,
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
                // Said while a turn ran that the restart ended, unread: memos now.
                let mut queued = queued;
                let unread = journal.waiting(record.id, "steer").unwrap_or_default();
                if !unread.is_empty() {
                    let _ = journal.set_waiting(record.id, "steer", &[]);
                    let mut memos = queued.clone();
                    memos.extend(unread.iter().cloned());
                    let _ = journal.set_waiting(record.id, "memo", &memos);
                }
                for text in unread {
                    queued.push(text);
                    memo_threads.push(None);
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
                        steer: None,
                        steering: Vec::new(),
                        briefing: journal.waiting(record.id, "briefing").unwrap_or_default(),
                        memory,
                        mode: record.mode,
                        thread,
                        estimate: None,
                        turn_began: None,
                        threads,
                        lane: None,
                        answers_person: false,
                        effort: None,
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
            let text = say!(self, "notice.restarted", id = ticket);
            self.publish(Event::Notice { session: id, text });
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
        let shown = match (full, cold) {
            _ if !summary => say!(self, "notice.fresh_clean"),
            (true, _) => say!(self, "notice.fresh_full", used = used / 1000, window = window / 1000),
            (_, true) => say!(self, "notice.fresh_cold", used = used / 1000),
            _ => say!(self, "notice.fresh_asked"),
        };
        let squad = session.squad.clone();
        if let Some(journal) = squad.as_deref().and_then(|s| journal_for(&mut self.journals, &self.config.home, s)) {
            let _ = journal.clear_engine_session(id);
        }
        self.publish(Event::Notice { session: id, text: shown });
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
    /// Members' build caches kept to a size, every half hour, off the loop.
    fn trim_builds(&mut self) {
        if self.builds_trimmed.is_some_and(|at| at.elapsed() < BUILDS_TRIM_EVERY) {
            return;
        }
        self.builds_trimmed = Some(std::time::Instant::now());
        // A member mid-turn may be building: its cache is left alone.
        let busy: Vec<PathBuf> = self
            .sessions
            .values()
            .filter(|s| s.is_working())
            .filter_map(|s| Some(journal::build_dir(&self.config.home, s.squad.as_deref()?, s.name()?)))
            .collect();
        let home = self.config.home.clone();
        std::thread::spawn(move || {
            for line in worktree::trim_builds(&home, &busy, BUILD_CAP) {
                eprintln!("equipe: {line}");
            }
        });
    }

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
                let shown = say!(self, "news.stale", count = stale.len());
                self.brief_pm(&squad, line, Some(shown));
            }
        }
    }

    fn close_idle_browsers(&mut self) {
        let working: Vec<(String, String)> =
            self.sessions.iter().filter(|(_, s)| s.is_working()).filter_map(|(id, _)| self.browser_key(*id)).collect();
        let present: Vec<(String, String)> = self.sessions.keys().filter_map(|id| self.browser_key(*id)).collect();
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
                let _ = journal.show_browser(Some(browser::member_of(&name)), false, &team);
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
        // Tickets' worktrees give back their space: builds where no one works,
        // the worktrees no ticket keeps any more. Gigabytes: off the loop.
        let in_use: Vec<PathBuf> = self.sessions.values().filter_map(|s| s.work.as_ref()?.sandbox.as_ref().map(|w| w.path.clone())).collect();
        let kept: Vec<PathBuf> = self
            .journals
            .values()
            .flat_map(|j| j.tickets().unwrap_or_default())
            .filter(|t| !t.status.is_closed())
            .filter_map(|t| t.worktree)
            .collect();
        let home = self.config.home.clone();
        std::thread::spawn(move || {
            for line in worktree::tidy(&home, &in_use, &kept) {
                eprintln!("equipe: {line}");
            }
        });
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
with what they need to do — each ask its own item, a full sentence, files by their path in backticks — and end your \
turn: you are free for other work until they have. If you need a decision, \
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
async fn codex_credentials(client: &reqwest::Client, store: &CredentialStore, lang: crate::i18n::Lang) -> Result<Credentials, (String, &'static str)> {
    let words = crate::i18n::core(lang);
    let cred = store
        .load("openai")
        .map_err(|err| (words.fill("error.codex_unreadable", &[("path", store.path("openai").display().to_string()), ("err", err.to_string())]), "equipe login codex"))?
        .filter(|c| c.method == AuthMethod::OauthSubscription)
        .ok_or((words.get("error.codex_signed_out").to_owned(), "equipe login codex"))?;
    codex::ensure_fresh(client, store, cred)
        .await
        .map_err(|err| (words.fill("error.codex_not_refreshed", &[("err", err.to_string())]), "equipe login codex"))
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

/// Turns shorter than this need no guess at when they end, and aren't kept
/// to guess from.
const GUESSED_AFTER: Duration = Duration::from_secs(60);

/// How big a member's build cache may grow before it is trimmed, and how often
/// that is looked at.
const BUILD_CAP: u64 = 10 * 1024 * 1024 * 1024;
const BUILDS_TRIM_EVERY: Duration = Duration::from_secs(30 * 60);

/// How long past its expected time something to go live is still waited for.
const LIVE_STALE: Duration = Duration::from_secs(30 * 60);

/// How long a lane waits for its next message before it goes back into its member.
const LANE_IDLE: Duration = Duration::from_secs(30 * 60);

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
