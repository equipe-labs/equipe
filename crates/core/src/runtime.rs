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
use equipe_providers::codex_app;
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
use crate::routines;
use crate::vault;
use equipe_protocol::routines::Routine;
use crate::squad_home;
use crate::squad_memory;
use crate::steers;
use crate::watchdog;
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

mod decide;
mod room;
mod engines;
mod design;
mod accept;
mod secrets;
mod design_qa;
mod team;
mod tickets;

/// How many recent lines a snapshot carries per session.
const SNAPSHOT_LINES: usize = 200;

/// How much of a member's answer the PM is told.
const BRIEF_REPLY_CHARS: usize = 400;

/// Closed tickets a snapshot still shows, newest first.
const CLOSED_TICKETS_SHOWN: usize = 20;
/// The routines one squad may keep.
const MAX_ROUTINES: usize = 20;

/// The MCP server equipe gives members; its tools are `mcp__equipe__*`.
const MCP_SERVER: &str = "equipe";
/// Claude Code's own question tool, which members don't use (`ask_decision` instead).
const ENGINE_QUESTION_TOOL: &str = "AskUserQuestion";
/// The Mac's desktop: its tools are `mcp__desktop__…`.
pub const DESKTOP_SERVER: &str = "desktop";
/// The tool name a member's own question shows under, among approvals.
const CONFIRM: &str = "Confirm";
/// An approval to sign a member in with a kept login (`vault`).
const SIGN_IN_TOOL: &str = "Sign in";

/// How long `busy` waits for the loop before it answers with what the loop is on.
pub const ANSWER_WAIT: Duration = Duration::from_secs(3);

/// How long the loop waits on a server (a Codex sign-in, its models) or on
/// `claude auth status` before it gives up: nothing else is done meanwhile.
const SERVER_WAIT: Duration = Duration::from_secs(30);

/// How often the sweep looks for done work that reached main outside equipe.
const SETTLE_EVERY: Duration = Duration::from_secs(10 * 60);


/// How long a member waits for the person to answer a request for permission
/// before it is declined.
const APPROVAL_WAIT: Duration = Duration::from_secs(30 * 60);

/// How long a stopped Codex turn has to wind down before it is cut.
const STOP_GRACE: Duration = Duration::from_secs(8);

/// How often the Codex plan's usage is read while a squad has Codex members,
/// and at most when a screen attaches.
const CODEX_USAGE_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long the Codex CLI's model list is kept.
const CODEX_MODELS_KEPT: Duration = Duration::from_secs(10 * 60);

pub struct Config {
    pub store: CredentialStore,
    /// equipe's home: squads, worktrees and logs live under it.
    pub home: PathBuf,
    /// Reported to backends as equipe's version.
    pub client_version: String,
    /// The Claude Code executable to run.
    pub claude_binary: String,
    /// The Codex CLI to run (`codex app-server`). Where it isn't installed,
    /// Codex falls back to equipe's own sign-in, without tools.
    pub codex_binary: String,
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
    /// The person's home folder, where secrets members ask for are saved
    /// (`ask_secret`); `None`: `$HOME`.
    pub person_home: Option<PathBuf>,
    /// The language equipe speaks to the person in until a screen says
    /// otherwise: notices, errors. What it tells the models stays English.
    pub lang: crate::i18n::Lang,
    /// An NVIDIA Nemotron model on Nebius Token Factory reviews each code
    /// ticket handed in (`nemotron`); `None`: no `NEBIUS_API_KEY`, no review.
    pub nemotron: Option<crate::nemotron::Config>,
}

impl Config {
    /// The user's real setup: `~/.equipe` and the `claude` on PATH, in memory only.
    pub fn for_user(client_version: &str) -> Self {
        Self {
            store: CredentialStore::default_location(),
            home: equipe_home(),
            client_version: client_version.to_owned(),
            claude_binary: claude_code::DEFAULT_BINARY.to_owned(),
            codex_binary: codex_app::DEFAULT_BINARY.to_owned(),
            persist: false,
            retention_days: journal::DEFAULT_RETENTION_DAYS,
            equipe_binary: std::env::current_exe().ok(),
            npx: crate::browser::find_npx(),
            mac: Box::new(desktop::ThisMac),
            suggest: true,
            browser_host: cfg!(target_os = "macos"),
            person_home: None,
            lang: crate::i18n::Lang::resolve(crate::i18n::Front::Terminal),
            nemotron: crate::nemotron::Config::from_env().ok(),
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
    /// May this member use the desktop: equipe's home and the squad's
    /// messaging apps if so.
    DesktopTool(ToolCall, oneshot::Sender<Result<(PathBuf, Vec<String>), ToolResult>>),
    Busy(oneshot::Sender<equipe_protocol::rpc::Busy>),
    /// What uses the disk (`equipe disk`).
    Disk(oneshot::Sender<crate::disk::DiskReport>),
    /// Give back these items of the disk (`equipe disk clean`).
    DiskClean(Vec<String>, oneshot::Sender<crate::disk::DiskCleaned>),
    PersonWindow(String, oneshot::Sender<bool>),
    BrowserHub(oneshot::Sender<std::sync::Arc<browser::Hub>>),
    Desktop(DesktopRequest),
    /// An engine's plan, read outside a turn.
    Usage(Engine, equipe_protocol::PlanUsage),
    Shutdown(oneshot::Sender<()>),
}

/// Where screens send commands. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    requests: UnboundedSender<Request>,
    /// What the loop is on, for an answer when it doesn't answer.
    pulse: std::sync::Arc<watchdog::Pulse>,
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
        rx.await.unwrap_or(ToolResult { text: "equipe is shutting down".into(), is_error: true, image: None })
    }

    /// A member's desktop tool call: the runtime checks the member may, it
    /// waits its turn at the screen ([`Handle::desktop_ask`]), an input passes
    /// the send gate ([`Handle::desktop_gate`]) on what the helper says it
    /// would act on, and the helper does it — outside the runtime's loop, so a
    /// click that takes a second holds no one else up.
    pub async fn desktop(&self, call: ToolCall) -> ToolResult {
        use equipe_desktop_control::helper;
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::DesktopTool(call.clone(), tx));
        let (home, apps) = match rx.await {
            Ok(Ok(allowed)) => allowed,
            Ok(Err(refused)) => return refused,
            Err(_) => return ToolResult::error("equipe is shutting down"),
        };
        let who = MemberRef { squad: call.squad.clone(), member: call.member.clone() };
        let member = format!("{}/{}", call.squad, call.member);
        let what = desktop::Use::of(&call.name);
        let mut start = match self.desktop_ask(who.clone(), what).await {
            Ok(start) => start,
            Err(refused) => return ToolResult::error(refused),
        };
        let act = Act::of(&call.name, &call.arguments);
        let mut arguments = call.arguments.clone();
        if !act.is_look() {
            match self.desktop_check(&home, &apps, &call, &member, act).await {
                Ok((mut scene, asked)) => {
                    // Only a draft the person said yes to must stay as it was.
                    if !asked {
                        for key in ["target", "focused"] {
                            if let Some(element) = scene.get_mut(key).and_then(Value::as_object_mut) {
                                element.remove("value");
                            }
                        }
                    }
                    arguments[equipe_desktop_control::controller::EXPECT] = scene;
                    // The person took their time: the screen may have gone on
                    // to another member, or been taken back, meanwhile.
                    if asked {
                        start = match self.desktop_ask(who.clone(), desktop::Use::Approved).await {
                            Ok(start) => start,
                            Err(refused) => return ToolResult::error(format!("The person said yes, but nothing was sent: {refused}")),
                        };
                    }
                }
                Err(refused) => {
                    self.desktop_done(who, start, false);
                    return refused;
                }
            }
        }
        let result = helper::call(&home, &member, &call.name, &arguments).await;
        self.desktop_done(who, start, what != desktop::Use::Look);
        result
    }

    /// An input, before it reaches the screen: what it would act on, from the
    /// helper, through the send gate. The scene the gate judged — the helper
    /// acts only while the screen is still that — and whether the person was asked.
    async fn desktop_check(&self, home: &Path, apps: &[String], call: &ToolCall, member: &str, act: Act) -> Result<(Value, bool), ToolResult> {
        use base64::Engine as _;
        use equipe_desktop_control::helper;
        let read = helper::call(home, member, helper::SCENE, &json!({ "tool": call.name, "arguments": call.arguments })).await;
        if read.is_error {
            return Err(read);
        }
        let raw: Value = serde_json::from_str(&read.text).map_err(|_| ToolResult::error("The desktop helper's answer was unreadable: nothing done."))?;
        let scene: desktop_gate::Scene =
            serde_json::from_value(raw.clone()).map_err(|_| ToolResult::error("The desktop helper's answer was unreadable: nothing done."))?;
        let asks = matches!(desktop_gate::judge(&act, &scene, apps), Verdict::Ask(_));
        let said = call.arguments.get("why").and_then(Value::as_str).is_some_and(|w| !w.trim().is_empty());
        // The person sees the screen they are asked about.
        let screen = if asks && said {
            let shot = helper::call(home, member, helper::CAPTURE, &json!({})).await;
            shot.image.and_then(|image| base64::engine::general_purpose::STANDARD.decode(image.data).ok())
        } else {
            None
        };
        let check = desktop_gate::Check {
            squad: call.squad.clone(),
            member: call.member.clone(),
            tool: call.name.clone(),
            input: call.arguments.clone(),
            scene,
            screen,
        };
        let verdict = self.desktop_gate(check).await;
        if verdict.is_error { Err(verdict) } else { Ok((raw, asks)) }
    }

    /// Whether a member's desktop input may go ahead: at once, after the
    /// person's yes when it may send in their name, or never. `is_error` means
    /// no, with why in `text`; the input must not reach the screen.
    pub async fn desktop_gate(&self, check: desktop_gate::Check) -> ToolResult {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::DesktopGate(Box::new(check), tx));
        rx.await.unwrap_or(ToolResult { text: "equipe is shutting down".into(), is_error: true, image: None })
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
    /// When the loop doesn't answer within [`ANSWER_WAIT`]: what it is on.
    pub async fn busy(&self) -> equipe_protocol::rpc::Busy {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Busy(tx));
        match tokio::time::timeout(ANSWER_WAIT, rx).await {
            Ok(answer) => answer.unwrap_or_default(),
            Err(_) => equipe_protocol::rpc::Busy { stalled: self.stall(), ..Default::default() },
        }
    }

    /// What uses the disk, looked at now (off the loop; within its time
    /// budget). `None` when the daemon doesn't answer.
    pub async fn disk_report(&self) -> Option<crate::disk::DiskReport> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Disk(tx));
        tokio::time::timeout(crate::disk::SCAN_BUDGET * 2, rx).await.ok()?.ok()
    }

    /// Give back these items of the disk, those still safe to remove.
    pub async fn disk_clean(&self, ids: Vec<String>) -> Option<crate::disk::DiskCleaned> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::DiskClean(ids, tx));
        tokio::time::timeout(Duration::from_secs(600), rx).await.ok()?.ok()
    }

    /// The loop's step under way, however short; `None` between steps.
    pub fn stall(&self) -> Option<equipe_protocol::rpc::Stall> {
        self.pulse.stall(Duration::ZERO)
    }

    /// An engine's plan usage read outside a turn (a poll of Codex's limits,
    /// say): told every screen as [`Event::PlanUsage`], and weighed for
    /// moving members between engines at once.
    pub fn usage_read(&self, engine: Engine, usage: equipe_protocol::PlanUsage) {
        let _ = self.requests.send(Request::Usage(engine, usage));
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
    if config.persist && config.nemotron.is_none() {
        crate::daemon_log!("equipe: no NEBIUS_API_KEY in the daemon's environment: handed-in work isn't reviewed by Nemotron");
    }
    // The logins: their passwords in the Keychain (in memory for tests).
    let secrets: Box<dyn vault::Secrets> = if std::env::var_os("EQUIPE_VAULT_IN_MEMORY").is_some() { Box::new(vault::InMemory::default()) } else { Box::new(vault::Keychain) };
    let vault = vault::Vault::open(&config.home, secrets);
    let logins = vault.logins.clone();
    let mut runtime = Runtime {
        vault: std::sync::Arc::new(std::sync::Mutex::new(vault)),
        logins,
        secret_asks: HashMap::new(),
        desktop: desktop::Desktop::new(mac),
        config,
        // A server that never answers a connection must not hold anything up.
        client: reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap_or_default(),
        subscribers: vec![(None, evt_tx)],
        internal: internal_tx,
        sessions: HashMap::new(),
        journals: HashMap::new(),
        next_join: 1,
        pending: HashMap::new(),
        next_approval: 1,
        focus: HashMap::new(),
        said_about: None,
        builds_trimmed: None,
        browsers: HashMap::new(),
        host: None,
        hub: browser::Hub::new(),
        host_events: host_tx,
        tabs: HashMap::new(),
        asked_in: HashMap::new(),
        held: HashMap::new(),
        checking: HashMap::new(),
        let_go: std::collections::HashSet::new(),
        watching: HashMap::new(),
        framing: std::collections::HashSet::new(),
        frame_watchers: HashMap::new(),
        frame_sent: HashMap::new(),
        last_url: HashMap::new(),
        show_url: HashMap::new(),
        plan: None,
        plans: Vec::new(),
        plan_types: HashMap::new(),
        codex_read: None,
        codex_reading: false,
        codex_models: None,
        questions: HashMap::new(),
        team_changes: HashMap::new(),
        accepts: HashMap::new(),
        accepts_declined: HashMap::new(),
        merged_cache: Some(worktree::MergedCache::default()),
        settle_again: false,
        settled_at: None,
        merging: std::collections::HashSet::new(),
        disk: Default::default(),
        awake: Default::default(),
        sign_ins: HashMap::new(),
        next_sign_in: 1,
        codex_binary: crate::engines::codex_binary(),
        engines: Default::default(),
        design_watch: HashMap::new(),
        design_busy: std::collections::HashSet::new(),
    };
    let mut memos = Vec::new();
    if runtime.config.persist {
        runtime.restore();
        runtime.prune();
        memos = runtime.resume_work();
    }
    let pulse = watchdog::Pulse::new();
    watchdog::watch(&pulse);
    tokio::spawn(CORE_LOOP.scope((), run(runtime, req_rx, internal_rx, host_rx, pulse.clone())));
    let handle = Handle { requests: req_tx, pulse };
    // Memos for a turn the restart ended go in first, as if just sent.
    for (session, text) in memos {
        handle.send(Command::Submit { session, text, about: None });
    }
    (handle, evt_rx)
}

tokio::task_local! {
    /// Set for the core loop's task: what must never run on it can tell.
    static CORE_LOOP: ();
}

/// Whether this runs on the core loop — the one task every screen waits on.
pub(crate) fn on_core_loop() -> bool {
    CORE_LOOP.try_with(|()| ()).is_ok()
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
    /// The pictures `submit_for_review` named, resolved: shown above the hand-in.
    shown: Vec<String>,
    /// Set by `block_on_person`: what the person has to do. The ticket waits
    /// for them when the turn ends.
    waiting: Option<String>,
    /// Set by `submit_for_review` on a code ticket that changes what users
    /// see, in a squad with a designer: it goes to design QA first.
    screens: bool,
    /// Set by `design_qa` on a design QA ticket: the designer's word.
    verdict: Option<design_qa::Verdict>,
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
    /// equipe's own Codex sign-in, for the fallback without the Codex CLI:
    /// loaded when the first such turn needs it.
    codex: Option<Credentials>,
    /// That fallback keeps nothing server-side; the conversation travels with each turn.
    history: Vec<responses::Message>,
    /// The engine keeps the conversation; this id continues it — Claude Code's
    /// session, or the Codex CLI's thread.
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
    /// Stops the turn running now through its engine, which then winds it
    /// down (Codex's `turn/interrupt`). `None`: stopping kills it.
    stop: Option<oneshot::Sender<()>>,
    /// For each of those, the project the person was looking at (`None`: the whole squad).
    steer_threads: Vec<Option<String>>,
    /// Steers taken back after they went into the turn: not shown when the
    /// engine echoes them, as it reads them with the word to disregard them.
    taken_back: Vec<String>,
    /// The words a message sent at once ([`Command::SendNow`]) goes to the
    /// engine as — each part with its project — in place of the message.
    send_now_prompt: Option<String>,
    /// The subagents it handed work to this turn and that haven't handed
    /// back — its id, what it was handed, its latest step, since when — for a
    /// screen that attaches now.
    subagents: Vec<(String, String, Option<String>, std::time::Instant)>,
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
    /// A lane that is a conversation the person opened with the PM: listed,
    /// kept through restarts, never folded away.
    kept: bool,
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

/// How a merge went off the loop: main's name and, when a push was asked
/// for, how that went — or why nothing was merged.
type MergeResult = Result<(String, Option<Result<(), String>>), String>;

enum Internal {
    Event(Box<Event>),
    Done(TurnDone),
    /// Nobody answered a request for permission in time.
    ApprovalTimedOut(u64),
    /// Codex asks before it acts, as Claude Code does through `approve`.
    Ask(ToolCall, oneshot::Sender<ToolResult>),
    /// The Codex plan's limits, as just read or reported; `None`: they couldn't be read.
    CodexLimits(Option<equipe_providers::turn::Limits>),
    /// A picture of a watched member's browser, or none to be had.
    Frame(SessionId, Option<(Vec<u8>, Option<String>)>),
    /// The look at done work off main, worked out off the loop: the tickets
    /// (by squad) whose branch has nothing left to bring, and what it remembers.
    Settled { merged: Vec<(String, TicketId)>, cache: worktree::MergedCache },
    /// A merge the person asked for, worked out off the loop: main's name and,
    /// when a push was asked for, how it went — or why nothing was merged.
    Merged { squad: String, ticket: TicketId, result: MergeResult },
    /// The free space on equipe's disk, as a look begun then found it.
    Disk(Option<crate::disk::DiskStatus>, std::time::Instant),
    /// What uses the disk, as a look begun then found it.
    DiskReport(Box<crate::disk::DiskReport>, std::time::Instant),
    /// What a clean gave back, the free space after it, and who asked.
    DiskCleaned(Box<crate::disk::DiskCleaned>, Option<crate::disk::DiskStatus>, std::time::Instant, Option<oneshot::Sender<crate::disk::DiskCleaned>>),
    /// What a trim of the build caches gave back when the disk ran short, one line each.
    DiskTrimmed(u64, Vec<String>),
    /// A project the person added, or why it wasn't.
    ProjectAdded { squad: String, result: Result<PathBuf, String> },
    /// The logins changed (or couldn't): what screens see of them now.
    Logins(Result<Vec<crate::protocol::vault::Login>, String>),
    /// Something waits for a conversation to act on — work handed in for the
    /// PM to accept: it is woken with `prompt` (read at its next step, at work).
    Wake { session: SessionId, prompt: String },
    /// Hand ready work out in every squad again (the disk has room once more).
    Dispatch,
    /// Where the engines stand, as just looked at off the loop.
    Engines(Vec<equipe_protocol::EngineStatus>),
    /// A sign-in (the one numbered so) ended.
    SignInEnded(Engine, u64),
    /// Whether Claude is signed in, as just looked at off the loop.
    ClaudeReady(bool),
    /// A squad's design systems looked at off the loop: their stamp, and —
    /// when they changed or were asked for — what they are.
    Design(String, u64, Option<design::Catalog>),
    /// Nemotron's review of a code ticket handed in by `member`, or why there is none.
    Reviewed { squad: String, ticket: TicketId, member: SessionId, result: Result<Box<crate::nemotron::Reviewed>, crate::nemotron::ReviewError> },
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
#[derive(Clone, PartialEq, Eq)]
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
    /// The person gave the ticket to someone else (`to`, a member or a role):
    /// it goes back on the board for them, its work so far on its branch.
    Reassigned { to: String },
    /// A designer finished a design QA ticket (`design_qa`): it is done, its
    /// files in the squad's folder, and the ticket it checked moves on.
    Checked,
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
    /// While a message of the person's is handled: the project the screen
    /// that sent it shows (`Some(None)`: the whole squad) — it, not the
    /// focus another screen set last, says what the message is about.
    said_about: Option<Option<String>>,
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
    /// Lanes put in the person's hands by equipe itself, for a page that
    /// waits for a person ([`browser::human_check`]): the page, its address
    /// then, and what it waits for.
    checking: HashMap<String, (u64, String, equipe_protocol::HumanCheck)>,
    /// Pages the person handed back still waiting: not taken from their
    /// member again (lane, page, address).
    let_go: std::collections::HashSet<(String, u64, String)>,
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
    /// Every engine's plan usage, as last read, and each plan's name.
    plans: Vec<equipe_protocol::EnginePlan>,
    plan_types: HashMap<Engine, String>,
    /// The Codex CLI's plan usage: when it was last read (or tried), and
    /// whether a read is under way.
    codex_read: Option<std::time::Instant>,
    codex_reading: bool,
    /// The models the Codex CLI offers, as last listed.
    codex_models: Option<(std::time::Instant, Vec<String>)>,
    /// The Mac's screen, one member at a time, across squads.
    desktop: desktop::Desktop,
    /// Members' questions waiting for the person, by squad (`decide`).
    questions: HashMap<String, Vec<equipe_protocol::decide::QuestionSet>>,
    /// Changes to the team the PM proposed, waiting for the person, by squad (`team`).
    team_changes: HashMap<String, Vec<equipe_protocol::team::TeamChange>>,
    /// Handed-in work put to the person to confirm, by squad (`accept`).
    accepts: HashMap<String, Vec<equipe_protocol::accept::AcceptProposal>>,
    /// Cards the person said no to lately, by squad: not put up again soon.
    accepts_declined: HashMap<String, Vec<crate::accept::Declined>>,
    /// What the look at done work off main remembers; `None` while a look is
    /// under way off the loop, and `settle_again` when another was asked meanwhile.
    merged_cache: Option<worktree::MergedCache>,
    settle_again: bool,
    /// When the last sweep began.
    settled_at: Option<std::time::Instant>,
    /// Merges under way off the loop, by squad and ticket.
    merging: std::collections::HashSet<(String, TicketId)>,
    /// The disk's room, as last looked at or as a write said (`room.rs`).
    disk: room::DiskWatch,
    /// The Mac held awake while members work or a screen is attached.
    awake: crate::awake::Awake,
    /// Sign-ins under way on this machine, by engine: which one, and its task.
    sign_ins: HashMap<Engine, (u64, JoinHandle<()>)>,
    next_sign_in: u64,
    /// The `codex` the person's Codex sign-in runs.
    codex_binary: String,
    /// Both engines' plans, and members moving between them (`engines`).
    engines: engines::Engines,
    /// Squads whose design systems a screen looks at, and their stamp as
    /// last sent; those being looked at off the loop now (`design`).
    design_watch: HashMap<String, u64>,
    /// The logins kept (`vault`): touched off the loop only — the Keychain
    /// may ask the person before it answers.
    vault: std::sync::Arc<std::sync::Mutex<vault::Vault>>,
    /// What the screens are told of them, as last kept.
    logins: Vec<crate::protocol::vault::Login>,
    design_busy: std::collections::HashSet<String>,
    /// Secrets members asked the person for (`ask_secret`), by squad — read
    /// from its journal when first wanted.
    secret_asks: HashMap<String, Vec<crate::protocol::secrets::SecretAsk>>,
}

/// What woke the loop.
enum Wake {
    Request(Option<Request>),
    Internal(Internal),
    Host(browser::HostEvent),
    Daily,
    DesktopTick,
    Frames,
    Minute,
    DesignTick,
}

impl Wake {
    /// What the loop is doing for it, as the watchdog tells it.
    fn doing(&self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow::{Borrowed, Owned};
        match self {
            Wake::Request(Some(Request::Command(command))) => {
                let name = serde_json::to_value(command).ok().and_then(|v| v["type"].as_str().map(str::to_owned));
                Owned(format!("command {}", name.unwrap_or_else(|| "?".into())))
            }
            Wake::Request(Some(Request::Tool(call, _))) => Owned(format!("tool {} for {}/{}", call.name, call.squad, call.member)),
            Wake::Request(Some(Request::Snapshot(..))) => Borrowed("snapshot"),
            Wake::Request(Some(Request::Subscribe(..))) => Borrowed("hello (a screen's snapshot)"),
            Wake::Request(Some(Request::DesktopGate(..))) => Borrowed("desktop gate"),
            Wake::Request(Some(Request::DesktopTool(..))) => Borrowed("desktop tool"),
            Wake::Request(Some(Request::Busy(_))) => Borrowed("busy"),
            Wake::Request(Some(Request::PersonWindow(..))) => Borrowed("person's browser window"),
            Wake::Request(Some(Request::BrowserHub(_))) => Borrowed("browser panel"),
            Wake::Request(Some(Request::Desktop(_))) => Borrowed("desktop lease"),
            Wake::Request(Some(Request::Usage(..))) => Borrowed("an engine's plan usage"),
            Wake::Request(Some(Request::Shutdown(_))) | Wake::Request(None) => Borrowed("shutdown"),
            Wake::Internal(Internal::Event(_)) => Borrowed("an engine's event"),
            Wake::Internal(Internal::Done(_)) => Borrowed("a turn's end"),
            Wake::Internal(Internal::ApprovalTimedOut(_)) => Borrowed("an approval timing out"),
            Wake::Internal(Internal::Ask(..)) => Borrowed("codex asking for approval"),
            Wake::Internal(Internal::CodexLimits(_)) => Borrowed("codex's plan usage"),
            Wake::Internal(Internal::Frame(..)) => Borrowed("a browser picture"),
            Wake::Internal(Internal::Settled { .. }) => Borrowed("settling merged work"),
            Wake::Internal(Internal::Merged { .. }) => Borrowed("a merge's end"),
            Wake::Internal(Internal::Disk(..)) => Borrowed("the disk's free space"),
            Wake::Internal(Internal::DiskReport(..)) => Borrowed("what uses the disk"),
            Wake::Internal(Internal::DiskCleaned(..)) => Borrowed("a clean of the disk"),
            Wake::Internal(Internal::ProjectAdded { .. }) => Borrowed("a project added"),
            Wake::Internal(Internal::Logins(_)) => Borrowed("the logins changed"),
            Wake::Internal(Internal::Wake { .. }) => Borrowed("work waits for a conversation"),
            Wake::Internal(Internal::DiskTrimmed(..)) => Borrowed("a trim of the build caches"),
            Wake::Internal(Internal::Dispatch) => Borrowed("handing out work again"),
            Wake::Request(Some(Request::Disk(_))) => Borrowed("a look at the disk"),
            Wake::Request(Some(Request::DiskClean(..))) => Borrowed("a clean of the disk"),
            Wake::Internal(Internal::Engines(_)) => Borrowed("where the engines stand"),
            Wake::Internal(Internal::SignInEnded(..)) => Borrowed("a sign-in's end"),
            Wake::Internal(Internal::ClaudeReady(_)) => Borrowed("whether Claude is signed in"),
            Wake::Internal(Internal::Design(..)) => Borrowed("a design system's look"),
            Wake::Internal(Internal::Reviewed { .. }) => Borrowed("a Nemotron review"),
            Wake::Host(_) => Borrowed("the browser host's news"),
            Wake::Daily => Borrowed("daily prune"),
            Wake::DesktopTick => Borrowed("desktop tick"),
            Wake::Frames => Borrowed("browser pictures"),
            Wake::Minute => Borrowed("minute tick"),
            Wake::DesignTick => Borrowed("design tick"),
        }
    }
}

async fn run(
    mut rt: Runtime,
    mut requests: UnboundedReceiver<Request>,
    mut internal: UnboundedReceiver<Internal>,
    mut host: UnboundedReceiver<browser::HostEvent>,
    pulse: std::sync::Arc<watchdog::Pulse>,
) {
    use tokio::time::MissedTickBehavior;
    let mut daily = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    let mut minute = tokio::time::interval(Duration::from_secs(60));
    // A screen held by a member that stopped using it goes to the next in line.
    let mut desktop_tick = tokio::time::interval(Duration::from_secs(5));
    let mut frames = tokio::time::interval(FRAME_EVERY);
    // A design system on a screen follows what its designer saves.
    let mut design_tick = tokio::time::interval(design::LOOK_EVERY);
    // A loop held up never catches up on the ticks it missed in a burst: that
    // only holds it up again.
    for tick in [&mut daily, &mut minute, &mut desktop_tick, &mut frames, &mut design_tick] {
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    }
    daily.tick().await; // the first tick is immediate; pruning already ran at start
    loop {
        let wake = tokio::select! {
            request = requests.recv() => Wake::Request(request),
            Some(message) = internal.recv() => Wake::Internal(message),
            Some(said) = host.recv() => Wake::Host(said),
            _ = daily.tick(), if rt.config.persist => Wake::Daily,
            _ = desktop_tick.tick() => Wake::DesktopTick,
            _ = frames.tick() => Wake::Frames,
            _ = minute.tick() => Wake::Minute,
            _ = design_tick.tick(), if !rt.design_watch.is_empty() => Wake::DesignTick,
        };
        pulse.begin(wake.doing());
        let go_on = rt.wake(wake, &pulse).await;
        pulse.end();
        if !go_on {
            return;
        }
    }
}

impl Runtime {
    /// One turn of the loop. `false`: the runtime is done.
    async fn wake(&mut self, wake: Wake, pulse: &watchdog::Pulse) -> bool {
        match wake {
            Wake::Request(request) => match request {
                Some(Request::Command(command)) => self.handle(command).await,
                Some(Request::Snapshot(squad, reply)) => {
                    let _ = reply.send(self.snapshot(&squad));
                }
                Some(Request::Subscribe(squad, reply)) => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    // The shared Chrome's tabs as they stand, then what changes.
                    for tabs in self.tabs.values() {
                        if tabs.for_everyone() || tabs.squad() == Some(squad.as_str()) {
                            let _ = tx.send(tabs.clone());
                        }
                    }
                    // And the members' browsers in the person's hands.
                    for (lane, why) in &self.held {
                        if lane.split_once('/').is_some_and(|(q, _)| q == squad) {
                            let check = self.checking.get(lane).map(|c| c.2);
                            let _ = tx.send(Event::BrowserControl { squad: Some(squad.clone()), lane: lane.clone(), by: Some("person".into()), why: why.clone(), check });
                        }
                    }
                    // A disk found full is told to each screen as it comes.
                    if self.disk.status.level == crate::disk::DiskLevel::Full {
                        let _ = tx.send(Event::Error { session: None, message: say!(self, "error.disk_full"), next: None });
                    }
                    self.subscribers.push((Some(squad.clone()), tx));
                    self.keep_awake();
                    let _ = reply.send((self.snapshot(&squad), rx));
                    // A screen that shows the plans' usage gets Codex's fresh, soon after.
                    self.read_codex_usage(true);
                }
                Some(Request::Tool(call, reply)) => self.tool(call, reply).await,
                Some(Request::PersonWindow(url, reply)) => {
                    let _ = reply.send(self.person_window(&url));
                }
                Some(Request::BrowserHub(reply)) => {
                    let _ = reply.send(self.hub.clone());
                }
                Some(Request::DesktopGate(check, reply)) => self.gate_desktop(*check, reply),
                Some(Request::DesktopTool(call, reply)) => {
                    let _ = reply.send(self.desktop_allowed(&call));
                }
                Some(Request::Busy(reply)) => {
                    let mut working: Vec<String> = self
                        .sessions
                        .values()
                        .filter(|s| s.is_working())
                        .map(|s| format!("{}/{}", s.squad.as_deref().unwrap_or("-"), s.name().unwrap_or("?")))
                        .collect();
                    working.sort();
                    let busy = equipe_protocol::rpc::Busy { working, approvals: self.pending.len(), desktop: self.desktop.state(), stalled: None };
                    let _ = reply.send(busy);
                }
                Some(Request::Desktop(request)) => self.on_desktop(request),
                Some(Request::Disk(reply)) => self.disk_report(Some(reply), false),
                Some(Request::DiskClean(ids, reply)) => self.disk_clean(ids, Some(reply)),
                Some(Request::Usage(engine, usage)) => {
                    self.publish(Event::PlanUsage { usage, engine });
                    for squad in self.squads_with_members() {
                        self.apply_engine_moves(&squad).await;
                    }
                }
                Some(Request::Shutdown(reply)) => {
                    self.stop_all_turns();
                    let _ = reply.send(());
                    return false;
                }
                None => {
                    self.stop_all_turns();
                    return false;
                }
            },
            Wake::Internal(message) => match message {
                Internal::Event(event) => match *event {
                    // What the engine echoes as it reads the person's steers: each said as it was.
                    Event::Steered { session, text } => self.steered(session, &text),
                    event => {
                        // A build that failed for want of room says so plainly.
                        let no_room = room::says_no_room(&event).then(|| event.session()).flatten();
                        self.publish(event);
                        if let Some(id) = no_room {
                            self.member_out_of_room(id);
                        }
                    }
                },
                Internal::Done(done) => self.on_turn_done(done).await,
                Internal::ApprovalTimedOut(id) => self.answer(id, Decision::Deny, "Nobody answered in time."),
                Internal::Ask(call, reply) => self.ask_permission(call, reply),
                Internal::CodexLimits(limits) => self.on_codex_limits(limits),
                Internal::Frame(session, frame) => {
                    self.framing.remove(&session);
                    if let Some((png, url)) = frame {
                        use base64::Engine as _;
                        let image = base64::engine::general_purpose::STANDARD.encode(png);
                        self.publish(Event::BrowserFrame { session, image, url });
                    }
                }
                Internal::Settled { merged, cache } => self.on_settled(merged, cache),
                Internal::Merged { squad, ticket, result } => self.on_merged(&squad, ticket, result),
                Internal::Disk(status, asked) => self.on_disk(status, asked),
                Internal::DiskReport(report, asked) => self.on_disk_report(*report, asked),
                Internal::DiskCleaned(cleaned, status, asked, reply) => self.on_disk_cleaned(*cleaned, status, asked, reply),
                Internal::ProjectAdded { squad, result } => self.project_added(&squad, result),
                Internal::Logins(result) => self.logins_changed(result),
                Internal::Wake { session, prompt } => self.set_going(session, prompt).await,
                Internal::DiskTrimmed(freed, lines) => self.on_disk_trimmed(freed, lines),
                Internal::Dispatch => {
                    let squads: Vec<String> = self.journals.keys().cloned().collect();
                    for squad in squads {
                        self.dispatch(&squad).await;
                    }
                }
                Internal::Engines(engines) => self.publish_engines(engines),
                Internal::ClaudeReady(ok) => self.claude_ready(ok),
                Internal::SignInEnded(engine, id) => {
                    if self.sign_ins.get(&engine).is_some_and(|(at, _)| *at == id) {
                        self.sign_ins.remove(&engine);
                    }
                    // Maybe another account now: its plan and usage read afresh.
                    if engine == Engine::Codex {
                        self.forget_codex_plan();
                        self.read_codex_usage(true);
                    }
                }
                Internal::Design(squad, stamp, catalog) => self.on_design(squad, stamp, catalog),
                Internal::Reviewed { squad, ticket, member, result } => self.on_reviewed(&squad, ticket, member, result),
            },
            Wake::Host(said) => self.on_host(said).await,
            Wake::Daily => self.prune(),
            Wake::DesktopTick => {
                if self.desktop.expire() {
                    self.publish_desktop();
                }
            }
            Wake::Frames => self.take_frames(),
            Wake::DesignTick => self.design_tick(),
            Wake::Minute => {
                self.keep_awake();
                pulse.step("minute tick: idle lanes");
                self.fold_idle_lanes();
                self.trim_builds();
                self.expire_estimates();
                pulse.step("minute tick: idle browsers");
                self.close_idle_browsers();
                pulse.step("minute tick: helpers");
                self.let_helpers_go().await;
                pulse.step("minute tick: stale tickets");
                self.tidy_check();
                pulse.step("minute tick: routines");
                self.run_due_routines().await;
                pulse.step("minute tick: engines");
                self.engines_tick().await;
                // Work that reached main outside equipe: every ten minutes is soon
                // enough (a merge or an accept looks at once), and off the loop.
                if self.settled_at.is_none_or(|at| at.elapsed() >= SETTLE_EVERY) {
                    self.settle_all_merged();
                }
                self.look_at_disk();
                self.read_codex_usage(false);
            }
        }
        true
    }
}

impl Runtime {
    // ── events: one way out ──

    fn publish(&mut self, event: Event) {
        let changed = match &event {
            Event::TicketChanged { squad, ticket } => Some((squad.clone(), ticket.id)),
            _ => None,
        };
        self.publish_in(None, event);
        // A card to confirm about it may have nothing left to confirm.
        if let Some((squad, ticket)) = changed {
            self.settle_accepts(&squad, ticket);
        }
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
            // It changed now: the screens know when it finished.
            ticket.updated_at = Some(now_secs());
        }
        // Where a member's browser goes, to bring it back there on screen.
        if let Event::ToolUse { session, tool, target: Some(url), .. } = &event
            && tool.ends_with("browser_navigate")
            && let Some(key) = self.browser_key(*session)
        {
            self.last_url.insert(key, url.clone());
        }
        let mut squad = squad.map(str::to_owned);
        let mut no_room = false;
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
                        if at < session.steer_threads.len() {
                            session.steer_threads.remove(at);
                        }
                    }
                }
                Event::SubagentStarted { id, what, .. } => session.subagents.push((id.clone(), what.clone(), None, std::time::Instant::now())),
                Event::SubagentStep { id, step, .. } => {
                    if let Some(sub) = session.subagents.iter_mut().find(|s| s.0 == *id) {
                        sub.2 = Some(step.clone());
                    }
                }
                Event::SubagentEnded { id, .. } => session.subagents.retain(|s| s.0 != *id),
                Event::TurnEnded { usage, .. } => {
                    // Whatever didn't hand back ended with the turn.
                    session.subagents.clear();
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
                    no_room = !record(journal, id, &event, &finished, title.as_deref());
                }
            }
        }
        if let Some(name) = event.squad() {
            squad = Some(name.to_owned());
        }
        if let Event::PlanUsage { usage, engine } = &event {
            if *engine == Engine::Claude {
                self.plan = Some(*usage);
            }
            let plan = equipe_protocol::EnginePlan { engine: *engine, usage: *usage, plan_type: self.plan_types.get(engine).cloned(), at: now_secs() as i64 };
            match self.plans.iter_mut().find(|p| p.engine == *engine) {
                Some(kept) => *kept = plan,
                None => self.plans.push(plan),
            }
            self.on_plan(*engine, *usage);
        }
        let everyone = event.for_everyone();
        self.subscribers.retain(|(follows, screen)| match follows {
            Some(follows) if squad.as_ref() != Some(follows) && !everyone => !screen.is_closed(),
            _ => screen.send(event.clone()).is_ok(),
        });
        if no_room {
            self.disk_is_full();
        }
    }

    // ── the Codex plan's usage ──

    /// Read the Codex plan's usage off the loop, through the person's Codex CLI:
    /// every few minutes while a squad has Codex members, and — `asked` — when
    /// a screen attaches and the last reading is that old. Never more often.
    fn read_codex_usage(&mut self, asked: bool) {
        if self.codex_reading || self.codex_read.is_some_and(|at| at.elapsed() < CODEX_USAGE_EVERY) {
            return;
        }
        let members = self.sessions.values().any(|s| s.engine == Engine::Codex && has_tools(s) && !s.model.as_deref().is_some_and(crate::nemotron::is_agent_model));
        if !(members || asked) || codex_app::find(&self.config.codex_binary).is_none() {
            return;
        }
        self.codex_reading = true;
        self.codex_read = Some(std::time::Instant::now());
        let (binary, home, internal) = (self.config.codex_binary.clone(), self.config.home.clone(), self.internal.clone());
        tokio::spawn(async move {
            let cwd = if home.is_dir() { home } else { std::env::temp_dir() };
            let read = tokio::time::timeout(SERVER_WAIT, codex_app::read_limits(&binary, &cwd)).await;
            let _ = internal.send(Internal::CodexLimits(read.ok().and_then(Result::ok)));
        });
    }

    /// The Codex plan's limits, read or reported by a turn: to the screens as
    /// its plan usage, windows known by their length.
    fn on_codex_limits(&mut self, limits: Option<equipe_providers::turn::Limits>) {
        let Some(limits) = limits else {
            // Unreadable (signed out, say): tried again after the usual wait.
            self.codex_reading = false;
            return;
        };
        self.codex_reading = false;
        self.codex_read = Some(std::time::Instant::now());
        if let Some(plan) = &limits.plan_type {
            self.plan_types.insert(Engine::Codex, plan.clone());
        }
        self.publish(Event::PlanUsage { usage: codex_plan(&limits), engine: Engine::Codex });
    }

    /// Whether members can run on Nemotron through Token Factory: a key in the
    /// daemon's environment, and the Codex CLI to run them on.
    fn nebius_ready(&self) -> bool {
        self.config.nemotron.is_some() && codex_app::find(&self.config.codex_binary).is_some()
    }

    /// Whether no engine is signed in: neither Claude Code nor Codex.
    async fn nothing_signed_in(&mut self) -> bool {
        first_ready_engine(&[Engine::Claude, Engine::Codex], &self.config.store, &self.config.claude_binary, &self.config.codex_binary).await.is_none()
    }

    /// The models the person's Codex CLI offers, its default first; kept a while.
    async fn codex_catalog(&mut self) -> Result<Vec<String>, String> {
        if let Some((at, models)) = &self.codex_models
            && at.elapsed() < CODEX_MODELS_KEPT
        {
            return Ok(models.clone());
        }
        let cwd = if self.config.home.is_dir() { self.config.home.clone() } else { std::env::temp_dir() };
        let listed = tokio::time::timeout(SERVER_WAIT, codex_app::list_models(&self.config.codex_binary, &cwd)).await;
        let models = match listed {
            Ok(Ok(models)) => models,
            Ok(Err(err)) => return Err(err.to_string()),
            Err(_) => return Err(format!("Codex didn't answer in {}s", SERVER_WAIT.as_secs())),
        };
        self.codex_models = Some((std::time::Instant::now(), models.clone()));
        Ok(models)
    }

    /// The squads screens follow now, and those with a journal open.
    fn followed_squads(&self) -> Vec<String> {
        let mut squads: Vec<String> = self.subscribers.iter().filter_map(|(s, _)| s.clone()).chain(self.journals.keys().cloned()).collect();
        squads.sort();
        squads.dedup();
        squads
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
            Command::Submit { session, text, about } => {
                // A conversation the person opened is about its own project, whatever the screen shows.
                self.said_about = self.conversation_about(session).or(about);
                let session = self.route(session);
                self.submit(session, text).await;
                self.said_about = None;
            }
            Command::SubmitLater { session, text, about } => {
                self.said_about = self.conversation_about(session).or(about);
                let session = self.route(session);
                match self.sessions.get(&session) {
                    Some(s) if s.is_working() => self.leave_memo(session, text),
                    _ => self.submit(session, text).await,
                }
                self.said_about = None;
            }
            Command::Interrupt { session } => self.interrupt(session).await,
            Command::Close { session } => self.close(session).await,
            Command::Accept { squad, ticket } => self.accept(&squad, ticket).await,
            Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),
            Command::KeepOnBranch { squad, ticket } => self.keep_on_branch(&squad, ticket),
            Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,
            Command::Resume { squad, ticket, note } => self.resume(&squad, ticket, note).await,
            Command::Cancel { squad, ticket } => self.cancel(&squad, ticket).await,
            Command::CreateTicket { squad, draft } => self.person_creates(&squad, draft).await,
            Command::EditTicket { squad, ticket, changes } => self.person_edits(&squad, ticket, changes).await,
            Command::EditTickets { squad, tickets, changes } => self.person_edits_many(&squad, tickets, changes).await,
            Command::MoveTicket { squad, ticket, status, before } => self.person_moves(&squad, ticket, status, before).await,
            Command::CommentTicket { squad, ticket, text } => self.person_comments(&squad, ticket, text).await,
            Command::TicketHistory { squad, ticket } => self.send_history(&squad, ticket),
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
            Command::BrowserNewTab { url, profile } => {
                // The person's own tab: the shared Chrome starts for it if it must
                // (another profile's, its own Chrome).
                if let Some(npx) = self.config.npx.clone()
                    && self.browser_host(&npx).is_some()
                {
                    let mut request = json!({ "op": "newtab", "url": url });
                    if let Some(p) = profile.filter(|p| browser::profiles(&self.config.home).iter().any(|k| k.name == *p)) {
                        browser::with_profile(&mut request, &self.config.home, &p);
                    }
                    self.hub.op(&request);
                }
            }
            Command::BrowserCloseTab { page } => {
                self.hub.op(&json!({ "op": "closetab", "page": page }));
                self.closed_by_person(page).await;
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
            Command::SendNow { session, texts } => self.send_now(session, texts).await,
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
            Command::AddProject { squad, path } => self.add_project(squad, path),
            Command::RemoveProject { squad, path } => self.remove_project(&squad, &path),
            Command::SetHq { squad, path } => self.set_hq(&squad, path),
            Command::SaveLogin { draft } => self.with_vault(move |vault| vault.save(draft, now_secs()).map(|_| ())),
            Command::DeleteLogin { id } => self.with_vault(move |vault| vault.delete(id)),
            Command::ForgetLoginAllowance { id, who } => self.with_vault(move |vault| vault.forget(id, &who)),
            Command::Logins => {
                let logins = self.logins.clone();
                self.publish(Event::Logins { logins });
            }
            Command::SaveBrowserProfile { profile } => {
                match browser::save_profile(&self.config.home, &profile.name, &profile.label) {
                    Ok(_) => {}
                    Err(why) => self.error(None, why, None),
                }
                self.publish_browser_profiles();
            }
            Command::DeleteBrowserProfile { name } => {
                if let Err(why) = browser::delete_profile(&self.config.home, &name) {
                    self.error(None, why, None);
                }
                // Its lanes close: members stop getting its browser from their next turn.
                let suffix = format!("#{name}");
                self.browsers.retain(|(_, key), _| !key.ends_with(&suffix));
                self.publish_browser_profiles();
            }
            Command::BrowserProfiles => self.publish_browser_profiles(),
            Command::SaveSecret { squad, id, values } => self.save_secret(&squad, id, values).await,
            Command::DismissSecret { squad, id } => self.dismiss_secret(&squad, id).await,
            Command::SaveRoutine { squad, routine } => self.save_routine(&squad, routine),
            Command::DeleteRoutine { squad, id } => self.delete_routine(&squad, id),
            Command::RunRoutine { squad, id } => {
                let now = now_secs();
                self.run_routine(&squad, id, now).await;
            }
            Command::NewConversation { squad, session, project } => self.new_conversation(&squad, session, project),
            Command::MoveConversation { session, project } => self.move_conversation(session, project),
            Command::CloseConversation { session } => self.close_conversation(session).await,
            Command::Focus { squad, project } => match project {
                Some(name) => {
                    self.focus.insert(squad, name);
                }
                None => {
                    self.focus.remove(&squad);
                }
            },
            Command::Decide { squad, approval, decision, words } => {
                let words = words.map(|w| w.trim().to_owned()).filter(|w| !w.is_empty());
                if let Some(pending) = self.pending.get(&approval).filter(|p| p.squad == squad) {
                    let (session, tool) = (pending.approval.session, pending.approval.tool.clone());
                    self.answer_with(approval, decision, "", words.as_deref());
                    // A permission's yes (or a sign-in's answer) can't carry words:
                    // they reach the member as the person's own, mid-work.
                    let carried = tool == CONFIRM || (decision == Decision::Deny && tool != SIGN_IN_TOOL);
                    if let Some(words) = words.filter(|_| !carried) {
                        self.submit(session, words).await;
                    }
                }
            }
            Command::AnswerQuestions { squad, id, answers, about } => {
                self.said_about = about;
                self.answer_questions(&squad, id, answers).await;
                self.said_about = None;
            }
            Command::DismissQuestions { squad, id } => self.dismiss_questions(&squad, id).await,
            Command::ApproveTeamChange { squad, id } => self.approve_team_change(&squad, id).await,
            Command::DeclineTeamChange { squad, id } => self.decline_team_change(&squad, id),
            Command::ProposeAccept { squad, session, verdict, tickets, note } => self.propose_accept_from_words(&squad, session, verdict, &tickets, &note),
            Command::ConfirmAccept { squad, id, tickets } => self.confirm_accept(&squad, id, tickets).await,
            Command::DeclineAccept { squad, id } => self.decline_accept(&squad, id),
            Command::Disallow { squad, rule } => {
                let taken = self.journals.get(&squad).and_then(|j| j.disallow(&rule).ok()).unwrap_or(false);
                if taken {
                    self.publish_allowed(&squad);
                }
            }
            // Phones are paired by the gateway, per screen, with the daemon's
            // phone link (`crate::pairing`); they never reach the runtime.
            Command::PairStart | Command::PairAnswer { .. } | Command::PairCancel | Command::Devices | Command::Unpair { .. } => {}
            Command::DiskScan => self.disk_report(None, true),
            Command::DiskClean { ids } => self.disk_clean(ids, None),
            Command::Engines => self.look_at_engines(),
            Command::SignIn { engine, device } => self.sign_in(engine, device),
            Command::SignInCancel { engine } => {
                if let Some((_, task)) = self.sign_ins.remove(&engine) {
                    task.abort();
                    self.publish(Event::SignIn { engine, step: equipe_protocol::SignInStep::Cancelled });
                    self.look_at_engines();
                }
            }
            Command::SignOut { engine } => self.sign_out(engine),
            Command::SetEngineSwitching { squad, mode } => self.set_engine_switching(&squad, mode).await,
            Command::SetPmAccepts { squad, on } => {
                if let Some(journal) = self.journal(&squad) {
                    let _ = journal.set_pm_accepts(on);
                }
                self.publish(Event::PmAcceptsChanged { squad, on });
            }
            Command::ApplyEngineProposal { squad, id } => self.apply_engine_proposal(&squad, id).await,
            Command::DismissEngineProposal { squad, id } => self.dismiss_engine_proposal(&squad, id),
            Command::UndoEngineProposal { squad, id } => self.undo_engine_proposal(&squad, id).await,
            Command::ShowEngineSetup { squad } => self.show_engine_setup(&squad),
            Command::MoveEngine { squad, members, engine } => self.move_engine(&squad, members, engine).await,
            Command::Design { squad } => self.design_asked(squad),
        }
    }

    // ── the engines ──

    /// Where each engine stands, looked at off the loop (each engine's own
    /// tool answers), then told to every screen.
    fn look_at_engines(&mut self) {
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            let engines = crate::engines::statuses(&claude, &codex, &store).await;
            let _ = internal.send(Internal::Engines(engines));
        });
    }

    /// The engines as just looked at, with who is on each and the sign-ins
    /// under way, for every screen.
    fn publish_engines(&mut self, mut engines: Vec<equipe_protocol::EngineStatus>) {
        for status in &mut engines {
            let mut members: Vec<MemberRef> = self.sessions.values().filter(|s| s.engine == status.engine).filter_map(session_member).collect();
            members.sort_by(|a, b| (&a.squad, &a.member).cmp(&(&b.squad, &b.member)));
            members.dedup();
            status.members = members;
            status.signing_in = self.sign_ins.contains_key(&status.engine);
        }
        self.publish(Event::Engines { engines });
    }

    /// Sign Codex in with its own tool, here on the daemon's machine; every
    /// screen hears each step (the code to enter, when `device`), then where
    /// the engines stand. A new sign-in stops the one under way.
    fn sign_in(&mut self, engine: Engine, device: bool) {
        use equipe_protocol::{SignInFailure, SignInStep};
        if engine != Engine::Codex {
            self.publish(Event::SignIn { engine, step: SignInStep::Failed { why: SignInFailure::Unsupported, detail: String::new() } });
            return;
        }
        if let Some((_, old)) = self.sign_ins.remove(&engine) {
            old.abort();
        }
        let id = self.next_sign_in;
        self.next_sign_in += 1;
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        let task = tokio::spawn(async move {
            let tell = |step| drop(internal.send(Internal::Event(Box::new(Event::SignIn { engine, step }))));
            let end = crate::engines::codex_login(&codex, device, tell).await;
            tell(end);
            let _ = internal.send(Internal::SignInEnded(engine, id));
            let _ = internal.send(Internal::Engines(crate::engines::statuses(&claude, &codex, &store).await));
        });
        self.sign_ins.insert(engine, (id, task));
    }

    /// What was read of the Codex account's plan, forgotten: signed out, or
    /// signed in again (perhaps as someone else).
    fn forget_codex_plan(&mut self) {
        self.codex_read = None;
        self.plan_types.remove(&Engine::Codex);
        self.plans.retain(|p| p.engine != Engine::Codex);
        // Another account may offer other models (a paid plan has more).
        self.codex_models = None;
    }

    /// Sign `engine` out with its own tool. Claude Code's login is the
    /// person's: equipe never touches it.
    fn sign_out(&mut self, engine: Engine) {
        if engine != Engine::Codex {
            return;
        }
        self.forget_codex_plan();
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            if let Err(why) = crate::engines::codex_logout(&codex).await {
                crate::daemon_log!("equipe: `codex logout` failed: {why}");
            }
            let _ = internal.send(Internal::Engines(crate::engines::statuses(&claude, &codex, &store).await));
        });
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
        let steer_threads: Vec<String> = session.steer_threads.iter().map(|p| p.clone().unwrap_or_default()).collect();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.set_waiting(id, "steer", &steering);
            let _ = journal.set_waiting(id, "steer-threads", &steer_threads);
            let _ = journal.set_waiting(id, "memo", &queued);
            let _ = journal.set_waiting(id, "memo-threads", &threads);
            let _ = journal.set_waiting(id, "briefing", &briefing);
        }
    }

    /// The person adds a folder as a project: checked, and started as a
    /// repository if it is none yet, off the loop (git).
    fn add_project(&mut self, squad: String, path: PathBuf) {
        let (home, internal) = (self.config.home.clone(), self.internal.clone());
        let user_home = std::env::var_os("HOME").map(PathBuf::from);
        tokio::task::spawn_blocking(move || {
            let result = crate::squads::add_folder(&home, &squad, &path.to_string_lossy(), user_home.as_deref());
            let _ = internal.send(Internal::ProjectAdded { squad, result });
        });
    }

    /// A project taken off the squad: its folder, tickets and branches stay;
    /// every screen has the list again, and the PM knows.
    fn remove_project(&mut self, squad: &str, path: &Path) {
        let removed = match self.journal(squad) {
            Some(journal) if journal.projects().unwrap_or_default().iter().any(|p| p == path) => journal.remove_project(path).map_err(|e| e.to_string()),
            Some(_) => Err(path.display().to_string()),
            None => return,
        };
        match removed {
            Ok(()) => {
                let name = crate::board::project_name(path);
                crate::daemon_log!("equipe: {squad} no longer works on {name} ({})", path.display());
                self.publish_projects(squad);
                self.brief_pm(squad, format!("The person took {name} ({}) off the squad's projects; its folder, tickets and branches stay.", path.display()), None);
            }
            Err(why) => {
                let message = say!(self, "error.project_not_removed", why = why);
                self.squad_error(squad, message);
            }
        }
    }

    /// A project added: every screen of the squad has the list again, and the
    /// PM knows where it is.
    fn project_added(&mut self, squad: &str, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                let name = crate::board::project_name(&path);
                crate::daemon_log!("equipe: {squad} has a new project, {name} ({})", path.display());
                self.publish_projects(squad);
                self.brief_pm(squad, format!("The person added a project to the squad: {name}, at {}.", path.display()), None);
            }
            Err(why) => {
                let message = say!(self, "error.project_not_added", why = why);
                self.squad_error(squad, message);
            }
        }
    }

    /// Awake while wanted: a member at work, or a screen attached (the first
    /// subscriber is the daemon's own handle).
    fn keep_awake(&mut self) {
        let screens = self.subscribers.iter().skip(1).any(|(follows, screen)| follows.is_some() && !screen.is_closed());
        let working = self.sessions.values().any(Session::is_working);
        let want = screens || working;
        if self.awake.want(want) {
            crate::daemon_log!("equipe: {}", if want { "keeping this Mac awake: the squad works or a screen is attached" } else { "this Mac may sleep again" });
        }
    }

    fn journal(&mut self, squad: &str) -> Option<&Journal> {
        journal_for(&mut self.journals, &self.config.home, squad)
    }

    /// Every screen of the squad has its projects again, and which is its HQ.
    fn publish_projects(&mut self, squad: &str) {
        let projects = self.projects(squad);
        let hq = self.hq(squad);
        self.publish(Event::ProjectsChanged { squad: squad.to_owned(), projects, hq });
    }

    fn hq(&mut self, squad: &str) -> Option<PathBuf> {
        self.journal(squad).and_then(|j| j.hq())
    }

    /// The person makes a project the squad's HQ (or none): the PM is told
    /// what goes there from now on.
    fn set_hq(&mut self, squad: &str, path: Option<PathBuf>) {
        if path.as_ref().is_some_and(|p| !self.projects(squad).contains(p)) {
            return;
        }
        let before = self.hq(squad);
        if before == path {
            return;
        }
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_hq(path.as_deref())) {
            crate::daemon_log!("equipe: {squad}: the HQ wasn't kept: {err}");
            return;
        }
        self.publish_projects(squad);
        let line = match &path {
            Some(hq) => format!(
                "The person made {} the squad's HQ: from now on the plan across the projects, the roadmap, decisions, \
runbooks and reports live there, and work across the projects or on running them is filed in it.",
                board::project_name(hq)
            ),
            None => "The person took the HQ mark off: no project is the squad's HQ now.".to_owned(),
        };
        self.brief_pm(squad, line, None);
    }

    // ── routines ──

    fn routines(&mut self, squad: &str) -> Vec<Routine> {
        self.journal(squad).map(Journal::routines).unwrap_or_default()
    }

    /// Kept, and every screen of the squad has them again.
    fn keep_routines(&mut self, squad: &str, routines: Vec<Routine>) {
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_routines(&routines)) {
            crate::daemon_log!("equipe: {squad}: the routines weren't kept: {err}");
        }
        self.publish(Event::RoutinesChanged { squad: squad.to_owned(), routines });
    }

    /// The person makes or changes a routine: numbered if new, its next run worked out.
    fn save_routine(&mut self, squad: &str, mut routine: Routine) {
        routine.title = routine.title.trim().to_owned();
        routine.prompt = routine.prompt.trim().to_owned();
        if routine.title.is_empty() || routine.prompt.is_empty() || !routine.when.is_valid() {
            let message = say!(self, "error.routine_invalid");
            return self.squad_error(squad, message);
        }
        let mut all = self.routines(squad);
        routine.next_run = Some(routines::next_run(routine.when, now_secs(), &routines::Local));
        let full = all.len() >= MAX_ROUTINES;
        match all.iter_mut().find(|r| r.id == routine.id && routine.id != 0) {
            Some(old) => {
                routine.last_run = old.last_run;
                *old = routine;
            }
            None if full => {
                let message = say!(self, "error.routines_full", max = MAX_ROUTINES);
                return self.squad_error(squad, message);
            }
            None => {
                routine.id = all.iter().map(|r| r.id).max().unwrap_or(0) + 1;
                routine.last_run = None;
                all.push(routine);
            }
        }
        self.keep_routines(squad, all);
    }

    fn delete_routine(&mut self, squad: &str, id: u32) {
        let mut all = self.routines(squad);
        let before = all.len();
        all.retain(|r| r.id != id);
        if all.len() != before {
            self.keep_routines(squad, all);
        }
    }

    /// A routine handed to the PM as the person's message — after its turn
    /// if one runs — about the HQ when the squad has one. `false`: no PM to hand it to.
    async fn run_routine(&mut self, squad: &str, id: u32, now: u64) -> bool {
        let mut all = self.routines(squad);
        let Some(routine) = all.iter().find(|r| r.id == id).cloned() else {
            return false;
        };
        let Some(pm) = self.pm_of(squad) else {
            return false;
        };
        crate::daemon_log!("equipe: {squad}: routine {} ({}) handed to the PM", routine.id, routine.title);
        let about = self.hq(squad).map(|hq| board::project_name(&hq));
        let text = routines::message(&routine);
        self.said_about = Some(about);
        match self.sessions.get(&pm) {
            Some(s) if s.is_working() => self.leave_memo(pm, text),
            _ => self.submit(pm, text).await,
        }
        self.said_about = None;
        if let Some(r) = all.iter_mut().find(|r| r.id == id) {
            r.last_run = Some(now);
            r.next_run = Some(routines::next_run(r.when, now, &routines::Local));
        }
        self.keep_routines(squad, all);
        true
    }

    /// Each minute: the routines due, run; a run missed for long, let go.
    async fn run_due_routines(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let all = self.routines(&squad);
            for r in &all {
                match routines::due(r, now) {
                    routines::Due::Run => {
                        self.run_routine(&squad, r.id, now).await;
                    }
                    routines::Due::Skip => {
                        let mut kept = self.routines(&squad);
                        if let Some(k) = kept.iter_mut().find(|k| k.id == r.id) {
                            k.next_run = Some(routines::next_run(k.when, now, &routines::Local));
                        }
                        self.keep_routines(&squad, kept);
                    }
                    routines::Due::Wait => {}
                }
            }
        }
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
        // Not into a conversation the person opened: those are theirs.
        let at = self.sessions.iter().find(|(_, s)| !s.kept && s.lane.as_ref().is_some_and(|(of, _)| *of == pm)).map_or(pm, |(&lane, _)| lane);
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

        // NVIDIA's models on Nebius Token Factory run on Codex's agent (`nemotron`).
        let mut on_nebius = spec.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
        let engine = match spec.engine {
            // Named, but nothing at all is signed in: Nemotron, rather than failing.
            Some(_) if !on_nebius && self.nebius_ready() && self.nothing_signed_in().await => {
                on_nebius = true;
                Engine::Codex
            }
            Some(engine) => engine,
            None if on_nebius => Engine::Codex,
            None => {
                let preference = match (role, edits) {
                    (Some(_), Some(true)) => [Engine::Claude, Engine::Codex],
                    (Some(role), _) => roles::engine_preference(role),
                    // Plain sessions as roles go: writing on Claude, reading on Codex.
                    (None, _) if spec.access == Access::EditFiles => [Engine::Claude, Engine::Codex],
                    (None, _) => [Engine::Codex, Engine::Claude],
                };
                match first_ready_engine(&preference, &self.config.store, &self.config.claude_binary, &self.config.codex_binary).await {
                    Some(engine) => engine,
                    // Nothing signed in, but a Token Factory key: Nemotron, on Codex's agent.
                    None if self.nebius_ready() => {
                        on_nebius = true;
                        Engine::Codex
                    }
                    None => {
                        let message = say!(self, "error.no_engine");
                        return fail(self, message, Some("equipe login"));
                    }
                }
            }
        };
        // The person's own Codex CLI runs an agent with tools; without it, Codex
        // is equipe's own sign-in, which only answers.
        let codex_cli = engine == Engine::Codex && codex_app::find(&self.config.codex_binary).is_some();
        on_nebius &= engine == Engine::Codex;
        if on_nebius && !codex_cli {
            return fail(self, say!(self, "error.nebius_needs_codex"), Some("npm install -g @openai/codex"));
        }
        if spec.access == Access::EditFiles && engine == Engine::Codex && !codex_cli {
            return fail(self, say!(self, "error.codex_edits"), Some("codex login"));
        }

        let mut model = spec.model.clone();
        let mut codex_cred = None;
        match engine {
            Engine::Codex if codex_cli => {
                if !on_nebius {
                    match codex_app::login_status(&self.config.codex_binary).await {
                        Ok(Some(sign_in)) if sign_in.signed_in() => {}
                        // Signed out, nothing else signed in, and a Token Factory key: Nemotron instead of failing.
                        Ok(_) if self.config.nemotron.is_some() && self.nothing_signed_in().await => on_nebius = true,
                        Ok(_) => return fail(self, say!(self, "error.codex_cli_signed_out"), Some("equipe login codex")),
                        Err(err) => return fail(self, err.to_string(), Some("equipe status")),
                    }
                }
                if on_nebius {
                    let Some(cfg) = self.config.nemotron.clone() else {
                        return fail(self, say!(self, "error.nebius_no_key"), Some("export NEBIUS_API_KEY=…; equipe daemon stop; equipe daemon run"));
                    };
                    if !model.as_deref().is_some_and(crate::nemotron::is_agent_model) {
                        let picked = match tokio::time::timeout(SERVER_WAIT, crate::nemotron::agent_model(&cfg)).await {
                            Ok(picked) => picked,
                            Err(_) => Err(crate::nemotron::ReviewError::Timeout),
                        };
                        match picked {
                            Ok(picked) => model = Some(picked),
                            Err(why) => return fail(self, say!(self, "error.nebius_no_model", why = why.why(self.config.lang)), Some("EQUIPE_NEBIUS_MODEL")),
                        }
                    }
                } else if model.is_none() {
                    // What this account offers, as its own Codex lists it — no id hardcoded.
                    let catalog = match self.codex_catalog().await {
                        Ok(catalog) => catalog,
                        Err(err) => return fail(self, err, Some("equipe status")),
                    };
                    model = match role {
                        Some(role) => roles::recommended_model_for(role, engine, &catalog),
                        None => catalog.into_iter().next(),
                    };
                    if model.is_none() {
                        return fail(self, say!(self, "error.no_models"), Some("equipe status"));
                    }
                }
            }
            Engine::Codex => {
                let cred = match codex_credentials(&self.client, &self.config.store, self.config.lang).await {
                    Ok(cred) => cred,
                    Err((message, next)) => return fail(self, message, Some(next)),
                };
                if model.is_none() {
                    // Ask the backend what this account has rather than hardcoding an id.
                    let asked = codex::list_models(&self.client, &cred, &self.config.client_version);
                    let catalog = match tokio::time::timeout(SERVER_WAIT, asked).await {
                        Ok(Ok(models)) => models,
                        Ok(Err(err)) => return fail(self, err.to_string(), Some("equipe status")),
                        Err(_) => return fail(self, format!("Codex didn't answer in {}s", SERVER_WAIT.as_secs()), Some("equipe status")),
                    };
                    model = match role {
                        Some(role) => roles::recommended_model_for(role, engine, &catalog),
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
                    Ok(Some(status)) if status.logged_in => {
                        if let Some(plan) = status.subscription_type {
                            self.plan_types.insert(Engine::Claude, plan);
                        }
                    }
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
                    model = roles::recommended_model_for(role, engine, &[]);
                }
            }
        }

        let opened_in = project_of(&spec.cwd);
        let member = match (&squad, spec.role.as_deref()) {
            (Some(squad), Some(role)) => {
                let role = if role == "po" { roles::PM } else { role };
                // A member moving engines keeps its name.
                let name = self.engines.name_for(id).filter(|n| self.member_id(squad, n).is_none()).unwrap_or_else(|| self.free_name(squad, role));
                Some(Member { role: role.to_owned(), name })
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
                steer_threads: Vec::new(),
                stop: None,
                taken_back: Vec::new(),
                send_now_prompt: None,
                subagents: Vec::new(),
                answers_person: false,
                effort: None,
                briefing: Vec::new(),
                memory,
                mode: None,
                thread: None,
                threads: HashMap::new(),
                lane: None,
                kept: false,
                estimate: None,
                turn_began: None,
            },
        );
        self.publish(Event::SessionStarted { session: id, project, engine, model: model.clone(), workspace, member, squad: squad.clone() });
        if on_nebius {
            let text = say!(self, "notice.on_nebius", model = model.unwrap_or_default());
            self.publish(Event::Notice { session: id, text });
        }
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
        // The Codex CLI signs itself in; only the fallback needs equipe's sign-in.
        if session.engine != Engine::Codex || session.codex.is_some() || codex_app::find(&self.config.codex_binary).is_some() {
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
            if let Some(steer) = session.steer.clone() {
                // Each steer says which project it is about, as a message does:
                // several read together at one step may be about different ones.
                let project = self.looking_at(id);
                let said = self.steer_words(id, &text, project.as_deref());
                if steer.send(said).is_ok() {
                    if let Some(session) = self.sessions.get_mut(&id) {
                        session.steering.push(text.clone());
                        session.steer_threads.push(project.clone());
                    }
                    self.save_waiting(id);
                    self.tell_the_pm(id, |name| format!("The person told {name}, mid-work: {text}"), Some(&text));
                    return self.publish(Event::SteerQueued { session: id, text, project });
                }
            }
            return self.leave_memo(id, text);
        }
        self.submit_now(id, text).await;
    }

    /// A memo: it waits for the turn to end rather than cutting in, and belongs
    /// with the project the person is looking at now.
    fn leave_memo(&mut self, id: SessionId, text: String) {
        let project = self.looking_at(id);
        self.leave_memo_about(id, text, project);
    }

    /// A memo about `project` (`None`: the whole squad).
    fn leave_memo_about(&mut self, id: SessionId, text: String, project: Option<String>) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.queued.push(text.clone());
        session.memo_threads.push(project.clone());
        self.save_waiting(id);
        self.publish(Event::MessageQueued { session: id, text, project });
    }

    /// The project what is said to `id` now is about: its ticket's, at work
    /// on one; else the one the person looks at (`None`: the whole squad).
    fn looking_at(&self, id: SessionId) -> Option<String> {
        let session = self.sessions.get(&id)?;
        match &session.work {
            Some(work) => Some(crate::board::project_name(&work.project)),
            None => self.said_about.clone().unwrap_or_else(|| session.squad.as_ref().and_then(|s| self.focus.get(s)).cloned()),
        }
    }

    /// A steer as it goes into the turn: with the project it is about first,
    /// as a message is sent — at the PM's desk, of a squad with several
    /// projects, the whole squad's said too.
    fn steer_words(&mut self, id: SessionId, text: &str, project: Option<&str>) -> String {
        let desk = self.sessions.get(&id).filter(|s| s.is_pm() && s.work.is_none()).and_then(|s| s.squad.clone());
        let whole = project.is_none() && desk.is_some_and(|squad| self.projects(&squad).len() > 1);
        steers::words(text, project, whole)
    }

    /// The engine read what the person said while it works: `raw` as it
    /// echoes it — one steer, or several read at once — each shown as it was
    /// said, in the conversation of the project it was about. A steer taken
    /// back, and the word that it was, are not shown.
    fn steered(&mut self, id: SessionId, raw: &str) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // Each with the project it was said about; `None`: not one equipe knows of.
        let mut read: Vec<(String, Option<Option<String>>)> = Vec::new();
        for part in steers::read(raw, &session.steering) {
            if part.starts_with(steers::TAKEN_BACK) {
                continue;
            }
            if let Some(at) = session.taken_back.iter().position(|t| *t == part) {
                session.taken_back.remove(at);
                continue;
            }
            let about = session.steering.iter().position(|t| *t == part).map(|at| session.steer_threads.get(at).cloned().flatten());
            read.push((part, about));
        }
        // At the PM's desk the line goes where it was said about, and the
        // turn goes on about its own after it.
        let marks = session.is_pm() && session.work.is_none();
        for (text, about) in read {
            let now = self.sessions.get(&id).and_then(|s| scope::current(s.transcript.entries())).flatten();
            let about = about.unwrap_or_else(|| now.clone());
            let elsewhere = marks && about != now;
            if elsewhere {
                self.publish(Event::Thread { session: id, project: about });
            }
            self.publish(Event::Steered { session: id, text });
            if elsewhere {
                self.publish(Event::Thread { session: id, project: now });
            }
        }
        self.save_waiting(id);
    }

    /// The session holding what waits with `held` for `id`: its own, or its lane's.
    fn holder(&self, id: SessionId, held: impl Fn(&Session) -> bool) -> Option<SessionId> {
        match self.sessions.get(&id) {
            Some(s) if held(s) => Some(id),
            _ => self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && held(s)).map(|(lane, _)| *lane),
        }
    }

    /// Send what waits for `id` now ([`Command::SendNow`]): its turn stops
    /// and the steers not read and the memos — `first` in its order, then
    /// the rest — go to it together, as one message about their project
    /// (each part with its own, when they are about several).
    async fn send_now(&mut self, id: SessionId, first: Vec<String>) {
        let waits = |s: &Session| !s.steering.is_empty() || !s.queued.is_empty();
        let Some(id) = self.holder(id, waits) else {
            return;
        };
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // Everything waiting, steers first as they were said, then memos.
        let mut waiting: Vec<(String, Option<String>)> = Vec::new();
        let steer_threads = std::mem::take(&mut session.steer_threads);
        for (i, text) in std::mem::take(&mut session.steering).into_iter().enumerate() {
            waiting.push((text, steer_threads.get(i).cloned().flatten()));
        }
        let memo_threads = std::mem::take(&mut session.memo_threads);
        for (i, text) in std::mem::take(&mut session.queued).into_iter().enumerate() {
            waiting.push((text, memo_threads.get(i).cloned().flatten()));
        }
        let items = steers::order(waiting, &first);
        session.steer = None;
        let texts: Vec<String> = items.iter().map(|(t, _)| t.clone()).collect();
        if texts.is_empty() {
            return;
        }
        let about: Vec<Option<String>> = items.iter().map(|(_, a)| a.clone()).collect();
        let one = about.iter().all(|a| *a == about[0]);
        let text = texts.join("\n\n");
        // About several projects: each part says its own to the engine.
        let prompt = (!one).then(|| items.iter().map(|(t, a)| steers::part(t, a.as_deref())).collect::<Vec<_>>().join("\n\n"));
        self.save_waiting(id);
        self.publish(Event::QueueSent { session: id, texts: texts.clone() });
        let project = if one { about[0].clone() } else { None };
        let stopped = self.stop_turn(id);
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.queued.insert(0, text);
        session.memo_threads.insert(0, project);
        session.send_now_prompt = prompt;
        // A turn that finished already ends as it lands, delivering this first.
        let landing = !stopped && session.turn.is_some();
        if !landing {
            // The turn's end, as Esc's: its work handed in, what waits delivered —
            // this message first.
            self.after_turn(id).await;
        }
        // What went into the stopped turn unread is in the message now: an
        // echo of it still on its way is not shown twice.
        if let Some(session) = self.sessions.get_mut(&id) {
            session.taken_back.extend(texts);
        }
    }

    /// Take back a memo not delivered yet: the first one with `text`, in the
    /// session it was left in (a lane's, when the member had one).
    fn unqueue(&mut self, id: SessionId, text: &str) {
        let held = |s: &Session| s.queued.iter().any(|q| q == text) || s.steering.iter().any(|q| q == text);
        let Some(id) = self.holder(id, held) else {
            return;
        };
        let Some(session) = self.sessions.get_mut(&id) else { return };
        if let Some(at) = session.queued.iter().position(|q| q == text) {
            session.queued.remove(at);
            if at < session.memo_threads.len() {
                session.memo_threads.remove(at);
            }
        } else if let Some(at) = session.steering.iter().position(|q| q == text) {
            // A steer is in the turn already, unread: the engine is told to
            // disregard it, and its echo isn't shown.
            session.steering.remove(at);
            if at < session.steer_threads.len() {
                session.steer_threads.remove(at);
            }
            session.taken_back.push(text.to_owned());
            if let Some(steer) = session.steer.clone() {
                let _ = steer.send(steers::taken_back(text));
            }
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
        // The project the message is about: the one the person looks at on
        // the screen they sent it from (else the one in focus) — a memo's,
        // the one it was left in. None: the whole squad.
        let about = match memo_thread {
            Some(left_in) => left_in,
            None => self.said_about.clone().unwrap_or_else(|| {
                let squad = self.sessions.get(&id).and_then(|s| s.squad.clone());
                squad.and_then(|squad| self.focus.get(&squad).cloned())
            }),
        };
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
        // A message sent at once about several projects: each part says its own.
        let text = session.send_now_prompt.take().unwrap_or(text);
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
        // Said to the PM itself: its own conversation, not one the person opened beside it.
        if let Some((&lane, _)) = self.sessions.iter().find(|(_, s)| !s.kept && s.lane.as_ref().is_some_and(|(of, _)| *of == id)) {
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
        let lane = lane_of(main, m, project.clone(), thread, false);
        self.publish_lane(id, lane, main, project, false);
        Some(id)
    }

    /// The new lane `id` of `main`, told to the screens.
    fn publish_lane(&mut self, id: SessionId, lane: Session, main: SessionId, project: Option<String>, kept: bool) {
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
        self.publish(Event::LaneOpened { session: id, of: main, project, kept });
    }

    /// The person opens a conversation of their own with the squad's PM,
    /// about `project` (`None`: the whole squad): a fresh one, kept.
    fn new_conversation(&mut self, squad: &str, id: SessionId, project: Option<String>) {
        if self.sessions.contains_key(&id) {
            return self.error(None, say!(self, "error.already_open", id = id), None);
        }
        let Some(pm) = self.pm_of(squad) else {
            return self.squad_error(squad, say!(self, "error.no_member", name = roles::PM));
        };
        let Some(m) = self.sessions.get(&pm) else { return };
        let lane = lane_of(pm, m, project.clone(), DeskThread::default(), true);
        if self.config.persist
            && let Some(journal) = journal_for(&mut self.journals, &self.config.home, squad)
        {
            let _ = journal.record_conversation(id, project.as_deref(), &lane.cwd);
        }
        self.publish_lane(id, lane, pm, project, true);
    }

    /// A conversation with the PM moves to another project.
    fn move_conversation(&mut self, id: SessionId, project: Option<String>) {
        let Some(s) = self.sessions.get_mut(&id).filter(|s| s.kept) else { return };
        if let Some((_, about)) = s.lane.as_mut() {
            *about = project.clone();
        }
        s.thread = project.clone();
        let squad = s.squad.clone();
        if self.config.persist
            && let Some(journal) = squad.as_deref().and_then(|q| journal_for(&mut self.journals, &self.config.home, q))
        {
            let _ = journal.set_conversation_project(id, project.as_deref());
        }
        self.publish(Event::LaneMoved { session: id, project });
    }

    /// A conversation with the PM put away: its turn stops, it leaves the list.
    async fn close_conversation(&mut self, id: SessionId) {
        let Some(squad) = self.sessions.get(&id).filter(|s| s.kept).map(|s| s.squad.clone()) else { return };
        if self.config.persist
            && let Some(journal) = squad.as_deref().and_then(|q| journal_for(&mut self.journals, &self.config.home, q))
        {
            let _ = journal.close_conversation(id);
        }
        self.close(id).await;
    }

    /// The project a conversation the person opened is about, if `id` is one.
    fn conversation_about(&self, id: SessionId) -> Option<Option<String>> {
        self.sessions.get(&id).filter(|s| s.kept).and_then(|s| s.lane.as_ref()).map(|(_, project)| project.clone())
    }

    /// Lanes left with nothing to do for a while go back into their members.
    fn fold_idle_lanes(&mut self) {
        let idle: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.lane.is_some() && !s.kept && !s.is_working() && s.queued.is_empty())
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
        for s in self.sessions.values().filter(|s| !s.kept && s.lane.as_ref().is_some_and(|(of, _)| *of == main)) {
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
        // A member that just moved engines: equipe's brief of where it was.
        let prompt = match self.engines.brief_for(id) {
            Some(brief) => format!("{brief}\n\n{prompt}"),
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
        let hq = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(Journal::hq).map(|p| board::project_name(&p));
        let equipe_path = self.config.equipe_binary.as_ref().map(|p| p.to_string_lossy().into_owned());
        let board_tools = session.member.is_some() && has_tools(session) && self.config.equipe_binary.is_some();
        let language = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(Journal::language);
        let edits = session.edits();
        let mode = session.mode.or_else(|| squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::mode)).unwrap_or_default();
        // A browser in any mode that may change things — signed in wherever the
        // person (or a teammate) signed in, if anywhere yet. In Auto and Full
        // it browses freely; in Ask each step comes to the person.
        let wants_browser = board_tools && mode != Mode::ReadOnly;
        // The Mac's desktop: experimental, for the members the person turned it on for.
        let desktop = wants_browser
            && cfg!(target_os = "macos")
            && squad.as_deref().zip(session.member.as_ref()).is_some_and(|(s, m)| self.journals.get(s).is_some_and(|j| j.desktop_enabled(&m.name)));
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
        // Other people's accounts: a browser in each other profile's Chrome, beside its own.
        let profile_browsers: Vec<(browser::BrowserProfile, u16)> = match (browser, self.browser_key(id)) {
            (Some(_), Some((squad, key))) if self.config.browser_host => browser::profiles(&self.config.home)
                .into_iter()
                .filter_map(|p| {
                    let port = self.open_browser(&squad, &browser::in_profile(&key, Some(&p.name)), false)?;
                    Some((p, port))
                })
                .collect(),
            _ => Vec::new(),
        };
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let browses_freely = browser.is_some() && matches!(mode, Mode::Auto | Mode::Full);
        // Code run in the browser's own process is outside any sandbox: in auto
        // the person sees the code and says yes; in full it runs.
        let ask_tools: Vec<String> =
            if browser.is_some() && mode == Mode::Auto {
                std::iter::once(permissions::BROWSER_CODE.to_owned())
                    .chain(profile_browsers.iter().map(|(p, _)| format!("mcp__{}__browser_run_code_unsafe", browser::server_name(Some(&p.name)))))
                    .collect()
            } else {
                Vec::new()
            };
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
                for (profile, port) in &profile_browsers {
                    servers[browser::server_name(Some(&profile.name))] = browser::entry(*port);
                }
                if desktop {
                    servers[DESKTOP_SERVER] = json!({
                        "type": "stdio",
                        "command": binary,
                        "args": ["mcp", "--server", DESKTOP_SERVER, "--squad", squad, "--member", m.name, "--role", m.role, "--home", self.config.home],
                    });
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
                hq: hq.as_deref(),
                equipe: equipe_path.as_deref(),
                board_tools,
                language: language.as_deref(),
                screen_language: Some(self.config.lang.english_name()),
                browser: browser.is_some(),
                profiles: &profile_browsers.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
                pm_accepts: squad.as_deref().and_then(|s| self.journals.get(s)).is_none_or(Journal::pm_accepts),
                desktop,
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
        let mut env: Vec<(String, String)> = builds.iter().map(|d| ("CARGO_TARGET_DIR".to_owned(), d.to_string_lossy().into_owned())).collect();
        // Leaner Rust builds unless the squad says otherwise: debug info is most of a target's size.
        if builds.is_some()
            && let Some(debug) = squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::build_debug).and_then(|d| d.cargo_value())
        {
            env.push(("CARGO_PROFILE_DEV_DEBUG".to_owned(), debug.to_owned()));
        }
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
        let mut stop = None;
        let turn = match session.engine {
            // The person's own Codex CLI: an agent with tools, in the worktree.
            Engine::Codex if session.codex.is_none() => {
                let binary = self.config.codex_binary.clone();
                let lang = self.config.lang;
                let model = session.model.clone();
                let mut effort = session.effort.clone();
                // NVIDIA's models: Codex's agent with Token Factory as its provider.
                let nebius = model.as_deref().is_some_and(crate::nemotron::is_agent_model);
                let provider = match (&self.config.nemotron, nebius) {
                    (Some(cfg), true) => {
                        env.push(cfg.codex_env());
                        effort = crate::nemotron::agent_effort(effort.as_deref()).map(str::to_owned);
                        Some(cfg.codex_provider())
                    }
                    (None, true) => return self.error(Some(id), say!(self, "error.nebius_no_key"), Some("export NEBIUS_API_KEY=…; equipe daemon stop; equipe daemon run")),
                    (_, false) => None,
                };
                // equipe's servers only, as Claude Code gets them: the board's tools
                // never ask; the browser's and the desktop's ask unless the mode lets them.
                let servers = match (&squad, &session.member, &self.config.equipe_binary) {
                    (Some(squad), Some(m), Some(binary)) if board_tools => {
                        let args = |server: Option<&str>| {
                            let mut args: Vec<String> = vec!["mcp".into()];
                            if let Some(server) = server {
                                args.extend(["--server".into(), server.into()]);
                            }
                            args.extend(["--squad", squad, "--member", &m.name, "--role", &m.role, "--home"].map(str::to_owned));
                            args.push(self.config.home.to_string_lossy().into_owned());
                            args
                        };
                        let mut servers = json!({ MCP_SERVER: codex_app::mcp_server(binary, &args(None), true) });
                        if let Some(port) = browser {
                            servers[browser::SERVER] = codex_app::mcp_http_server(&format!("http://localhost:{port}/mcp"), browses_freely);
                        }
                        for (profile, port) in &profile_browsers {
                            servers[browser::server_name(Some(&profile.name))] = codex_app::mcp_http_server(&format!("http://localhost:{port}/mcp"), browses_freely);
                        }
                        if desktop {
                            servers[DESKTOP_SERVER] = codex_app::mcp_server(binary, &args(Some(DESKTOP_SERVER)), matches!(mode, Mode::Auto | Mode::Full));
                        }
                        Some(servers)
                    }
                    _ => None,
                };
                let writable: Vec<PathBuf> = writable.iter().map(PathBuf::from).collect();
                let (steer_tx, steer_rx) = mpsc::unbounded_channel::<String>();
                steer = Some(steer_tx);
                let (stop_tx, stop_rx) = oneshot::channel::<()>();
                stop = Some(stop_tx);
                // What Codex asks goes to the person as Claude Code's permission prompts do.
                let (ask_tx, mut ask_rx) = mpsc::unbounded_channel::<codex_app::Ask>();
                let asker = session_member(session);
                let forward = internal.clone();
                tokio::spawn(async move {
                    while let Some(ask) = ask_rx.recv().await {
                        let Some(who) = &asker else {
                            let _ = ask.answer.send(false);
                            continue;
                        };
                        let (tx, rx) = oneshot::channel();
                        let arguments = json!({ "tool_name": ask.tool, "input": ask.input });
                        let call = ToolCall { squad: who.squad.clone(), member: who.member.clone(), name: permissions::APPROVE_TOOL.into(), arguments };
                        let _ = forward.send(Internal::Ask(call, tx));
                        let answer = ask.answer;
                        tokio::spawn(async move {
                            let allowed = rx.await.ok().and_then(|r| serde_json::from_str::<Value>(&r.text).ok()).is_some_and(|v| v["behavior"] == "allow");
                            let _ = answer.send(allowed);
                        });
                    }
                });
                tokio::spawn(async move {
                    let emit = |event| {
                        let _ = internal.send(Internal::Event(Box::new(event)));
                    };
                    let mut streamed = false;
                    let request = codex_app::TurnRequest {
                        prompt: &prompt,
                        model: model.as_deref(),
                        effort: effort.as_deref(),
                        access,
                        resume: resume.as_deref(),
                        instructions: charter.as_deref(),
                        mcp_servers: servers.as_ref(),
                        writable: &writable,
                        env: &env,
                        provider: provider.as_ref(),
                    };
                    if let Some(port) = browser {
                        browser::ready(port, Duration::from_secs(90)).await;
                    }
                    let outcome = codex_app::run_turn(&binary, &cwd, request, Some(steer_rx), Some(ask_tx), Some(stop_rx), |activity| {
                        if matches!(activity, Activity::Text(_)) {
                            streamed = true;
                        }
                        match activity {
                            // Token Factory bills per token: no plan, no limits to show.
                            Activity::Limits(_) if nebius => {}
                            Activity::Limits(limits) => {
                                let _ = internal.send(Internal::CodexLimits(Some(limits)));
                            }
                            activity => {
                                if let Some(event) = activity_event(id, activity) {
                                    emit(event);
                                }
                            }
                        }
                    })
                    .await;
                    // Stopped: the turn's end was said already, and what follows it done.
                    if outcome.as_ref().is_ok_and(|turn| turn.result.status == "interrupted") {
                        return;
                    }
                    let thread = outcome.as_ref().ok().and_then(|turn| turn.thread_id.clone());
                    let result = outcome.map(|turn| turn.result).map_err(|err| if nebius { nebius_failure(err, lang) } else { codex_cli_failure(err, lang) });
                    let reply = finish_turn(&emit, id, result, streamed);
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply, claude_session: thread, ticket }));
                })
            }
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
                    allowed.extend(profile_browsers.iter().map(|(p, _)| format!("mcp__{}", browser::server_name(Some(&p.name)))));
                }
                if desktop && matches!(mode, Mode::Auto | Mode::Full) {
                    allowed.push(format!("mcp__{DESKTOP_SERVER}"));
                }
                // What the person always allows no longer asks, and reaches its service from inside the fence.
                let outward = permissions::still_asks(&squad_rules);
                let mut domains: Vec<String> = permissions::ALLOWED_DOMAINS.iter().map(|d| (*d).to_owned()).collect();
                domains.extend(permissions::domains_allowed(&squad_rules));
                if member_rules {
                    allowed.extend(squad_rules);
                }
                let mut blocked: Vec<String> =
                    if member_rules { permissions::BLOCKED.iter().map(|r| (*r).to_owned()).collect() } else { Vec::new() };
                // Claude Code's own question tool asks a terminal nobody watches:
                // a member with equipe's tools puts decisions with `ask_decision`.
                if member_rules && mcp_config.is_some() {
                    blocked.push(ENGINE_QUESTION_TOOL.to_owned());
                }
                // The person's browser sign-ins and equipe's logins: no fenced command reads them.
                // Nor other squads' homes.
                let (steer_tx, steer_rx) = mpsc::unbounded_channel::<String>();
                steer = Some(steer_tx);
                // The desktop helper's token and signing key too.
                let private: Vec<String> =
                    [self.config.home.join("browser"), self.config.home.join("credentials"), equipe_desktop_control::helper::dir(&self.config.home)]
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
                    let outcome = claude_code::run_turn(&binary, &cwd, request, Some(steer_rx), |activity| {
                        if matches!(activity, Activity::Text(_)) {
                            streamed = true;
                        }
                        if let Some(event) = activity_event(id, activity) {
                            emit(event);
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
            session.stop = stop;
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
        // Codex winds its turn down itself, keeping its thread whole; one that
        // doesn't in time is cut. Its end is said here, not by it.
        if session.stop.take().is_some_and(|stop| stop.send(()).is_ok()) {
            tokio::spawn(async move {
                tokio::time::sleep(STOP_GRACE).await;
                turn.abort();
            });
        } else {
            // Dropping the turn kills the engine process with it.
            turn.abort();
        }
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
            && session.codex.is_some()
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
            s.taken_back.clear();
            let threads = std::mem::take(&mut s.steer_threads);
            std::mem::take(&mut s.steering).into_iter().enumerate().map(|(i, t)| (t, threads.get(i).cloned().flatten())).collect::<Vec<_>>()
        });
        for (text, project) in unread.unwrap_or_default() {
            self.leave_memo_about(id, text, project);
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
        let leaving = session.name().filter(|_| session.lane.is_none()).map(str::to_owned);
        if session.work.is_some() {
            self.end_work(id, WorkEnd::Left);
        }
        if let (Some(squad), Some(name)) = (&squad, leaving) {
            self.drop_questions(squad, |set| set.member == name);
            // Letting it go no longer waits on the person: it left.
            self.drop_team_changes(squad, |c| c.member.as_deref() == Some(name.as_str()));
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
        // A member moving engines moves before it picks up its next ticket.
        self.apply_engine_moves(squad).await;
        let tickets = self.tickets(squad);
        let single_project = self.projects(squad).len() == 1;
        // Members equipe once brought in for a rush finish what they hold and take nothing new.
        let helpers = self.journals.get(squad).map(Journal::helpers).unwrap_or_default();
        let members: Vec<(String, String, bool)> = self
            .sessions
            .iter()
            .filter(|(id, s)| s.squad.as_deref() == Some(squad) && s.lane.is_none() && !helpers.contains(id))
            // An engine out of its plan, or waited for, takes no new ticket.
            .filter_map(|(_, s)| s.member.as_ref().map(|m| (m.name.clone(), m.role.clone(), s.is_free() && !self.engines.held(squad, s.engine))))
            .collect();
        let candidates: Vec<Candidate> =
            members.iter().map(|(name, role, free)| Candidate { name, role, free: *free }).collect();
        for (ticket, name) in board::assignments(&tickets, &candidates, single_project) {
            // Short of room: code work, which builds, waits; the PM was told.
            if tickets.iter().find(|t| t.id == ticket).is_some_and(|t| self.held_for_room(t.kind)) {
                continue;
            }
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
        let board_tools = has_tools(session) && self.config.equipe_binary.is_some();
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
        let reopened = ticket
            .note
            .as_ref()
            .filter(|n| n.starts_with("Reopened") || n.starts_with("Resumed") || n.starts_with("Design QA by "))
            .cloned();
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
                shown: Vec::new(),
                waiting: None,
                screens: false,
                verdict: None,
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
        if let Some(checked) = self.checked_by(&squad, ticket_id) {
            prompt.push_str(&design_qa::qa_prompt(ticket_id, &checked));
        }
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
        ticket.note = Some(format!("{}{reason}", crate::protocol::COULD_NOT_START));
        let id = ticket.id;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.ticket_event(ticket.id, "equipe", "blocked", reason);
        }
        let line = format!("T-{} couldn't start for {name}: {reason}", ticket.id);
        let shown = say!(self, "news.cannot_start", id = ticket.id, name = name, reason = reason);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line, Some(shown));
        // Where the PM runs the work it is the PM's to sort out, now — not the
        // person's, and not when someone next speaks to it.
        if self.journal(squad).is_some_and(|j| j.pm_accepts())
            && let Some(pm) = self.pm_of(squad)
        {
            let prompt = format!(
                "[equipe: T-{id} couldn't start for {name}: {reason}. Sort it out — fix what stops it (where the work it builds on \
conflicts, have that work's owner merge main in and resolve it), then take it on again with `update_ticket` (status \
todo); or drop it if it's no longer wanted.]"
            );
            let _ = self.internal.send(Internal::Wake { session: pm, prompt });
        }
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
        // A design QA handed in is finished by its verdict, not the person's review.
        let how = if how == WorkEnd::HandedIn && work.verdict.is_some() { WorkEnd::Checked } else { how };
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
            } else if matches!(how, WorkEnd::Accepted | WorkEnd::Checked) && ticket.kind.in_home() {
                if how == WorkEnd::Checked {
                    ticket.images = work.shown.clone();
                }
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
        // Code handed in, committed on its branch: Nemotron reads its diff, off the loop.
        if how == WorkEnd::HandedIn
            && !ticket.kind.in_home()
            && let Some(sandbox) = &work.sandbox
        {
            self.ask_for_review(&squad, &ticket, sandbox.clone(), id);
        }
        let handed_in = match (&how, &summary) {
            (WorkEnd::HandedIn, Some(summary)) if ticket.kind.in_home() => Some(self.home_hand_in(&squad, &ticket, work.sandbox.as_ref(), summary)),
            _ => summary.clone(),
        };
        // What the hand-in shows: the pictures named, then the images its branch changed.
        let images = match &work.sandbox {
            Some(sandbox) if how == WorkEnd::HandedIn => {
                let roots = [squad_home::path(&self.config.home, &squad), sandbox.path.clone()];
                crate::visuals::hand_in(&work.shown, &sandbox.changed_files().unwrap_or_default(), &sandbox.path, &roots)
            }
            _ => work.shown.clone(),
        };
        if !matches!(how, WorkEnd::HandedIn | WorkEnd::Accepted | WorkEnd::Checked) {
            ticket.images.clear();
        }
        // What the hand-in's news is about: the ticket, or the one a design QA checked.
        let (mut about, mut to): (Option<(TicketId, String)>, Option<String>) = (None, None);
        // What the PM is told, in English, and what the person reads (`news`).
        let (by, event, line, news) = match how {
            WorkEnd::HandedIn => {
                ticket.status = TicketStatus::Review;
                ticket.note = handed_in;
                ticket.images = images;
                match work.screens.then(|| self.ask_design_qa(&squad, &mut ticket, &name)).flatten() {
                    Some(qa) => {
                        to = Some(design_qa::DESIGNER.to_owned());
                        let line = format!("{name} handed in T-{}; the designer checks its screens on T-{qa} before the person reviews it", ticket.id);
                        (name.as_str(), "handed in", line, "news.design_qa_asked")
                    }
                    None => (name.as_str(), "handed in", format!("{name} handed in T-{} for your review", ticket.id), "news.handed_in"),
                }
            }
            WorkEnd::Checked => {
                let verdict = work.verdict.clone().unwrap_or(design_qa::Verdict { passed: true, findings: String::new() });
                ticket.note = summary.clone();
                // Its files are in the squad's folder: done; when they couldn't go in, the person sees to it.
                ticket.status = if matches!(landed, Some(Err(_))) { TicketStatus::Review } else { TicketStatus::Done };
                if ticket.status == TicketStatus::Done {
                    ticket.merged = merged_state(&ticket);
                }
                let checked = self.design_qa_checked(&squad, &ticket, &name, &verdict);
                let (id, engineer) = checked.as_ref().map_or((ticket.id, String::new()), |(t, e)| (t.id, e.clone()));
                if let Some((checked, engineer)) = checked {
                    about = Some((checked.id, checked.title));
                    to = Some(engineer);
                }
                if verdict.passed {
                    (name.as_str(), "design QA passed", format!("{name} passed the design QA of T-{id}: it is ready for the person's review"), "news.design_qa_passed")
                } else {
                    (name.as_str(), "design QA: changes", format!("{name}'s design QA sent T-{id} back to {engineer}: its screens need changes"), "news.design_qa_changes")
                }
            }
            WorkEnd::Waiting => {
                ticket.status = TicketStatus::Waiting;
                ticket.note = summary.clone();
                // Asked again in the note, with the rest.
                ticket.remaining.clear();
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
            WorkEnd::Reassigned { to } => {
                ticket.status = TicketStatus::Todo;
                ticket.assignee = None;
                ticket.owner = Some(to.clone());
                ticket.note = Some(match &ticket.branch {
                    Some(branch) => format!("The person gave this to {to}; {name} had started — the work so far is on {branch}."),
                    None => format!("The person gave this to {to}; {name} had started."),
                });
                ("you", "reassigned", format!("The person gave T-{} to {to}; {name} stopped working on it", ticket.id), "news.reassigned")
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
        // Work now ready for review — this ticket handed in, or the one a
        // designer's QA just passed — when the PM accepts it.
        let ready_for_pm = if !self.journal(&squad).is_some_and(|j| j.pm_accepts()) {
            None
        } else if ticket.ready_for_review() {
            Some(ticket.id)
        } else {
            about.as_ref().map(|(id, _)| *id).filter(|id| self.journal(&squad).and_then(|j| j.ticket(*id).ok().flatten()).is_some_and(|t| t.ready_for_review()))
        };
        let ticket_id = ticket.id;
        let (about_id, about_title) = about.unwrap_or_else(|| (ticket.id, ticket.title.clone()));
        let to = to.or_else(|| ticket.owner.clone()).unwrap_or_default();
        let shown = self.said(news, &[("id", about_id.to_string()), ("name", name.clone()), ("title", about_title), ("to", to)]);
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
            // Where the PM accepts, work ready for review is its to act on now —
            // a note it would read only when next spoken to leaves it waiting.
            if let Some(ready) = ready_for_pm
                && let Some(pm) = self.pm_of(&squad)
            {
                let prompt = format!(
                    "[equipe: T-{ready} is handed in and ready for you. Check it and accept it with `accept_work` (with \
`push` when the person asked for it to go live), send it back with `comment`, or — if it needs the person — put it to \
them with `propose_accept`. Then carry on with what else waits.]"
                );
                let _ = self.internal.send(Internal::Wake { session: pm, prompt });
            }
        }
    }

    /// An NVIDIA Nemotron model on Nebius Token Factory reads `ticket`'s
    /// branch against it, off the loop — git, then the network: the review
    /// comes back as [`Internal::Reviewed`]. Nothing without a key.
    fn ask_for_review(&mut self, squad: &str, ticket: &Ticket, sandbox: Sandbox, member: SessionId) {
        // A review of the work handed in before is of other work now.
        if let Some(journal) = self.journal(squad) {
            let _ = journal.forget_review(ticket.id);
        }
        let Some(cfg) = self.config.nemotron.clone() else {
            return;
        };
        // The language the person writes in, else their screen's.
        let language = self.journal(squad).and_then(Journal::language).unwrap_or_else(|| self.config.lang.english_name().to_owned());
        let mut input = crate::nemotron::ReviewInput { ticket: ticket.id, title: ticket.title.clone(), body: ticket.body.clone(), language, ..Default::default() };
        let (internal, squad, id) = (self.internal.clone(), squad.to_owned(), ticket.id);
        crate::daemon_log!("equipe: asking Nemotron on Nebius Token Factory to review T-{} of {squad}", ticket.id);
        tokio::spawn(async move {
            let diff = tokio::task::spawn_blocking(move || sandbox.diff()).await;
            let result = match diff {
                Ok(Ok(diff)) => {
                    input.diff = diff;
                    crate::nemotron::review_detailed(&cfg, input).await.map(Box::new)
                }
                Ok(Err(why)) => Err(crate::nemotron::ReviewError::Diff(why)),
                Err(err) => Err(crate::nemotron::ReviewError::Diff(err.to_string())),
            };
            let _ = internal.send(Internal::Reviewed { squad, ticket: id, member, result });
        });
    }

    /// A review back: kept, on the ticket's record, and on every screen of the
    /// squad. None: the log says which error (never the key or the diff), and
    /// the ticket and the member's conversation why.
    fn on_reviewed(&mut self, squad: &str, ticket: TicketId, member: SessionId, result: Result<Box<crate::nemotron::Reviewed>, crate::nemotron::ReviewError>) {
        match result {
            Ok(reviewed) => {
                let review = reviewed.review;
                let usage = reviewed.usage.map_or_else(|| "no usage reported".to_owned(), |u| format!("{} prompt + {} completion tokens", u.prompt, u.completion));
                crate::daemon_log!(
                    "equipe: Nemotron reviewed T-{ticket} of {squad}: {:?}, model {}, request {}, {usage}, {} ms{}",
                    review.verdict,
                    review.model,
                    reviewed.request_id.as_deref().unwrap_or("-"),
                    reviewed.latency.as_millis(),
                    if review.truncated { ", diff cut short" } else { "" },
                );
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.save_review(&review);
                    let _ = journal.ticket_event(ticket, "nemotron", "reviewed", &review.summary);
                }
                self.publish(Event::CodeReviewed { squad: squad.to_owned(), review });
            }
            Err(err) => {
                crate::daemon_log!("equipe: Nemotron couldn't review T-{ticket} of {squad}: {}", err.kind());
                let text = say!(self, "review.couldnt_run", id = ticket, why = err.why(self.config.lang));
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.ticket_event(ticket, "nemotron", "review failed", &text);
                }
                // In the conversation of the member who handed it in, else the PM's.
                let session = Some(member).filter(|id| self.sessions.contains_key(id)).or_else(|| self.pm_of(squad));
                if let Some(session) = session {
                    self.publish(Event::Notice { session, text });
                }
            }
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
        // The worktree goes once merged: its pictures are shown from the folder they landed in.
        let from = sandbox.path.canonicalize().unwrap_or_else(|_| sandbox.path.clone());
        if let Err(err) = worktree::merge_into_main(&sandbox.repo, &sandbox.branch, &message) {
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket.id, "equipe", "not merged", &err);
            }
            return Err(err);
        }
        let folder = squad_home::path(&self.config.home, squad);
        ticket.images = crate::visuals::moved(&ticket.images, &from, &folder.canonicalize().unwrap_or(folder));
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
        // The person's word comes before a designer's.
        self.drop_design_qa(squad, ticket);
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        // A proposal accepted is taken on: ready for its owner, not done.
        if found.status == TicketStatus::Proposed {
            return self.take_on(squad, found).await;
        }
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
        self.design_qa_closed(squad, ticket);
        // Work already in main stops asking to be merged as soon as the sweep says so.
        self.settle_all_merged();
        self.dispatch(squad).await;
    }

    /// The person merges a done ticket's branch into main — their gate, never a
    /// member's — and pushes main when they said so. A conflict merges nothing.
    /// The merge and the push are worked out off the loop (a push talks to a
    /// server); [`Runtime::on_merged`] takes it from there.
    fn merge(&mut self, squad: &str, ticket: TicketId, push: bool) {
        let Some(found) = self.find_ticket(squad, ticket) else {
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
        // Asked twice: the merge under way answers both.
        if !self.merging.insert((squad.to_owned(), ticket)) {
            return;
        }
        let message = format!("equipe: merge T-{ticket} {}\n\nAccepted; merged from {branch}.", found.title);
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let result = worktree::merge_into_main(&repo, &branch, &message)
                .map(|main| (main, push.then(|| worktree::push_main(&repo).map(|_| ()))));
            let _ = internal.send(Internal::Merged { squad, ticket, result });
        });
    }

    /// A merge the person asked for is through, or merged nothing.
    fn on_merged(&mut self, squad: &str, ticket: TicketId, result: MergeResult) {
        self.merging.remove(&(squad.to_owned(), ticket));
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        let (main, pushed) = match result {
            Ok(merged) => merged,
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
                self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found.clone() });
                // A conflict is work: its owner merges main in and resolves it,
                // and the person merges again from the board.
                let owner = found.owner.clone().or_else(|| found.assignee.clone()).filter(|_| err.contains(" conflicts with "));
                let message = match owner {
                    Some(owner) => {
                        let branch = found.branch.clone().unwrap_or_else(|| "its branch".into());
                        let line = format!(
                            "T-{ticket} couldn't be merged into main: {err}. In the ticket's worktree, merge main into {branch}, resolve the conflict keeping what both sides meant, run the tests and commit. Then say it's ready: the person merges it again from the board."
                        );
                        let shown = say!(self, "news.merge_conflict_owner", id = ticket);
                        self.brief_member(squad, &owner, &line, &shown);
                        self.brief_pm(squad, line, None);
                        let files = worktree::conflict_files(&err).unwrap_or_default();
                        say!(self, "error.not_merged_conflict", id = ticket, files = files, who = owner)
                    }
                    None => say!(self, "error.not_merged", id = ticket, err = err),
                };
                return self.squad_error(squad, message);
            }
        };
        let mut line = format!("You merged T-{ticket} · {} into {main}", found.title);
        let mut news = "news.merged";
        match pushed {
            Some(Ok(())) => {
                line.push_str(" and pushed it");
                news = "news.merged_pushed";
            }
            // A project kept on this machine only: merged is all there is to do.
            Some(Err(err)) if err == worktree::NO_ORIGIN => {
                line.push_str(" (it has no remote to push to)");
                news = "news.merged_local";
            }
            Some(Err(err)) => {
                let message = say!(self, "error.not_pushed", branch = main, err = err);
                self.squad_error(squad, message);
            }
            None => {}
        }
        let shown = self.said(news, &[("id", ticket.to_string()), ("title", found.title.clone()), ("branch", main.clone())]);
        found.merged = Some(true);
        found.off_main = None;
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "merged", &main);
        }
        self.clear_finished_builds(squad, &found);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: found });
        // Work it was built on came along with it.
        self.settle_all_merged();
        self.brief_pm(squad, format!("{line}."), Some(shown));
    }

    /// Done tickets said not to be on main whose branch has nothing left to
    /// bring there (it got in another way, or it changes nothing: `+0 −0`)
    /// are merged: nothing to ask the person. Their worktree went when they
    /// were accepted; their branch stays, as after any merge.
    ///
    /// Every squad open is looked at, every minute and after a merge — off the
    /// loop, a few git commands per repository, and a merge worked out only for
    /// a branch that moved, or whose main did ([`worktree::MergedCache`]).
    /// [`Runtime::on_settled`] takes the answer. One look at a time.
    fn settle_all_merged(&mut self) {
        self.settled_at = Some(std::time::Instant::now());
        let Some(mut cache) = self.merged_cache.take() else {
            self.settle_again = true;
            return;
        };
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        // By project: the tickets on it (squad, ticket, branch).
        type OnBranch = (String, TicketId, String);
        let mut asked: Vec<(PathBuf, Vec<OnBranch>)> = Vec::new();
        for squad in squads {
            for t in self.tickets(&squad).into_iter().filter(|t| t.not_on_main() && !t.kind.in_home()) {
                let (Some(project), Some(branch)) = (t.project.clone(), t.branch.clone()) else { continue };
                match asked.iter_mut().find(|(p, _)| *p == project) {
                    Some((_, tickets)) => tickets.push((squad.clone(), t.id, branch)),
                    None => asked.push((project, vec![(squad.clone(), t.id, branch)])),
                }
            }
        }
        if asked.is_empty() {
            self.merged_cache = Some(cache);
            return;
        }
        let internal = self.internal.clone();
        tokio::task::spawn_blocking(move || {
            let mut merged = Vec::new();
            for (project, tickets) in asked {
                let branches: Vec<String> = tickets.iter().map(|(_, _, b)| b.clone()).collect();
                for ((squad, id, _), told) in tickets.into_iter().zip(cache.merged(&project, &branches)) {
                    if told == Some(true) {
                        merged.push((squad, id));
                    }
                }
            }
            let _ = internal.send(Internal::Settled { merged, cache });
        });
    }

    /// The look at done work off main is back: what has nothing left to bring
    /// is merged — if it still waits to be.
    fn on_settled(&mut self, merged: Vec<(String, TicketId)>, cache: worktree::MergedCache) {
        self.merged_cache = Some(cache);
        for (squad, id) in merged {
            let Some(mut t) = self.find_ticket(&squad, id).filter(Ticket::not_on_main) else { continue };
            t.merged = Some(true);
            t.off_main = None;
            if let Some(journal) = self.journal(&squad) {
                let _ = journal.save_ticket(&t);
                let _ = journal.ticket_event(t.id, "equipe", "merged", "nothing left to merge: its work is in main");
            }
            self.clear_finished_builds(&squad, &t);
            self.publish(Event::TicketChanged { squad, ticket: t });
        }
        if std::mem::take(&mut self.settle_again) {
            self.settle_all_merged();
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
        self.drop_design_qa(squad, ticket);
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
        // What the answers leave open — not yet, or later — stays on the ticket.
        ticket.remaining = crate::protocol::asks::still_open(&ticket.asks(), note);
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
        self.drop_design_qa(squad, ticket);
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
            None => self.close_ticket(squad, found.clone(), TicketStatus::Cancelled),
        }
        self.clear_finished_builds(squad, &found);
        self.design_qa_closed(squad, ticket);
        self.drop_questions(squad, |set| set.ticket == Some(ticket));
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
        if call.name == board::SIGN_IN {
            return self.ask_sign_in(call, reply);
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
                    text: "Your browser is open in the person's browser panel, at that page, in their hands, with your `why` \
shown over it as what to do. Say in one line what they should do there, then end your turn. When they press \"done\" \
and hand it back you get a message and carry on by yourself — don't ask them to tell you."
                        .into(),
                    is_error: false,
                    image: None,
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
                image: None,
            });
            return;
        }
        let squad = call.squad.clone();
        // The PM accepting its team's work: it waits for the acceptance itself.
        if call.name == crate::accept::ACCEPT_WORK {
            let result = match self.member_id(&squad, &call.member) {
                Some(id) => self.accept_work(&squad, &call.member, id, &call.arguments).await,
                None => Err("You're not on the squad.".into()),
            };
            self.dispatch(&squad).await;
            let _ = reply.send(match result {
                Ok(text) => ToolResult { text, is_error: false, image: None },
                Err(text) => ToolResult { text, is_error: true, image: None },
            });
            return;
        }
        let result = self.run_tool(call);
        // Whatever changed, the rules look again.
        self.dispatch(&squad).await;
        let _ = reply.send(match result {
            Ok(text) => ToolResult { text, is_error: false, image: None },
            Err(text) => ToolResult { text, is_error: true, image: None },
        });
    }

    // ── permission ──

    /// The engine asks whether a member may use a tool its mode doesn't allow on
    /// its own. Blocked things are refused, "always allow" rules answered, and
    /// everything else put to the person.
    fn ask_permission(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let tool = permissions::canonical(call.arguments.get("tool_name").and_then(Value::as_str).unwrap_or("?")).to_owned();
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
            let _ = reply.send(ToolResult { text: "Say what you'll do in `action`.".into(), is_error: true, image: None });
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
        let refuse = |text: String| ToolResult::error(text);
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
                let _ = reply.send(ToolResult { text: String::new(), is_error: false, image: None });
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
        self.answer_with(approval_id, decision, why, None);
    }

    /// [`Self::answer`], with what the person wrote: a no's reason, or what
    /// they add to a yes to their member's own question.
    fn answer_with(&mut self, approval_id: u64, decision: Decision, why: &str, words: Option<&str>) {
        let Some(pending) = self.pending.remove(&approval_id) else {
            return;
        };
        let Pending { squad, approval, input, reply } = pending;
        if approval.tool == SIGN_IN_TOOL {
            self.publish(Event::ApprovalResolved { squad: squad.clone(), approval: approval_id, decision });
            return self.sign_in_answered(squad, approval, input, decision, reply);
        }
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
        let said = words.map(|w| format!("The person declined and said: \u{201c}{w}\u{201d}. Don't do it as asked; do what they said instead, or ask."));
        let why = match &said {
            Some(said) if decision == Decision::Deny => said.as_str(),
            _ if why.is_empty() && decision == Decision::Deny => "The person declined.",
            _ => why,
        };
        if let Some(preview) = &approval.preview {
            let _ = std::fs::remove_file(preview);
        }
        let _ = reply.send(if approval.tool == desktop_gate::SEND {
            // The desktop tool waits on this: nothing reaches the screen on a no.
            match decision {
                Decision::Allow | Decision::Always => ToolResult { text: String::new(), is_error: false, image: None },
                Decision::Deny => ToolResult { text: format!("Not sent: {why} Leave the draft as it is and say so."), is_error: true, image: None },
            }
        } else if approval.tool == CONFIRM {
            // A question the member asked itself: an answer in words, not a permission verdict.
            let text = match (decision, words) {
                (Decision::Allow | Decision::Always, None) => format!("The person said yes. Go ahead: {}", approval.action),
                (Decision::Allow | Decision::Always, Some(w)) => format!("The person said yes. Go ahead: {}\nThey added: \u{201c}{w}\u{201d} — do it their way.", approval.action),
                (Decision::Deny, Some(_)) => why.to_owned(),
                (Decision::Deny, None) => format!("The person said no ({why}). Don't do it; carry on without it, or say what you need."),
            };
            ToolResult { text, is_error: false, image: None }
        } else {
            verdict(decision, &input, why)
        });
        let line = match (decision, &rule, words) {
            (Decision::Deny, _, Some(w)) => say!(self, "notice.declined_said", words = w, action = approval.action),
            (_, _, Some(w)) => say!(self, "notice.allowed_said", words = w, action = approval.action),
            (Decision::Always, Some(rule), _) => say!(self, "notice.always_allowed", rule = rule, action = approval.action),
            (Decision::Allow | Decision::Always, _, _) => say!(self, "notice.allowed", action = approval.action),
            (Decision::Deny, _, _) if why == "The person declined." => say!(self, "notice.declined", action = approval.action),
            (Decision::Deny, _, _) => say!(self, "notice.declined_why", why = why, action = approval.action),
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

    // ── logins (`vault`) ──

    /// Change the logins off the loop (the Keychain may ask the person first),
    /// then tell every screen.
    fn with_vault(&mut self, change: impl FnOnce(&mut vault::Vault) -> Result<(), String> + Send + 'static) {
        let (vault, internal) = (self.vault.clone(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let result = match vault.lock() {
                Ok(mut v) => change(&mut v).map(|()| v.logins.clone()),
                Err(_) => Err("the logins are unreadable".into()),
            };
            let _ = internal.send(Internal::Logins(result));
        });
    }

    /// Every screen's list of browser profiles.
    fn publish_browser_profiles(&mut self) {
        let profiles = browser::profiles(&self.config.home);
        self.publish(Event::BrowserProfiles { profiles });
    }

    fn logins_changed(&mut self, result: Result<Vec<crate::protocol::vault::Login>, String>) {
        match result {
            Ok(logins) => {
                self.logins = logins.clone();
                self.publish(Event::Logins { logins });
            }
            Err(why) => {
                crate::daemon_log!("equipe: a login wasn't kept: {why}");
                let message = say!(self, "error.login_not_kept", why = why);
                self.publish(Event::Error { session: None, message, next: None });
            }
        }
    }

    /// A member asks to sign in to a site: with a login the person lets it use
    /// already, at once; otherwise a card puts it to the person. The tool's
    /// answer waits for how it went.
    fn ask_sign_in(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let text = |key: &str| call.arguments.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
        let (site, account, why) = (text("site"), text("account"), text("why"));
        let Some(id) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(ToolResult::error("Unknown member."));
            return;
        };
        if self.config.npx.is_none() {
            let _ = reply.send(ToolResult::error("You have no browser on this machine (Node.js isn't installed), so there is nothing to sign in with."));
            return;
        }
        let kept: Vec<&crate::protocol::vault::Login> = self.logins.iter().filter(|l| vault::serves(&l.site, &site)).collect();
        let login = match (kept.as_slice(), account.is_empty()) {
            ([], _) => {
                let _ = reply.send(ToolResult::error(format!(
                    "The person keeps no login for {}. Ask them to add one in equipe (Settings › Accounts), or hand them your browser with `show_my_browser` to sign in themselves.",
                    vault::site_of(&site)
                )));
                return;
            }
            ([one], true) => (*one).clone(),
            (many, true) => {
                let names: Vec<&str> = many.iter().map(|l| l.username.as_str()).collect();
                let _ = reply.send(ToolResult::error(format!("The person keeps several logins for {}: {}. Call `sign_in` again with `account`.", vault::site_of(&site), names.join(", "))));
                return;
            }
            (many, false) => match many.iter().find(|l| l.username.eq_ignore_ascii_case(&account)) {
                Some(l) => (*l).clone(),
                None => {
                    let names: Vec<&str> = many.iter().map(|l| l.username.as_str()).collect();
                    let _ = reply.send(ToolResult::error(format!("No login {account} for {}; kept: {}.", vault::site_of(&site), names.join(", "))));
                    return;
                }
            },
        };
        let input = json!({ "login": login.id, "site": site });
        let who = format!("{}/{}", call.squad, call.member);
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let approval = Approval {
            id: self.next_approval,
            session: id,
            member: call.member.clone(),
            ticket,
            tool: SIGN_IN_TOOL.to_owned(),
            action: format!("{} · {}", login.site, login.username),
            reason: (!why.is_empty()).then_some(why),
            preview: None,
        };
        // Let through already: no card, straight to the form.
        if login.allowed.contains(&who) {
            return self.sign_in_answered(call.squad, approval, input, Decision::Allow, reply);
        }
        self.next_approval += 1;
        self.pending.insert(approval.id, Pending { squad: call.squad.clone(), approval: approval.clone(), input, reply });
        self.publish(Event::ApprovalRequested { squad: call.squad.clone(), approval: approval.clone() });
        let (internal, approval_id) = (self.internal.clone(), approval.id);
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// The person answered a sign-in card (or the member may already): on a
    /// yes, equipe opens the member's browser on the site and types the login
    /// in itself — the member hears only how it went.
    fn sign_in_answered(&mut self, squad: String, approval: Approval, input: Value, decision: Decision, reply: oneshot::Sender<ToolResult>) {
        let id = input["login"].as_u64().unwrap_or_default() as u32;
        let member = approval.member.clone();
        let who = format!("{squad}/{member}");
        let login = self.logins.iter().find(|l| l.id == id).cloned();
        let Some(login) = login.filter(|_| decision != Decision::Deny) else {
            let _ = reply.send(ToolResult { text: "The person said no: don't sign in there; carry on without it, or say what you need.".into(), is_error: false, image: None });
            let declined_in = squad.clone();
            self.with_vault(move |v| {
                v.used(id, now_secs(), &declined_in, &member, "declined");
                Ok(())
            });
            self.audit(&squad, approval.ticket, "you", "declined", &approval.action);
            return;
        };
        if decision == Decision::Always {
            let who = who.clone();
            self.with_vault(move |v| v.allow(id, &who));
        }
        self.audit(&squad, approval.ticket, "you", if decision == Decision::Always { "always allowed" } else { "allowed" }, &approval.action);
        let Some(port) = self.browser_key(approval.session).and_then(|(sq, key)| self.open_browser(&sq, &key, false)) else {
            let _ = reply.send(ToolResult::error("Your browser couldn't be started."));
            return;
        };
        let (client, vault, internal) = (self.client.clone(), self.vault.clone(), self.internal.clone());
        let start = login.url.clone().unwrap_or_else(|| format!("https://{}", login.site));
        let (site, username) = (login.site.clone(), login.username.clone());
        tokio::spawn(async move {
            let outcome = sign_in_page(&client, port, &vault, id, &site, &start, &username).await;
            let (text, is_error, kept) = match &outcome {
                Ok(step) => (format!("Signed in to {site} as {username} — the form was filled and submitted ({step}). Look at the page to confirm, then carry on."), false, "signed_in"),
                Err(SignIn::NoForm) => (format!("No sign-in form on the page equipe opened for {site}. Open the page with the form yourself, then call `sign_in` again."), true, "no_form"),
                Err(SignIn::Failed(why)) => (format!("Couldn't sign in to {site}: {why}"), true, "failed"),
            };
            let _ = reply.send(ToolResult { text, is_error, image: None });
            let result = tokio::task::spawn_blocking(move || match vault.lock() {
                Ok(mut v) => {
                    v.used(id, now_secs(), &squad, &member, kept);
                    Ok(v.logins.clone())
                }
                Err(_) => Err("the logins are unreadable".into()),
            })
            .await
            .unwrap_or_else(|_| Err("stopped".into()));
            let _ = internal.send(Internal::Logins(result));
        });
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

    /// Whether a member may use the desktop: only one the person turned it on for.
    fn desktop_allowed(&self, call: &ToolCall) -> Result<(PathBuf, Vec<String>), ToolResult> {
        let journal = self.journals.get(&call.squad);
        let on = self.team(&call.squad).iter().any(|(_, name, _)| *name == call.member) && journal.is_some_and(|j| j.desktop_enabled(&call.member));
        if on {
            Ok((self.config.home.clone(), journal.map(Journal::messaging_apps).unwrap_or_default()))
        } else {
            Err(ToolResult::error(format!(
                "The Mac's desktop is off for {}. Only the person turns it on (`equipe desktop on {} --experimental`).",
                call.member, call.member
            )))
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
                Some(host) => browser::lane(&host, &self.config.home, squad, name, &output, shown).ok(),
                None => None,
            };
            let running = match running {
                Some(running) => running,
                // Another profile's browser lives only in the shared Chrome's host.
                None if browser::split_profile(name).1.is_some() => return None,
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
                let pages: Vec<equipe_protocol::BrowserPage> = pages.into_iter().map(browser::HostPage::page).collect();
                self.watch_checks(&lane, session, &pages, agent);
                let event = Event::BrowserTabs { squad, lane: lane.clone(), session, member, color, pages, agent_page: agent };
                self.tabs.insert(lane, event.clone());
                self.publish(event);
            }
            HostEvent::Control { lane, by, refused } => {
                let (squad, session) = self.lane_session(&lane);
                let asked = if by.is_none() { self.asked_in.remove(&lane) } else { None };
                // Handed back from a page that waited for them: passed — or, still
                // waiting, left to the member, and not taken from it again.
                let checked = if by.is_none() { self.checking.remove(&lane) } else { None };
                if let Some((page, url, _)) = &checked
                    && self.waits_for_person(&lane, *page)
                {
                    self.let_go.insert((lane.clone(), *page, url.clone()));
                }
                let check = self.checking.get(&lane).map(|c| c.2);
                match &by {
                    Some(_) => {
                        let why = self.asked_in.get(&lane).cloned();
                        self.held.insert(lane.clone(), why);
                    }
                    None => {
                        self.held.remove(&lane);
                    }
                }
                self.publish(Event::BrowserControl { squad, lane: lane.clone(), by: by.clone(), why: None, check });
                let Some(session) = session else { return };
                let working = self.sessions.get(&session).is_some_and(Session::is_working);
                // Its tools were turned away at a page that waited for the person,
                // and its turn ended: it carries on now.
                if checked.is_some() && asked.is_none() {
                    if refused && !working {
                        let text = say!(self, "notice.browser_given_back");
                        self.carry_on(session, text, "[equipe: the person handled the page that needed them in your browser (a human check or a sign-in) and handed it back. Carry on where you were.]".into()).await;
                    } else if working {
                        let text = say!(self, "notice.browser_given_back");
                        self.publish(Event::Notice { session, text });
                    }
                    return;
                }
                match (by.is_some(), asked.is_some()) {
                    // Put in their hands for a page that waits for them, or because
                    // its member asked: told when it was.
                    (true, _) if check.is_some() || self.asked_in.contains_key(&lane) => {}
                    // Only a turn under way needs telling: between turns nothing waits on it.
                    (true, _) if working => {
                        let text = say!(self, "notice.browser_taken");
                        self.publish(Event::Notice { session, text });
                    }
                    // Done with what it asked of them: it carries on.
                    (false, true) => {
                        let why = asked.unwrap_or_default();
                        let text = say!(self, "notice.browser_given_back");
                        self.carry_on(session, text, handed_back(&why)).await;
                    }
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
            HostEvent::Wants { lane, page, action } => {
                let (squad, _) = self.lane_session(&lane);
                self.publish(Event::BrowserWants { squad, lane, page, action });
            }
            HostEvent::Exited => {
                // Its tabs went with it.
                for (lane, tabs) in std::mem::take(&mut self.tabs) {
                    if let Event::BrowserTabs { squad, session, member, color, .. } = tabs {
                        self.publish(Event::BrowserTabs { squad, lane, session, member, color, pages: Vec::new(), agent_page: None });
                    }
                }
                self.asked_in.clear();
                self.held.clear();
                self.checking.clear();
                self.let_go.clear();
            }
        }
    }

    /// A member's page waits for a person — a "verify you are human" check, a
    /// sign-in, a code from their phone: its browser goes into the person's
    /// hands (its tools wait, and say why), and the person is told. Passed
    /// before they put a hand on it, it goes back to its member by itself.
    fn watch_checks(&mut self, lane: &str, session: Option<SessionId>, pages: &[equipe_protocol::BrowserPage], agent: Option<u64>) {
        if lane.is_empty() {
            return;
        }
        let let_go = |page: u64, url: &str| self.let_go.contains(&(lane.to_owned(), page, url.to_owned()));
        match browser::check_step(pages, agent, self.checking.contains_key(lane), self.held.contains_key(lane), let_go) {
            browser::CheckStep::Pause { page, url, check } => {
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": "person", "why": "check", "page": page }));
                if let Some(session) = session {
                    let site = browser::site_of(&url).to_owned();
                    let text = say!(self, "notice.browser_check", site = site);
                    self.publish(Event::Notice { session, text });
                }
                self.checking.insert(lane.to_owned(), (page, url, check));
            }
            browser::CheckStep::Release => {
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": null, "ifUntouched": true }));
            }
            browser::CheckStep::Nothing => {}
        }
    }

    /// The person closed a tab of a member's browser: its member is told —
    /// at its next step if it is at work, else in its conversation.
    async fn closed_by_person(&mut self, page: u64) {
        let found = self.tabs.iter().find_map(|(lane, tabs)| match tabs {
            Event::BrowserTabs { pages, .. } if !lane.is_empty() => pages.iter().find(|p| p.id == page).map(|p| (lane.clone(), p.url.clone())),
            _ => None,
        });
        let Some((lane, url)) = found else { return };
        let (_, Some(session)) = self.lane_session(&lane) else { return };
        let text = say!(self, "notice.browser_tab_closed", url = url);
        if self.sessions.get(&session).is_some_and(Session::is_working) {
            let prompt = format!("[equipe: the person closed your browser tab {url}. Don't open it again unless they ask; carry on without it.]");
            self.carry_on(session, text, prompt).await;
        } else {
            self.publish(Event::Notice { session, text });
        }
    }

    /// equipe tells a member something on the person's behalf — their browser
    /// came back, a tab of it was closed: a notice in the conversation
    /// (`notice`, in the person's language), never a message of theirs, and
    /// `prompt` to the member — read at its next step if it is at work, else
    /// starting a turn of its own.
    async fn carry_on(&mut self, id: SessionId, notice: String, prompt: String) {
        self.publish(Event::Notice { session: id, text: notice });
        self.set_going(id, prompt).await;
    }

    /// Set conversation `id` going on `prompt`: read at its next step when at
    /// work, else a turn of its own — with what it missed meanwhile.
    async fn set_going(&mut self, id: SessionId, prompt: String) {
        let Some(session) = self.sessions.get(&id) else { return };
        if session.is_working() {
            if let Some(steer) = session.steer.clone() {
                let _ = steer.send(prompt);
            }
            return;
        }
        if !self.ensure_codex(id).await {
            return;
        }
        let Some(session) = self.sessions.get_mut(&id) else { return };
        // What it missed travels with it, as with the person's own messages.
        let prompt = match std::mem::take(&mut session.briefing) {
            briefing if briefing.is_empty() => prompt,
            briefing => format!("[equipe: since your last turn —\n{}]\n\n{prompt}", briefing.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")),
        };
        self.save_waiting(id);
        self.start_turn(id, prompt);
    }

    /// Whether `page` of `lane` waits for a person, as last told.
    fn waits_for_person(&self, lane: &str, page: u64) -> bool {
        matches!(self.tabs.get(lane), Some(Event::BrowserTabs { pages, .. }) if pages.iter().any(|p| p.id == page && p.check.is_some()))
    }

    /// A lane's squad and the conversation it is the browser of: `squad/key`;
    /// `""` is the person's own.
    fn lane_session(&self, lane: &str) -> (Option<String>, Option<SessionId>) {
        let Some((squad, key)) = lane.split_once('/') else {
            return (None, None);
        };
        let key = browser::split_profile(key).0;
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
        self.publish(Event::BrowserControl { squad, lane, by: Some("person".into()), why: Some(why.to_owned()), check: None });
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
                let proposal = args.get("proposal").and_then(Value::as_bool).unwrap_or(false);
                // Where the PM runs the work, a member's proposal is simply done:
                // on the board, its own unless it names someone else.
                let just_do = !is_pm && self.journal(&squad).is_some_and(|j| j.pm_accepts());
                let owner = board::arg_str(&args, "owner").map(str::to_owned).or_else(|| just_do.then(|| member.clone()));
                let ticket = Ticket {
                    id: 0,
                    title: title.to_owned(),
                    body: body.to_owned(),
                    // The PM's are ready, unless it puts one up for the person to take on.
                    status: if (is_pm && !proposal) || just_do { TicketStatus::Todo } else { TicketStatus::Proposed },
                    kind,
                    owner: owner.clone(),
                    assignee: None,
                    project,
                    depends_on,
                    branch: None,
                    worktree: None,
                    merged: None,
                    off_main: None,
                    design_qa: None,
                    created_by: member.clone(),
                    note: None,
                    asks: vec![],
                    remaining: Vec::new(), images: Vec::new(),
                    updated_at: None,
                    priority: Default::default(), rank: 0,
                };
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let new_id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(new_id, &member, "created", title);
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket: Ticket { id: new_id, ..ticket } });
                let mut reply = format!("Created T-{new_id}.");
                if kind.in_home() {
                    reply = format!("Created T-{new_id} ({}).", kind.label());
                }
                if is_pm && proposal {
                    reply = format!("Proposed T-{new_id} — it waits for the person to take it on.");
                } else if just_do {
                    reply = format!("T-{new_id} is on the board and starts as soon as its owner is free; the PM is told.");
                    let line = format!("{member} put T-{new_id} on the board: {title} — drop it (update_ticket, cancelled) if it shouldn't be done");
                    let shown = say!(self, "news.proposed", name = member, id = new_id, title = title);
                    self.brief_pm(&squad, line, Some(shown));
                } else if !is_pm {
                    reply = format!("Proposed T-{new_id} — it waits for the PM or the person to accept it.");
                    let line = format!("{member} proposed T-{new_id}: {title}");
                    let shown = say!(self, "news.proposed", name = member, id = new_id, title = title);
                    self.brief_pm(&squad, line, Some(shown));
                }
                let staffed = |o: &str| self.member_id(&squad, o).is_some() || self.team(&squad).iter().any(|(_, _, r)| r == o);
                if let Some(owner) = owner.filter(|o| !staffed(o)) {
                    if is_pm {
                        reply.push_str(&format!(" Nobody on the squad is {owner} yet — propose one with propose_team_change (role {owner}): the person approves it on a card."));
                    } else {
                        reply.push_str(&format!(" Nobody on the squad is {owner} yet — the PM can propose adding one."));
                    }
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
                // Its pictures, found where it works; one that isn't there is said, to fix.
                let (cwd, roots) = self.picture_places(&squad, id);
                let shown = crate::visuals::resolve_all(&board::arg_strs(&args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
                    .map_err(|e| format!("{e}. Save the pictures in your worktree and give their paths, then hand in again."))?;
                let screens = self.screens_handed_in(&squad, id, &args, shown.len())?;
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let work = session.work.as_mut().ok_or("You're not on a ticket — there is nothing to hand in.")?;
                work.handed_in = Some(summary);
                work.shown = shown;
                work.waiting = None;
                work.screens = screens;
                Ok(if screens {
                    format!(
                        "Handed in T-{}. Your changes are committed when this turn ends; the squad's designer checks \
the screens, then the person reviews them. End your turn now with a short summary.",
                        work.ticket
                    )
                } else {
                    format!(
                        "Handed in T-{}. Your changes are committed when this turn ends and the person reviews them. \
End your turn now with a short summary.",
                        work.ticket
                    )
                })
            }
            board::BLOCK_ON_PERSON => {
                let asked = board::arg_strs(&args, "needs");
                if asked.is_empty() {
                    return Err("List what the person has to do, one thing per item.".into());
                }
                // What the person left open last time (not yet, later) is asked again with these.
                let on = self.sessions.get(&id).and_then(|s| s.work.as_ref()).map(|w| w.ticket);
                let open = on.and_then(|t| self.journal(&squad).and_then(|j| j.ticket(t).ok().flatten())).map(|t| t.remaining).unwrap_or_default();
                let needs = crate::protocol::asks::with_open(&asked, &open);
                let again = needs.len() - asked.len();
                let note = board::waiting_note(&needs, board::arg_str(&args, "why"));
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let work = session.work.as_mut().ok_or("You're not on a ticket — there is nothing to put down.")?;
                if work.handed_in.is_some() {
                    return Err(format!("T-{} is already handed in.", work.ticket));
                }
                work.waiting = Some(note);
                let again = if again > 0 {
                    format!(" The person left {again} thing(s) open last time; they are asked again after yours.")
                } else {
                    String::new()
                };
                Ok(format!(
                    "T-{} waits for the person when this turn ends; your changes are kept in your worktree and you get \
it back there once they have done it.{again} End your turn now with a short line on what you need.",
                    work.ticket
                ))
            }
            board::ASK_DECISION => self.ask_decision(&squad, &member, id, &args),
            crate::secrets::ASK_SECRET => self.ask_secret(&squad, &member, id, &args),
            board::DESIGN_QA => self.finish_design_qa(&squad, id, &args),
            crate::team::PROPOSE_TEAM_CHANGE => self.propose_team_change(&squad, &member, id, &args),
            crate::accept::PROPOSE_ACCEPT => self.propose_accept(&squad, &member, id, &args),
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
                let reply = board::estimate_reply(done, !session.subagents.is_empty(), session.engine == Engine::Claude);
                self.publish(Event::Estimated { session: id, estimate });
                Ok(reply)
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
                    steering_in: s.steer_threads.clone(),
                    lane: s.lane.clone().map(|(of, project)| equipe_protocol::Lane { of, project, kept: s.kept }),
                    estimate: s.estimate.clone().filter(|e| !e.is_empty()),
                    work: s.work.as_ref().map(|w| WorkInfo {
                        ticket: w.ticket,
                        project: w.project.clone(),
                        workspace: w.sandbox.as_ref().map(workspace_of),
                    }),
                    mode: s.mode,
                    effort: s.effort.clone(),
                    subagents: s
                        .subagents
                        .iter()
                        .map(|(id, what, step, since)| equipe_protocol::SubagentInfo { id: id.clone(), what: what.clone(), step: step.clone(), secs: since.elapsed().as_secs() })
                        .collect(),
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
        let questions = self.question_sets(squad).clone();
        let team_changes = self.team_changes(squad).clone();
        let accepts = self.accepts(squad).clone();
        let hq = self.hq(squad);
        let routines = self.routines(squad);
        // Nemotron's reviews of the tickets shown.
        let shown: Vec<TicketId> = tickets.iter().map(|t| t.id).collect();
        let reviews = self.journals.get(squad).and_then(|j| j.reviews(&shown).ok()).unwrap_or_default();
        Snapshot {
            reviews,
            routines,
            team_changes,
            accepts,
            squad: squad.to_owned(),
            projects,
            hq,
            sessions,
            tickets,
            mode,
            approvals,
            questions,
            scale,
            suggestions_off,
            plan: self.plan,
            plans: self.plans.clone(),
            engine_switching: self.journals.get(squad).map(Journal::engine_switching).unwrap_or_default(),
            pm_accepts: self.journals.get(squad).is_none_or(Journal::pm_accepts),
            secret_asks: self.secret_asks(squad).clone(),
            engine_proposals: self.engines.proposals(squad),
            desktop: self.desktop.state(),
            allowed,
            browsers_shown,
            disk: self.disk_for_snapshot(),
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
                crate::daemon_log!("equipe: {squad}'s folder couldn't be made: {err}");
            }
            // Members of one role once each kept their own lessons; now they share them.
            if let Err(err) = journal::fold_member_memories(&self.config.home, &squad) {
                crate::daemon_log!("equipe: {squad}'s members' memories couldn't be gathered: {err}");
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
                    if record.engine_session.is_some() {
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
                let unread_in: Vec<Option<String>> = journal
                    .waiting(record.id, "steer-threads")
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| Some(p).filter(|p| !p.is_empty()))
                    .collect();
                for (i, text) in unread.into_iter().enumerate() {
                    queued.push(text);
                    memo_threads.push(unread_in.get(i).cloned().flatten());
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
                        steer_threads: Vec::new(),
                        stop: None,
                        taken_back: Vec::new(),
                        send_now_prompt: None,
                        subagents: Vec::new(),
                        briefing: journal.waiting(record.id, "briefing").unwrap_or_default(),
                        memory,
                        mode: record.mode,
                        thread,
                        estimate: None,
                        turn_began: None,
                        threads,
                        lane: None,
                        kept: false,
                        answers_person: false,
                        effort: None,
                    },
                );
            }
            // The conversations the person opened with the PM, as they were left.
            let pm = self.sessions.iter().find(|(_, s)| s.squad.as_deref() == Some(squad.as_str()) && s.lane.is_none() && s.is_pm()).map(|(&id, _)| id);
            if let Some(pm) = pm {
                for c in journal.conversations().unwrap_or_default() {
                    let thread = DeskThread { claude_session: c.engine_session, ..DeskThread::default() };
                    let mut lane = lane_of(pm, &self.sessions[&pm], c.project, thread, true);
                    let lines = journal.conversation(c.id).unwrap_or_default();
                    lane.title = c.title.or_else(|| lines.iter().find_map(|e| if let Entry::User { text } = e { text.lines().next().map(str::to_owned) } else { None }));
                    lane.transcript = Transcript::from_entries(lines);
                    self.sessions.insert(c.id, lane);
                }
            }
            self.journals.insert(squad, journal);
        }
    }

    /// Members that were on a ticket when the daemon stopped carry on with it —
    /// a squad keeps working through a restart. (Codex members without the
    /// Codex CLI wait for their next message: equipe's sign-in loads on first use.)
    fn resume_work(&mut self) -> Vec<(SessionId, String)> {
        let codex_cli = codex_app::find(&self.config.codex_binary).is_some();
        let resumes = |s: &Session| s.engine == Engine::Claude || codex_cli;
        let on_tickets: Vec<(SessionId, TicketId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| resumes(s))
            .filter_map(|(&id, s)| s.work.as_ref().map(|w| (id, w.ticket)))
            .collect();
        // Memos for a turn the restart ended are handed back to be sent; a
        // member back on its ticket gets them when that turn ends, as before.
        let idle: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| resumes(s) && s.work.is_none() && !s.queued.is_empty())
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
        if session.work.is_some() || !has_tools(session) || session.claude_session.is_none() {
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
                crate::daemon_log!("equipe: {line}");
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
                crate::daemon_log!("equipe: {line}");
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
        shown: Vec::new(),
        waiting: None,
        screens: false,
        verdict: None,
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
        if crate::protocol::asks::read(note).is_some_and(|r| r.whole) {
            out.push_str(crate::roles::ANSWERS_GUIDE);
            out.push('\n');
        }
    } else if let Some(note) = reopened.filter(|n| n.starts_with("Design QA by ")) {
        out.push_str(&format!(
            "The squad's designer checked its screens and sent it back. {note}\nFix each difference it lists in your \
worktree, then hand in again with fresh screenshots of the screens you changed.\n"
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
        "\nWhen it meets the acceptance criteria, call submit_for_review with a short summary — and, when the work is \
visual, its pictures in `images` — then end your turn. \
If you can't go on without something only the person can do (a token, an account, a setting), call block_on_person \
with what they need to do — each ask its own item, a full sentence, files by their path in backticks — and end your \
turn: you are free for other work until they have. If you need a decision or an answer you can go on without for \
now, ask it with ask_decision (options to pick, or none for an answer in their own words) — never as a numbered list \
of questions in your reply."
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
fn save_preview(id: u64, image: &[u8]) -> Option<String> {
    use std::io::Write;
    // The desktop helper's screenshots are JPEG.
    let kind = if image.starts_with(&[0xff, 0xd8]) { "jpg" } else { "png" };
    let path = std::env::temp_dir().join(format!("equipe-send-{}-{id}.{kind}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&path).and_then(|mut f| f.write_all(image)).ok()?;
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
    ToolResult { text: text.to_string(), is_error: false, image: None }
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
/// What a member is told when the person hands back the browser it handed
/// them (`show_my_browser`, in the panel): its turn starts again on it.
fn handed_back(why: &str) -> String {
    let why = why.trim();
    let what = if why.is_empty() { String::new() } else { format!(" (you asked: {why})") };
    format!(
        "[equipe: the person finished in your browser{what} and handed it back to you. Look at the page as it is now \
and carry on where you left off.]"
    )
}

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
/// Whether a ticket just accepted is on its project's main branch, as far as
/// the loop tells without git: not yet (`Some(false)`) when it has a branch in
/// a repository — the sweep ([`Runtime::settle_all_merged`]), off the loop,
/// settles what has nothing left to bring; `None` when that can't be told (no
/// branch, no repository).
fn merged_state(ticket: &Ticket) -> Option<bool> {
    if ticket.kind.in_home() || ticket.branch.is_none() {
        return None;
    }
    let project = ticket.project.as_deref()?;
    project.ancestors().any(|dir| dir.join(".git").exists()).then_some(false)
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
async fn first_ready_engine(preference: &[Engine], store: &CredentialStore, claude_binary: &str, codex_binary: &str) -> Option<Engine> {
    for &engine in preference {
        let ready = match engine {
            // The person's Codex CLI, signed in; without it, equipe's own sign-in.
            Engine::Codex => match codex_app::login_status(codex_binary).await {
                Ok(Some(sign_in)) => sign_in.signed_in(),
                Ok(None) => matches!(store.load("openai"), Ok(Some(_))),
                Err(_) => false,
            },
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
    // Asked from the loop: a sign-in server that never answers holds nothing up for long.
    match tokio::time::timeout(SERVER_WAIT, codex::ensure_fresh(client, store, cred)).await {
        Ok(fresh) => fresh.map_err(|err| (words.fill("error.codex_not_refreshed", &[("err", err.to_string())]), "equipe login codex")),
        Err(_) => {
            let err = format!("no answer in {}s", SERVER_WAIT.as_secs());
            Err((words.fill("error.codex_not_refreshed", &[("err", err)]), "equipe login codex"))
        }
    }
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
/// A lane of member `m`: its conversation about `project`, picked up from
/// `thread`, run beside its others; `kept` for one the person opened.
fn lane_of(main: SessionId, m: &Session, project: Option<String>, thread: DeskThread, kept: bool) -> Session {
    Session {
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
        steer_threads: Vec::new(),
        stop: None,
        taken_back: Vec::new(),
        send_now_prompt: None,
        subagents: Vec::new(),
        answers_person: false,
        effort: None,
        briefing: Vec::new(),
        memory: m.memory.clone(),
        mode: m.mode,
        thread: project.clone(),
        threads: HashMap::new(),
        estimate: None,
        turn_began: None,
        lane: Some((main, project)),
        kept,
    }
}

/// `false` when the disk had no room for it. Nothing else stops for a write
/// that failed: the screens still get the event.
fn record(journal: &Journal, id: SessionId, event: &Event, finished: &[Entry], title: Option<&str>) -> bool {
    let mut room = true;
    let mut saw = |result: journal::Result<()>| {
        if let Err(err) = result
            && journal::is_full(&err)
        {
            room = false;
        }
    };
    for entry in finished {
        saw(journal.append(id, entry));
    }
    match event {
        Event::UserMessage { .. } => {
            if let Some(title) = title {
                saw(journal.set_title_if_empty(id, title));
            }
        }
        Event::ModelKnown { model, .. } => saw(journal.set_model(id, model)),
        Event::TurnEnded { usage: Some(u), .. } => saw(journal.add_tokens(id, u.total())),
        Event::BranchLanded { branch, .. } => saw(journal.set_outcome(id, &format!("landed {branch}"))),
        Event::NothingChanged { .. } => saw(journal.set_outcome(id, "no changes")),
        Event::SessionClosed { .. } => saw(journal.close_session(id)),
        _ => {}
    }
    room
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

/// Whether a session's engine runs an agent with tools: Claude Code, or the
/// person's Codex CLI — not equipe's own Codex sign-in, which only answers.
fn has_tools(session: &Session) -> bool {
    session.engine == Engine::Claude || session.codex.is_none()
}

/// What an engine's activity shows on the screens, as an event of session `id`.
/// Codex's plan limits are not one of its own: [`Internal::CodexLimits`].
fn activity_event(id: SessionId, activity: Activity) -> Option<Event> {
    Some(match activity {
        Activity::Text(text) => Event::TextDelta { session: id, text },
        Activity::Tool { name, target, label, background } => Event::ToolUse { session: id, tool: name, target, label, background },
        Activity::ToolDone(done) => {
            let (output, skipped) = equipe_protocol::ToolOutcome::tail(&done.output);
            let outcome = equipe_protocol::ToolOutcome { ok: done.ok, exit: done.exit, ms: Some(done.ms), output, skipped, diff: done.diff };
            Event::ToolResult { session: id, tool: done.name, target: done.target, outcome }
        }
        Activity::Thinking(text) => Event::Thinking { session: id, text },
        Activity::Model(model) => Event::ModelKnown { session: id, model },
        Activity::Output(tokens) => Event::Output { session: id, tokens },
        Activity::SubagentStarted { id: sub, what } => Event::SubagentStarted { session: id, id: sub, what },
        Activity::SubagentStep { id: sub, step } => Event::SubagentStep { session: id, id: sub, step },
        Activity::SubagentEnded { id: sub } => Event::SubagentEnded { session: id, id: sub },
        Activity::Steered(text) => Event::Steered { session: id, text },
        Activity::Plan { five_hour, seven_day } => {
            let window = |w: Option<(f64, i64)>| {
                w.map(|(used, resets_at)| equipe_protocol::PlanWindow { used_percent: (used * 100.0).round().clamp(0.0, 100.0) as u8, resets_at })
            };
            Event::PlanUsage { usage: equipe_protocol::PlanUsage { five_hour: window(five_hour), seven_day: window(seven_day) }, engine: Engine::Claude }
        }
        Activity::Limits(_) => return None,
    })
}

/// The Codex plan's limits as plan usage: its windows known by their length —
/// five hours, seven days — whichever it lists first. Pro, say, has no
/// five-hour window, so its first is the weekly one.
fn codex_plan(limits: &equipe_providers::turn::Limits) -> equipe_protocol::PlanUsage {
    use equipe_providers::turn::{FIVE_HOURS, SEVEN_DAYS};
    let window = |minutes| {
        limits.window(minutes).and_then(|w| {
            Some(equipe_protocol::PlanWindow { used_percent: w.used_percent.round().clamp(0.0, 100.0) as u8, resets_at: w.resets_at? })
        })
    };
    equipe_protocol::PlanUsage { five_hour: window(FIVE_HOURS), seven_day: window(SEVEN_DAYS) }
}

/// A Codex CLI turn's failure, and what to do about it.
fn codex_cli_failure(err: equipe_providers::Error, lang: crate::i18n::Lang) -> (String, Option<&'static str>) {
    match &err {
        equipe_providers::Error::CodexSignedOut => (crate::i18n::core(lang).get("error.codex_cli_signed_out").to_owned(), Some("equipe login codex")),
        equipe_providers::Error::Login(_) => (err.to_string(), Some("equipe status")),
        _ => (err.to_string(), None),
    }
}

/// A turn on Token Factory that failed: a refused key is Token Factory's, not Codex's sign-in.
fn nebius_failure(err: equipe_providers::Error, lang: crate::i18n::Lang) -> (String, Option<&'static str>) {
    let words = crate::i18n::core(lang);
    let message = err.to_string();
    let lower = message.to_ascii_lowercase();
    let refused = matches!(err, equipe_providers::Error::CodexSignedOut) || lower.contains("401") || lower.contains("403") || lower.contains("unauthorized");
    if refused {
        return (words.get("error.nebius_unauthorized").to_owned(), Some("https://tokenfactory.nebius.com"));
    }
    codex_cli_failure(err, lang)
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

/// How a sign-in on a page went wrong.
enum SignIn {
    NoForm,
    Failed(String),
}

/// Sign in on the member's browser at `port`: to the site's sign-in page
/// unless it is on the site already, then the fill script — twice for a
/// sign-in that asks the username and the password on pages of their own.
/// The password is fetched off the loop and goes nowhere but the page.
async fn sign_in_page(client: &reqwest::Client, port: u16, vault: &std::sync::Arc<std::sync::Mutex<vault::Vault>>, id: u32, site: &str, start: &str, username: &str) -> Result<&'static str, SignIn> {
    let text_of = |items: Option<Vec<Value>>| -> String { items.unwrap_or_default().iter().filter_map(|i| i.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n") };
    let here = text_of(browser::call(client, port, "browser_evaluate", json!({ "function": "() => location.href" })).await);
    if !here.lines().any(|l| vault::serves(site, l.trim().trim_matches('"'))) {
        browser::navigate(client, port, start).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    let vault = vault.clone();
    let password = tokio::task::spawn_blocking(move || vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|v| v.password(id)))
        .await
        .map_err(|_| SignIn::Failed("stopped".into()))?
        .map_err(|why| SignIn::Failed(format!("the password couldn't be read from the Keychain ({why})")))?;
    let mut last = "";
    for _ in 0..2 {
        let answer = text_of(browser::call(client, port, "browser_evaluate", json!({ "function": vault::fill_script(username, &password) })).await);
        let step = if answer.contains(r#""step":"password""#) || answer.contains(r#"\"step\":\"password\""#) {
            "password"
        } else if answer.contains("username") {
            "username"
        } else {
            ""
        };
        match step {
            "" if last.is_empty() => return Err(SignIn::NoForm),
            "" => return Ok("username, then password"),
            "password" => return Ok(if last.is_empty() { "username and password" } else { "username, then password" }),
            _ => {
                // The username alone: its next page asks the password.
                last = "username";
                tokio::time::sleep(Duration::from_millis(2500)).await;
            }
        }
    }
    Ok("username; the password page didn't come")
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
    fn a_turn_on_token_factory_with_its_key_refused_says_whose_key() {
        let lang = crate::i18n::Lang::En;
        // As codex 0.154 reports a 401 from the provider (mock-verified).
        let refused = equipe_providers::Error::Backend { message: "unexpected status 401 Unauthorized: Invalid API key, url: http://127.0.0.1:1/responses".into(), code: Some("other".into()) };
        let (said, next) = nebius_failure(refused, lang);
        assert!(said.contains("Nebius Token Factory refused the key"), "{said}");
        assert_eq!(next, Some("https://tokenfactory.nebius.com"));
        assert!(nebius_failure(equipe_providers::Error::CodexSignedOut, lang).0.contains("NEBIUS_API_KEY"));
        let other = nebius_failure(equipe_providers::Error::Protocol("stream ended".into()), lang);
        assert_eq!(other, ("stream ended".to_owned(), None));
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    #[should_panic(expected = "must not run on the core loop")]
    async fn git_that_may_take_long_stops_a_debug_build_on_the_core_loop() {
        CORE_LOOP.scope((), async { worktree::is_merged(Path::new("/nonexistent"), "equipe/x").ok() }).await;
    }

    #[tokio::test]
    async fn off_the_core_loop_it_runs() {
        assert!(!on_core_loop());
        let told = tokio::task::spawn_blocking(|| worktree::is_merged(Path::new("/nonexistent"), "equipe/x")).await.unwrap();
        assert!(told.is_err(), "no repository there");
        assert!(CORE_LOOP.scope((), async { on_core_loop() }).await);
    }

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
