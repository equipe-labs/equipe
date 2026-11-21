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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use equipe_protocol::harness;
use equipe_protocol::tags;
use equipe_protocol::rpc::{ToolCall, ToolResult};
use equipe_protocol::transcript::scope;
use equipe_protocol::{
    Access, Approval, AuthState, BrowserNavAction, Command, CompactTrigger, Decision, Engine, Entry, Event, Member, MemberRef, Mode, SessionId, SessionInfo,
    Estimate, ExecutionState, ModelChoice, OffMain, RoleModel, SessionSpec, SessionState, Snapshot, Ticket, TicketId, TicketKind, TicketStatus, Transcript, TurnOutcome, Usage, WorkInfo, Workspace,
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

use crate::board;
use crate::browser;
use crate::desktop_gate::{self, Act, Verdict};
use crate::desktop;
use crate::eta;
use crate::guard;
use crate::journal::{self, Journal, Requeued, SessionRecord, TicketWork};
use crate::permissions;
use crate::roles::{self, Teammate};
use crate::routines;
use crate::skills;
use crate::turn_context;
use crate::vault;
use equipe_protocol::routines::Routine;
use crate::squad_home;
use crate::squad_memory;
use crate::steers;
use crate::tidy;
use crate::watchdog;
use crate::worktree::{self, Sandbox};

/// Runtime turns renew this on activity and on the daemon's minute tick.
const EXECUTION_LEASE_SECS: u64 = 150;

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

mod blocked_on;
mod decide;
mod glance;
mod room;
mod machine;
mod engines;
mod design;
mod design_session;
mod revise;
mod live_preview;
mod shown;
mod refcards;
mod briefs;
mod plans;
mod accept;
mod gate;
mod main_red;
pub(crate) use gate::checks_scratch;
mod secrets;
mod cli_signin;
mod machine_creds;
mod squad_mcp;
mod squad_keys;
mod logins;
mod sign_ins;
mod window;
mod autopilot;
mod design_qa;
mod design_review;
mod follow_ups;
mod requests;
mod flow;
mod stops;
mod settled;
mod person_gate;
mod usage_digest;
mod team;
mod heartbeat;
mod signals;
mod deploys;
mod go_live;
mod tickets;
mod compact;
mod hygiene;
mod controls;
mod autonomy;
mod review;
mod retro;
mod growth;
mod sprint;
mod watch;
mod words;
mod put_away;
mod plan_use;
mod helping;
mod side_chat;
mod providers;
mod phone;
mod call;
mod tiers;
mod workspace;
mod turn_end;
mod status;
pub use hygiene::{NATIVE_DESK_TOKENS, NATIVE_TICKET_TOKENS, ROTATE_DESK_TOKENS, ROTATE_TICKET_TOKENS, wants_large_window};
mod pace;
mod verify;
mod guards;
mod told;
mod withdraw;
pub use turn_end::turn_ending;
use turn_end::{RunEnd, TurnEnds, TurnWatch};

/// How many recent lines a snapshot carries per session.
const SNAPSHOT_LINES: usize = 200;

/// How much of a member's answer the PM is told.
const BRIEF_REPLY_CHARS: usize = 400;

/// Closed tickets a snapshot still shows, newest first.
const CLOSED_TICKETS_SHOWN: usize = 20;
/// Conversations put away a screen lists, the latest first.
const PUT_AWAY_SHOWN: usize = 50;
/// The routines one squad may keep.
const MAX_ROUTINES: usize = 20;

/// The MCP server equipe gives members; its tools are `mcp__equipe__*`.
const MCP_SERVER: &str = "equipe";
/// Claude Code's own question tool, which members don't use (`ask_decision` instead).
const ENGINE_QUESTION_TOOL: &str = "AskUserQuestion";
/// The Mac's desktop: its tools are `mcp__desktop__…`.
pub const DESKTOP_SERVER: &str = "desktop";
/// equipe-design's extraction and remix tools, for designers (`equipe-design mcp`).
pub const DESIGN_SERVER: &str = "design";
/// The tool name a member's own question shows under, among approvals.
pub(crate) const CONFIRM: &str = "Confirm";

/// What a member is told when its own request (`ask_person`) has waited
/// on the person's card for a few minutes: it doesn't wait there any longer.
pub(crate) const STILL_ASKED: &str = "The person hasn't answered yet; your request stays on their card. Don't do it yet. \
Carry on now with other work that doesn't depend on it; their answer comes to you as a message (\"[equipe] The person \
answered your request …\"). If nothing else is left on this ticket, end your turn: it waits on them and comes back to \
you with their answer.";
/// An approval to sign a member in with a kept login (`vault`).
pub(crate) const SIGN_IN_TOOL: &str = "Sign in";

/// How long `busy` waits for the loop before it answers with what the loop is on.
pub const ANSWER_WAIT: Duration = Duration::from_secs(3);

/// How long the loop waits on a server (a Codex sign-in, its models) or on
/// `claude auth status` before it gives up: nothing else is done meanwhile.
const SERVER_WAIT: Duration = Duration::from_secs(30);

/// How often the sweep looks for done work that reached main outside equipe.
const SETTLE_EVERY: Duration = Duration::from_secs(10 * 60);

/// How long after a merge failed (not for a conflict) equipe tries it again,
/// and how many times before the PM is told to see to it.
const MERGE_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const MERGE_TRIES: u32 = 6;
/// Past [`MERGE_TRIES`], still tried this often: once what is in the way is
/// cleared, accepted work lands on its own — nobody accepts it again.
const MERGE_RETRY_SLOW: Duration = Duration::from_secs(30 * 60);
/// A done ticket whose merge failed this lately is tried again after a restart.
const MERGE_RETRY_WITHIN: u64 = 3 * 86_400;

/// A failed merge tried again: whether to push after, how many times it
/// failed, and when it is tried next.
#[derive(Debug, Clone)]
struct MergeRetry {
    push: bool,
    failed: u32,
    next: std::time::Instant,
    /// Tried again now, by equipe: its failure isn't said to the person again.
    running: bool,
}


/// How long a member waits for the person to answer a request for permission
/// before it is declined.
const APPROVAL_WAIT: Duration = Duration::from_secs(30 * 60);

/// How long Codex lets a call to one of equipe's own servers run: past the
/// longest such a call may wait on the person ([`APPROVAL_WAIT`]) — a yes
/// asked with `ask_person`, a sign-in, a message the desktop would send — so
/// the turn waits for their answer, as Claude Code's does, instead of giving
/// up on the call after Codex's default few minutes and ending its turn with
/// the card still open in front of them.
pub fn person_call_timeout_secs() -> u64 {
    APPROVAL_WAIT.as_secs() + 60
}

/// How long Codex lets a call to equipe's board server run: past any wait on
/// the person, and past `helper_result`'s longest wait for a member with helpers.
pub fn board_call_timeout_secs() -> u64 {
    person_call_timeout_secs().max(crate::helpers::tool_timeout_secs())
}

/// How long a stopped Codex turn has to wind down before it is cut.
const STOP_GRACE: Duration = Duration::from_secs(8);

/// How often the Codex plan's usage is read while a squad has Codex members,
/// and at most when a screen attaches.
const CODEX_USAGE_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long (seconds) a model the Codex account refused is skipped before it
/// is tried again: a refusal may be passing, so it is never for good.
const REFUSED_FOR: u64 = 10 * 60;
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
    /// Where to look for `equipe-design`, the designer's `design` tools: the
    /// daemon's PATH, searched each turn so an install shows up without a
    /// restart. `None`: no `design` server.
    pub design_path: Option<std::ffi::OsString>,
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
    /// Look at the free space of the disk now and then, and on a short disk
    /// hold code work back and trim build caches (the daemon). Off, the disk's
    /// room is only what a write that failed or a screen's look says: a test's
    /// runtime doesn't stall on the Mac's real disk running short.
    pub watch_disk: bool,
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
            design_path: std::env::var_os("PATH"),
            mac: Box::new(desktop::ThisMac),
            suggest: true,
            browser_host: cfg!(target_os = "macos"),
            person_home: None,
            lang: crate::i18n::Lang::resolve(crate::i18n::Front::Terminal),
            nemotron: crate::nemotron::Config::from_env_or_saved(&equipe_home()).ok(),
            watch_disk: true,
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
    /// A person-only command came from a member's own process and was refused
    /// at the door ([`crate::gateway`], [`crate::origin`]); the person is told.
    /// `squad` is the command's, when it named one.
    MemberRefused(Option<String>),
    Snapshot(String, oneshot::Sender<Snapshot>),
    Subscribe(String, oneshot::Sender<Subscription>),
    Tool(ToolCall, oneshot::Sender<ToolResult>),
    DesktopGate(Box<desktop_gate::Check>, oneshot::Sender<ToolResult>),
    /// May this member use the desktop: equipe's home and the squad's
    /// messaging apps if so.
    DesktopTool(ToolCall, oneshot::Sender<Result<(PathBuf, Vec<String>), ToolResult>>),
    Busy(oneshot::Sender<equipe_protocol::rpc::Busy>),
    /// A screen's `growth.*` request: its method and params → its answer.
    Growth(String, Value, oneshot::Sender<Result<Value, (i64, String)>>),
    /// A screen's `sprint.*` request: its method and params → its answer.
    Sprint(String, Value, oneshot::Sender<Result<Value, (i64, String)>>),
    /// What uses the disk (`equipe disk`).
    Disk(oneshot::Sender<crate::disk::DiskReport>),
    /// A squad's status at a glance (`squad_status`).
    Status(equipe_protocol::status::StatusAsk, oneshot::Sender<Result<equipe_protocol::status::SquadStatus, String>>),
    /// Give back these items of the disk (`equipe disk clean`).
    DiskClean(Vec<String>, oneshot::Sender<crate::disk::DiskCleaned>),
    PersonWindow(String, oneshot::Sender<bool>),
    /// Sign in to a site in equipe's own browser, where the person is (`start_sign_in`).
    SignIn(String, oneshot::Sender<equipe_protocol::rpc::BrowserSignInOpened>),
    BrowserHub(oneshot::Sender<std::sync::Arc<browser::Hub>>),
    Desktop(DesktopRequest),
    /// A member's phone tools ([`crate::phone`]).
    Phone(phone::PhoneRequest),
    /// An outside MCP server's proxy asks where to reach it, with its
    /// credential; `Some(value)`: the server just refused that one.
    McpUpstream(equipe_protocol::rpc::McpProxyOpen, Option<String>, oneshot::Sender<Result<crate::mcp_gateway::Upstream, String>>),
    /// What a proxy's message showed of its server.
    McpSeen(equipe_protocol::rpc::McpProxyOpen, crate::mcp_gateway::Seen),
    /// The member session whose turn runs as this engine process, if any.
    EngineMember(u32, oneshot::Sender<Option<equipe_protocol::MemberRef>>),
    /// An engine's plan, read outside a turn.
    Usage(Engine, equipe_protocol::PlanUsage),
    /// The lease check, as if at this unix time (tests).
    CheckLeases(u64, oneshot::Sender<()>),
    /// The minute's look at turns running, as if this much later (tests).
    LookAtTurns(Duration, oneshot::Sender<()>),
    /// The minute tick, now (a measurement's).
    Minute(oneshot::Sender<()>),
    /// Paused runs looked at, as if at this unix time (tests).
    TurnEndsDue(u64, oneshot::Sender<()>),
    Shutdown(oneshot::Sender<()>),
    /// Set the Mac's desktop up by itself, from now on (the person's daemon).
    DesktopAuto,
}

/// Where screens send commands. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    requests: UnboundedSender<Request>,
    /// What the loop is on, for an answer when it doesn't answer.
    pulse: std::sync::Arc<watchdog::Pulse>,
    /// The screens following each squad, and where they are ([`crate::screens`]).
    screens: std::sync::Arc<crate::screens::Screens>,
}

impl Handle {
    /// The screens following each squad: a screen takes a seat as it says
    /// hello, and leaves with its connection.
    pub fn screens(&self) -> &std::sync::Arc<crate::screens::Screens> {
        &self.screens
    }

    pub fn send(&self, command: Command) {
        let _ = self.requests.send(Request::Command(command));
    }

    /// A person-only command reached the socket from a member's own process and
    /// was refused ([`crate::gateway`]); the person is told, in their language.
    /// `squad` is the command's, when it named one.
    pub fn refused_person_only(&self, squad: Option<String>) {
        let _ = self.requests.send(Request::MemberRefused(squad));
    }

    /// Where an outside MCP server's proxy reaches its server, and the
    /// credential it adds — for a member who gets it ([`crate::mcp_gateway`]).
    pub async fn mcp_upstream(&self, open: &equipe_protocol::rpc::McpProxyOpen, rejected: Option<String>) -> Result<crate::mcp_gateway::Upstream, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::McpUpstream(open.clone(), rejected, tx));
        rx.await.unwrap_or_else(|_| Err("equipe is shutting down".to_owned()))
    }

    /// The member session whose turn's engine is process `pid`: what a
    /// connection from under that engine is bound to ([`crate::gateway`]).
    pub async fn engine_member(&self, pid: u32) -> Option<equipe_protocol::MemberRef> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::EngineMember(pid, tx));
        rx.await.ok().flatten()
    }

    /// What a proxy's message showed of its server: the squad's status follows.
    pub fn mcp_seen(&self, open: &equipe_protocol::rpc::McpProxyOpen, seen: crate::mcp_gateway::Seen) {
        let _ = self.requests.send(Request::McpSeen(open.clone(), seen));
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

    /// A screen's `growth.*` request (docs/GROWTH.md): its answer, or a
    /// JSON-RPC error's code and message.
    pub async fn growth(&self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Growth(method.to_owned(), params, tx));
        rx.await.unwrap_or_else(|_| Err((-32000, "equipe is shutting down".into())))
    }

    /// A screen's `sprint.*` request (docs/SPRINTS.md): its answer, or a
    /// JSON-RPC error's code and message.
    pub async fn sprint(&self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Sprint(method.to_owned(), params, tx));
        rx.await.unwrap_or_else(|_| Err((-32000, "equipe is shutting down".into())))
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
        let what = desktop::Use::of(&call.name, &call.arguments);
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
                    // to another member, or been taken back, meanwhile. A
                    // headless ref action still takes no screen after a yes.
                    if asked {
                        arguments[equipe_desktop_control::controller::ASKED] = json!(true);
                        let again = if what == desktop::Use::Headless { desktop::Use::Headless } else { desktop::Use::Approved };
                        start = match self.desktop_ask(who.clone(), again).await {
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
        let mut result = helper::call(&home, &member, &call.name, &arguments).await;
        self.desktop_done(who, start, what.physical());
        // Allowed while locked by the person's choice, and refused by macOS or the app anyway.
        if result.is_error && what == desktop::Use::Headless && desktop::Probe::screen(&desktop::ThisMac) == desktop::Screen::Locked {
            result.text.push_str(
                "\n(The Mac's screen is locked. Accessibility actions run while it is, but some apps don't answer them until it is unlocked, and nothing typed or clicked physically can reach an app. Try reading the app again, or wait for the person to unlock.)",
            );
        }
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

    /// A sign-in to `url` (a site or an address) in equipe's own browser, as
    /// the app's "Sign in to a site" does it: in the panel of an attached
    /// screen, else the separate Chrome window.
    pub async fn sign_in(&self, url: String) -> equipe_protocol::rpc::BrowserSignInOpened {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::SignIn(url, tx));
        rx.await.unwrap_or(equipe_protocol::rpc::BrowserSignInOpened { opened: false, window: false })
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

    /// A squad's status at a glance: kept for [`crate::status::CACHE_FOR`], read
    /// from its journal off the loop. `Err` says why not (no such squad).
    pub async fn squad_status(&self, ask: equipe_protocol::status::StatusAsk) -> Result<equipe_protocol::status::SquadStatus, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.send(Request::Status(ask, tx));
        match tokio::time::timeout(ANSWER_WAIT * 4, rx).await {
            Ok(answer) => answer.unwrap_or_else(|_| Err("the daemon didn't answer".to_owned())),
            Err(_) => Err("the daemon didn't answer".to_owned()),
        }
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

    /// The daemon's own: ask the loop what `status` asks every
    /// [`watchdog::PROBE_EVERY`], and once nothing has answered for `after`
    /// with the loop on no step ([`watchdog::hung`]), the watchdog ends the
    /// process — its threads dumped to `dump` first — for the login service
    /// to start it again. The asking stops with the loop.
    pub fn heal_when_hung(&self, after: Duration, dump: Option<PathBuf>) -> tokio::task::JoinHandle<()> {
        self.pulse.heal_after(after, dump);
        let (requests, pulse) = (self.requests.clone(), self.pulse.clone());
        tokio::spawn(async move {
            loop {
                let (tx, rx) = oneshot::channel();
                if requests.send(Request::Busy(tx)).is_err() || rx.await.is_err() {
                    return;
                }
                pulse.answered();
                tokio::time::sleep(watchdog::PROBE_EVERY).await;
            }
        })
    }

    /// An engine's plan usage read outside a turn (a poll of Codex's limits,
    /// say): told every screen as [`Event::PlanUsage`], and weighed for
    /// moving members between engines at once.
    pub fn usage_read(&self, engine: Engine, usage: equipe_protocol::PlanUsage) {
        let _ = self.requests.send(Request::Usage(engine, usage));
    }

    /// The minute tick's lease check, run now as if at unix time `at`, and
    /// answered once done: for tests of what stalls when, without waiting.
    #[doc(hidden)]
    pub async fn check_leases_at(&self, at: u64) {
        let (tx, rx) = oneshot::channel();
        if self.requests.send(Request::CheckLeases(at, tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// The minute's look at turns running — what went silent — as if `later`
    /// from now, and answered once done: for tests, without waiting.
    #[doc(hidden)]
    pub async fn look_at_turns_after(&self, later: Duration) {
        let (tx, rx) = oneshot::channel();
        if self.requests.send(Request::LookAtTurns(later, tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// The minute tick, run now and answered once done: for measuring what
    /// it holds the loop for, without waiting for it.
    #[doc(hidden)]
    pub async fn minute_now(&self) {
        let (tx, rx) = oneshot::channel();
        if self.requests.send(Request::Minute(tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// Keep each step of the loop at least `after` long, with its parts, for
    /// [`Self::slow_steps`] (those past [`watchdog::SLOW_LOGGED`] always are).
    #[doc(hidden)]
    pub fn keep_steps_from(&self, after: Duration) {
        self.pulse.keep_steps_from(after);
    }

    /// The latest steps of the loop that took long, oldest first, taken.
    #[doc(hidden)]
    pub fn slow_steps(&self) -> Vec<watchdog::SlowStep> {
        self.pulse.slow_steps()
    }

    /// Look at the runs equipe paused — a retry after the engine failed, an
    /// engine's limit — as if at unix time `at` (tests), instead of waiting.
    pub async fn turn_ends_due_at(&self, at: u64) {
        let (tx, rx) = oneshot::channel();
        if self.requests.send(Request::TurnEndsDue(at, tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// Have the daemon set the desktop up by itself: the accessibility engine
    /// in the background, the helper and its one macOS switch once a member
    /// works the desktop ([`desktop::auto`]). The person's daemon asks for it;
    /// a test's runtime doesn't, and so downloads and installs nothing.
    pub fn start_desktop_auto(&self) {
        let _ = self.requests.send(Request::DesktopAuto);
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
    let config_home = config.home.clone();
    // A window of the person's own an earlier run left open ends.
    if config.persist {
        Runtime::recover_windows(&config_home);
    }
    let mac = std::mem::replace(&mut config.mac, Box::new(desktop::ThisMac));
    // Git's merge driver for the design registries runs this program.
    if let Some(binary) = &config.equipe_binary {
        crate::design::registry::use_binary(binary);
    }
    if config.persist && config.nemotron.is_none() {
        crate::daemon_log!("equipe: no Nebius key yet (Settings › AI platforms, or `equipe login nebius`): handed-in work isn't reviewed by Nemotron");
    }
    // The logins: their passwords in the system's secret store (in memory for tests).
    let in_memory = std::env::var_os("EQUIPE_VAULT_IN_MEMORY").is_some();
    let secrets: Box<dyn vault::Secrets> = if in_memory { Box::new(vault::InMemory::default()) } else { vault::system_store() };
    // Kept sign-ins need the secret store for their key: equipe's own home's, or a test's in memory —
    // a scratch home never reaches the person's Keychain for them.
    let kept_sign_ins = sign_ins::Kept::new(in_memory || config.home == equipe_home());
    let vault = vault::Vault::open(&config.home, secrets);
    let logins = vault.logins.clone();
    let (never_offer, can_keep_logins) = (vault.never_offer.clone(), vault.can_keep());
    let telemetry = if config.persist { crate::telemetry::Telemetry::start(&config.home, &config.client_version) } else { crate::telemetry::Telemetry::off() };
    let screens = std::sync::Arc::new(crate::screens::Screens::default());
    let mut runtime = Runtime {
        guards: Default::default(),
        telemetry,
        screens: screens.clone(),
        remote_told: HashMap::new(),
        vault: std::sync::Arc::new(std::sync::Mutex::new(vault)),
        logins,
        never_offer,
        can_keep_logins,
        login_offers: Vec::new(),
        next_login_offer: 1,
        login_offered: std::collections::HashSet::new(),
        login_insights_days: 7,
        signing_in: Default::default(),
        person_sign_in: Default::default(),
        relogins: HashMap::new(),
        relogin_tries: HashMap::new(),
        kept_sign_ins,
        sign_in_joined: HashMap::new(),
        login_waiters: HashMap::new(),
        asked_site: HashMap::new(),
        verify_back: HashMap::new(),
        finished_in_hands: HashSet::new(),
        host_pages: HashMap::new(),
        looked_first: HashMap::new(),
        recent_asks: crate::logins::RecentAsks::default(),
        site_waits: crate::logins::SiteWaits::default(),
        asking_cause: None,
        fill_waits: Default::default(),
        next_fill: 0,
        autopilot: Default::default(),
        secret_asks: HashMap::new(),
        cli_signins: HashMap::new(),
        machine_creds: Default::default(),
        mcp: squad_mcp::McpBook::default(),
        verify: Default::default(),
        secret_env: HashMap::new(),
        key_replacements: HashMap::new(),
        secret_nudge: Default::default(),
        delta_tail: HashMap::new(),
        calls: HashMap::new(),
        call_llm: crate::call::judge::LlmConfig::from_env(),
        calls_ended_at: HashMap::new(),
        desktop: desktop::Desktop::new(mac).with_home(&config.home),
        desktop_auto: None,
        config,
        // A server that never answers a connection must not hold anything up.
        client: reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap_or_default(),
        subscribers: vec![(None, evt_tx)],
        internal: internal_tx,
        sessions: HashMap::new(),
        dispatch_deferred: std::collections::HashSet::new(),
        journals: HashMap::new(),
        next_join: 1,
        pending: HashMap::new(),
        next_approval: 1,
        focus: HashMap::new(),
        said_about: None,
        previews: crate::live_preview::Watched::default(),
        design_sessions: HashMap::new(),
        on_phone: false,
        builds_trimmed: None,
        compactions: compact::CompactTimes::default(),
        browsers: HashMap::new(),
        host: None,
        hub: browser::Hub::new(),
        host_events: host_tx,
        tabs: HashMap::new(),
        asked_in: HashMap::new(),
        asked_by: HashMap::new(),
        browser_back: HashMap::new(),
        unmarked_asked: HashSet::new(),
        held: HashMap::new(),
        checking: HashMap::new(),
        let_go: std::collections::HashSet::new(),
        watching: HashMap::new(),
        framing: std::collections::HashSet::new(),
        frame_watchers: HashMap::new(),
        frame_sent: HashMap::new(),
        last_url: HashMap::new(),
        show_url: HashMap::new(),
        windows: window::Windows::new(&config_home),
        plan: None,
        plans: Vec::new(),
        plan_types: HashMap::new(),
        codex_read: None,
        codex_reading: false,
        codex_models: None,
        codex_kind: None,
        codex_refused: HashMap::new(),
        model_refused_turns: HashSet::new(),
        engine_pages: HashMap::new(),
        tool_repeats: HashMap::new(),
        qa_budgets: HashMap::new(),
        codex_listing: false,
        providers: HashMap::new(),
        questions: HashMap::new(),
        team_changes: HashMap::new(),
        team_by_policy: HashSet::new(),
        routine_proposals: HashMap::new(),
        design_reviews: HashMap::new(),
        retros: HashMap::new(),
        approvals_asked: HashMap::new(),
        approvals_turn: HashMap::new(),
        closed_seen: std::collections::HashSet::new(),
        lane_tmp_tidied: std::time::Instant::now(),
        tmp_copies_told: HashSet::new(),
        tmp_budget_told: HashSet::new(),
        tidied_at: None,
        signals_running: std::collections::HashSet::new(),
        growth_running: HashSet::new(),
        growth_tried: HashMap::new(),
        growth_measuring: HashSet::new(),
        sprint_gathering: HashSet::new(),
        sprint_busy: HashSet::new(),
        sprint_counted: HashMap::new(),
        deploys_looking: HashSet::new(),
        go_live: HashMap::new(),
        accepts: HashMap::new(),
        accepts_declined: HashMap::new(),
        withdraw_reminded: HashMap::new(),
        merged_cache: Some(worktree::MergedCache::default()),
        settle_again: false,
        settled_at: None,
        merging: std::collections::HashSet::new(),
        merge_retries: HashMap::new(),
        merge_retries_seeded: false,
        accepting: std::collections::HashSet::new(),
        handed_in_files: HashMap::new(),
        handed_in_sql: HashMap::new(),
        closing: HashMap::new(),
        closing_git: HashSet::new(),
        gates: HashMap::new(),
        prechecks: HashMap::new(),
        disk: Default::default(),
        machine: Default::default(),
        plan_use: Default::default(),
        commands: Default::default(),
        words: Default::default(),
        helping: HashMap::new(),
        side_chats: HashMap::new(),
        awake: Default::default(),
        sign_ins: HashMap::new(),
        sign_out_after: std::collections::HashSet::new(),
        next_sign_in: 1,
        sign_in_codes: HashMap::new(),
        auth: HashMap::new(),
        status_cache: Default::default(),
        status_told: HashMap::new(),
        escalation: HashMap::new(),
        waiting_told: HashMap::new(),
        installs: std::collections::HashSet::new(),
        codex_binary: crate::engines::codex_binary(),
        engines: Default::default(),
        pacers: Default::default(),
        turn_ends: Default::default(),
        design_watch: HashMap::new(),
        design_busy: std::collections::HashSet::new(),
        review_nudged: HashMap::new(),
        person_said: HashMap::new(),
        decided_told: HashMap::new(),
        workspaces: Default::default(),
    };
    let mut memos = Vec::new();
    // What waited on the person in words when the daemon stopped: logged as dropped.
    runtime.words_dropped_everywhere();
    if runtime.config.persist {
        runtime.restore();
        runtime.prune();
        memos = runtime.resume_work();
        runtime.restore_stopped();
        // Change requests the PM once left as plain comments: to their members, once.
        runtime.recover_change_requests();
        // Work that still names members who left: to their roles.
        runtime.hand_on_work_of_departed();
        // Chrome's first start, off the loop, before a reply's card waits on it.
        let _ = crate::visuals::card::warm_up(&runtime.config.home);
        // The keys members kept, into the list every transcript is masked by.
        runtime.load_squad_secrets();
    }
    // Outside servers' tokens are read anew by a daemon starting (`crate::mcp_keys`).
    crate::mcp_oauth::forget_known();
    let pulse = watchdog::Pulse::new();
    watchdog::watch(&pulse);
    tokio::spawn(CORE_LOOP.scope((), watchdog::scoped(pulse.clone(), run(runtime, req_rx, internal_rx, host_rx, pulse.clone()))));
    let handle = Handle { requests: req_tx, pulse, screens };
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
    /// Fresh for every dispatch, even when the same member/engine continues.
    execution_session: SessionId,
    execution_generation: u64,
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
    /// Set by `submit_for_review`'s `waive_findings`: why the engineer's
    /// self-check findings aren't real problems; a designer judges it.
    waive: Option<String>,
    /// A light design QA session (`design_review`): the precheck's report
    /// as its only folder, a short charter, few tools.
    lite: bool,
    /// What a designer's last hand-in was refused for: the same again goes
    /// in with it marked, rather than round and round.
    refused: Option<Vec<String>>,
    /// Its workspace is still being made off the loop ([`workspace`]): no
    /// turn runs yet, and what is said to it waits as a memo does.
    preparing: bool,
    /// How its turns end when they don't end the work: paused by equipe,
    /// reminded how to finish, handed back ([`turn_end`]).
    end: RunEnd,
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
    /// The model it is set to, as its engine is asked for it: an alias
    /// (`opus`) or an id.
    model: Option<String>,
    /// The model its engine named when it last started a turn, if it did.
    ran: Option<String>,
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
    /// A first-exchange title side turn is already in flight (or finished).
    title_started: bool,
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
    /// Stops one subagent of the turn running now, by its thread, through
    /// its engine (Codex's `turn/interrupt` on the subagent's own turn).
    stop_subagent: Option<mpsc::UnboundedSender<String>>,
    /// Told to the turn running now (Claude Code's) once its ticket's work is
    /// over — handed in, waiting, handed back: a turn whose answer is in then
    /// isn't kept for commands it left running in the background.
    work_over: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Ends the turn running now (Claude Code's) as far as it went, through
    /// its provider, its conversation kept: for a turn that went silent.
    wind_down: Option<oneshot::Sender<()>>,
    /// For each of those, the project the person was looking at (`None`: the whole squad).
    steer_threads: Vec<Option<String>>,
    /// Steers taken back after they went into the turn: not shown when the
    /// engine echoes them, as it reads them with the word to disregard them.
    taken_back: Vec<String>,
    /// The words a message sent at once ([`Command::SendNow`]) goes to the
    /// engine as — each part with its project — in place of the message.
    send_now_prompt: Option<String>,
    /// What its conversation has been told of the blocks in front of a
    /// message, to say only what changed after ([`turn_context`]).
    told: turn_context::Told,
    /// The subagents it handed work to this turn and that haven't handed
    /// back, for a screen that attaches now.
    subagents: Vec<SubRun>,
    /// Its conversation being compacted now: as the screens were told, the
    /// key its time is kept under, and since when.
    compacting: Option<(equipe_protocol::CompactingInfo, String, std::time::Instant)>,
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
    /// For a ticket run: the member's own session. A run works one ticket
    /// (its `work`) in a session of its own — its own engine conversation,
    /// turn, steering and stop — beside the member's conversation and its
    /// other runs; the team lists the member once. Never a lane, never a
    /// member of its own. See [`crate::lanes`].
    run_of: Option<SessionId>,
    /// A run's place among its member's runs at once: where it builds, and
    /// which browser it drives. 0 — the member's own — for its first.
    slot: usize,
    /// When it expects its work done, and what it ships live, as it said.
    estimate: Option<Estimate>,
    /// When its turn running now began (unix seconds).
    turn_began: Option<u64>,
    /// Its own mode; `None` follows the squad's.
    mode: Option<Mode>,
    /// The turn running answers the person (not the board, not the team), so
    /// a guess at what they say next may follow it.
    answers_person: bool,
    /// The person last spoke to it from their phone: what it answers is read
    /// there, and its messages say so ([`crate::roles::PHONE_MARK`]).
    on_phone: bool,
    /// How hard it thinks (`--effort` for Claude Code, reasoning effort for
    /// Codex); `None`: the engine's default.
    effort: Option<String>,
    /// What its turn running now does and says, for how it ends ([`turn_end`]).
    watch: TurnWatch,
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

    /// A run whose workspace is still being made: it holds its ticket and
    /// its lease, and takes its first turn once the workspace is ready.
    fn is_preparing(&self) -> bool {
        self.work.as_ref().is_some_and(|w| w.preparing)
    }

    /// The member itself: neither a lane of it nor one of its ticket runs.
    fn is_member_itself(&self) -> bool {
        self.lane.is_none() && self.run_of.is_none()
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

    /// Whether it changes a project's files on a code ticket it works, in that
    /// ticket's worktree: as it edits — and the PM, on a code ticket of its own.
    fn edits_on_code_tickets(&self) -> bool {
        self.member.as_ref().is_some_and(|m| roles::edits_on_code_tickets(&m.role, self.edits()))
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

struct ConversationTitleDone {
    session: SessionId,
    squad: String,
    title: String,
}

/// How a merge went off the loop: main's name and, when a push was asked
/// for, how that went — or why nothing was merged.
type MergeResult = Result<(String, Option<Result<(), String>>), String>;

enum Internal {
    Event(Box<Event>),
    /// A member's desk turn's changes to the squad's folder, saved off the loop.
    DeskSaved { session: SessionId, name: String, saved: Result<Option<squad_home::Change>, String> },
    /// A turn's engine started, as this process.
    EngineStarted { session: SessionId, pid: u32 },
    /// A turn whose answer was in but whose engine didn't end it was ended
    /// by its provider, as completed: why ([`Activity::Settled`]).
    TurnSettled { session: SessionId, why: String },
    /// Every process, as a look at members' commands found them (`None`: `ps` couldn't say).
    Processes(Option<Vec<crate::command_watch::Proc>>),
    Done(TurnDone),
    /// A tool-less side turn named a conversation (or its local fallback did).
    ConversationTitle(ConversationTitleDone),
    /// Nobody answered a request for permission in time.
    ApprovalTimedOut(u64),
    /// The person answered a request its member didn't wait for: what they said goes to it.
    LaterAnswered { squad: String, approval: Box<Approval>, said: String },
    /// A member's own request has waited its few minutes: the member is let go.
    ReleaseAsk(u64),
    /// A decision in words: its undo or cancel window, or its read-back's wait, is over.
    WordsDue(u64),
    /// What the person said yes to on a card, done as their screen would ask.
    Apply(Box<Command>),
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
    Merged { squad: String, ticket: TicketId, result: MergeResult, kept: Option<crate::design::captures::Report>, push: bool },
    /// A ticket put down whose work went into the squad's folder off the loop.
    HomePutDown(Box<HomePutDown>),
    /// A change to a squad's folder taken back off the loop, or why not.
    HomeUndone { squad: String, session: SessionId, result: Result<(squad_home::Change, squad_home::Change), String> },
    /// A design decision kept in a squad's record off the loop.
    DecisionKept(String),
    /// A note of the person's taste saved off the loop, or why not.
    TasteSaved { squad: String, project: Option<String>, result: Result<(), crate::taste::store::Refused> },
    /// A fact remembered or forgotten off the loop: the notice its member's conversation shows.
    Remembered { session: SessionId, undo: String, text: String },
    /// A ticket closed by the person: its git work, done off the loop.
    ClosedGit { squad: String, ticket: TicketId, status: TicketStatus, closed: Option<Closed> },
    /// A project's checks ran on accepted work merged with main, off the loop.
    Checked(Box<gate::Gate>, crate::checks::Outcome),
    /// The git side of the PM's `accept_work`, worked out off the loop.
    Accepting(Box<accept::AcceptWorked>),
    /// Work ready that needs no person: its member lands it (`runtime::flow`).
    SelfLand { squad: String, ticket: TicketId },
    /// The free space on equipe's disk, as a look begun then found it.
    Disk(Option<crate::disk::DiskStatus>, std::time::Instant),
    /// A pass over the engines' transcripts ended: the burns going on now.
    PlanUsePassed(Vec<equipe_protocol::plan_use::Burn>, Vec<(Engine, crate::plan_use::Load)>),
    /// What uses the disk, as a look begun then found it.
    DiskReport(Box<crate::disk::DiskReport>, std::time::Instant),
    /// What a clean gave back, the free space after it, and who asked.
    DiskCleaned(Box<crate::disk::DiskCleaned>, Option<crate::disk::DiskStatus>, std::time::Instant, Option<oneshot::Sender<crate::disk::DiskCleaned>>),
    /// What a trim of the build caches gave back when the disk ran short, one line each.
    DiskTrimmed(u64, Vec<String>),
    /// What a look at the lanes' temporary folders found: copies of a project, folders past their budget.
    LaneTmpLooked(Box<crate::disk::LaneTmpLook>),
    /// A project the person added, or why it wasn't.
    ProjectAdded { squad: String, result: Result<PathBuf, String> },
    /// A squad deleted off the loop, or why it wasn't.
    SquadDeleted { squad: String, result: Result<crate::squads::Deleted, String> },
    /// The logins changed (or couldn't): what screens see of them now.
    Logins(Result<Vec<crate::protocol::vault::Login>, String>),
    /// What the Logins page shows of the sign-in record, read off the loop.
    LoginInsights(Box<crate::protocol::vault::LoginInsights>),
    /// equipe's own sign-in in a member's browser (by lane) ended: what the fill did.
    Relogin { lane: String, outcome: Result<vault::Filled, String> },
    /// Login autopilot's work off the loop came back ([`autopilot::Note`]).
    Autopilot(autopilot::Note),
    /// A command-line sign-in's process, off the loop, has news ([`cli_signin::Note`]).
    CliSignIn(cli_signin::Note),
    /// Where a squad's outside MCP server stands, as looked at off the loop ([`squad_mcp::Checked`]).
    McpChecked(squad_mcp::Checked),
    /// What a proxy's credential, renewed off the loop, showed of its server.
    McpSeen(equipe_protocol::rpc::McpProxyOpen, crate::mcp_gateway::Seen),
    /// A verification card's fill, or connecting Gmail, came back ([`verify::Note`]).
    Verify(verify::Note),
    /// A row of the sign-in record, from work done off the loop.
    LoginRecord { squad: String, entry: crate::logins::Entry },
    /// A squad's kept sign-ins as read or just kept off the loop
    /// (`crate::signin_store`); `None`: they couldn't be read.
    /// `turn`: the order the read or change was begun in (a later one wins).
    /// `squad`: the store's name (`signin_store::store_of`: `""` the shared one).
    SignIns { squad: String, record: Option<Box<crate::signin_store::Record>>, turn: u64 },
    /// Something waits for a conversation to act on — work handed in for the
    /// PM to accept: it is woken with `prompt` (read at its next step, at work).
    Wake { session: SessionId, prompt: String },
    /// A squad's projects' git facts, read off the loop for its self-watch (`guards`).
    RepoFacts { squad: String, facts: Vec<crate::health::RepoFacts> },
    /// Hand ready work out in every squad again (the disk has room once more).
    Dispatch,
    /// A `show_references` call's pictures, fetched and kept off the loop ([`refcards::Shown`]).
    References(Box<refcards::Shown>),
    /// A task on a folder's worktrees ended off the loop: a run's workspace
    /// made (or why not), or a worktree made too late removed (`None`).
    Workspace { folder: PathBuf, prepared: Option<Box<workspace::Prepared>> },
    /// A turn failed for want of `engine`'s sign-in: kept to carry on.
    SignInLost { engine: Engine, session: SessionId, prompt: String },
    /// A run paused by equipe may go on now ([`turn_end`]).
    TurnEndsDue,
    /// Why `engine` can't serve members here, as just looked at.
    AuthChecked(Engine, equipe_protocol::AuthState, String),
    /// Time to look at `engine` again while its turns wait.
    AuthRecheck(Engine),
    /// A signal's command ran off the loop: its reading.
    SignalRead { squad: String, key: String, reading: crate::signals::Reading },
    /// A growth run's numbers, gathered off the loop (`growth`).
    GrowthGathered { squad: String, gathered: Box<growth::Gathered> },
    /// A growth card's follow-up, measured off the loop.
    GrowthMeasured { squad: String, card: u32, after_days: u32, outcome: Box<equipe_protocol::growth::Outcome> },
    /// A sprint's review or planning numbers, gathered off the loop (`sprint`).
    SprintGathered { squad: String, gathered: Box<sprint::Gathered> },
    /// A hypothesis judged off the loop.
    SprintJudged { squad: String, experiment: u32, judged: Box<equipe_protocol::sprint::Judged> },
    /// Whether a live hypothesis' events arrive, looked at off the loop.
    SprintChecked { squad: String, experiment: u32, arriving: Result<bool, String> },
    /// A squad's projects were looked at off the loop for what went live:
    /// each project's watch now, and what the look found.
    DeploysLooked { squad: String, looks: Vec<(PathBuf, crate::deploys::Watch, crate::deploys::Looked)>, declared: Vec<(String, bool)> },
    /// What a project goes live with, read off the loop after a landing
    /// pushed main (`go_live`); whether GitHub confirmed a deploy channel.
    GoLiveRead { squad: String, project: PathBuf, found: crate::go_live::Found, channel_known: bool },
    /// A project's landings may be due to go live now (`go_live`).
    GoLiveDue { squad: String, project: PathBuf },
    /// One go at deploying a project ended, off the loop (`go_live`).
    GoLiveRan { squad: String, project: PathBuf, tickets: Vec<TicketId>, outcome: crate::go_live::Outcome },
    /// Where the engines stand, as just looked at off the loop.
    Engines(Vec<equipe_protocol::EngineStatus>),
    /// A sign-in (the one numbered so) ended.
    SignInEnded(Engine, u64),
    /// An engine's own sign-in handed back its page, to open in equipe's
    /// own browser on this machine (where its last step comes back).
    EnginePage(Engine, String),
    /// The Codex account refused `model` for a turn of `session`'s: asked
    /// again on `instead`, or — none — the turn failed.
    CodexModelRefused { session: SessionId, model: String, instead: Option<String> },
    /// Installing an engine's tool ended.
    InstallEnded(Engine),
    /// The Codex CLI's models, as just listed off the loop for the screens;
    /// `None`: they couldn't be.
    CodexModels(Option<Vec<codex_app::ModelInfo>>),
    /// A model provider's models, as just listed off the loop, or why not.
    ProviderListed { id: String, result: Result<Vec<String>, crate::nemotron::ReviewError> },
    /// Whether Claude is signed in, as just looked at off the loop.
    ClaudeReady(bool),
    /// The Claude plan's usage, as just read off the loop; `None`: it couldn't be.
    ClaudePlan(Option<equipe_protocol::PlanUsage>),
    /// A squad's design systems looked at off the loop: their stamp, and —
    /// when they changed or were asked for — what they are.
    Design(String, u64, Option<design::Catalog>),
    /// A squad's briefs and plans, read off the loop.
    Docs(String, crate::briefs::Catalog),
    /// What a search of a squad's briefs and plans found, off the loop: the
    /// squad, the words, the paths.
    DocsFound(String, String, Vec<String>),
    /// One copy of a brief or plan kept off the loop, the others removed from
    /// the squad's folder: the squad, the session told, and how it went.
    DocCopyKept(String, SessionId, Result<crate::briefs::Kept, String>),
    /// A plan written off the loop (`write_plan`): what follows, and the answer.
    PlanWritten(Box<plans::Written>),
    /// A plan read off the loop for a ticket cut from it (`create_ticket` with `plan`).
    PlanCut(Box<plans::Cut>),
    /// A ticket listed in its plan, off the loop: why not, if it wasn't.
    PlanListed(String, Option<String>),
    /// A design system's accepted versions, read off the loop.
    DesignVersions { squad: String, project: Option<String>, versions: Vec<equipe_protocol::design::DesignVersion> },
    /// A design folder's references read off the loop: the event to send.
    DesignRefs(Box<Event>),
    /// The version the person asked to go back to, looked up off the loop (`None`: not one).
    DesignRestore { squad: String, project: Option<String>, commit: String, version: Option<equipe_protocol::design::DesignVersion> },
    /// A design QA precheck ended off the loop: its report is in the
    /// ticket's design QA folder, or why there is none.
    /// `loaded`: the machine was overloaded at some time while it ran.
    Prechecked { squad: String, qa: TicketId, result: Result<(), String>, secs: u64, loaded: bool },
    /// Main's precheck for a daily design review ended off the loop: main's
    /// commit per project, or why there is none.
    ReviewPrechecked { squad: String, review: TicketId, heads: Vec<(PathBuf, Result<String, String>)>, secs: u64 },
    /// Nemotron's review of a code ticket handed in by `member`, or why there is none.
    Reviewed { squad: String, ticket: TicketId, member: SessionId, result: Result<Box<crate::nemotron::Reviewed>, crate::nemotron::ReviewError> },
    /// A subagent of a turn's engine was heard from on its own thread.
    SubagentHeard { session: SessionId, id: String },
    /// A member's helper did something ([`crate::helpers`]).
    Helper { session: SessionId, id: String, activity: Activity },
    /// A member's helper's turn ended: what it wrote, or why it failed.
    HelperEnded { session: SessionId, id: String, outcome: Result<equipe_providers::turn::TurnResult, String> },
    /// A helper's time is up.
    HelperTimeUp { session: SessionId, id: String },
    /// A `helper_result` has waited as long as it does.
    HelperWaitOver(u64),
    /// Something happened in a side chat ([`crate::side_chat`]).
    SideChat { side: equipe_protocol::SideChatId, event: Box<Event> },
    /// A side chat's turn ended.
    SideDone(side_chat::SideDone),
    /// The judge read an utterance of call `call` (`call.rs`).
    CallJudged { call: String, judged: Box<crate::call::judge::Judged> },
    /// Words about the board for a call (`call::status`).
    CallTold { call: String, text: String },
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
    /// It called `hand_back` (or equipe did, its turns ending with no end to
    /// the work): the ticket waits on others, or goes to the PM; its work so
    /// far stays on its branch and worktree.
    HandedBack,
}

struct Runtime {
    config: Config,
    /// The waste stops and the squad's self-watch (`guards`).
    guards: guards::Guards,
    client: reqwest::Client,
    /// Anonymous usage events, after the person's yes (docs/TELEMETRY.md):
    /// the daemon's only, never a terminal's own runtime.
    telemetry: crate::telemetry::Telemetry,
    /// The screens following each squad, and where they are: the person's
    /// screen on this Mac or another ([`crate::screens`]).
    screens: std::sync::Arc<crate::screens::Screens>,
    /// Where each member's conversation was last told the person looks from
    /// (`true`: another machine), so it hears again only when that changes.
    remote_told: HashMap<SessionId, bool>,
    /// Every attached screen, with the squad it follows (`None`: everything).
    /// Closed ones drop out on the next event.
    subscribers: Vec<(Option<String>, UnboundedSender<Event>)>,
    internal: UnboundedSender<Internal>,
    sessions: HashMap<SessionId, Session>,
    /// Starts made as one approved team change wait for its preferred-ticket
    /// pass instead of taking ordinary work one member at a time.
    dispatch_deferred: std::collections::HashSet<SessionId>,
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
    /// The pages of members' the person's screens have open in a live preview.
    previews: crate::live_preview::Watched,
    /// The design sessions on, by the designer's conversation
    /// (`crate::design::session::Active`): their turns open with the
    /// prototype round's rules, and the person's words cut into them.
    design_sessions: HashMap<SessionId, crate::design::session::Active>,
    /// While a command the person sent from their phone is handled ([`Command::OnPhone`]).
    on_phone: bool,
    /// When members' build caches were last looked at.
    builds_trimmed: Option<std::time::Instant>,
    /// How long members' conversations took to compact, for the next one's guess.
    compactions: compact::CompactTimes,
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
    /// Which conversation handed each lane to the person (`show_my_browser`):
    /// a ticket's run hands its member's browser, and it is the run that
    /// carries on when the person is done.
    asked_by: HashMap<String, SessionId>,
    /// When the person last handed a member's browser back (squad and member
    /// to unix seconds), until its member's first decision asked after: the
    /// member looks at the page itself rather than asking what they saw
    /// ([`Runtime::ask_decision`]).
    browser_back: HashMap<(String, String), u64>,
    /// Choices a member asked without a recommended option and was sent back
    /// once to mark one (squad, member, first question): asked again as they
    /// are, they go to the person.
    unmarked_asked: HashSet<(String, String, String)>,
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
    /// Members' browsers handed to the person in a Chrome window of their own.
    windows: window::Windows,
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
    codex_models: Option<(std::time::Instant, Vec<codex_app::ModelInfo>)>,
    /// How the Codex CLI is signed in, as last asked: a refused model is the
    /// account kind's (a ChatGPT plan's, an API key's).
    codex_kind: Option<codex_app::SignIn>,
    /// Models the Codex account refused lately (`model_not_found`), with when
    /// and on which kind of account: skipped for [`REFUSED_FOR`], then tried
    /// again — a refusal may pass.
    codex_refused: HashMap<String, (u64, Option<codex_app::SignIn>)>,
    /// Runs whose last turn failed on a refused model, with nothing to fall
    /// back on: never an engine's limit.
    model_refused_turns: HashSet<SessionId>,
    /// Each engine's sign-in page under way, as its tool handed it back.
    engine_pages: HashMap<Engine, String>,
    /// For each run, its latest tool call and how many times in a row it
    /// came: a loop raises its ticket's next lane (`runtime::tiers`).
    tool_repeats: HashMap<SessionId, (String, u32)>,
    /// For each design QA, review or audit turn: its tool calls so far, and
    /// whether it was told its budget ran out (`runtime::design_qa`).
    qa_budgets: HashMap<SessionId, design_qa::Budget>,
    /// A listing for the screens is under way.
    codex_listing: bool,
    /// What each model provider connected here listed, by its id.
    providers: HashMap<String, providers::Listed>,
    /// The Mac's screen, one member at a time, across squads.
    desktop: desktop::Desktop,
    /// The desktop's own setup, once asked for ([`Handle::start_desktop_auto`]).
    desktop_auto: Option<desktop::auto::Auto>,
    /// Members' questions waiting for the person, by squad (`decide`).
    questions: HashMap<String, Vec<equipe_protocol::decide::QuestionSet>>,
    /// Changes to the team the PM proposed, waiting for the person, by squad (`team`).
    team_changes: HashMap<String, Vec<equipe_protocol::team::TeamChange>>,
    /// Changes the squad's own autonomy mode made at once (not asked of the person): told, not approved.
    team_by_policy: HashSet<u64>,
    /// Routines the PM proposed, waiting for the person, by squad (`heartbeat`).
    routine_proposals: HashMap<String, Vec<equipe_protocol::routines::RoutineProposal>>,
    /// Designers handed their weekly review, and when (`review`): the
    /// decision each puts next is the review's proposal.
    design_reviews: HashMap<SessionId, u64>,
    /// PMs handed their weekly retro, and when (`retro`): the decision each
    /// puts next is the retro's changes.
    retros: HashMap<SessionId, u64>,
    /// When each permission waiting for the person was asked, by its id: how
    /// long it waited goes to the retro once answered.
    approvals_asked: HashMap<u64, u64>,
    /// The turn each request waiting for the person was asked in, by its id:
    /// the session and when that turn began. An answer that comes after that
    /// turn ended reaches nobody, and goes to the member as a turn of its own
    /// ([`Runtime::late_reply`]).
    approvals_turn: HashMap<u64, (SessionId, u64)>,
    /// Tickets seen closed (done or cancelled) since the daemon started, by
    /// squad: a ticket closing is noticed once, however it closed.
    closed_seen: std::collections::HashSet<(String, TicketId)>,
    /// When lanes' temporary folders were last tidied.
    lane_tmp_tidied: std::time::Instant,
    /// Copies of a project in lanes' temporary folders already told to their
    /// members, and tickets whose folder was told past its budget: once each a daemon run.
    tmp_copies_told: HashSet<PathBuf>,
    tmp_budget_told: HashSet<(String, TicketId)>,
    /// When conversations were last tidied ([`tidy`]); `None`: not yet.
    tidied_at: Option<std::time::Instant>,
    /// Signals whose command runs now, by squad and key (`signals`).
    signals_running: std::collections::HashSet<(String, String)>,
    /// Squads whose growth run gathers now, when each weekly run was last
    /// tried, and the follow-ups being measured (`growth`).
    growth_running: HashSet<String>,
    growth_tried: HashMap<String, u64>,
    growth_measuring: HashSet<(String, u32, u32)>,
    /// Squads whose sprint review or planning gathers now, the hypotheses
    /// being checked or judged, and when each squad's sprint spend was last
    /// counted (`sprint`).
    sprint_gathering: HashSet<String>,
    sprint_busy: HashSet<(String, u32)>,
    sprint_counted: HashMap<String, u64>,
    /// Squads whose projects are being looked at for what went live (`deploys`).
    deploys_looking: HashSet<String>,
    /// How each squad's projects go live after their landings, by project
    /// path (`go_live`): read from the journal when first needed.
    go_live: HashMap<String, std::collections::BTreeMap<String, crate::go_live::Lane>>,
    /// Decisions in the person's words waiting on them (`words`).
    words: words::Words,
    /// Handed-in work put to the person to confirm, by squad (`accept`).
    accepts: HashMap<String, Vec<equipe_protocol::accept::AcceptProposal>>,
    /// Cards the person said no to lately, by squad: not put up again soon.
    accepts_declined: HashMap<String, Vec<crate::accept::Declined>>,
    /// The cards each squad's PM was reminded of after saying they were withdrawn while still up: once each (`withdraw`).
    withdraw_reminded: HashMap<String, HashSet<(crate::withdraw::Kind, u64)>>,
    /// What the look at done work off main remembers; `None` while a look is
    /// under way off the loop, and `settle_again` when another was asked meanwhile.
    merged_cache: Option<worktree::MergedCache>,
    settle_again: bool,
    /// When the last sweep began.
    settled_at: Option<std::time::Instant>,
    /// Merges under way off the loop, by squad and ticket.
    merging: std::collections::HashSet<(String, TicketId)>,
    /// Accepted work whose merge failed for another reason than a conflict,
    /// by squad and ticket: tried again on its own ([`Runtime::retry_merges`]).
    merge_retries: HashMap<(String, TicketId), MergeRetry>,
    /// Whether failed merges a run before this one left were looked for.
    merge_retries_seeded: bool,
    /// Tickets the PM's `accept_work` is working out off the loop, by squad
    /// and ticket: asked again meanwhile, they are said to be under way.
    accepting: std::collections::HashSet<(String, TicketId)>,
    /// The files each ticket's branch changed when it was last handed in, by
    /// squad and ticket: what decides whether its member may land it (`runtime::flow`).
    handed_in_files: HashMap<(String, TicketId), Vec<String>>,
    /// The contents of the SQL migrations each handed-in branch changed, by
    /// path, read from its worktree at hand-in (`crate::person_gate::read_sql`).
    handed_in_sql: HashMap<(String, TicketId), std::collections::BTreeMap<String, String>>,
    /// A ticket's closing git work, done off the loop already, by squad and
    /// ticket: [`Runtime::close_ticket`] takes it instead of doing it again.
    closing: HashMap<(String, TicketId), Closed>,
    /// Tickets the person closed whose git work runs off the loop now ([`Runtime::close_ticket`]).
    closing_git: HashSet<(String, TicketId)>,
    /// Accepted work whose project's checks run on its merge before it lands,
    /// by repository: the first of each runs off the loop, the rest wait
    /// their turn (`gate.rs`).
    gates: HashMap<PathBuf, std::collections::VecDeque<gate::Gate>>,
    /// Design QA tickets held off the board while their precheck runs, by
    /// squad and ticket, with when it started (`design_qa.rs`).
    prechecks: HashMap<(String, TicketId), std::time::Instant>,
    /// The disk's room, as last looked at or as a write said (`room.rs`).
    disk: room::DiskWatch,
    /// How busy the machine is, and whether new work waits for it (`machine.rs`).
    machine: machine::MachineWatch,
    /// Where the plans went, as followed (`plan_use.rs`).
    plan_use: plan_use::PlanUseWatch,
    /// Members' commands under way, as watched running (`watch.rs`).
    commands: watch::Commands,
    /// Members' helpers, by the conversation they work for (`helping.rs`).
    helping: HashMap<SessionId, helping::Helping>,
    /// Side chats the person opened, by their id (`side_chat.rs`).
    side_chats: HashMap<equipe_protocol::SideChatId, side_chat::SideChat>,
    /// The Mac held awake while members work or a screen is attached.
    awake: crate::awake::Awake,
    /// Sign-ins under way on this machine, by engine: which one, and its task.
    sign_ins: HashMap<Engine, (u64, JoinHandle<()>)>,
    /// Engines the person asked to sign out once the members mid-turn on
    /// them finish that turn ([`Command::SignOutAfterTurns`]).
    sign_out_after: std::collections::HashSet<Engine>,
    next_sign_in: u64,
    /// The code each sign-in under way waits for (Claude Code's, pasted on a screen).
    sign_in_codes: HashMap<Engine, mpsc::UnboundedSender<String>>,
    /// Engines whose lost sign-in stopped turns here: why, as last looked
    /// at, and the turns to carry on once it is signed in again.
    auth: HashMap<Engine, AuthFix>,
    /// Engines whose tool is being installed on this machine.
    installs: std::collections::HashSet<Engine>,
    /// The `codex` the person's Codex sign-in runs.
    codex_binary: String,
    /// Squads' statuses as last worked out, for [`crate::status::CACHE_FOR`]
    /// (`runtime/status.rs`).
    status_cache: status::Cache,
    /// When each squad's screens were last told its status changed.
    status_told: HashMap<String, std::time::Instant>,
    /// Each squad's escalation of what stays a bottleneck ([`crate::escalation`]).
    escalation: HashMap<String, crate::escalation::Planner>,
    /// What the person and the PM were told waits on the person, per squad (`runtime/status.rs`).
    waiting_told: HashMap<String, status::WaitingTold>,
    /// Both engines' plans, and members moving between them (`engines`).
    engines: engines::Engines,
    /// Pacing to the plans' budgets, per squad and engine ([`pacing`]).
    pacers: pace::Pacers,
    /// Failing engines, per ticket and per squad ([`turn_end`]).
    turn_ends: TurnEnds,
    /// Squads whose design systems a screen looks at, and their stamp as
    /// last sent; those being looked at off the loop now (`design`).
    design_watch: HashMap<String, u64>,
    /// The logins kept (`vault`): touched off the loop only — the Keychain
    /// may ask the person before it answers.
    vault: std::sync::Arc<std::sync::Mutex<vault::Vault>>,
    /// What the screens are told of them, as last kept.
    logins: Vec<crate::protocol::vault::Login>,
    /// Sites the person said never to offer keeping a login for, as last kept.
    never_offer: Vec<String>,
    /// Whether this machine has a secret store to keep a password in.
    can_keep_logins: bool,
    /// Offers to keep a login the person just signed in with by hand,
    /// waiting for their answer ([`logins`]).
    login_offers: Vec<crate::protocol::vault::LoginOffer>,
    next_login_offer: u64,
    /// Sites offered since the daemon started: once is enough.
    login_offered: std::collections::HashSet<String>,
    /// The look-back the Logins page asked for last, kept to read it again after a change.
    login_insights_days: u32,
    /// Members' pages seen on a sign-in, by page id ([`logins`]).
    signing_in: crate::logins::SignInWatch,
    /// The person's own tabs and the sign-in they started in equipe's own browser, followed until it is done.
    person_sign_in: crate::signin::Tracker,
    /// Members' browsers equipe is signing in by itself now, by lane.
    relogins: HashMap<String, logins::Relogin>,
    /// When equipe last tried signing in with each login by itself.
    relogin_tries: HashMap<u32, Vec<std::time::Instant>>,
    /// Each squad's kept sign-ins, once read ([`crate::signin_store`]).
    kept_sign_ins: sign_ins::Kept,
    /// Members that asked to sign in with a login a card already asks the
    /// person about: they wait on that card (by its id), not one of their own.
    sign_in_joined: HashMap<u64, Vec<Pending>>,
    /// Members waiting on a sign-in the person does in another member's
    /// browser, by that browser's lane: who, and for which site.
    login_waiters: HashMap<String, Vec<(SessionId, String)>>,
    /// The site a browser was handed to the person to sign in to, by lane.
    asked_site: HashMap<String, String>,
    /// Sign-ins back from the person's hands — a window, a sign-in view, the
    /// panel — whose finish isn't seen yet: the lane's page is looked at as
    /// it comes back (`logins::verify_back`). By lane: the site, and since when.
    verify_back: HashMap<String, (String, std::time::Instant)>,
    /// Lanes whose sign-in was seen finished while in the person's hands.
    finished_in_hands: HashSet<String>,
    /// Each lane's pages as the host last told them, with what was read off
    /// them, and its member's tab: for a hand-back whose page is loaded already.
    host_pages: HashMap<String, (Vec<browser::HostPage>, Option<u64>)>,
    /// `(lane, site)` a member was told to look at its own page before handing
    /// it over for a sign-in it may already have: when.
    looked_first: HashMap<(String, String), u64>,
    /// The sign-ins the person was asked for and hasn't answered, so a
    /// member asking again for the same site waits on that one ask.
    recent_asks: crate::logins::RecentAsks,
    /// Sign-ins waiting on the person, or blocked for automation, told to the next member who asks.
    site_waits: crate::logins::SiteWaits,
    /// The cause of the hand-off being asked for now, for what the person is told.
    asking_cause: Option<crate::logins::Cause>,
    /// equipe's fills of a kept login waiting for the browser host's answer, by token.
    fill_waits: logins::FillWaits,
    next_fill: u64,
    /// Login autopilot: codes, keep-alive, command-line credentials ([`autopilot`]).
    autopilot: autopilot::State,
    design_busy: std::collections::HashSet<String>,
    /// Handed-in work the PM was reminded of, by squad: when (`heartbeat`).
    review_nudged: HashMap<String, HashMap<TicketId, u64>>,
    /// What the person last said in each conversation, for the request a
    /// ticket made in answer serves ([`requests`]).
    person_said: HashMap<SessionId, requests::Said>,
    /// The digest each squad's screens last heard ([`person_gate`]).
    decided_told: HashMap<String, Vec<(TicketId, u64)>>,
    /// Runs' workspaces being made off the loop ([`workspace`]).
    workspaces: workspace::Workspaces,
    /// Secrets members asked the person for (`ask_secret`), by squad — read
    /// from its journal when first wanted.
    secret_asks: HashMap<String, Vec<crate::protocol::secrets::SecretAsk>>,
    /// Command-line sign-ins the person is finishing on cards (`cli_sign_in`), by squad.
    cli_signins: HashMap<String, Vec<cli_signin::Live>>,
    /// The machine's own command-line credentials, by name and shape, read
    /// now and then ([`crate::machine_creds`]).
    machine_creds: crate::machine_creds::Cache,
    /// The squads' outside MCP servers: where each stands, checks under way,
    /// and the sign-ins the person put off ([`squad_mcp`]).
    mcp: squad_mcp::McpBook,
    /// Verification cards: codes sites emailed while members' pages wait on them ([`verify`]).
    verify: verify::State,
    /// Kept keys members asked their commands to get as environment
    /// variables (`secret_use`), by session: `(variable, key name)`. The value
    /// is looked up — and the right to it checked again — when a turn starts.
    secret_env: HashMap<SessionId, Vec<(String, String)>>,
    /// `secret_save` calls naming a key already kept, held until the
    /// person's card answers whether the new value replaces it — by the
    /// card's id. In memory only, like the value itself.
    key_replacements: HashMap<u64, squad_keys::Replacement>,
    /// Sessions whose command printed something that looks like a key that
    /// wasn't saved: told to `secret_save` it with their next tool's answer.
    secret_nudge: std::collections::HashSet<SessionId>,
    /// The end of a streaming answer held back while kept values exist, so a
    /// value split across fragments is still cut out whole.
    delta_tail: HashMap<SessionId, String>,
    /// Calls under way, by id (`call.rs`).
    calls: HashMap<String, crate::call::Call>,
    /// The model a call's judge asks.
    call_llm: crate::call::judge::LlmConfig,
    /// When each squad's last call ended (unix ms), for the next one's briefing;
    /// kept in memory only.
    calls_ended_at: HashMap<String, i64>,
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
    WindowTick,
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
            Wake::Request(Some(Request::MemberRefused(_))) => Borrowed("a member's refused command"),
            Wake::Request(Some(Request::Tool(call, _))) => Owned(format!("tool {} for {}/{}", call.name, call.squad, call.member)),
            Wake::Request(Some(Request::Snapshot(..))) => Borrowed("snapshot"),
            Wake::Request(Some(Request::Subscribe(..))) => Borrowed("hello (a screen's snapshot)"),
            Wake::Request(Some(Request::DesktopGate(..))) => Borrowed("desktop gate"),
            Wake::Request(Some(Request::DesktopTool(..))) => Borrowed("desktop tool"),
            Wake::Request(Some(Request::Busy(_))) => Borrowed("busy"),
            Wake::Request(Some(Request::Growth(..))) => Borrowed("a growth request"),
            Wake::Request(Some(Request::Sprint(..))) => Borrowed("a sprint request"),
            Wake::Request(Some(Request::Status(..))) => Borrowed("a squad's status"),
            Wake::Request(Some(Request::PersonWindow(..))) => Borrowed("person's browser window"),
            Wake::Request(Some(Request::SignIn(..))) => Borrowed("person's sign-in"),
            Wake::Request(Some(Request::BrowserHub(_))) => Borrowed("browser panel"),
            Wake::Request(Some(Request::Desktop(_))) => Borrowed("desktop lease"),
            Wake::Request(Some(Request::Phone(_))) => Borrowed("phone tool"),
            Wake::Request(Some(Request::McpUpstream(..))) => Borrowed("an outside MCP server's proxy"),
            Wake::Request(Some(Request::McpSeen(..))) => Borrowed("an outside MCP server's call"),
            Wake::Request(Some(Request::EngineMember(..))) => Borrowed("whose engine a connection runs under"),
            Wake::Request(Some(Request::Usage(..))) => Borrowed("an engine's plan usage"),
            Wake::Request(Some(Request::CheckLeases(..))) => Borrowed("lease check"),
            Wake::Request(Some(Request::LookAtTurns(..))) => Borrowed("turns looked at"),
            Wake::Request(Some(Request::Minute(_))) => Borrowed("minute tick"),
            Wake::Request(Some(Request::TurnEndsDue(..))) => Borrowed("paused runs' time"),
            Wake::Request(Some(Request::Shutdown(_))) | Wake::Request(None) => Borrowed("shutdown"),
            Wake::Request(Some(Request::DesktopAuto)) => Borrowed("desktop setup"),
            Wake::Internal(Internal::Event(_)) => Borrowed("an engine's event"),
            Wake::Internal(Internal::EngineStarted { .. }) => Borrowed("an engine's start"),
            Wake::Internal(Internal::TurnSettled { .. }) => Borrowed("a settled turn ended"),
            Wake::Internal(Internal::Processes(_)) => Borrowed("what members' commands run"),
            Wake::Internal(Internal::Done(_)) => Borrowed("a turn's end"),
            Wake::Internal(Internal::ConversationTitle(_)) => Borrowed("a conversation's title"),
            Wake::Internal(Internal::ApprovalTimedOut(_)) => Borrowed("an approval timing out"),
            Wake::Internal(Internal::LaterAnswered { .. }) => Borrowed("an answer to a request its member didn't wait for"),
            Wake::Internal(Internal::ReleaseAsk(_)) => Borrowed("a member let go from waiting on a request"),
            Wake::Internal(Internal::WordsDue(_)) => Borrowed("a decision in words coming due"),
            Wake::Internal(Internal::Apply(_)) => Borrowed("a change the person said yes to"),
            Wake::Internal(Internal::Ask(..)) => Borrowed("codex asking for approval"),
            Wake::Internal(Internal::CodexLimits(_)) => Borrowed("codex's plan usage"),
            Wake::Internal(Internal::Frame(..)) => Borrowed("a browser picture"),
            Wake::Internal(Internal::Settled { .. }) => Borrowed("settling merged work"),
            Wake::Internal(Internal::Merged { .. }) => Borrowed("a merge's end"),
            Wake::Internal(Internal::HomePutDown(..)) => Borrowed("work landed in the squad's folder"),
            Wake::Internal(Internal::HomeUndone { .. }) => Borrowed("a change taken back"),
            Wake::Internal(Internal::DecisionKept(..)) => Borrowed("a design decision kept"),
            Wake::Internal(Internal::TasteSaved { .. }) => Borrowed("a taste note saved"),
            Wake::Internal(Internal::Remembered { .. }) => Borrowed("a fact remembered"),
            Wake::Internal(Internal::ClosedGit { .. }) => Borrowed("a closed ticket's git work"),
            Wake::Internal(Internal::Checked(..)) => Borrowed("a project's checks ending"),
            Wake::Internal(Internal::Accepting(..)) => Borrowed("the PM's acceptance, worked out"),
            Wake::Internal(Internal::SelfLand { .. }) => Borrowed("a member landing its own work"),
            Wake::Internal(Internal::Disk(..)) => Borrowed("the disk's free space"),
            Wake::Internal(Internal::DiskReport(..)) => Borrowed("what uses the disk"),
            Wake::Internal(Internal::PlanUsePassed(..)) => Borrowed("where the plans went"),
            Wake::Internal(Internal::DiskCleaned(..)) => Borrowed("a clean of the disk"),
            Wake::Internal(Internal::ProjectAdded { .. }) => Borrowed("a project added"),
            Wake::Internal(Internal::SquadDeleted { .. }) => Borrowed("a squad deleted"),
            Wake::Internal(Internal::Logins(_)) => Borrowed("the logins changed"),
            Wake::Internal(Internal::LoginInsights(_)) => Borrowed("the sign-in record read"),
            Wake::Internal(Internal::Relogin { .. }) => Borrowed("equipe's own sign-in ending"),
            Wake::Internal(Internal::Autopilot(_)) => Borrowed("login autopilot"),
            Wake::Internal(Internal::CliSignIn(_)) => Borrowed("a command-line sign-in"),
            Wake::Internal(Internal::McpChecked(_)) => Borrowed("an outside MCP server checked"),
            Wake::Internal(Internal::McpSeen(..)) => Borrowed("an outside MCP server's sign-in"),
            Wake::Internal(Internal::Verify(_)) => Borrowed("a verification code card"),
            Wake::Internal(Internal::LoginRecord { .. }) => Borrowed("a sign-in kept on the record"),
            Wake::Internal(Internal::SignIns { .. }) => Borrowed("a squad's kept sign-ins"),
            Wake::Internal(Internal::DeskSaved { .. }) => Borrowed("a desk turn's changes saved"),
            Wake::Internal(Internal::Wake { .. }) => Borrowed("work waits for a conversation"),
            Wake::Internal(Internal::DiskTrimmed(..)) => Borrowed("a trim of the build caches"),
            Wake::Internal(Internal::LaneTmpLooked(..)) => Borrowed("a look at the lanes' temporary folders"),
            Wake::Internal(Internal::Dispatch) => Borrowed("handing out work again"),
            Wake::Internal(Internal::References(_)) => Borrowed("reference pictures fetched"),
            Wake::Internal(Internal::Workspace { .. }) => Borrowed("a run's workspace made"),
            Wake::Internal(Internal::SignInLost { .. }) => Borrowed("a turn stopped by a lost sign-in"),
            Wake::Internal(Internal::AuthChecked(..)) => Borrowed("why an engine can't sign in"),
            Wake::Internal(Internal::TurnEndsDue) => Borrowed("a paused run's time"),
            Wake::Internal(Internal::AuthRecheck(_)) => Borrowed("an engine's sign-in looked at again"),
            Wake::Internal(Internal::SignalRead { .. }) => Borrowed("a signal's reading"),
            Wake::Internal(Internal::GrowthGathered { .. }) => Borrowed("a growth run's numbers"),
            Wake::Internal(Internal::GrowthMeasured { .. }) => Borrowed("a growth card's follow-up"),
            Wake::Internal(Internal::SprintGathered { .. }) => Borrowed("a sprint's numbers"),
            Wake::Internal(Internal::SprintJudged { .. }) => Borrowed("a hypothesis judged"),
            Wake::Internal(Internal::SprintChecked { .. }) => Borrowed("a hypothesis' events looked for"),
            Wake::Internal(Internal::DeploysLooked { .. }) => Borrowed("a look at what went live"),
            Wake::Internal(Internal::GoLiveRead { .. }) => Borrowed("how a project goes live"),
            Wake::Internal(Internal::GoLiveDue { .. }) => Borrowed("a project's deploy coming due"),
            Wake::Internal(Internal::GoLiveRan { .. }) => Borrowed("a project's deploy's end"),
            Wake::Request(Some(Request::Disk(_))) => Borrowed("a look at the disk"),
            Wake::Request(Some(Request::DiskClean(..))) => Borrowed("a clean of the disk"),
            Wake::Internal(Internal::Engines(_)) => Borrowed("where the engines stand"),
            Wake::Internal(Internal::SignInEnded(..)) => Borrowed("a sign-in's end"),
            Wake::Internal(Internal::EnginePage(..)) => Borrowed("an engine's sign-in page"),
            Wake::Internal(Internal::CodexModelRefused { .. }) => Borrowed("a Codex model refused"),
            Wake::Internal(Internal::InstallEnded(..)) => Borrowed("an engine's install's end"),
            Wake::Internal(Internal::CodexModels(_)) => Borrowed("the Codex models listed"),
            Wake::Internal(Internal::ProviderListed { .. }) => Borrowed("a provider's models listed"),
            Wake::Internal(Internal::ClaudeReady(_)) => Borrowed("whether Claude is signed in"),
            Wake::Internal(Internal::ClaudePlan(_)) => Borrowed("claude's plan usage"),
            Wake::Internal(Internal::Design(..)) => Borrowed("a design system's look"),
            Wake::Internal(Internal::Docs(..)) => Borrowed("the briefs and plans"),
            Wake::Internal(Internal::DocsFound(..)) => Borrowed("a search of the briefs and plans"),
            Wake::Internal(Internal::DocCopyKept(..)) => Borrowed("one copy of a brief or plan kept"),
            Wake::Internal(Internal::PlanWritten(..)) => Borrowed("a plan written"),
            Wake::Internal(Internal::PlanCut(..)) => Borrowed("a plan read for a ticket"),
            Wake::Internal(Internal::PlanListed(..)) => Borrowed("a ticket listed in its plan"),
            Wake::Internal(Internal::DesignVersions { .. }) => Borrowed("a design system's versions"),
            Wake::Internal(Internal::DesignRefs(..)) => Borrowed("a design system's references"),
            Wake::Internal(Internal::DesignRestore { .. }) => Borrowed("a design version to go back to"),
            Wake::Internal(Internal::Reviewed { .. }) => Borrowed("a Nemotron review"),
            Wake::Internal(Internal::Prechecked { .. }) => Borrowed("a design QA precheck"),
            Wake::Internal(Internal::ReviewPrechecked { .. }) => Borrowed("a daily design review's precheck of main"),
            Wake::Internal(Internal::SubagentHeard { .. }) => Borrowed("a subagent heard from"),
            Wake::Internal(Internal::Helper { .. }) => Borrowed("a helper's step"),
            Wake::Internal(Internal::HelperEnded { .. }) => Borrowed("a helper's end"),
            Wake::Internal(Internal::HelperTimeUp { .. }) => Borrowed("a helper's time up"),
            Wake::Internal(Internal::HelperWaitOver(_)) => Borrowed("a wait for helpers over"),
            Wake::Internal(Internal::SideChat { .. }) => Borrowed("a side chat's step"),
            Wake::Internal(Internal::SideDone(_)) => Borrowed("a side chat's turn ended"),
            Wake::Internal(Internal::CallJudged { .. }) => Borrowed("a call's utterance judged"),
            Wake::Internal(Internal::CallTold { .. }) => Borrowed("a call's words about the board"),
            Wake::Internal(Internal::RepoFacts { .. }) => Borrowed("a squad's projects looked at for its self-watch"),
            Wake::Host(_) => Borrowed("the browser host's news"),
            Wake::Daily => Borrowed("daily prune"),
            Wake::DesktopTick => Borrowed("desktop tick"),
            Wake::Frames => Borrowed("browser pictures"),
            Wake::Minute => Borrowed("minute tick"),
            Wake::DesignTick => Borrowed("design tick"),
            Wake::WindowTick => Borrowed("sign-in window tick"),
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
    // A sign-in window the person is in: watched for it closing, and for the
    // browser's folder coming free.
    let mut window_tick = tokio::time::interval(Duration::from_secs(1));
    // A loop held up never catches up on the ticks it missed in a burst: that
    // only holds it up again.
    for tick in [&mut daily, &mut minute, &mut desktop_tick, &mut frames, &mut design_tick, &mut window_tick] {
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
            _ = window_tick.tick(), if rt.window_busy() => Wake::WindowTick,
        };
        pulse.begin(wake.doing());
        let go_on = rt.wake(wake, &pulse).await;
        pulse.end();
        if !go_on {
            pulse.stop_healing();
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
                Some(Request::MemberRefused(squad)) => self.member_refused(squad.as_deref()),
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
                    // And the ones in a Chrome window of the person's own.
                    for lane in self.window_lanes(&squad) {
                        let _ = tx.send(Event::BrowserWindow { squad: Some(squad.clone()), lane, open: true, view: None, challenged: false });
                    }
                    for ask in self.view_asks(&squad) {
                        let _ = tx.send(ask);
                    }
                    // A disk found full is told to each screen as it comes.
                    if self.disk.status.level == crate::disk::DiskLevel::Full {
                        let _ = tx.send(Event::Error { session: None, message: say!(self, "error.disk_full"), next: None });
                    }
                    // Why an engine can't sign in, as last looked at.
                    for (engine, fix) in &self.auth {
                        let _ = tx.send(fix.event(*engine));
                    }
                    // A plan burning fast, lately.
                    for burn in self.recent_burns() {
                        let _ = tx.send(Event::PlanBurn { burn });
                    }
                    self.subscribers.push((Some(squad.clone()), tx));
                    self.keep_awake();
                    let _ = reply.send((self.snapshot(&squad), rx));
                    // A screen that shows the plans' usage gets Codex's fresh, soon after.
                    self.read_codex_usage(true);
                }
                Some(Request::Tool(call, reply)) => self.tool(call, reply).await,
                Some(Request::Growth(method, params, reply)) => {
                    let _ = reply.send(self.growth_request(&method, params));
                }
                Some(Request::Sprint(method, params, reply)) => {
                    let _ = reply.send(self.sprint_request(&method, params));
                }
                Some(Request::PersonWindow(url, reply)) => {
                    let _ = reply.send(self.start_sign_in(&url, true).opened);
                }
                Some(Request::SignIn(url, reply)) => {
                    let _ = reply.send(self.start_sign_in(&url, false));
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
                Some(Request::Phone(request)) => self.on_phone(request),
                Some(Request::McpUpstream(open, rejected, reply)) => self.mcp_upstream(open, rejected, reply),
                Some(Request::McpSeen(open, seen)) => self.mcp_seen(open, seen),
                Some(Request::EngineMember(pid, reply)) => {
                    let _ = reply.send(self.engine_member(pid));
                }
                Some(Request::Disk(reply)) => self.disk_report(Some(reply), false),
                Some(Request::Status(ask, reply)) => self.squad_status(ask, reply),
                Some(Request::DiskClean(ids, reply)) => self.disk_clean(ids, Some(reply)),
                Some(Request::Usage(engine, usage)) => {
                    self.publish(Event::PlanUsage { usage, engine });
                    for squad in self.squads_with_members() {
                        self.apply_engine_moves(&squad).await;
                    }
                }
                Some(Request::Minute(reply)) => {
                    self.minute(pulse).await;
                    let _ = reply.send(());
                }
                Some(Request::CheckLeases(at, reply)) => {
                    self.renew_execution_leases();
                    self.recover_execution_leases(at).await;
                    let _ = reply.send(());
                }
                Some(Request::LookAtTurns(later, reply)) => {
                    self.look_at_turns(std::time::Instant::now() + later).await;
                    let _ = reply.send(());
                }
                Some(Request::TurnEndsDue(at, reply)) => {
                    self.turn_ends_due(at);
                    let _ = reply.send(());
                }
                Some(Request::DesktopAuto) => self.start_desktop_auto(),
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
                Internal::Event(event) => {
                    // The engine said something: it isn't silent, whatever it runs.
                    if let Some(id) = event.session() {
                        self.engine_heard(id);
                    }
                    match *event {
                        // What the engine echoes as it reads the person's steers: each said as it was.
                        Event::Steered { session, text, .. } => self.steered(session, &text),
                        event @ (Event::Compacting { .. } | Event::Compacted { .. } | Event::CompactFailed { .. }) => self.on_engine_compaction(event),
                        event => {
                            // A build that failed for want of room says so plainly.
                            let no_room = room::says_no_room(&event).then(|| event.session()).flatten();
                            // A page a member showed the person: served for their screen, before
                            // the screen reads it; the member hears at once if nobody can see it.
                            let shown = match &event {
                                Event::ToolResult { session, outcome, .. } => self.member_showed(*session, outcome).map(|note| (*session, note)),
                                _ => None,
                            };
                            self.publish(event);
                            if let Some(id) = no_room {
                                self.member_out_of_room(id);
                            }
                            if let Some((id, note)) = shown {
                                self.set_going(id, format!("[equipe: {note}]")).await;
                            }
                        }
                    }
                }
                Internal::DeskSaved { session, name, saved } => self.desk_saved(session, &name, saved),
                Internal::EngineStarted { session, pid } => self.engine_started(session, pid),
                Internal::TurnSettled { session, why } => self.turn_settled(session, &why),
                Internal::Processes(procs) => self.on_processes(procs),
                Internal::Done(done) => self.on_turn_done(done).await,
                Internal::ConversationTitle(done) => self.on_conversation_title(done),
                Internal::ApprovalTimedOut(id) => self.answer(id, Decision::Deny, "Nobody answered in time."),
                Internal::LaterAnswered { squad, approval, said } => self.later_answered(&squad, *approval, said).await,
                Internal::ReleaseAsk(id) => self.release_ask(id),
                Internal::WordsDue(id) => self.words_due(id).await,
                Internal::Apply(command) => Box::pin(self.handle(*command)).await,
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
                Internal::Merged { squad, ticket, result, kept, push } => self.on_merged(&squad, ticket, result, kept, push),
                Internal::HomePutDown(put) => {
                    // Its work in the folder or back with its member: the next is handed out.
                    let squad = put.squad.clone();
                    self.home_put_down(*put);
                    self.settle_all_merged();
                    self.dispatch(&squad).await;
                }
                Internal::HomeUndone { squad, session, result } => self.home_undone(&squad, session, result),
                Internal::DecisionKept(squad) => {
                    if self.design_watch.contains_key(&squad) {
                        self.look_at_design(&squad, true);
                    }
                }
                Internal::TasteSaved { squad, project, result } => self.taste_saved(squad, project, result),
                Internal::Remembered { session, undo, text } => self.publish(Event::Notice { undo: Some(undo), session, text }),
                Internal::ClosedGit { squad, ticket, status, closed } => {
                    self.closed_git(&squad, ticket, status, closed);
                    self.settle_all_merged();
                    self.dispatch(&squad).await;
                }
                Internal::Checked(gate, outcome) => Box::pin(self.on_checked(*gate, outcome)).await,
                Internal::Accepting(worked) => Box::pin(self.accept_worked(*worked)).await,
                Internal::SelfLand { squad, ticket } => Box::pin(self.self_land(&squad, ticket)).await,
                Internal::Disk(status, asked) => self.on_disk(status, asked),
                Internal::DiskReport(report, asked) => self.on_disk_report(*report, asked),
                Internal::PlanUsePassed(burns, loads) => self.on_plan_use_passed(burns, loads),
                Internal::DiskCleaned(cleaned, status, asked, reply) => self.on_disk_cleaned(*cleaned, status, asked, reply),
                Internal::ProjectAdded { squad, result } => self.project_added(&squad, result),
                Internal::SquadDeleted { squad, result } => self.squad_deleted(&squad, result),
                Internal::Logins(result) => self.logins_changed(result),
                Internal::LoginInsights(insights) => self.publish(Event::LoginInsights { insights: *insights }),
                Internal::Relogin { lane, outcome } => self.relogin_done(lane, outcome).await,
                Internal::Autopilot(note) => self.autopilot_note(note).await,
                Internal::CliSignIn(note) => self.cli_sign_in_note(note).await,
                Internal::McpChecked(checked) => self.mcp_checked(checked),
                Internal::McpSeen(open, seen) => self.mcp_seen(open, seen),
                Internal::Verify(note) => self.verify_note(note).await,
                Internal::LoginRecord { squad, entry } => self.login_record(&squad, entry),
                Internal::SignIns { squad, record, turn } => self.sign_ins_read(&squad, record.map(|r| *r), turn),
                Internal::Wake { session, prompt } => self.set_going(session, prompt).await,
                Internal::SignalRead { squad, key, reading } => self.signal_read(&squad, &key, reading).await,
                Internal::GrowthGathered { squad, gathered } => Box::pin(self.growth_gathered(&squad, *gathered)).await,
                Internal::GrowthMeasured { squad, card, after_days, outcome } => self.growth_measured(&squad, card, after_days, *outcome),
                Internal::SprintGathered { squad, gathered } => Box::pin(self.sprint_gathered(&squad, *gathered)).await,
                Internal::SprintJudged { squad, experiment, judged } => self.sprint_judged(&squad, experiment, *judged),
                Internal::SprintChecked { squad, experiment, arriving } => self.sprint_checked(&squad, experiment, arriving),
                Internal::DeploysLooked { squad, looks, declared } => self.deploys_looked(&squad, looks, declared),
                Internal::GoLiveRead { squad, project, found, channel_known } => self.go_live_read(&squad, project, found, channel_known),
                Internal::GoLiveDue { squad, project } => self.go_live_due(&squad, project),
                Internal::GoLiveRan { squad, project, tickets, outcome } => self.go_live_ran(&squad, project, tickets, outcome),
                Internal::DiskTrimmed(freed, lines) => self.on_disk_trimmed(freed, lines),
                Internal::LaneTmpLooked(look) => self.lane_tmp_looked(*look),
                Internal::TurnEndsDue => self.turn_ends_due(now_secs()),
                Internal::References(shown) => self.references_shown(*shown),
                Internal::Dispatch => {
                    // The disk has room again: a squad held by failing engines tries at once.
                    self.try_breakers_now();
                    let squads: Vec<String> = self.journals.keys().cloned().collect();
                    for squad in squads {
                        self.dispatch(&squad).await;
                    }
                }
                Internal::Workspace { folder, prepared } => self.workspace_done(folder, prepared).await,
                Internal::Engines(engines) => self.publish_engines(engines),
                Internal::SignInLost { engine, session, prompt } => self.sign_in_lost(engine, session, prompt),
                Internal::AuthChecked(engine, state, detail) => self.auth_checked(engine, state, detail).await,
                Internal::AuthRecheck(engine) => self.check_engine(engine),
                Internal::ClaudeReady(ok) => self.claude_ready(ok),
                Internal::ClaudePlan(usage) => self.on_claude_plan(usage).await,
                Internal::CodexModels(listed) => {
                    self.codex_listing = false;
                    if let Some(models) = listed.filter(|m| !m.is_empty()) {
                        let choices = models.iter().map(model_choice).collect();
                        self.codex_models = Some((std::time::Instant::now(), models));
                        self.publish(Event::Models { engine: Engine::Codex, models: choices });
                    }
                }
                Internal::ProviderListed { id, result } => self.provider_listed(id, result),
                Internal::InstallEnded(engine) => {
                    self.installs.remove(&engine);
                    self.publish_nebius();
                    self.publish_providers();
                }
                Internal::EnginePage(engine, url) => {
                    self.engine_pages.insert(engine, url.clone());
                    self.open_engine_page(&url);
                }
                Internal::CodexModelRefused { session, model, instead } => self.codex_model_refused(session, model, instead),
                Internal::SignInEnded(engine, id) => {
                    if self.sign_ins.get(&engine).is_some_and(|(at, _)| *at == id) {
                        self.sign_ins.remove(&engine);
                        self.sign_in_codes.remove(&engine);
                        self.engine_pages.remove(&engine);
                    }
                    // Signed in (or not): looked at again, and what it stopped carried on.
                    if let Some(fix) = self.auth.get_mut(&engine) {
                        fix.carried_on_at = None;
                        self.check_engine(engine);
                    }
                    // Maybe another account now: its plan and usage read afresh.
                    if engine == Engine::Codex {
                        self.forget_codex_plan();
                        self.read_codex_usage(true);
                    }
                }
                Internal::Design(squad, stamp, catalog) => self.on_design(squad, stamp, catalog),
                Internal::Docs(squad, catalog) => self.on_docs(squad, catalog),
                Internal::DocsFound(squad, query, paths) => self.publish(Event::DocsFound { squad, query, paths }),
                Internal::DocCopyKept(squad, session, kept) => self.on_doc_copy_kept(squad, session, kept),
                Internal::PlanWritten(written) => self.plan_written(*written),
                Internal::PlanCut(cut) => self.plan_cut(*cut).await,
                Internal::PlanListed(squad, error) => self.plan_listed(&squad, error),
                Internal::DesignVersions { squad, project, versions } => self.on_design_versions(squad, project, versions),
                Internal::DesignRefs(event) => self.publish(*event),
                Internal::DesignRestore { squad, project, commit, version } => self.on_design_restore(squad, project, commit, version).await,
                Internal::Reviewed { squad, ticket, member, result } => self.on_reviewed(&squad, ticket, member, result),
                Internal::Prechecked { squad, qa, result, secs, loaded } => Box::pin(self.on_prechecked(&squad, qa, result, secs, loaded)).await,
                Internal::ReviewPrechecked { squad, review, heads, secs } => Box::pin(self.on_review_prechecked(&squad, review, heads, secs)).await,
                Internal::SubagentHeard { session, id } => {
                    self.engine_heard(session);
                    self.subagent_heard(session, &id);
                }
                Internal::Helper { session, id, activity } => self.helper_did(session, &id, activity),
                Internal::HelperEnded { session, id, outcome } => self.helper_ended(session, &id, outcome),
                Internal::HelperTimeUp { session, id } => self.stop_helper(session, &id, crate::helpers::Ended::TimeLimit),
                Internal::HelperWaitOver(key) => self.helper_wait_over(key),
                Internal::SideChat { side, event } => self.side_event(side, *event),
                Internal::SideDone(done) => self.side_done(done),
                Internal::CallJudged { call, judged } => self.call_judged(call, *judged).await,
                Internal::CallTold { call, text } => self.call_told(call, text).await,
                Internal::RepoFacts { squad, facts } => self.repo_facts_read(squad, facts),
            },
            Wake::Host(said) => self.on_host(said).await,
            Wake::Daily => self.prune(),
            Wake::DesktopTick => {
                if self.desktop.expire() {
                    self.publish_desktop();
                }
                self.look_at_commands();
                self.watch_subagents();
            }
            Wake::Frames => self.take_frames(),
            Wake::DesignTick => self.design_tick(),
            Wake::WindowTick => self.window_tick().await,
            Wake::Minute => self.minute(pulse).await,
        }
        true
    }
}

impl Runtime {
    /// The minute tick: the board's and the machine's upkeep, each part named
    /// to the watchdog as it starts.
    async fn minute(&mut self, pulse: &watchdog::Pulse) {
        self.keep_awake();
        // First: whether equipe's own new work waits below.
        pulse.step("minute tick: machine load");
        self.look_at_load();
        // What each run waiting on its workspace waits on, told again for screens attached since.
        self.tell_waiting(true);
        // What members left running for tickets now closed, the shell that started it gone.
        self.reap_orphans();
        self.keep_out_of_spotlight();
        let overloaded = self.overloaded();
        pulse.step("minute tick: login autopilot");
        self.autopilot_minute();
        pulse.step("minute tick: leases");
        self.renew_execution_leases();
        self.recover_execution_leases(now_secs()).await;
        pulse.step("minute tick: paused runs");
        self.turn_ends_due(now_secs());
        pulse.step("minute tick: outside MCP servers");
        self.mcp_health_due(now_secs());
        pulse.step("minute tick: quiet turns");
        self.look_at_turns(std::time::Instant::now()).await;
        self.plan_use_pass();
        pulse.step("minute tick: idle lanes");
        self.fold_idle_lanes();
        if self.config.persist && self.tidied_at.is_none_or(|at| at.elapsed() >= tidy::EVERY) {
            self.tidy_conversations().await;
        }
        self.trim_builds();
        self.expire_estimates();
        pulse.step("minute tick: idle browsers");
        self.close_idle_browsers();
        pulse.step("minute tick: helpers");
        self.let_helpers_go().await;
        self.let_side_chats_go();
        pulse.step("minute tick: stale tickets");
        self.tidy_check();
        // New turns of equipe's own wait while the machine is overloaded:
        // they stay due, and go on a minute tick once it calms down.
        if !overloaded {
            pulse.step("minute tick: routines");
            self.run_due_routines().await;
        }
        // A decision whose window passed is taken on time, whatever the load.
        pulse.step("minute tick: decide-and-tell");
        self.autonomy_due(now_secs()).await;
        // Prechecks a restart cut short start again (each waits for calm by itself).
        pulse.step("minute tick: prechecks");
        self.resume_prechecks();
        if !overloaded {
            pulse.step("minute tick: design reviews");
            self.design_qa_tick(now_secs()).await;
        }
        self.usage_digest_tick(now_secs());
        if !overloaded {
            pulse.step("minute tick: idle boards");
            self.wake_idle_pms().await;
            pulse.step("minute tick: signals");
            self.run_due_signals();
            pulse.step("minute tick: growth");
            self.growth_tick();
            pulse.step("minute tick: sprints");
            Box::pin(self.sprint_tick()).await;
        }
        pulse.step("minute tick: deploys");
        self.deploys_tick();
        self.go_live_tick();
        pulse.step("minute tick: spend guards and self-watch");
        self.guards_tick().await;
        pulse.step("minute tick: engines");
        self.engines_tick().await;
        pulse.step("minute tick: calls");
        self.calls_tick().await;
        pulse.step("minute tick: upkeep");
        // Work that reached main outside equipe: every ten minutes is soon
        // enough (a merge or an accept looks at once), and off the loop.
        if self.settled_at.is_none_or(|at| at.elapsed() >= SETTLE_EVERY) {
            self.settle_all_merged();
        }
        // Accepted work a merge failed on (main's checkout busy, say) lands once it can.
        self.retry_merges();
        // Short of room, a copy of a project left in a temporary folder goes sooner.
        let every = if self.disk.status.level.is_warning() { LANE_TMP_TIDY_EVERY_SHORT } else { LANE_TMP_TIDY_EVERY };
        // Its repacks are heavy: not on an overloaded machine, unless the disk is short.
        if self.config.persist && self.lane_tmp_tidied.elapsed() >= every && (!overloaded || self.disk.status.level.is_warning()) {
            self.tidy_lane_tmp();
        }
        self.look_at_disk();
        self.read_codex_usage(false);
    }
}

impl Runtime {
    // ── events: one way out ──

    fn publish(&mut self, event: Event) {
        let changed = match &event {
            Event::TicketChanged { squad, ticket } => Some((squad.clone(), ticket.id)),
            _ => None,
        };
        let closed = match &event {
            Event::TicketChanged { squad, ticket } if ticket.status.is_closed() => Some((squad.clone(), ticket.id)),
            _ => None,
        };
        let asked = match &event {
            Event::QuestionsAsked { squad, questions } => questions.ticket.map(|t| (squad.clone(), t)),
            _ => None,
        };
        // A permission put to the person: from now it waits on them.
        if let Event::ApprovalRequested { approval, .. } = &event {
            self.approvals_asked.insert(approval.id, now_secs());
            // Asked from a turn at work: its answer is that turn's to read.
            if let Some(began) = self.sessions.get(&approval.session).filter(|s| s.is_working()).and_then(|s| s.turn_began) {
                self.approvals_turn.insert(approval.id, (approval.session, began));
            }
        }
        // A ticket closing, however it closed (the person's word, the PM's, a
        // designer's verdict, a member leaving): its lane's temporary folder goes.
        let mut event = event;
        if let Event::TicketChanged { squad, ticket } = &mut event {
            if !ticket.status.is_closed() {
                self.closed_seen.remove(&(squad.clone(), ticket.id));
            } else {
                let first = self.closed_seen.insert((squad.clone(), ticket.id));
                if first {
                    self.clear_lane_tmp_of(squad, ticket.id);
                }
                // A model the person picked for this ticket alone ends with it, wherever it closed.
                if first || ticket.model.as_ref().is_some_and(|m| m.pick.is_some()) {
                    if let Some(m) = ticket.model.as_ref().filter(|m| m.pick.is_some()) {
                        ticket.model = Some(crate::tiers::close(m));
                    }
                    let (squad, id) = (squad.clone(), ticket.id);
                    if let Some(journal) = self.journal(&squad) {
                        let _ = journal.clear_ticket_pick(id);
                    }
                }
            }
        }
        self.publish_in(None, event);
        // A card to confirm about it may have nothing left to confirm.
        if let Some((squad, ticket)) = changed {
            self.settle_accepts(&squad, ticket);
            // The person's request it serves moves with it.
            self.request_ticket_changed(&squad, ticket);
        }
        // Settled: what was asked about it no longer waits on the person.
        if let Some((squad, ticket)) = closed {
            self.withdraw_settled_questions(&squad, ticket);
        }
        if let Some((squad, ticket)) = asked {
            self.request_question_changed(&squad, ticket);
        }
    }

    /// Fold an event into its session, journal what became final, and pass it to
    /// the screens following its squad. `squad` names the squad of an event that
    /// has no session to say it (a failed start, a board error).
    fn publish_in(&mut self, squad: Option<&str>, mut event: Event) {
        // Every key equipe keeps is cut out of what leaves a member (`squad_keys`).
        if let Some(flush) = self.guard_event(&mut event) {
            self.publish_in_guarded(squad, flush);
        }
        if matches!(&event, Event::TextDelta { text, .. } if text.is_empty()) {
            return;
        }
        self.publish_in_guarded(squad, event);
    }

    fn publish_in_guarded(&mut self, squad: Option<&str>, mut event: Event) {
        // A turn's end, a member gone, the person's word: its helpers follow.
        self.follow_helpers(&mut event);
        // A command step starting or answered, a turn's end: its watch follows.
        self.follow_commands(&event);
        // A turn that ends mid-compaction cuts it short: said first.
        if let Event::TurnEnded { session, .. } = &event
            && self.sessions.get(session).is_some_and(|s| s.compacting.is_some())
        {
            self.compaction_ended(*session, Some(compact::CUT_SHORT.to_owned()));
        }
        // A ticket waiting for the person carries what it asks, item by item
        // (kept in its note); any other, nothing.
        if let Event::TicketChanged { squad, ticket } = &mut event {
            ticket.asks = Vec::new();
            ticket.asks = ticket.asks();
            // On its way to main: the screens say so rather than "not in main".
            ticket.landing = self.landing(squad, ticket);
            // It changed now: the screens know when it finished.
            ticket.updated_at = Some(now_secs());
        }
        // The model the engine named, beside the one the member is set to.
        if let Event::ModelKnown { session, asked: asked @ None, .. } = &mut event
            && let Some(s) = self.sessions.get(session)
        {
            asked.clone_from(&s.model);
        }
        // The person's message: what it attached is kept from now on, and
        // the words of its text files go with it into the conversation.
        if let Event::UserMessage { text, kept, .. } | Event::Steered { text, kept, .. } = &mut event
            && kept.is_empty()
        {
            crate::kept::keep(&self.config.home, text);
            *kept = crate::kept::words(&self.config.home, text);
        }
        // How the turn goes, for how it ends: a turn its engine's limit ended is kept as limited.
        let limited = self.watch_turn(&event);
        // A run making the same call over and over: its ticket's next lane goes up a tier.
        if let Event::ToolUse { session, tool, target: Some(target), .. } = &event {
            self.watch_loop(*session, tool, target);
        }
        // A design QA turn past its budget is told to finish with what it has.
        if let Event::ToolUse { session, .. } = &event {
            self.watch_qa_budget(*session);
        }
        if let Event::TurnEnded { session, .. } = &event {
            self.tool_repeats.remove(session);
            self.qa_budgets.remove(session);
            if !self.sign_out_after.is_empty() {
                self.turn_ended_for_sign_out(*session);
            }
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
        let mut to_draw = None;
        // A ticket whose premium-family spend just grew: told to the screens after.
        let mut premium_spent = None;
        if let Some(id) = event.session()
            && let Some(session) = self.sessions.get_mut(&id)
        {
            squad = session.squad.clone();
            let finished = session.transcript.apply(&event);
            // A reply's diagrams, drawn off the loop for the screens that can't.
            if let Some(name) = &squad
                && finished.iter().any(|line| matches!(line, Entry::Reply { text } if has_visuals(text)))
            {
                to_draw = Some((id, name.clone(), finished.clone()));
            }
            // A member's turn, kept for the week's retro: who, on what, how long, how it ended.
            let turn = match (&event, &session.member, session.turn_began) {
                (Event::TurnEnded { outcome, usage, .. }, Some(member), Some(began)) => Some(crate::retro::Turn {
                    member: member.name.clone(),
                    role: member.role.clone(),
                    ticket: session.work.as_ref().map(|w| w.ticket),
                    began,
                    ended: now_secs().max(began),
                    outcome: if limited {
                        turn_ending::LIMITED.to_owned()
                    } else {
                        serde_json::to_value(outcome).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
                    },
                    tokens: usage.as_ref().map_or(0, |u| u.total()),
                }),
                _ => None,
            };
            // …and what it used, by kind, for where the plans went.
            let turn_use = match (&event, &turn) {
                (Event::TurnEnded { usage: Some(u), .. }, Some(_)) => Some(journal::TurnUse {
                    // The effort it ran at, beside the model the engine named.
                    effort: effort_of(session),
                    ..journal::TurnUse::of(session.engine, session.ran.clone().or_else(|| session.model.clone()), session.claude_session.clone(), u)
                }),
                _ => None,
            };
            match &event {
                // A member's own session is named by its first message; a
                // conversation the person keeps gets a generated topic instead.
                Event::UserMessage { text, .. } if session.title.is_none() && !session.kept => {
                    session.title = text.lines().next().map(str::to_owned);
                }
                // What runs; what it is set to stays (an alias is asked for again each turn).
                Event::ModelKnown { model, .. } => session.ran = Some(model.clone()),
                Event::Steered { text, .. } => {
                    if let Some(at) = session.steering.iter().position(|t| t == text) {
                        session.steering.remove(at);
                        if at < session.steer_threads.len() {
                            session.steer_threads.remove(at);
                        }
                    }
                }
                Event::SubagentStarted { id, what, brief, .. } => session.subagents.push(SubRun {
                    id: id.clone(),
                    what: what.clone(),
                    brief: brief.clone(),
                    step: None,
                    since: std::time::Instant::now(),
                    entries: std::collections::VecDeque::new(),
                    heard: std::time::Instant::now(),
                    watched: session.engine == Engine::Codex,
                    stuck: false,
                }),
                Event::SubagentStep { id, step, .. } => {
                    if let Some(sub) = session.subagents.iter_mut().find(|s| s.id == *id) {
                        sub.step = Some(step.clone());
                        sub.heard = std::time::Instant::now();
                    }
                }
                Event::SubagentEntry { id, entry, .. } => {
                    if let Some(sub) = session.subagents.iter_mut().find(|s| s.id == *id) {
                        sub.keep(entry.clone());
                        sub.heard = std::time::Instant::now();
                    }
                }
                Event::SubagentEnded { id, .. } => session.subagents.retain(|s| s.id != *id),
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
                && let Some(journal) = journal_for(&mut self.journals, &self.config.home, &name)
            {
                no_room = !record(journal, id, &event, &finished);
                if matches!(event, Event::UserMessage { .. })
                    && !session.kept
                    && let Some(title) = &session.title
                {
                    let _ = journal.set_title_if_empty(id, title);
                }
                if let Some(turn) = &turn {
                    let _ = journal.record_turn_use(turn, turn_use.as_ref());
                    // Tokens spent on the premium family stay on the ticket, for the person to see.
                    if let (Some(ticket), Some(used)) = (turn.ticket, &turn_use)
                        && used.model.as_deref().is_some_and(|m| crate::tiers::is_premium(used.engine, m))
                        && journal.add_premium_tokens(ticket, turn.tokens).unwrap_or(false)
                    {
                        premium_spent = Some((name.clone(), ticket));
                    }
                }
            }
        }
        if let Some((id, name, finished)) = to_draw {
            self.draw_visuals(id, &name, &finished);
        }
        if let Some((squad, ticket)) = premium_spent
            && let Some(t) = self.find_ticket(&squad, ticket)
        {
            self.publish(Event::TicketChanged { squad, ticket: t });
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
    /// Whether a read started.
    fn read_codex_usage(&mut self, asked: bool) -> bool {
        if self.codex_reading || self.codex_read.is_some_and(|at| at.elapsed() < CODEX_USAGE_EVERY) {
            return false;
        }
        let members = self.sessions.values().any(|s| s.engine == Engine::Codex && has_tools(s) && !s.model.as_deref().is_some_and(crate::nemotron::is_agent_model));
        if !(members || asked) || codex_app::find(&self.config.codex_binary).is_none() {
            return false;
        }
        self.codex_reading = true;
        self.codex_read = Some(std::time::Instant::now());
        let (binary, home, internal) = (self.config.codex_binary.clone(), self.config.home.clone(), self.internal.clone());
        tokio::spawn(async move {
            let cwd = if home.is_dir() { home } else { std::env::temp_dir() };
            let read = tokio::time::timeout(SERVER_WAIT, codex_app::read_limits(&binary, &cwd)).await;
            let _ = internal.send(Internal::CodexLimits(read.ok().and_then(Result::ok)));
        });
        true
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
    /// A key saved since the daemon started (`equipe login nebius`), taken up
    /// when there was none.
    fn refresh_nemotron(&mut self) {
        if self.config.nemotron.is_none() && crate::nemotron::saved_key(&self.config.home).is_some() {
            self.config.nemotron = crate::nemotron::Config::from_env_or_saved(&self.config.home).ok();
        }
    }

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
            return Ok(models.iter().filter(|m| !self.codex_refuses(&m.id)).map(|m| m.id.clone()).collect());
        }
        let cwd = if self.config.home.is_dir() { self.config.home.clone() } else { std::env::temp_dir() };
        let listed = tokio::time::timeout(SERVER_WAIT, codex_app::list_model_infos(&self.config.codex_binary, &cwd)).await;
        let models = match listed {
            Ok(Ok(models)) => models,
            Ok(Err(err)) => return Err(err.to_string()),
            Err(_) => return Err(format!("Codex didn't answer in {}s", SERVER_WAIT.as_secs())),
        };
        let ids = models.iter().filter(|m| !self.codex_refuses(&m.id)).map(|m| m.id.clone()).collect();
        self.codex_models = Some((std::time::Instant::now(), models));
        Ok(ids)
    }

    /// The models `engine` offers, for the screens ([`Command::Models`]):
    /// Claude Code's from the catalog; the Codex CLI's as it lists them, off
    /// the loop, kept a while. Nothing when the Codex CLI isn't here.
    fn models_asked(&mut self, engine: Engine) {
        match engine {
            Engine::Claude => {
                let aliases = crate::models::CLAUDE_FAMILIES.iter().map(|f| f.alias);
                let ids = crate::models::CLAUDE_MODELS.iter().map(|p| p.id);
                let models = aliases
                    .chain(ids)
                    .map(|id| {
                        let efforts = crate::models::efforts(Engine::Claude, Some(id)).iter().map(|e| (*e).to_owned()).collect();
                        ModelChoice { id: id.to_owned(), name: crate::models::name(id), efforts, ..Default::default() }
                    })
                    .collect();
                self.publish(Event::Models { engine, models });
            }
            Engine::Codex => {
                if let Some((at, models)) = &self.codex_models
                    && at.elapsed() < CODEX_MODELS_KEPT
                {
                    let models = models.iter().map(model_choice).collect();
                    return self.publish(Event::Models { engine, models });
                }
                if self.codex_listing || codex_app::find(&self.config.codex_binary).is_none() {
                    return;
                }
                self.codex_listing = true;
                let (binary, internal) = (self.config.codex_binary.clone(), self.internal.clone());
                let cwd = if self.config.home.is_dir() { self.config.home.clone() } else { std::env::temp_dir() };
                tokio::spawn(async move {
                    let listed = tokio::time::timeout(SERVER_WAIT, codex_app::list_model_infos(&binary, &cwd)).await;
                    let _ = internal.send(Internal::CodexModels(listed.ok().and_then(Result::ok)));
                });
            }
        }
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

    /// The desktop sets itself up: [`desktop::auto`], told to every screen.
    fn start_desktop_auto(&mut self) {
        if self.desktop_auto.is_some() {
            return;
        }
        let internal = self.internal.clone();
        let sink: desktop::auto::Sink = std::sync::Arc::new(move |ready| drop(internal.send(Internal::Event(Box::new(Event::DesktopReady { ready })))));
        let ops = std::sync::Arc::new(desktop::auto::ThisMachine { home: self.config.home.clone() });
        self.desktop_auto = Some(desktop::auto::Auto::start(self.config.home.clone(), ops, sink));
    }

    fn publish_desktop(&mut self) {
        let desktop = self.desktop.state();
        // The setup's loop decides the one card about a locked screen.
        if let Some(auto) = &self.desktop_auto {
            auto.lock_held(!desktop.locked_waiting.is_empty());
        }
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

    /// A person-only command came from a member's own process and was refused at
    /// the door: logged as a security event, and told to the person — to the
    /// squad it named, or to every screen when it named none.
    fn member_refused(&mut self, squad: Option<&str>) {
        crate::daemon_log!("equipe: refused a person-only command from a member's own process (not the person's screen)");
        let text = say!(self, "security.member_refused");
        match squad {
            Some(squad) => self.squad_error(squad, text),
            None => self.publish(Event::Error { session: None, message: text, next: None }),
        }
    }

    async fn handle(&mut self, command: Command) {
        // What the person sends keeps its attachments from the moment it is
        // sent, even while it waits for a member (`crate::kept`).
        match &command {
            Command::Submit { text, session, .. } | Command::SubmitLater { text, session, .. } => {
                crate::kept::keep(&self.config.home, text);
                // The person at work in a squad that failing engines hold: it tries a test turn now.
                if let Some(squad) = self.sessions.get(session).and_then(|s| s.squad.clone()) {
                    self.person_acted(&squad);
                }
            }
            Command::SideChatSubmit { text, .. } => {
                crate::kept::keep(&self.config.home, text);
            }
            Command::SendNow { texts, .. } => {
                for text in texts {
                    crate::kept::keep(&self.config.home, text);
                }
            }
            _ => {}
        }
        match command {
            Command::Start { session, spec } => self.start(session, spec).await,
            Command::OnPhone { command } => {
                self.on_phone = true;
                Box::pin(self.handle(*command)).await;
                self.on_phone = false;
            }
            Command::CallStart { squad } => self.call_start(squad).await,
            Command::CallUtterance { squad, call, text, via: _ } => self.call_utterance(squad, call, text).await,
            Command::CallEnd { squad, call } => self.call_end(squad, call).await,
            Command::Submit { session, text, about } => {
                // A conversation the person opened is about its own project, whatever the screen shows.
                self.said_about = self.conversation_about(session).or(about);
                let session = self.route(session);
                self.note_person_active(session);
                self.note_screen(session);
                // What they asked for here: a ticket made in answer serves it.
                self.person_said(session, &text);
                let busy = self.sessions.get(&session).is_some_and(Session::is_working);
                self.submit(session, text.clone()).await;
                self.person_first(session, &text).await;
                // Asking after a request while its member is busy: answered from the board now.
                if busy {
                    self.answer_from_board(session, &text);
                }
                self.said_about = None;
            }
            Command::SubmitLater { session, text, about } => {
                self.said_about = self.conversation_about(session).or(about);
                let session = self.route(session);
                self.note_person_active(session);
                self.note_screen(session);
                self.person_said(session, &text);
                match self.sessions.get(&session) {
                    Some(s) if s.is_working() => {
                        self.leave_memo(session, text.clone());
                        self.answer_from_board(session, &text);
                    }
                    _ => self.submit(session, text).await,
                }
                self.said_about = None;
            }
            Command::Interrupt { session } => self.interrupt(session).await,
            // A run is stopped, never closed alone: its member stays, and the
            // ticket with its run until it hands in, waits or is recovered.
            Command::Close { session } if self.sessions.get(&session).is_some_and(|s| s.run_of.is_some()) => self.interrupt(session).await,
            Command::Close { session } => self.close(session).await,
            Command::Accept { squad, ticket } => self.accept(&squad, ticket, crate::telemetry::Accepter::Person).await,
            Command::Merge { squad, ticket, push } => self.merge(&squad, ticket, push),
            Command::KeepOnBranch { squad, ticket } => self.keep_on_branch(&squad, ticket),
            Command::Reopen { squad, ticket, note } => self.reopen(&squad, ticket, note).await,
            Command::Resume { squad, ticket, note } => self.resume(&squad, ticket, note).await,
            Command::Requeue { squad, ticket, operation, how } => {
                self.person_acted(&squad);
                self.requeue(&squad, ticket, &operation, how).await
            }
            Command::Cancel { squad, ticket } => self.cancel(&squad, ticket).await,
            Command::CreateTicket { squad, draft } => self.person_creates(&squad, draft).await,
            Command::StopWork { squad, request, ticket, words } => self.person_stops(&squad, request, ticket, &words).await,
            Command::LiftStop { squad, stop } => self.lift_stop(&squad, stop),
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
            Command::DesktopCheck { guide } => {
                // Off the loop: the helper may take a moment to answer.
                let (home, internal) = (self.config.home.clone(), self.internal.clone());
                let auto = self.desktop_auto.clone();
                tokio::spawn(async move {
                    let mut ready = desktop::ready::readiness(&home, guide).await;
                    if let Some(auto) = auto {
                        auto.decorate(&mut ready);
                        // The person pressed "Allow…": the card has done its job.
                        if guide {
                            auto.wanted();
                        }
                    }
                    let _ = internal.send(Internal::Event(Box::new(Event::DesktopReady { ready })));
                });
            }
            Command::DesktopLocked { on } => {
                if let Err(e) = desktop::set_work_locked(&self.config.home, on) {
                    crate::daemon_log!("equipe: desktop: couldn't keep \"work while locked\": {e}");
                }
                crate::daemon_log!("equipe: desktop: work while the screen is locked {}", if on { "on" } else { "off" });
                // Every screen sees the switch where it now is.
                let (home, internal, auto) = (self.config.home.clone(), self.internal.clone(), self.desktop_auto.clone());
                tokio::spawn(async move {
                    let mut ready = desktop::ready::readiness(&home, false).await;
                    if let Some(auto) = auto {
                        auto.decorate(&mut ready);
                    }
                    let _ = internal.send(Internal::Event(Box::new(Event::DesktopReady { ready })));
                });
            }
            Command::DesktopAuto { action } => {
                if let Some(auto) = &self.desktop_auto {
                    match action {
                        equipe_protocol::DesktopAutoAction::Retry => auto.retry(),
                        equipe_protocol::DesktopAutoAction::Dismiss => auto.dismiss(),
                    }
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
            Command::BrowserCookies { lane, hosts, cookies, failed } => {
                self.view_cookies(lane, hosts, cookies, failed).await;
            }
            Command::BrowserControl { lane, take } => {
                // A browser in the person's own window: Done closes that window and
                // brings the automated browser back; taking it is for the panel.
                if self.window_done(&lane, take).await {
                    return;
                }
                // A check that refuses automated browsers: taken in a normal Chrome window, not the panel.
                if take && self.check_to_window(&lane).await {
                    return;
                }
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": take.then_some("person") }));
            }
            Command::BrowserDialog { page, accept, text } => {
                self.hub.op(&json!({ "op": "dialog", "page": page, "accept": accept, "text": text }));
            }
            // An engine's sign-in page, opened again from its card: as it was,
            // no login to keep at its end — it is the engine's own.
            Command::BrowserSignIn { url, .. } if self.engine_pages.values().any(|page| *page == url) => {
                self.open_engine_page(&url);
            }
            Command::BrowserSignIn { url, window } => {
                self.start_sign_in(&url, window);
            }
            Command::BrowserPopOut { page, bounds } => {
                // No page: the person's own Chrome window of a hand-off, brought forward.
                if page == 0 {
                    let raise = self.windows.handoffs.raise();
                    self.run_window_actions(raise).await;
                    return;
                }
                // A page stopped at a check that refuses automated browsers: bringing up the
                // automated Chrome's own window wouldn't help — a normal one opens instead.
                if let Some(lane) = self.lane_of_page(page)
                    && self.checking.get(&lane).is_some_and(|c| c.0 == page)
                    && self.check_to_window(&lane).await
                {
                    return;
                }
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
                    self.publish(Event::Notice { undo: None, session, text });
                }
            }
            Command::SetModel { session, model } => {
                let model = model.trim().to_owned();
                // A provider's model runs on its API's engine (Codex's agent for an
                // OpenAI-compatible one, Claude Code for an Anthropic-compatible one):
                // a member on the other engine moves there.
                if let Some(target) = crate::model_providers::engine_of_ref(&model)
                    && let Some(s) = self.sessions.get(&session).filter(|s| s.engine != target)
                {
                    match (s.squad.clone(), s.name().map(str::to_owned)) {
                        (Some(squad), Some(name)) => self.move_to_model(&squad, name, session, model).await,
                        _ => {
                            let provider = self.on_provider(&model).map(|(_, p)| p).unwrap_or_default();
                            let message = if target == Engine::Claude { say!(self, "error.provider_model_claude", provider = provider) } else { say!(self, "error.provider_model_codex", provider = provider) };
                            self.error(Some(session), message, None);
                        }
                    }
                    return;
                }
                if let Some(s) = self.sessions.get_mut(&session).filter(|_| !model.is_empty()) {
                    s.model = Some(model.clone());
                    // Its engine names what this runs as at its next turn.
                    s.ran = None;
                    let squad = s.squad.clone();
                    let member = s.member.as_ref().map(|m| m.name.clone());
                    if let Some(journal) = squad.as_deref().and_then(|q| journal_for(&mut self.journals, &self.config.home, q)) {
                        let _ = journal.set_model(session, &model);
                        // The person's choice: equipe's tier policies keep it as the member's floor.
                        if let Some(member) = &member {
                            let _ = journal.mark_person_model(member);
                        }
                    }
                    self.publish(Event::ModelKnown { session, model: model.clone(), asked: Some(model.clone()) });
                    let text = say!(self, "notice.model", model = model);
                    self.publish(Event::Notice { undo: None, session, text });
                    if crate::tiers::cap_premium(Engine::Claude, Some(&model), false).as_deref() != Some(model.as_str()) {
                        let text = say!(self, "notice.model_premium");
                        self.publish(Event::Notice { undo: None, session, text });
                    }
                }
            }
            Command::Models { engine } => self.models_asked(engine),
            Command::SetRoleModel { squad, default } => {
                let default = RoleModel {
                    model: default.model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()),
                    effort: default.effort.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty()),
                    ..default
                };
                if let Some(journal) = self.journal(&squad) {
                    match journal.set_role_model(default) {
                        Ok(defaults) => self.publish(Event::RoleModelsChanged { squad, defaults }),
                        Err(err) => self.error(None, err.to_string(), None),
                    }
                }
            }
            Command::Unqueue { session, text } => self.unqueue(session, &text),
            Command::SendNow { session, texts } => {
                self.note_screen(session);
                self.send_now(session, texts).await
            }
            Command::StopCommand { session } => self.stop_command(session),
            Command::StopSubagent { session, id } => self.stop_subagent(session, &id),
            Command::StartSideChat { session, side, passage } => self.start_side_chat(session, side, passage),
            Command::SideChatSubmit { session, side, passage, text } => self.side_chat_submit(session, side, passage, text),
            Command::CloseSideChat { side } => self.close_side_chat(side),
            Command::SetEffort { session, effort } => {
                let effort = effort.map(|e| e.trim().to_ascii_lowercase()).filter(|e| !e.is_empty());
                if let Some(s) = self.sessions.get_mut(&session) {
                    s.effort = effort.clone();
                    let text = match &effort {
                        Some(effort) => say!(self, "notice.effort", effort = effort),
                        None => say!(self, "notice.effort_default"),
                    };
                    self.publish(Event::Notice { undo: None, session, text });
                }
            }
            Command::AddProject { squad, path } => self.add_project(squad, path),
            Command::RemoveProject { squad, path } => self.remove_project(&squad, &path),
            Command::SetHq { squad, path } => self.set_hq(&squad, path),
            Command::DeleteSquad { squad, purge } => self.delete_squad(&squad, purge),
            Command::SaveLogin { draft } => {
                // A login for a site offered: the offer is answered.
                self.close_login_offers(&vault::site_of(&draft.site));
                if let Some(id) = draft.id
                    && (draft.auto == Some(true) || draft.password.as_deref().is_some_and(|p| !p.is_empty()))
                {
                    self.relogin_tries.remove(&id);
                }
                self.with_vault(move |vault| vault.save(draft, now_secs()).map(|_| ()))
            }
            Command::DeleteLogin { id } => self.with_vault(move |vault| vault.delete(id)),
            Command::ForgetLoginAllowance { id, who } => self.with_vault(move |vault| vault.forget(id, &who)),
            Command::AllowLogin { id, who } => self.allow_login(id, who),
            Command::SetLoginAuto { id, auto } => self.set_login_auto(id, auto),
            Command::SetLoginScope { id, squads } => self.with_vault(move |vault| vault.set_scope(id, &squads)),
            Command::ForgetNeverOffer { site } => self.forget_never_offer(site),
            Command::LoginInsights { days } => self.read_login_insights(days),
            Command::DismissLoginOffer { id, never } => self.dismiss_login_offer(id, never),
            Command::Logins => {
                let logins = self.logins.clone();
                self.publish(Event::Logins { logins });
            }
            command @ (Command::SetLoginAutopilot { .. }
            | Command::SetLoginTotp { .. }
            | Command::ScanLoginTotp { .. }
            | Command::RegisterLoginPasskey { .. }
            | Command::ForgetLoginPasskey { .. }
            | Command::SaveMailbox { .. }
            | Command::SetSmsCodes { .. }
            | Command::SetKeepAlive { .. }
            | Command::SaveCliLogin { .. }
            | Command::DeleteCliLogin { .. }
            | Command::Autopilot) => self.autopilot_command(command),
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
            Command::SubmitCliSignInCode { squad, id, code } => self.submit_cli_sign_in_code(&squad, id, code),
            Command::RestartCliSignIn { squad, id } => self.restart_cli_sign_in(&squad, id),
            Command::CancelCliSignIn { squad, id } => self.cancel_cli_sign_in(&squad, id).await,
            Command::FillVerificationCode { squad, id } => self.fill_verification(&squad, id),
            Command::TypeVerificationCode { squad, id } => self.type_verification(&squad, id),
            Command::ConnectGmail { client } => self.connect_gmail(client),
            Command::DisconnectGmail => self.disconnect_gmail(),
            Command::RequestServiceKey { squad, service } => self.request_service_key(&squad, &service),
            Command::AddMcpServer { squad, server, sign_in } => self.add_mcp_server(&squad, server, sign_in),
            Command::RemoveMcpServer { squad, name } => self.remove_mcp_server(&squad, &name),
            Command::SetMcpServerEnabled { squad, name, on } => self.set_mcp_server_enabled(&squad, &name, on),
            Command::SignInMcpServer { squad, name } => self.sign_in_mcp_server(&squad, &name),
            Command::CheckMcpServers { squad } => self.check_mcp_servers(&squad, None, None),
            command @ (Command::SquadSecrets
            | Command::SetSquadSecretProjects { .. }
            | Command::DeleteSquadSecret { .. }
            | Command::RevealSquadSecret { .. }) => self.squad_secret_command(command),
            Command::SaveRoutine { squad, routine } => self.save_routine(&squad, routine),
            Command::DeleteRoutine { squad, id } => self.delete_routine(&squad, id),
            Command::RunRoutine { squad, id } => {
                let now = now_secs();
                self.run_routine(&squad, id, now).await;
            }
            Command::ApproveRoutine { squad, id } => self.approve_routine(&squad, id),
            Command::DeclineRoutine { squad, id } => self.decline_routine(&squad, id),
            Command::SetIdleWake { squad, hours } => self.set_idle_wake(&squad, hours).await,
            Command::SetNotify { squad, settings } => self.set_notify(&squad, settings),
            Command::SetResting { squad, member, resting, stop } => self.set_resting(&squad, &member, resting, stop).await,
            Command::WatchTicket { squad, ticket, watch } => self.watch_ticket(&squad, ticket, watch),
            Command::SetSignalOn { squad, project, name, on } => self.set_signal_on(&squad, &project, &name, on),
            Command::NewConversation { squad, session, project } => self.new_conversation(&squad, session, project),
            Command::MoveConversation { session, project } => self.move_conversation(session, project),
            Command::RenameConversation { squad, session, title } => self.rename_conversation(&squad, session, &title),
            Command::CloseConversation { session } => self.close_conversation(session).await,
            Command::PinConversation { session, pinned } => self.pin_conversation(session, pinned),
            Command::SetAutoPutAway { squad, on } => self.set_auto_put_away(squad, on),
            Command::ReopenConversation { squad, session } => self.reopen_conversation(&squad, session),
            Command::Focus { squad, project } => match project {
                Some(name) => {
                    self.focus.insert(squad, name);
                }
                None => {
                    self.focus.remove(&squad);
                }
            },
            Command::Decide { squad, approval, decision, words, said } => {
                self.words_decide_said(&squad, approval, decision, said.as_ref());
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
            Command::ConfirmAccept { squad, id, tickets, said } => self.confirm_accept(&squad, id, tickets, said).await,
            Command::DeclineAccept { squad, id } => self.decline_accept(&squad, id),
            Command::Say { squad, session, text, via, all, later, pasted, about, device, nonce } => self.say(squad, session, text, via, all, later, pasted, about, device, nonce).await,
            Command::UndoWords { squad, id } => self.undo_words(&squad, id),
            Command::WordsLog { squad } => self.words_log(squad),
            Command::ClearWordsLog { squad } => self.clear_words_log(squad),
            Command::SetWordsAll { squad, on } => self.set_words_all(squad, on),
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
            Command::PlanUse { engine, span } => self.plan_use_look(engine, span),
            Command::DiskClean { ids } => self.disk_clean(ids, None),
            Command::DiskRun { id } => self.disk_run(id),
            Command::Engines => {
                self.refresh_nemotron();
                self.look_at_engines();
                self.publish_nebius();
            }
            Command::SetNebiusKey { key } => self.set_nebius_key(key),
            Command::Providers => self.providers_asked(),
            Command::AddProvider { preset, base_url, key } => self.add_provider(preset, base_url, key.map(|k| k.0)),
            Command::TestProvider { id } => self.test_provider(id),
            Command::RemoveProvider { id } => self.remove_provider(id),
            Command::SignIn { engine, device } => self.sign_in(engine, device),
            Command::SignInCancel { engine } => {
                if let Some((_, task)) = self.sign_ins.remove(&engine) {
                    task.abort();
                    self.publish(Event::SignIn { engine, step: equipe_protocol::SignInStep::Cancelled });
                    self.look_at_engines();
                }
            }
            Command::SignInCode { engine, code } => {
                // Straight to the tool's prompt: never kept, never logged.
                if let (Some(tx), Some(code)) = (self.sign_in_codes.get(&engine), crate::engine_auth::pasted(&code)) {
                    let _ = tx.send(code);
                }
            }
            Command::CheckEngine { engine } => {
                // The person says they did something: what was carried on before counts no more.
                self.auth.entry(engine).or_default().carried_on_at = None;
                self.check_engine(engine);
            }
            Command::MoveIntoLogin => self.move_into_login(),
            Command::SignOut { engine } => {
                self.sign_out_after.remove(&engine);
                self.sign_out(engine);
            }
            Command::SignOutAfterTurns { engine } => self.sign_out_after_turns(engine),
            Command::SignOutCancel { engine } => {
                if self.sign_out_after.remove(&engine) {
                    self.look_at_engines();
                }
            }
            Command::InstallEngine { engine } => self.install_engine(engine),
            Command::SetEngineSwitching { squad, mode } => self.set_engine_switching(&squad, mode).await,
            Command::SetParallel { squad, parallel } => self.set_parallel(&squad, parallel).await,
            Command::SetModelChoice { squad, choice } => self.set_model_choice(&squad, choice),
            Command::SetTicketTier { squad, ticket, tier, pinned } => self.set_ticket_tier(&squad, ticket, tier, pinned),
            Command::SetTicketModel { squad, ticket, model } => self.set_ticket_pick(&squad, ticket, model),
            Command::SetPmAccepts { squad, on } => {
                if let Some(journal) = self.journal(&squad) {
                    let _ = journal.set_pm_accepts(on);
                }
                self.publish(Event::PmAcceptsChanged { squad: squad.clone(), on });
                self.publish_autonomy(&squad);
            }
            Command::SetAutonomy { squad, kind, level } => self.set_autonomy(&squad, kind, level),
            Command::SetAutonomyWindow { squad, minutes } => self.set_autonomy_window(&squad, minutes),
            Command::SetAutonomyMode { squad, mode } => self.set_autonomy_mode(&squad, mode),
            Command::AllowKind { squad, approval, days } => self.allow_kind(&squad, approval, days),
            Command::RevokeAllowance { squad, id } => self.revoke_allowance(&squad, id),
            Command::ReopenAutoPick { squad, set } => self.reopen_auto_pick(&squad, set),
            Command::SetSelfLand { squad, on } => {
                if let Some(journal) = self.journal(&squad) {
                    let _ = journal.set_self_land(on);
                }
            }
            Command::RevertLanded { squad, ticket } => {
                let why = say!(self, "flow.revert_by_person");
                match self.revert_landed(&squad, "you", &json!({ "id": ticket, "why": why })) {
                    Ok(line) => self.brief_pm(&squad, format!("The person asked from the digest to undo T-{ticket}. {line}"), None),
                    Err(why) => self.squad_error(&squad, why),
                }
                self.dispatch(&squad).await;
            }
            // Each member's next turn runs it and is told it.
            Command::SetSubagentsAtOnce { squad, at_once } => {
                let at_once = at_once.map(crate::protocol::subagents_at_once);
                if let Some(Err(err)) = self.journal(&squad).map(|j| j.set_subagents_at_once(at_once)) {
                    crate::daemon_log!("equipe: {squad}: subagents at once wasn't kept: {err}");
                }
                self.publish(Event::SubagentsAtOnceChanged { squad, at_once });
            }
            // Each member's next turn is told the reviewer it gets (`Runtime::design_reviewer`).
            Command::SetDesignReviewer { squad, reviewer } => {
                if let Some(Err(err)) = self.journal(&squad).map(|j| j.set_design_reviewer(reviewer)) {
                    crate::daemon_log!("equipe: {squad}: the design reviewer wasn't kept: {err}");
                }
                self.publish(Event::DesignReviewerChanged { squad, reviewer });
            }
            Command::ApplyEngineProposal { squad, id, now } => self.apply_engine_proposal(&squad, id, now).await,
            Command::DismissEngineProposal { squad, id } => self.dismiss_engine_proposal(&squad, id),
            Command::UndoEngineProposal { squad, id } => self.undo_engine_proposal(&squad, id).await,
            Command::UndoHomeChange { squad, session, change } => self.undo_home_change(&squad, session, &change),
            Command::ShowEngineSetup { squad } => self.show_engine_setup(&squad),
            Command::HandOver { squad, session } => self.send_hand_over(&squad, session),
            Command::MoveEngine { squad, members, engine, model, effort, now } => self.move_engine(&squad, members, engine, model, effort, now).await,
            Command::Design { squad } => self.design_asked(squad),
            Command::ProjectDocs { squad } => self.docs_asked(squad),
            Command::SearchDocs { squad, query } => self.docs_searched(squad, query),
            Command::KeepDocCopy { squad, session, keep } => self.keep_doc_copy(squad, session, keep),
            Command::DesignVersions { squad, project } => self.design_versions_asked(squad, project),
            Command::DesignRefs { squad, project, ticket, query, offset } => self.design_refs_asked(squad, project, ticket, query, offset),
            Command::DesignResults { squad } => self.design_results_asked(squad),
            Command::RestoreDesign { squad, project, commit } => self.restore_design_asked(squad, project, commit),
            Command::Taste { squad, project, change } => self.taste_changed(squad, project, change),
            Command::TasteSignals { squad, project, signals } => self.taste_signals(squad, project, signals),
            Command::KeepReference { squad, session, project, card } => self.keep_reference(squad, session, project, card),
            Command::TryReference { squad, project, card } => self.try_reference(squad, project, card),
            Command::StartDesignSession { squad, project, page, from } => self.start_design_session(squad, project, page, from).await,
            Command::LivePreview { squad, session, url, open } => self.live_preview(squad, session, url, open),
            Command::EndDesignSession { squad, designer } => self.end_design_session(&squad, designer),
            Command::UseDesignVariant { squad, designer, project, page, variant, url } => {
                let picked = crate::design::session::Picked { page, project, variant, url };
                self.use_design_variant(&squad, designer, picked).await;
            }
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

    /// Nebius Token Factory as it stands, for every screen: where the key
    /// comes from, whether Codex's agent is here — never the key.
    fn publish_nebius(&mut self) {
        let cfg = self.config.nemotron.as_ref();
        let status = equipe_protocol::NebiusStatus {
            key: if cfg.is_some() { crate::nemotron::key_source(&self.config.home) } else { equipe_protocol::NebiusKey::None },
            codex: codex_app::find(&self.config.codex_binary).is_some(),
            model: cfg.and_then(|c| c.model.clone()),
            provider: cfg.map(|c| c.provider_name.clone()).unwrap_or_else(|| crate::nemotron::PROVIDER_NAME.to_owned()),
        };
        self.publish(Event::Nebius { status });
    }

    /// Save the person's Nebius key here (or forget it), and use it from now
    /// on: members starting on Nebius and reviews. The environment's key, when
    /// set, still comes first.
    fn set_nebius_key(&mut self, key: Option<String>) {
        let home = self.config.home.clone();
        let done = match key {
            Some(key) if !crate::nemotron::key_is_plausible(&key) => Err(say!(self, "error.nebius_key_invalid")),
            Some(key) => crate::nemotron::save_key(&home, &key).map_err(|err| say!(self, "error.nebius_key_not_saved", why = err)),
            // Off the providers' list too, where it is Nebius's entry.
            None => crate::model_providers::remove(&home, crate::model_providers::NEBIUS).map(|_| ()).map_err(|err| say!(self, "error.nebius_key_not_saved", why = err)),
        };
        if let Err(why) = done {
            self.error(None, why, None);
        }
        self.providers.remove(crate::model_providers::NEBIUS);
        self.config.nemotron = crate::nemotron::Config::from_env_or_saved(&home).ok();
        self.publish_nebius();
        self.publish_providers();
    }

    /// The engines as just looked at, with who is on each and the sign-ins
    /// under way, for every screen.
    fn publish_engines(&mut self, mut engines: Vec<equipe_protocol::EngineStatus>) {
        self.engines.saw(&engines);
        for status in &mut engines {
            let mut members: Vec<MemberRef> = self.sessions.values().filter(|s| s.engine == status.engine).filter_map(session_member).collect();
            members.sort_by(|a, b| (&a.squad, &a.member).cmp(&(&b.squad, &b.member)));
            members.dedup();
            status.members = members;
            status.signing_in = self.sign_ins.contains_key(&status.engine);
            let mut working: Vec<MemberRef> = self.sessions.values().filter(|s| s.engine == status.engine && s.is_working()).filter_map(session_member).collect();
            working.sort_by(|a, b| (&a.squad, &a.member).cmp(&(&b.squad, &b.member)));
            working.dedup();
            status.working = working;
            // Signed out some other way meanwhile: nothing left to wait for.
            if status.state != equipe_protocol::EngineState::SignedIn {
                self.sign_out_after.remove(&status.engine);
            }
            status.sign_out_after_turns = self.sign_out_after.contains(&status.engine);
        }
        // Signed in by whatever means — a terminal at the Mac's own screen —
        // while turns wait: they are looked at again, and carried on.
        let back: Vec<Engine> = engines
            .iter()
            .filter(|s| s.state == equipe_protocol::EngineState::SignedIn && self.auth.get(&s.engine).is_some_and(|f| !f.stopped.is_empty() && !f.checking && f.carried_on_at.is_none()))
            .map(|s| s.engine)
            .collect();
        self.publish(Event::Engines { engines });
        for engine in back {
            self.check_engine(engine);
        }
    }

    /// Sign an engine in with its own tool, here on the daemon's machine;
    /// every screen hears each step (the code to enter, when `device`; for
    /// Claude Code, the page whose code is pasted back), then where the
    /// engines stand. A new sign-in stops the one under way. Claude Code
    /// isn't signed in from outside the Mac's login session: its login would
    /// land where the members never read it.
    fn sign_in(&mut self, engine: Engine, device: bool) {
        use equipe_protocol::{SignInFailure, SignInStep};
        if let Some((_, old)) = self.sign_ins.remove(&engine) {
            old.abort();
        }
        self.sign_in_codes.remove(&engine);
        let id = self.next_sign_in;
        self.next_sign_in += 1;
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        let codes = (engine == Engine::Claude).then(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            self.sign_in_codes.insert(engine, tx);
            rx
        });
        let task = tokio::spawn(async move {
            let tell = |step| drop(internal.send(Internal::Event(Box::new(Event::SignIn { engine, step }))));
            let end = match codes {
                None => crate::engines::codex_sign_in(&codex, device, tell, |url| drop(internal.send(Internal::EnginePage(engine, url)))).await,
                Some(_) if crate::engine_auth::in_login_session() == Some(false) => {
                    let _ = internal.send(Internal::AuthRecheck(engine));
                    SignInStep::Failed { why: SignInFailure::Unsupported, detail: String::new() }
                }
                Some(codes) => crate::engine_auth::claude_login(&claude, codes, tell).await,
            };
            tell(end);
            let _ = internal.send(Internal::SignInEnded(engine, id));
            let _ = internal.send(Internal::Engines(crate::engines::statuses(&claude, &codex, &store).await));
        });
        self.sign_ins.insert(engine, (id, task));
    }

    /// Install `engine`'s tool here with its maker's installer, off the loop
    /// (once at a time), then look at the engines again.
    fn install_engine(&mut self, engine: Engine) {
        use equipe_protocol::InstallStep;
        if !self.installs.insert(engine) {
            return;
        }
        self.publish(Event::EngineInstall { engine, step: InstallStep::Running });
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            let step = crate::engines::install_or_update(engine, &claude).await;
            let _ = internal.send(Internal::Event(Box::new(Event::EngineInstall { engine, step })));
            let _ = internal.send(Internal::Engines(crate::engines::statuses(&claude, &codex, &store).await));
            let _ = internal.send(Internal::InstallEnded(engine));
        });
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

    /// Whether a member is mid-turn on `engine`, not counting `except`
    /// (a session whose turn just ended, its task not yet finished).
    fn engine_in_turn(&self, engine: Engine, except: Option<SessionId>) -> bool {
        self.sessions.iter().any(|(id, s)| s.engine == engine && Some(*id) != except && s.is_working())
    }

    /// "After current turns": signs `engine` out at once when no member is
    /// mid-turn on it, else once the last of those turns has ended
    /// ([`Runtime::turn_ended_for_sign_out`]).
    fn sign_out_after_turns(&mut self, engine: Engine) {
        if self.engine_in_turn(engine, None) {
            self.sign_out_after.insert(engine);
            self.look_at_engines();
        } else {
            self.sign_out_after.remove(&engine);
            self.sign_out(engine);
        }
    }

    /// A turn ended: a sign-out waiting for the turns on its engine goes
    /// ahead once none is left.
    fn turn_ended_for_sign_out(&mut self, session: SessionId) {
        let Some(engine) = self.sessions.get(&session).map(|s| s.engine) else { return };
        if self.sign_out_after.contains(&engine) && !self.engine_in_turn(engine, Some(session)) {
            self.sign_out_after.remove(&engine);
            self.sign_out(engine);
        }
    }

    /// Sign `engine` out with its own tool, when the person asks on its card.
    fn sign_out(&mut self, engine: Engine) {
        if engine == Engine::Codex {
            self.forget_codex_plan();
        }
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            let out = match engine {
                Engine::Codex => crate::engines::codex_logout(&codex).await.map_err(|why| ("codex logout", why)),
                _ => crate::engines::claude_logout(&claude).await.map_err(|why| ("claude auth logout", why)),
            };
            if let Err((tool, why)) = out {
                crate::daemon_log!("equipe: `{tool}` failed: {why}");
            }
            let _ = internal.send(Internal::Engines(crate::engines::statuses(&claude, &codex, &store).await));
        });
    }

    // ── a lost sign-in ──

    /// A turn failed for want of `engine`'s sign-in: kept (with what it was
    /// asked) to carry on once it is signed in again, and why looked at.
    fn sign_in_lost(&mut self, engine: Engine, session: SessionId, prompt: String) {
        let member = self.sessions.get(&session).filter(|s| s.is_member_itself()).and_then(session_member);
        let fix = self.auth.entry(engine).or_default();
        match fix.stopped.iter_mut().find(|s| s.session == session) {
            Some(stopped) if !stopped.prompts.contains(&prompt) => stopped.prompts.push(prompt),
            Some(_) => {}
            None => fix.stopped.push(crate::engine_auth::Stopped { engine, session, member, prompts: vec![prompt], at: now_secs() }),
        }
        fix.since.get_or_insert(now_secs());
        self.save_stopped();
        self.check_engine(engine);
    }

    /// Look at why `engine` can't serve members here, off the loop; every
    /// screen hears it's being looked at the first time, and what was found.
    fn check_engine(&mut self, engine: Engine) {
        let fix = self.auth.entry(engine).or_default();
        if fix.checking || fix.state == AuthState::Moving {
            return;
        }
        fix.checking = true;
        // Looked at again while the card says why: it keeps saying it until something changes.
        if matches!(fix.state, AuthState::Checking | AuthState::SignedIn { .. } | AuthState::Unknown) && fix.detail.is_empty() {
            fix.state = AuthState::Checking;
            let event = fix.event(engine);
            self.publish(event);
        }
        let (claude, codex, store, internal) = (self.config.claude_binary.clone(), self.codex_binary.clone(), self.config.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            let (state, detail) = crate::engine_auth::check(engine, &claude, &codex, &store).await;
            let _ = internal.send(Internal::AuthChecked(engine, state, detail));
        });
    }

    /// What a look at `engine` found. Signed in: the turns it stopped carry
    /// on — unless they just did and failed again, when the sign-in its
    /// tool still lists no longer works and only signing in again will do.
    /// Anything else is told, and looked at again while turns wait, so an
    /// unlocked Keychain or a sign-in made at the Mac's own screen is
    /// noticed without anyone asking.
    async fn auth_checked(&mut self, engine: Engine, state: AuthState, detail: String) {
        let now = now_secs();
        let Some(fix) = self.auth.get_mut(&engine) else { return };
        fix.checking = false;
        if fix.state == AuthState::Moving {
            return;
        }
        let state = match state {
            AuthState::SignedIn { .. } if fix.carried_on_at.is_some_and(|at| now.saturating_sub(at) < CARRIED_ON_FOR) && !fix.stopped.is_empty() => AuthState::SignedOut,
            state => state,
        };
        if let AuthState::SignedIn { .. } = state {
            let stopped = std::mem::take(&mut fix.stopped);
            fix.carried_on_at = (!stopped.is_empty()).then_some(now);
            fix.since = None;
            fix.auto_started = false;
            let mut resumed = 0;
            for stopped in stopped {
                if self.carry_on_signed_in(&stopped).await {
                    resumed += 1;
                }
            }
            // A squad held by engines failing at once — a lost sign-in, likely — tries again now.
            self.try_breakers_now();
            let Some(fix) = self.auth.get_mut(&engine) else { return };
            (fix.state, fix.detail) = (AuthState::SignedIn { resumed }, String::new());
            let event = fix.event(engine);
            self.save_stopped();
            self.publish(event);
            return;
        }
        (fix.state, fix.detail) = (state, detail);
        // Signed out with work stopped on it: equipe starts the engine's own
        // sign-in itself — once for this sign-out, whoever it stopped — rather
        // than telling the person to run a command. Its page opens in
        // equipe's own browser; approving there stays the person's.
        let start = state == AuthState::SignedOut && !fix.stopped.is_empty() && !fix.auto_started && !self.sign_ins.contains_key(&engine);
        if start {
            fix.auto_started = true;
        }
        let event = fix.event(engine);
        let again = !fix.stopped.is_empty()
            && matches!(state, AuthState::KeychainLocked | AuthState::SignedOut | AuthState::Unknown)
            && fix.since.is_some_and(|since| now.saturating_sub(since) < AUTH_WATCH.as_secs());
        self.publish(event);
        if start {
            crate::daemon_log!("equipe: {} is signed out here and stopped work: equipe starts its own sign-in, once, for every member it stopped", engine_name(engine));
            self.sign_in(engine, false);
        }
        if again {
            let internal = self.internal.clone();
            tokio::spawn(async move {
                tokio::time::sleep(AUTH_RECHECK).await;
                let _ = internal.send(Internal::AuthRecheck(engine));
            });
        }
    }

    /// One stopped turn carried on, as its member is told why it stopped:
    /// its own session, or the member's as this daemon knows it. Not when
    /// it works again already, or is gone.
    async fn carry_on_signed_in(&mut self, stopped: &crate::engine_auth::Stopped) -> bool {
        let found = self
            .sessions
            .get(&stopped.session)
            .filter(|s| stopped.member.is_none() || session_member(s) == stopped.member)
            .map(|_| stopped.session)
            .or_else(|| self.sessions.iter().find(|(_, s)| s.is_member_itself() && stopped.member.is_some() && session_member(s) == stopped.member).map(|(id, _)| *id));
        let Some(id) = found.filter(|id| self.sessions.get(id).is_some_and(|s| !s.is_working())) else {
            return false;
        };
        let text = say!(self, "news.sign_in_carried_on", engine = engine_name(stopped.engine));
        self.publish(Event::Notice { undo: None, session: id, text });
        self.set_going(id, crate::engine_auth::carry_on_prompt(stopped.engine, &stopped.prompts)).await;
        true
    }

    /// A turn of `session`'s answered: nothing of it waits on a sign-in.
    fn no_longer_stopped(&mut self, session: SessionId) {
        let mut changed = false;
        for fix in self.auth.values_mut() {
            let before = fix.stopped.len();
            fix.stopped.retain(|s| s.session != session);
            changed |= fix.stopped.len() != before;
        }
        if changed {
            self.save_stopped();
        }
    }

    /// The members' stopped turns, for the daemon that comes after this one.
    fn save_stopped(&self) {
        if !self.config.persist {
            return;
        }
        let all: Vec<crate::engine_auth::Stopped> = self.auth.values().flat_map(|f| f.stopped.iter().cloned()).collect();
        crate::engine_auth::save_stopped(&self.config.home, &all);
    }

    /// What an earlier daemon kept of its members' stopped turns, matched to
    /// their sessions here, then looked at: signed in, they carry on.
    fn restore_stopped(&mut self) {
        for mut stopped in crate::engine_auth::load_stopped(&self.config.home, now_secs()) {
            let Some(id) = self.sessions.iter().find(|(_, s)| s.is_member_itself() && session_member(s) == stopped.member).map(|(id, _)| *id) else { continue };
            stopped.session = id;
            let engine = stopped.engine;
            let fix = self.auth.entry(engine).or_default();
            fix.since.get_or_insert(stopped.at);
            fix.stopped.push(stopped);
        }
        for engine in self.auth.keys() {
            let _ = self.internal.send(Internal::AuthRecheck(*engine));
        }
    }

    /// The daemon, started outside the Mac's login session, starts again
    /// inside it through its login service, as the person asked: its
    /// members' stopped turns kept for the one that comes next.
    fn move_into_login(&mut self) {
        if crate::engine_auth::in_login_session() != Some(false) {
            // Inside already: nothing to move; what's wrong is looked at.
            return self.check_engine(Engine::Claude);
        }
        let exe = std::env::current_exe();
        let (Some(_), Ok(exe)) = (crate::engine_auth::service_plist(), exe) else {
            let fix = self.auth.entry(Engine::Claude).or_default();
            fix.state = AuthState::OutsideLogin { movable: false };
            let event = fix.event(Engine::Claude);
            return self.publish(event);
        };
        self.save_stopped();
        let fix = self.auth.entry(Engine::Claude).or_default();
        match crate::engine_auth::move_into_login(&exe) {
            Ok(()) => {
                crate::daemon_log!("equipe: started outside the Mac's login session, the daemon starts again inside it, as the person asked");
                (fix.state, fix.detail) = (AuthState::Moving, String::new());
            }
            Err(err) => (fix.state, fix.detail) = (AuthState::Unknown, err.to_string()),
        }
        let event = fix.event(Engine::Claude);
        self.publish(event);
    }

    /// An engine's own sign-in page, in equipe's own browser on this
    /// machine — the browser panel of an attached screen, else its Chrome
    /// window — where the person's web sign-ins usually are already, and
    /// where the page's last step comes back to the tool's login server. No
    /// login is kept for the members at its end: it is the engine's.
    fn open_engine_page(&mut self, url: &str) -> bool {
        use equipe_protocol::BrowserSignInState as State;
        let window = !self.hub.has_panel();
        let host = self.config.npx.clone().and_then(|npx| self.browser_host(&npx));
        let opened = host.is_some_and(|host| {
            host.lock()
                .map(|mut h| {
                    if window {
                        h.person(url);
                    } else {
                        h.op(&json!({ "op": "newtab", "url": url }));
                    }
                })
                .is_ok()
        });
        if opened {
            self.publish(Event::BrowserSignIn { site: vault::site_of(url), state: State::Opened, window });
        } else {
            crate::daemon_log!("equipe: an engine's sign-in page couldn't open in equipe's browser here; its card offers it");
        }
        opened
    }

    // ── a model the Codex account refuses ──

    /// Whether the Codex account refused `model` lately, on the kind of
    /// account it is signed in with now.
    fn codex_refuses(&self, model: &str) -> bool {
        self.codex_refused.get(model).is_some_and(|(at, kind)| now_secs().saturating_sub(*at) < REFUSED_FOR && (kind.is_none() || self.codex_kind.is_none() || *kind == self.codex_kind))
    }

    /// The model a Codex turn on `model` goes to when the account refuses it:
    /// never a dearer one — the strongest cheaper family it lists (Sol goes to
    /// Luna, never up to Astra), else another version of the same family;
    /// for a model of no known family, the account's default unless that is
    /// the dearest. None refused lately; `effort` when that model takes it,
    /// else its own default effort.
    fn codex_fallback(&self, model: &str, effort: Option<&str>) -> Option<codex_app::Fallback> {
        let (_, infos) = self.codex_models.as_ref()?;
        let usable: Vec<&codex_app::ModelInfo> = infos.iter().filter(|m| m.id != model && !self.codex_refuses(&m.id)).collect();
        let pick = codex_fallback_of(model, &usable)?;
        let effort = match effort {
            Some(e) if pick.efforts.is_empty() || pick.efforts.iter().any(|x| x == e) => Some(e.to_owned()),
            _ => pick.default_effort.clone(),
        };
        Some(codex_app::Fallback { model: pick.id.clone(), effort })
    }

    /// The Codex account refused `model` for a turn of `id`'s: skipped for a
    /// while (never for good), logged, and — asked again on `instead` — the
    /// person told in a line; with nothing to fall back on, the run's failure
    /// is never taken for the engine's limit.
    fn codex_model_refused(&mut self, id: SessionId, model: String, instead: Option<String>) {
        let kind = self.codex_kind;
        if !model.is_empty() {
            self.codex_refused.insert(model.clone(), (now_secs(), kind));
        }
        let who = self.sessions.get(&id).and_then(|s| s.name().map(str::to_owned)).unwrap_or_else(|| "a conversation".to_owned());
        let account = match kind {
            Some(codex_app::SignIn::ChatGpt) => "a ChatGPT plan",
            Some(codex_app::SignIn::ApiKey) => "an API key",
            _ => "this account",
        };
        match instead {
            Some(instead) => {
                crate::daemon_log!("equipe: Codex refused {model} for {who}'s turn (model_not_found, on {account}): asked again on {instead}; {model} is skipped for {} min", REFUSED_FOR / 60);
                let text = say!(self, "news.codex_model_fallback", model = model, instead = instead);
                self.publish(Event::Notice { undo: None, session: id, text });
            }
            None => {
                crate::daemon_log!("equipe: Codex refused {model} for {who}'s turn (model_not_found, on {account}), with no other model listed to ask; {model} is skipped for {} min", REFUSED_FOR / 60);
                self.model_refused_turns.insert(id);
            }
        }
        if self.codex_models.is_none() {
            self.models_asked(Engine::Codex);
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

    /// The person deletes `squad`. Its members stop as they do when the daemon
    /// stops: their turns, browsers, side chats and helpers end, and every
    /// screen hears they closed. What the journal holds of them stays as it
    /// was, so a squad whose folder is moved back is whole again. Its journal
    /// closes, nothing opens it again meanwhile, its checks waiting in line
    /// are dropped, and off the loop what can be made again goes and its
    /// folder moves — or, `purge`, goes too ([`crate::squads::delete`]).
    fn delete_squad(&mut self, squad: &str, purge: bool) {
        let home = self.config.home.clone();
        if !crate::squads::exists(&home, squad) || crate::squads::is_gone(&home, squad) {
            let why = say!(self, "error.no_squad", squad = squad);
            return self.publish(Event::SquadNotDeleted { squad: squad.to_owned(), why });
        }
        crate::daemon_log!("equipe: deleting the squad {squad}");
        crate::squads::mark_going(&home, squad, true);
        // Closed first: what the members' ends say isn't written into it.
        self.journals.remove(squad);
        // Lanes before the members they belong to.
        let mut ids: Vec<(bool, SessionId)> = self.sessions.iter().filter(|(_, s)| s.squad.as_deref() == Some(squad)).map(|(&id, s)| (s.is_member_itself(), id)).collect();
        ids.sort();
        let mut desktop_changed = false;
        for (_, id) in ids {
            self.stop_turn(id);
            self.close_side_chats_of(id);
            self.publish(Event::SessionClosed { session: id });
            self.dispatch_deferred.remove(&id);
            if let Some(who) = self.sessions.remove(&id).as_ref().and_then(session_member) {
                desktop_changed |= self.desktop.leave(&who);
            }
        }
        if desktop_changed {
            self.publish_desktop();
        }
        // What waited on the person there waits no more.
        self.pending.retain(|_, p| p.squad != squad);
        self.browsers.retain(|(q, _), _| q != squad);
        self.last_url.retain(|(q, _), _| q != squad);
        self.show_url.retain(|(q, _), _| q != squad);
        let of_squad = |lane: &String| lane.split_once('/').is_some_and(|(q, _)| q == squad);
        self.held.retain(|lane, _| !of_squad(lane));
        self.checking.retain(|lane, _| !of_squad(lane));
        self.asked_in.retain(|lane, _| !of_squad(lane));
        self.browser_back.retain(|(q, _), _| q != squad);
        self.focus.remove(squad);
        self.watching.remove(squad);
        self.frame_watchers.remove(squad);
        self.questions.remove(squad);
        self.team_changes.remove(squad);
        self.routine_proposals.remove(squad);
        self.accepts.remove(squad);
        self.accepts_declined.remove(squad);
        self.design_watch.remove(squad);
        self.signals_running.retain(|(q, _)| q != squad);
        self.deploys_looking.remove(squad);
        self.go_live.remove(squad);
        // Its checks waiting in line: none will run. One running ends on its own.
        for queue in self.gates.values_mut() {
            let mut at = 0;
            queue.retain(|g| {
                at += 1;
                at == 1 || g.squad != squad
            });
        }
        let (name, internal) = (squad.to_owned(), self.internal.clone());
        // Worktrees and folders: off the loop.
        std::thread::spawn(move || {
            let result = crate::squads::delete(&home, &name, now_secs(), purge);
            crate::squads::mark_going(&home, &name, false);
            let _ = internal.send(Internal::SquadDeleted { squad: name, result });
        });
    }

    /// A squad deleted, or not: every screen hears it.
    fn squad_deleted(&mut self, squad: &str, result: Result<crate::squads::Deleted, String>) {
        match result {
            Ok(deleted) => {
                let folder = if deleted.purged { "deleted for good".to_owned() } else { format!("kept at {} for {} days", deleted.moved_to.display(), crate::squads::KEPT_DAYS) };
                crate::daemon_log!(
                    "equipe: deleted the squad {squad}: {} MB given back; its folder {folder}; {} worktrees removed, {} kept for other squads",
                    deleted.freed / 1_000_000,
                    deleted.worktrees_removed.len(),
                    deleted.worktrees_kept.len()
                );
                self.publish(Event::SquadDeleted { squad: squad.to_owned(), moved_to: deleted.moved_to, next: deleted.next, freed: deleted.freed, purged: deleted.purged });
            }
            Err(why) => {
                crate::daemon_log!("equipe: the squad {squad} wasn't deleted: {why}");
                let why = say!(self, "error.squad_not_deleted", why = why);
                self.publish(Event::SquadNotDeleted { squad: squad.to_owned(), why });
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

    /// How many subagents a member of `squad` runs at once, where the person
    /// set it (`Journal::subagents_at_once`); `None`, each engine's own.
    fn subagents_at_once(&self, squad: Option<&str>) -> Option<u32> {
        squad.and_then(|s| self.journals.get(s)).and_then(Journal::subagents_at_once)
    }

    /// An anonymous usage event of `squad`'s, with its team's size and engines;
    /// nothing goes out until the person said yes.
    fn tell(&self, squad: &str, happened: crate::telemetry::Happened) {
        let members: Vec<&Session> = self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad) && s.name().is_some()).collect();
        let mut engines: Vec<Engine> = members.iter().map(|s| s.engine).collect();
        engines.sort_by_key(|e| *e as u8);
        engines.dedup();
        self.telemetry.record(squad, &crate::telemetry::Shape { team_size: members.len(), engines }, happened);
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

    /// A role that brings a weekly routine (security's check, marketing's)
    /// adds it on joining — once: the person changes or deletes it like any
    /// routine, and it stays so.
    fn add_joining_routine(&mut self, squad: &str, role: &str) {
        let lang = crate::i18n::Lang::resolve_in(&self.config.home, crate::i18n::Front::App);
        let Some(journal) = self.journal(squad) else { return };
        if journal.role_routine_added(role) {
            return;
        }
        let mut all = journal.routines();
        let id = all.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let Some(mut routine) = crate::heartbeat::joining_routine(role, lang, id) else { return };
        if all.len() >= MAX_ROUTINES {
            crate::daemon_log!("equipe: {squad}: no room for {role}'s weekly routine ({MAX_ROUTINES} routines)");
            return;
        }
        routine.next_run = Some(routines::next_run(routine.when, now_secs(), &routines::Local));
        all.push(routine);
        if let Err(err) = journal.set_role_routine_added(role) {
            crate::daemon_log!("equipe: {squad}: {role}'s routine wasn't marked added: {err}");
        }
        if role == crate::design::review::ROLE
            && let Err(err) = journal.set_design_review_routine(Some(id))
        {
            crate::daemon_log!("equipe: {squad}: the design review wasn't marked: {err}");
        }
        if role == crate::retro::ROLE
            && let Err(err) = journal.set_retro_routine(Some(id))
        {
            crate::daemon_log!("equipe: {squad}: the weekly retro wasn't marked: {err}");
        }
        self.keep_routines(squad, all);
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
            // The review deleted: a routine numbered as it was, later, is no review.
            if let Some(journal) = self.journal(squad).filter(|j| j.design_review_routine() == Some(id)) {
                let _ = journal.set_design_review_routine(None);
            }
            // So with the retro.
            if let Some(journal) = self.journal(squad).filter(|j| j.retro_routine() == Some(id)) {
                let _ = journal.set_retro_routine(None);
            }
            self.keep_routines(squad, all);
        }
    }

    /// A routine handed to the PM as the person's message — after its turn
    /// if one runs — about the HQ when the squad has one; the weekly retro
    /// with the week's facts (`retro`); the design review, to the designer
    /// instead (`review`). `false`: no PM to hand it to.
    async fn run_routine(&mut self, squad: &str, id: u32, now: u64) -> bool {
        let mut all = self.routines(squad);
        let Some(routine) = all.iter().find(|r| r.id == id).cloned() else {
            return false;
        };
        if self.journal(squad).and_then(|j| j.design_review_routine()) == Some(id) {
            self.run_design_review(squad, &routine, now).await;
        } else {
            let Some(pm) = self.pm_of(squad) else {
                return false;
            };
            crate::daemon_log!("equipe: {squad}: routine {} ({}) handed to the PM", routine.id, routine.title);
            let about = self.hq(squad).map(|hq| board::project_name(&hq));
            let retro = self.journal(squad).and_then(|j| j.retro_routine()) == Some(id);
            // The sprint loop's review is the retro while it runs: not two in a week.
            if retro && self.sprint_on(squad).is_ok() {
                crate::daemon_log!("equipe: {squad}: the weekly retro is the sprint review's while the sprint loop runs");
                if let Some(r) = all.iter_mut().find(|r| r.id == id) {
                    r.last_run = Some(now);
                    r.next_run = Some(routines::next_run(r.when, now, &routines::Local));
                }
                self.keep_routines(squad, all);
                return true;
            }
            let mut text = if retro { self.retro_message(squad, &routine, now) } else { routines::message(&routine) };
            if retro {
                self.retros.insert(pm, now);
            }
            let signals = self.journal(squad).map_or(0, |j| j.signals().len());
            if signals > 0 {
                text.push_str(&format!("\n\n(The squad keeps {signals} signal(s): read them with `signals` — what fails, what moved.)"));
            }
            self.said_about = Some(about);
            match self.sessions.get(&pm) {
                Some(s) if s.is_working() => self.leave_memo(pm, text),
                _ => self.submit(pm, text).await,
            }
            self.said_about = None;
        }
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

    /// The squad's board, shared: kept in memory and brought up to date
    /// ticket by ticket ([`Journal::board`]), so asking is cheap however big
    /// the board grew. `.to_vec()` for a copy to change.
    fn tickets(&mut self, squad: &str) -> std::sync::Arc<Vec<Ticket>> {
        self.journal(squad).and_then(|j| j.board().ok()).unwrap_or_default()
    }

    /// The member of `squad` called `name`.
    fn member_id(&self, squad: &str, name: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.name() == Some(name))
            .map(|(&id, _)| id)
    }

    /// Member `member`'s ticket runs, by their place ([`Session::slot`]).
    fn runs_of(&self, member: SessionId) -> Vec<SessionId> {
        let mut runs: Vec<(usize, SessionId)> = self.sessions.iter().filter(|(_, s)| s.run_of == Some(member)).map(|(&id, s)| (s.slot, id)).collect();
        runs.sort_unstable();
        runs.into_iter().map(|(_, id)| id).collect()
    }

    /// The member a session is: a run's or a lane's own, else itself.
    fn member_of(&self, id: SessionId) -> SessionId {
        self.sessions.get(&id).and_then(|s| s.run_of.or(s.lane.as_ref().map(|(of, _)| *of))).unwrap_or(id)
    }

    /// Where a session builds: its member's own place, or — for a run
    /// beside another — one of the run's place's own, so two builds at once
    /// never overwrite each other's binaries.
    fn build_dir_of(&self, session: &Session) -> Option<PathBuf> {
        let (squad, name) = (session.squad.as_deref()?, session.name()?);
        let key = if session.slot == 0 { name.to_owned() } else { format!("{name}~{}", session.slot + 1) };
        Some(journal::build_dir(&self.config.home, squad, &key))
    }

    /// Run member `member`'s ticket `work` in a session of its own, beside
    /// the member's conversation and its other runs ([`crate::lanes`]): the
    /// run's session is the work's runtime session. Told to the screens
    /// before it starts ([`Event::RunOpened`], then [`Event::SessionStarted`]).
    fn open_run(&mut self, member: SessionId, work: Work) -> Option<SessionId> {
        let id = work.execution_session;
        // The model and effort its ticket's lane was picked to start on, when on this engine.
        let squad = self.sessions.get(&member).and_then(|m| m.squad.clone());
        let picked = squad.and_then(|squad| self.journals.get(&squad)).and_then(|j| j.ticket(work.ticket).ok().flatten()).and_then(|t| t.model);
        let used: Vec<usize> = self.sessions.values().filter(|s| s.run_of == Some(member)).map(|s| s.slot).collect();
        let slot = (0..).find(|n| !used.contains(n)).unwrap_or_default();
        let m = self.sessions.get(&member)?;
        let mut run = lane_of(member, m, None, DeskThread::default(), false);
        run.lane = None;
        run.run_of = Some(member);
        run.slot = slot;
        run.thread = None;
        run.effort = m.effort.clone();
        if let Some(picked) = picked.filter(|p| p.engine == Some(run.engine)) {
            run.model = picked.model;
            run.effort = picked.effort;
        }
        run.context = None;
        run.desk_context = None;
        let (ticket, generation) = (work.ticket, work.execution_generation);
        run.work = Some(work);
        let started = Event::SessionStarted {
            session: id,
            project: run.project.clone(),
            engine: run.engine,
            model: run.model.clone(),
            workspace: None,
            member: run.member.clone(),
            squad: run.squad.clone(),
        };
        self.sessions.insert(id, run);
        self.publish(Event::RunOpened { session: id, of: member, ticket, generation });
        self.publish(started);
        Some(id)
    }

    /// Run `id` put its ticket down: it closes. What was left for it and
    /// not read goes to its member's conversation; what it asked the person
    /// and is still unanswered is answered no — no other run takes it.
    fn close_run(&mut self, id: SessionId) {
        let Some(member) = self.sessions.get(&id).filter(|s| s.work.is_none()).and_then(|s| s.run_of) else { return };
        self.stop_turn(id);
        let asked: Vec<u64> = self.pending.iter().filter(|(_, p)| p.approval.session == id).map(|(&a, _)| a).collect();
        for approval in asked {
            self.answer(approval, Decision::Deny, "That ticket's run ended.");
        }
        let (memos, threads) = self.sessions.get_mut(&id).map(|s| (std::mem::take(&mut s.queued), std::mem::take(&mut s.memo_threads))).unwrap_or_default();
        for (i, text) in memos.into_iter().enumerate() {
            self.leave_memo_about(member, text, threads.get(i).cloned().flatten());
        }
        self.close_side_chats_of(id);
        self.publish(Event::SessionClosed { session: id });
        if let Some(who) = self.sessions.remove(&id).as_ref().and_then(session_member)
            && self.desktop.leave(&who)
        {
            self.publish_desktop();
        }
    }

    /// Who a tool call comes from: the run it names, while that run still
    /// holds its ticket's current generation; the member itself when it
    /// names none. `Err` for a run that holds it no longer — a stale call.
    /// `metrics_query`: a PM, data or marketing member's ad-hoc read-only
    /// query, asked through the daemon's metrics (which hold the key) off the
    /// runtime's loop.
    fn metrics_query(&mut self, call: &ToolCall, reply: oneshot::Sender<ToolResult>) {
        let role = self.member_id(&call.squad, &call.member).and_then(|id| self.sessions.get(&id)).and_then(|s| s.member.as_ref()).map(|m| m.role.clone());
        if !role.as_deref().is_some_and(crate::metrics::tool::may_query) {
            let _ = reply.send(ToolResult::error("Only the PM and the data and marketing members ask ad-hoc queries; ask the PM, or read the kept numbers with `metrics`."));
            return;
        }
        let params = match crate::metrics::tool::query_params(&call.squad, &call.arguments) {
            Ok(p) => p,
            Err(e) => {
                let _ = reply.send(ToolResult::error(e));
                return;
            }
        };
        let metrics = crate::metrics::shared(&self.config.home);
        tokio::spawn(async move {
            let result = match metrics.hogql(&params, now_secs()).await {
                Ok(r) => ToolResult { text: crate::metrics::tool::query_answer(&r), is_error: false, image: None },
                Err(e) => ToolResult::error(e.message),
            };
            let _ = reply.send(result);
        });
    }

    fn caller(&self, call: &ToolCall) -> Result<SessionId, String> {
        let Some(member) = self.member_id(&call.squad, &call.member) else {
            return Err(format!("{} isn't on the squad {}.", call.member, call.squad));
        };
        let Some(run) = call.run else {
            // No run named. equipe names one for every run it starts, so this
            // is the member's own conversation — unless that isn't at work: a
            // call from an older board server, made for the one ticket the
            // member runs. With several, none is picked: no run is changed.
            let desk_at_work = self.sessions.get(&member).is_some_and(Session::is_working);
            return Ok(match self.runs_of(member).as_slice() {
                [only] if !desk_at_work => *only,
                _ => member,
            });
        };
        let live = self.sessions.get(&run.session).is_some_and(|s| {
            s.run_of == Some(member) && s.work.as_ref().is_some_and(|w| w.ticket == run.ticket && w.execution_generation == run.generation)
        });
        let current = self
            .journals
            .get(&call.squad)
            .and_then(|j| j.execution(run.ticket).ok().flatten())
            .is_some_and(|e| e.generation == run.generation && e.state == ExecutionState::Active && e.session == run.session);
        if live && current {
            Ok(run.session)
        } else {
            Err(format!(
                "That run of T-{} (generation {}) is over: the ticket moved on without it, so nothing was changed. Stop here.",
                run.ticket, run.generation
            ))
        }
    }

    /// The session a tool call acts for: its run when it is still current,
    /// else the member's own (a stale run's state-changing calls are refused
    /// before they get here, [`Self::tool`]).
    fn caller_id(&self, call: &ToolCall) -> Option<SessionId> {
        self.caller(call).ok().or_else(|| self.member_id(&call.squad, &call.member))
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
            .filter(|s| s.squad.as_deref() == Some(squad) && s.is_member_itself())
            .filter_map(|s| s.member.as_ref().map(|m| (s.joined, m.name.clone(), m.role.clone())))
            .collect();
        members.sort();
        members
    }

    fn pm_of(&self, squad: &str) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.is_pm())
            .map(|(&id, _)| id)
    }

    /// Tell `squad`'s PM `line` with its next message; with `shown`, its
    /// conversation also shows what happened, so a screen on the PM sees it.
    /// Landing `ticket` kept design captures out of git, or found a commit
    /// heavy with images: its member hears what to do instead, the PM and
    /// the person what happened.
    fn captures_kept_out(&mut self, squad: &str, ticket: &Ticket, kept: &crate::design::captures::Report) {
        let line = kept.line(ticket.id.into());
        let size = crate::design::captures::size_words;
        let mut shown = Vec::new();
        if kept.kept_out > 0 {
            shown.push(say!(self, "news.captures_kept_out", id = ticket.id, count = kept.kept_out, size = size(kept.bytes)));
        }
        if kept.handed_in_kept_out > 0 {
            shown.push(say!(self, "news.handed_in_kept_out", id = ticket.id, count = kept.handed_in_kept_out, size = size(kept.handed_in_bytes)));
        }
        for heavy in &kept.heavy {
            shown.push(say!(self, "news.captures_heavy", id = ticket.id, size = size(heavy.bytes), commit = &heavy.commit[..heavy.commit.len().min(10)]));
        }
        let shown = shown.join("\n");
        if let Some(journal) = self.journal(squad) {
            let _ = journal.ticket_event(ticket.id, "equipe", "design captures kept out of git", &line);
        }
        if let Some(member) = ticket.owner.clone().or_else(|| ticket.assignee.clone()) {
            self.brief_member(squad, &member, &line, &shown);
        }
        self.brief_pm(squad, line, Some(shown));
    }

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
        self.show_news_about(pm, text, None);
    }

    /// [`Runtime::show_news`], about `project` when it is given — else the
    /// project of the ticket the news names.
    fn show_news_about(&mut self, pm: SessionId, text: String, project: Option<String>) {
        // Not into a conversation the person opened: those are theirs.
        let at = self.sessions.iter().find(|(_, s)| !s.kept && s.lane.as_ref().is_some_and(|(of, _)| *of == pm)).map_or(pm, |(&lane, _)| lane);
        let desk = self.sessions.get(&at).filter(|s| s.work.is_none()).map(|s| (scope::current(s.transcript.entries()).flatten(), s.is_working(), s.squad.clone()));
        let Some((now, working, Some(squad))) = desk else {
            return self.publish(Event::Notice { undo: None, session: at, text });
        };
        let about = match project {
            Some(project) => Some(project),
            None => self.news_project(&squad, &text),
        };
        if about == now {
            return self.publish(Event::Notice { undo: None, session: at, text });
        }
        self.publish(Event::Thread { session: at, project: about });
        self.publish(Event::Notice { undo: None, session: at, text });
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
        self.refresh_nemotron();
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

        // The role set to a provider's model (`@openrouter/…`): on it, whichever
        // engine the screen set it for — it runs on Codex's agent either way.
        let role_open = match (&squad, role, &spec.model, spec.engine) {
            (Some(squad), Some(role), None, None) => self
                .journal(squad)
                .and_then(|j| j.role_models().into_iter().find(|d| d.role == role.id && d.model.as_deref().is_some_and(|m| crate::model_providers::split_ref(m).is_some() && crate::nemotron::is_agent_model(m)))),
            _ => None,
        };
        // A role set to an Anthropic-compatible provider's model runs on Claude Code on it.
        let role_claude = match (&squad, role, &spec.model, spec.engine) {
            (Some(squad), Some(role), None, None) if role_open.is_none() => self
                .journal(squad)
                .and_then(|j| j.role_models().into_iter().find(|d| d.role == role.id && d.model.as_deref().and_then(crate::model_providers::engine_of_ref) == Some(Engine::Claude))),
            _ => None,
        };
        // NVIDIA's models on Nebius Token Factory, and every provider's, run on Codex's agent (`nemotron`).
        let mut on_nebius = spec.model.as_deref().or(role_open.as_ref().and_then(|d| d.model.as_deref())).is_some_and(crate::nemotron::is_agent_model);
        let engine = match spec.engine {
            // Named, but nothing at all is signed in: Nemotron, rather than failing.
            Some(_) if !on_nebius && self.nebius_ready() && self.nothing_signed_in().await => {
                on_nebius = true;
                Engine::Codex
            }
            Some(engine) => engine,
            None if on_nebius => Engine::Codex,
            None if role_claude.is_some() || spec.model.as_deref().and_then(crate::model_providers::engine_of_ref) == Some(Engine::Claude) => Engine::Claude,
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
                        // A terminal's way; a screen says Settings › AI platforms in the message.
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
            let message = match spec.model.as_deref().or(role_open.as_ref().and_then(|d| d.model.as_deref())).and_then(|m| self.on_provider(m)) {
                Some((_, provider)) => say!(self, "error.provider_needs_codex", provider = provider),
                None => say!(self, "error.nebius_needs_codex"),
            };
            return fail(self, message, Some(crate::engines::installer(Engine::Codex)));
        }
        if spec.access == Access::EditFiles && engine == Engine::Codex && !codex_cli {
            return fail(self, say!(self, "error.codex_edits"), Some("codex login"));
        }

        // What the person set for the role on this engine, if anything.
        let role_default = match (&squad, role) {
            _ if role_open.is_some() && engine == Engine::Codex => role_open,
            _ if role_claude.is_some() && engine == Engine::Claude => role_claude,
            (Some(squad), Some(role)) => self.journal(squad).and_then(|j| j.role_models().into_iter().find(|d| d.role == role.id && d.engine == engine)),
            _ => None,
        };
        // A model named in the spec is the caller's choice; a role default applied to a
        // member that starts without one is automatic, so never the premium family.
        let mut model = spec.model.clone().or_else(|| crate::tiers::cap_premium(engine, role_default.as_ref().and_then(|d| d.model.as_deref()), false));
        let role_effort = role_default.and_then(|d| d.effort);
        let mut codex_cred = None;
        match engine {
            Engine::Codex if codex_cli => {
                if !on_nebius {
                    match codex_app::login_status(&self.config.codex_binary).await {
                        Ok(Some(sign_in)) if sign_in.signed_in() => self.codex_kind = Some(sign_in),
                        // Signed out, nothing else signed in, and a Token Factory key: Nemotron instead of failing.
                        Ok(_) if self.config.nemotron.is_some() && self.nothing_signed_in().await => on_nebius = true,
                        Ok(_) => return fail(self, say!(self, "error.codex_cli_signed_out"), Some("equipe login codex")),
                        Err(err) => return fail(self, err.to_string(), Some("equipe status")),
                    }
                }
                if on_nebius {
                    let cfg = match self.open_config(model.as_deref()) {
                        Ok(Some(cfg)) => cfg,
                        Ok(None) => return fail(self, say!(self, "error.nebius_no_key"), Some("equipe login nebius")),
                        Err(message) => return fail(self, message, None),
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
            // A model on an Anthropic-compatible provider needs Claude Code itself,
            // not the person's Claude sign-in: its key is the provider's.
            Engine::Claude if model.as_deref().and_then(crate::model_providers::engine_of_ref) == Some(Engine::Claude) => {
                if let Some(Err(message)) = self.claude_provider(model.as_deref()) {
                    return fail(self, message, None);
                }
                if equipe_providers::tools_path::find(&self.config.claude_binary).is_none() {
                    return fail(self, say!(self, "error.claude_missing"), Some("see https://code.claude.com/docs/en/setup"));
                }
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
            && let Some(role) = role
        {
            self.add_joining_routine(squad, role.id);
        }

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
                ran: None,
                access,
                cwd,
                sandbox,
                codex: codex_cred,
                history: Vec::new(),
                claude_session: None,
                work: None,
                prompts: Vec::new(),
                title: None,
                title_started: false,
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
                stop_subagent: None,
                work_over: None,
                wind_down: None,
                taken_back: Vec::new(),
                send_now_prompt: None,
                told: Default::default(),
                subagents: Vec::new(),
                compacting: None,
                answers_person: false,
                on_phone: false,
                effort: role_effort,
                briefing: Vec::new(),
                memory,
                mode: None,
                thread: None,
                threads: HashMap::new(),
                lane: None,
                kept: false,
                run_of: None,
                slot: 0,
                estimate: None,
                turn_began: None,
                watch: TurnWatch::default(),
            },
        );
        self.publish(Event::SessionStarted { session: id, project, engine, model: model.clone(), workspace, member, squad: squad.clone() });
        if on_nebius {
            let model = model.unwrap_or_default();
            let named = self.on_provider(&model);
            let text = match &self.config.nemotron {
                // A provider the person connected, by name.
                _ if named.is_some() => {
                    let (shown, provider) = named.clone().unwrap_or_default();
                    let nemotron = crate::model_providers::split_ref(&model).is_some_and(|(p, m)| p == crate::model_providers::NEBIUS && m.to_ascii_lowercase().starts_with("nvidia/"));
                    if nemotron { say!(self, "notice.on_nebius", model = shown) } else { say!(self, "notice.on_open_model", model = shown, provider = provider) }
                }
                // Token Factory and one of NVIDIA's: Nemotron, said as such.
                Some(cfg) if cfg.provider == crate::nemotron::CODEX_PROVIDER && model.to_ascii_lowercase().starts_with("nvidia/") => say!(self, "notice.on_nebius", model = model),
                Some(cfg) => say!(self, "notice.on_open_model", model = model, provider = cfg.provider_name.clone()),
                None => say!(self, "notice.on_nebius", model = model),
            };
            self.publish(Event::Notice { undo: None, session: id, text });
            // What it can't do there yet, so the person isn't surprised.
            let provider = named.map(|(_, p)| p).or_else(|| self.config.nemotron.as_ref().map(|c| c.provider_name.clone())).unwrap_or_else(|| crate::nemotron::PROVIDER_NAME.to_owned());
            let text = say!(self, "notice.provider_limits", provider = provider);
            self.publish(Event::Notice { undo: None, session: id, text });
        }
        // A new member may be just who a waiting ticket needs.
        if let Some(squad) = squad
            && !self.dispatch_deferred.remove(&id)
        {
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
        // The person waits on a run whose workspace is still in line: it goes first.
        if self.sessions.get(&id).is_some_and(|s| s.is_preparing()) {
            self.hurry_workspace(id);
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return self.error(Some(id), say!(self, "error.no_session", id = id), None);
        };
        if session.is_working() || session.is_preparing() {
            // The same design session kickoff again while its turn runs is sent
            // once: twice, it reads as a block of instructions repeated. On
            // Codex a kickoff opens a turn of its own, never cut into one as a
            // steer, so its framing goes apart from the person's words.
            if crate::design::session::split_kickoff(&text).is_some() {
                if session.prompts.last() == Some(&text) {
                    crate::daemon_log!("equipe: session {id} got the same design session kickoff while its turn runs; sent once");
                    return;
                }
                if session.engine == Engine::Codex {
                    return self.leave_memo(id, text);
                }
            }
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
        let phone = self.sessions.get(&id).is_some_and(|s| s.on_phone);
        steers::words(text, project, whole, phone)
    }

    /// The person's word to `id` came from the screen it came from: their
    /// phone ([`Command::OnPhone`]) or not. Its answers are for that screen.
    pub(super) fn note_screen(&mut self, id: SessionId) {
        let phone = self.on_phone;
        if let Some(session) = self.sessions.get_mut(&id) {
            session.on_phone = phone;
        }
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
            self.publish(Event::Steered { session: id, text, kept: Vec::new() });
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

    /// Have what waits for `id` read now ([`Command::SendNow`]) — never by
    /// stopping the work: a message is for the member to weigh, not the end
    /// of what it does. At work, the memos go in as steers, read at its next
    /// step with those already sent; a command it runs now is stopped (just
    /// it), so that step comes at once. An engine that can't read mid-turn
    /// keeps them for the turn's end, as memos are. With no turn under way,
    /// what waits goes to it now as one message.
    async fn send_now(&mut self, id: SessionId, first: Vec<String>) {
        let waits = |s: &Session| !s.steering.is_empty() || !s.queued.is_empty();
        let Some(id) = self.holder(id, waits) else {
            return;
        };
        if self.sessions.get(&id).is_some_and(Session::is_working) {
            self.memos_to_steers(id, &first);
            if self.command_running(id) {
                self.stop_command(id);
            }
            return;
        }
        self.deliver_waiting(id, first).await;
    }

    /// The memos waiting for `id`, at work, sent into its turn as steers —
    /// `first` in its order, then the rest — each with its project. Where the
    /// engine can't take one, it stays a memo.
    fn memos_to_steers(&mut self, id: SessionId, first: &[String]) {
        let Some(session) = self.sessions.get_mut(&id) else { return };
        let Some(steer) = session.steer.clone() else { return };
        let memos: Vec<(String, Option<String>)> =
            std::mem::take(&mut session.queued).into_iter().zip(std::mem::take(&mut session.memo_threads).into_iter().chain(std::iter::repeat(None))).collect();
        for (text, project) in steers::order(memos, first) {
            let said = self.steer_words(id, &text, project.as_deref());
            let Some(session) = self.sessions.get_mut(&id) else { return };
            if steer.send(said).is_ok() {
                session.steering.push(text.clone());
                session.steer_threads.push(project.clone());
                self.publish(Event::MemoWithdrawn { session: id, text: text.clone() });
                self.publish(Event::SteerQueued { session: id, text, project });
            } else {
                session.queued.push(text);
                session.memo_threads.push(project);
            }
        }
        self.save_waiting(id);
    }

    /// What waits for `id`, with no turn under way: the steers not read and
    /// the memos — `first` in its order, then the rest — go to it together,
    /// as one message about their project (each part with its own, when they
    /// are about several).
    async fn deliver_waiting(&mut self, id: SessionId, first: Vec<String>) {
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
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.queued.insert(0, text);
        session.memo_threads.insert(0, project);
        session.send_now_prompt = prompt;
        // A turn that finished already ends as it lands, delivering this first.
        let landing = session.turn.is_some();
        if !landing {
            // What waits delivered — this message first.
            self.after_turn(id).await;
        }
        // What went into the finished turn unread is in the message now: an
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
        self.publish(Event::UserMessage { session: id, text: text.clone(), kept: Vec::new() });
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
        // What waits on the person, the most blocking first: the PM reminds them of the right things.
        let waiting = self.sessions.get(&id).filter(|s| s.is_pm() && s.work.is_none()).and_then(|s| s.squad.clone()).and_then(|squad| self.waiting_note(&squad));
        // Its pages the person has open in a live preview: their servers stay up.
        let previewing = self.preview_note(id);
        // What the person asked for in this conversation, and where each stands.
        let requests = self.requests_context(id);
        // The kinds of work the person stopped: nothing of them is filed.
        let stops = self.sessions.get(&id).and_then(|s| s.squad.clone()).and_then(|squad| self.stops_context(&squad));
        // A design session on: each turn is a quick prototype round.
        let designing = self.design_note(id);
        // Its cards still open: a message that settles one has it withdrawn, not answered for the person.
        let cards = self.open_cards_note(id);
        // The squad's sprint, at its desk: the PM plans and reviews by it.
        let sprint = self.sessions.get(&id).filter(|s| s.is_pm() && s.work.is_none()).and_then(|s| s.squad.clone()).and_then(|squad| self.sprint_context(&squad));
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.prompts.push(text.clone());
        // A message sent at once about several projects: each part says its own.
        let text = session.send_now_prompt.take().unwrap_or(text);
        // What the PM missed travels with its next message; the person sees only theirs.
        let prompt = match std::mem::take(&mut session.briefing) {
            briefing if briefing.is_empty() => text,
            briefing => format!(
                "[equipe: since your last turn —\n{}]\n\n{text}",
                briefing.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")
            ),
        };
        // What goes in front of it, in the order the message reads: the
        // project it is about first (the conversation reads that back), then
        // the phone mark; then the blocks said once per conversation and
        // after that only as they change (`turn_context`).
        let mut parts: Vec<turn_context::Part> = Vec::new();
        let whole = about.is_none() && overview.is_some();
        if let Some(project) = &about {
            parts.push(turn_context::Part::every_time("scope", format!("[equipe: the person is looking at the project {project} — new work is for it unless they say otherwise]")));
        } else if whole {
            parts.push(turn_context::Part::every_time("scope", turn_context::WHOLE_SQUAD_FRAMING.to_owned()));
        }
        // Read on the phone: the member answers for its screen.
        if session.on_phone {
            parts.push(turn_context::Part::every_time("phone", crate::roles::PHONE_MARK.to_owned()));
        }
        if whole {
            parts.push(turn_context::Part::told("overview", overview.map(|o| format!("Where each project stands now:\n{o}"))).still("Where each project stands: as told earlier in this conversation, unchanged."));
        }
        parts.push(turn_context::Part::told("cards", cards).gone("None of your decision cards is open now."));
        parts.push(turn_context::Part::told("sprint", sprint));
        parts.push(turn_context::Part::told("design", designing));
        parts.push(turn_context::Part::told("stops", stops.map(|s| turn_context::inner(&s))).gone("No stop is in force now: the person lifted them."));
        parts.push(turn_context::Part::told("requests", requests.map(|r| turn_context::inner(&r))).gone("None of the person's requests you were told of is open now."));
        parts.push(turn_context::Part::told("previewing", previewing));
        if self.sessions.get(&id).is_some_and(|s| s.is_pm() && s.work.is_none()) {
            let still = waiting.as_deref().map(waiting_still);
            let mut part = turn_context::Part::told("waiting", waiting).gone("Nothing waits on the person now.");
            part.still = still;
            parts.push(part);
        }
        self.save_waiting(id);
        self.start_turn_with(id, parts, prompt);
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
            line.push_str(&turn_context::project_tickets(&here));
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
        // Sent again by a screen that didn't hear it was made (a reconnect
        // between the two): it is made already, and that is all.
        if self.sessions.get(&id).is_some_and(|s| s.kept && s.squad.as_deref() == Some(squad)) {
            return;
        }
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

    /// The person names a conversation with the PM themselves: kept (the
    /// journal's conversation and session together) and told to every screen,
    /// or refused as it was — no such conversation in `squad`, nothing left of
    /// the name or too long a one, a write that failed.
    fn rename_conversation(&mut self, squad: &str, id: SessionId, raw: &str) {
        let ours = self.sessions.get(&id).is_some_and(|s| s.kept && s.squad.as_deref() == Some(squad));
        let Some(title) = equipe_protocol::conversation_title(raw).filter(|_| ours) else {
            return self.publish(Event::ConversationRenameRefused { session: id });
        };
        if self.config.persist {
            let kept = journal_for(&mut self.journals, &self.config.home, squad).map(|j| j.rename_conversation(id, &title));
            if !matches!(kept, Some(Ok(true))) {
                return self.publish(Event::ConversationRenameRefused { session: id });
            }
        }
        if let Some(s) = self.sessions.get_mut(&id) {
            s.title = Some(title.clone());
        }
        self.publish(Event::ConversationTitle { session: id, title, named: true });
    }

    /// A conversation with the PM put away: its turn stops, it leaves the list.
    async fn close_conversation(&mut self, id: SessionId) {
        if let Some(squad) = self.put_one_away(id, false).await {
            self.publish_put_away(&squad);
        }
    }

    /// A conversation put away is taken up again, its lines as they were left.
    fn reopen_conversation(&mut self, squad: &str, id: SessionId) {
        if self.sessions.contains_key(&id) {
            return;
        }
        let Some(pm) = self.pm_of(squad) else {
            return self.squad_error(squad, say!(self, "error.no_member", name = roles::PM));
        };
        let Some(journal) = journal_for(&mut self.journals, &self.config.home, squad) else { return };
        let Ok(Some(c)) = journal.reopen_conversation(id) else { return };
        let (lines, lines_at) = journal.conversation_at(id).unwrap_or_default();
        let Some(m) = self.sessions.get(&pm) else { return };
        let thread = DeskThread { claude_session: c.engine_session, ..DeskThread::default() };
        let mut lane = lane_of(pm, m, c.project.clone(), thread, true);
        // One whose topic was never named (made before topics, or its naming
        // turn didn't finish) shows the person's first words meanwhile.
        let title = c.title.or_else(|| {
            lines.iter().find_map(|e| match e {
                Entry::User { text, .. } => Some(crate::conversation_title::fallback(text)).filter(|t| !t.is_empty()),
                _ => None,
            })
        });
        lane.title = title.clone();
        lane.transcript = Transcript::with_times(lines.clone(), lines_at.clone());
        self.publish_lane(id, lane, pm, c.project, true);
        self.publish(Event::ConversationBack { session: id, title, lines, lines_at });
        self.publish_put_away(squad);
    }

    /// The conversations put away, told to the screens again.
    fn publish_put_away(&mut self, squad: &str) {
        let Some(journal) = journal_for(&mut self.journals, &self.config.home, squad) else { return };
        let list = journal.put_away(PUT_AWAY_SHOWN).unwrap_or_default();
        self.publish(Event::PutAwayChanged { squad: squad.to_owned(), list });
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
        self.start_turn_with(id, Vec::new(), prompt);
    }

    /// [`Self::start_turn`] with `parts` in front of the prompt's `body`:
    /// each said whole the first time in its conversation, then only as it
    /// changes ([`turn_context`]) — decided once it is known whether this
    /// turn goes on in the same conversation or a fresh one.
    fn start_turn_with(&mut self, id: SessionId, parts: Vec<turn_context::Part>, body: String) {
        // A run whose workspace is still being made takes no turn yet: what
        // comes for it waits, as a memo does, for its first turn's end.
        if let Some(session) = self.sessions.get_mut(&id)
            && session.is_preparing()
        {
            session.queued.push(turn_context::full(&parts, &body));
            session.memo_threads.push(None);
            return;
        }
        // On a lighter model since its own reached its limit: back on its own once that started over.
        self.back_from_lighter(id);
        // On an engine that is out of its limit nothing would run: kept for later, whole.
        if self.hold_for_limit(id, &turn_context::full(&parts, &body)) {
            return;
        }
        if let Some(session) = self.sessions.get_mut(&id) {
            session.answers_person = false;
            session.turn_began = Some(now_secs());
            session.watch.begin(&body);
            if let Some(work) = session.work.as_mut() {
                work.end.nudged = false;
                work.end.stopped = false;
                work.end.with_person = None;
            }
        }
        // A run equipe paused goes on with this turn, however it was asked.
        watchdog::mark("turn start: estimate");
        self.unpark_on_turn(id);
        self.guess_estimate(id);
        // A desk conversation grown too large, or gone cold, starts afresh.
        // Else one that reached its context budget — at the desk or on a
        // ticket — goes on in a fresh session, opened with a packet.
        let opening = match self.refresh_desk(id) {
            Some(handoff) => Some(handoff),
            None => self.rotate_context(id, &body),
        };
        // The blocks in front, as this conversation has been told them so far.
        let (prompt, mut context_parts) = if parts.is_empty() { (body.clone(), Vec::new()) } else { self.compose_turn(id, &parts, &body) };
        if !context_parts.is_empty() {
            context_parts.push(turn_context::Measure { key: "message", full: body.len(), sent: body.len() });
            if let Some(opening) = &opening {
                context_parts.push(turn_context::Measure { key: "opening", full: opening.len(), sent: opening.len() });
            }
        }
        let prompt = match opening {
            Some(opening) => format!("{opening}\n\n{prompt}"),
            None => prompt,
        };
        // A member that just moved engines: equipe's brief of where it was.
        // The brief is the member's; its first run's first turn carries it
        // when that comes before a turn at its desk.
        let member = self.sessions.get(&id).and_then(|s| s.run_of);
        let brief = self.engines.brief_for(id).or_else(|| member.and_then(|m| self.engines.brief_for(m)));
        let prompt = match brief {
            Some(brief) => format!("{brief}\n\n{prompt}"),
            None => prompt,
        };
        // Where the person looks from, when that changed since it was last told:
        // from another machine, nothing of this Mac's is theirs to open.
        let prompt = match self.screens_note(id) {
            Some(note) => format!("[equipe: {note}]\n\n{prompt}"),
            None => prompt,
        };
        // The squad's memory as its files say now — the person's edits in Finder
        // included, its index brought up to date before their changes are saved.
        watchdog::mark("turn start: squad memory");
        let facts = self.squad_memory(id);
        watchdog::mark("turn start: person's changes saved");
        let saved_first = self.save_persons_changes(id);
        // A light design QA session: its report's folder, and a review's changes' reports.
        let lite_on = self.sessions.get(&id).and_then(|s| s.work.as_ref().filter(|w| w.lite).map(|w| w.ticket).zip(s.squad.clone()));
        let lite = lite_on.is_some();
        let lite_reads = match &lite_on {
            Some((ticket, squad)) => self.lite_reads(squad, *ticket),
            None => Vec::new(),
        };
        watchdog::mark("turn start: browser");
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
        let edits_code = session.edits_on_code_tickets();
        let mode = session.mode.or_else(|| squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::mode)).unwrap_or_default();
        // A browser in any mode that may change things — signed in wherever the
        // person (or a teammate) signed in, if anywhere yet. In Auto and Full
        // it browses freely; in Ask each step comes to the person.
        let wants_browser = board_tools && mode != Mode::ReadOnly && !lite;
        // The Mac's desktop: everyone's once the person's Accessibility switch
        // is on, but whoever they turned it off for.
        let granted = self.desktop_granted();
        let desktop = wants_browser
            && cfg!(target_os = "macos")
            && squad.as_deref().zip(session.member.as_ref()).is_some_and(|(s, m)| self.journals.get(s).is_some_and(|j| j.desktop_enabled(&m.name, granted)));
        // A squad's mobile app: its QA and mobile members test it on the ticket's own phone.
        let phone = board_tools && !lite && squad.as_deref().zip(session.member.as_ref()).is_some_and(|(s, m)| self.phone_offered(s, &m.role, mode));
        // A designer's `design` tools, when equipe-design is installed: never installed for it.
        let design = (board_tools && !lite && session.member.as_ref().is_some_and(|m| m.role == "designer"))
            .then(|| self.config.design_path.as_deref().and_then(|path| std::env::split_paths(path).map(|d| d.join("equipe-design")).find(|p| p.is_file())))
            .flatten();
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
        // A designer on brand work gets its reference tools too (`board::design_tools_tool`).
        let on_ticket = self.sessions.get(&id).filter(|s| s.member.as_ref().is_some_and(|m| m.role == design_qa::DESIGNER)).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let branding = match (on_ticket, squad.clone()) {
            (Some(on), Some(sq)) => self.tickets(&sq).iter().find(|t| t.id == on).is_some_and(|t| crate::design::brand::is_branding(&t.title, &t.body)),
            _ => false,
        };
        // The outside MCP servers the squad connected for this member's role,
        // on its engine — a card for one that needs a sign-in, once (`squad_mcp`).
        let (outside, unavailable): squad_mcp::ForMember = match &squad {
            Some(sq) if board_tools && !lite => self.mcp_for_member(sq, id),
            _ => Default::default(),
        };
        // The machine's own command-line credentials, by name: so a member picks one that works first.
        let machine = if board_tools && !lite { self.machine_briefing() } else { None };
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
        // A run's servers say which run they serve, on every call.
        let run_key = session.run_of.and(session.work.as_ref()).map(|w| equipe_protocol::rpc::RunKey { session: id, ticket: w.ticket, generation: w.execution_generation });
        let run_arg = run_key.map(|k| k.arg());
        let mcp_config = match (&squad, &session.member, &self.config.equipe_binary) {
            (Some(squad), Some(m), Some(binary)) if board_tools => {
                let mut servers = json!({ MCP_SERVER: {
                    "type": "stdio",
                    "command": binary,
                    "args": ["mcp", "--squad", squad, "--member", m.name, "--role", m.role, "--home", self.config.home],
                }});
                if branding && let Some(args) = servers[MCP_SERVER]["args"].as_array_mut() {
                    args.push(json!("--branding"));
                }
                // A light design QA session's few tools.
                if lite && let Some(args) = servers[MCP_SERVER]["args"].as_array_mut() {
                    args.push(json!("--qa-lite"));
                }
                if let Some(run) = &run_arg && let Some(args) = servers[MCP_SERVER]["args"].as_array_mut() {
                    args.extend([json!("--run"), json!(run)]);
                }
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
                    if let Some(run) = &run_arg && let Some(args) = servers[DESKTOP_SERVER]["args"].as_array_mut() {
                        args.extend([json!("--run"), json!(run)]);
                    }
                }
                if let Some(design) = &design {
                    servers[DESIGN_SERVER] = json!({ "type": "stdio", "command": design, "args": ["mcp"] });
                }
                if phone {
                    servers[crate::phone::SERVER] = json!({
                        "type": "stdio",
                        "command": binary,
                        "args": ["mcp", "--server", crate::phone::SERVER, "--squad", squad, "--member", m.name, "--role", m.role, "--home", self.config.home],
                    });
                    if let Some(run) = &run_arg && let Some(args) = servers[crate::phone::SERVER]["args"].as_array_mut() {
                        args.extend([json!("--run"), json!(run)]);
                    }
                }
                // The squad's outside servers: equipe's proxy, which holds no
                // credential (the daemon adds it); the engine-signed fallback by address.
                let proxy = crate::squad_mcp::ProxyCommand { binary: binary.clone(), squad: squad.clone(), member: m.name.clone(), home: self.config.home.clone() };
                for (server, credential) in &outside {
                    servers[server.name.as_str()] = crate::squad_mcp::claude_entry(server, credential, &proxy);
                }
                // A key no longer goes in a config file: one an earlier equipe wrote goes.
                let _ = std::fs::remove_file(crate::squad_mcp::private_config_path(&self.config.home, id));
                Some(json!({ "mcpServers": servers }).to_string())
            }
            _ => None,
        };
        let home = squad.as_deref().map(|s| squad_home::path(&self.config.home, s));
        let memory = squad_memory::for_charter(&facts);
        // A member's skills: written as a plugin for Claude Code, as files for
        // Codex's CLI to read by path; Codex over the API reads nothing, so it
        // gets them inline — as it does when they can't be written.
        let reads_files = has_tools(session);
        // A member on Claude Code has equipe's guard asked before each of its
        // calls (`guard`): what it refuses, the charter needn't ask for.
        let guarded = session.member.is_some() && session.engine == Engine::Claude && self.config.equipe_binary.is_some();
        // The subagents its engine runs, as many at once as the squad says —
        // given to the engine below — or as the engine runs on its own; said
        // in the charter either way.
        let set_at_once = self.subagents_at_once(squad.as_deref());
        let subagents = session.member.as_ref().and_then(|_| subagents_of(session, set_at_once));
        // With none, and the board's tools: equipe's helpers stand in.
        let helped = board_tools && subagents.is_none() && self.helpers_offered(session);
        let mut skill_plugin: Option<(PathBuf, Vec<skills::Skill>)> = None;
        watchdog::mark("turn start: charter and skills");
        let charter = session.member.as_ref().map(|m| {
            // A light design QA session: a short charter of its own, no skills.
            if lite {
                return crate::design_qa::lite_charter(&m.name, language.as_deref());
            }
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
                hands_off: squad.as_deref().is_some_and(|s| self.autonomy_mode(s) == equipe_protocol::autonomy::Autonomy::Full),
                self_land: squad.as_deref().and_then(|s| self.journals.get(s)).is_none_or(Journal::self_land),
                desktop,
                design: design.is_some(),
                phone,
                home: home.as_deref(),
                memory: memory.as_deref(),
                facts: facts.len(),
                edits,
                skills: skills::Delivery::Inline,
                guard: guarded,
                subagents,
                helpers: helped,
            };
            let role = roles::role_or_custom(&m.role);
            let offered = skills::offered(role, &ctx);
            let written = (reads_files && !offered.is_empty())
                .then(|| {
                    // On Claude Code, a designer's and a frontend engineer's edits are checked as they make them.
                    let hooks = self.config.equipe_binary.as_deref().filter(|_| session.engine == Engine::Claude).and_then(|bin| skills::hooks(role.id, bin));
                    skills::materialise(&self.config.home, &self.config.client_version, role.id, &offered, hooks.as_deref())
                })
                .and_then(|written| written.map_err(|err| crate::daemon_log!("{}'s skills weren't written: {err}", m.name)).ok());
            let Some(dir) = written else {
                return roles::charter(role, &m.name, &ctx);
            };
            let at = dir.to_string_lossy().into_owned();
            let delivery = if session.engine == Engine::Claude { skills::Delivery::Plugin(&at) } else { skills::Delivery::Files(&at) };
            let charter = roles::charter(role, &m.name, &roles::Context { skills: delivery, ..ctx });
            skill_plugin = Some((dir, offered));
            charter
        });
        // Told of its outside servers only when it has some: nobody else's charter changes.
        let charter = charter.map(|text| match crate::squad_mcp::charter_note(&outside.iter().map(|(server, _)| server).collect::<Vec<_>>()) {
            Some(note) => format!("{text}\n\n{note}"),
            None => text,
        });
        // Those it would have but doesn't this turn: why, in a line, so it never guesses.
        let charter = charter.map(|text| match squad_mcp::unavailable_note(&unavailable) {
            Some(note) => format!("{text}\n\n{note}"),
            None => text,
        });
        let charter = charter.map(|text| match &machine {
            Some(note) => format!("{text}\n\n{note}"),
            None => text,
        });
        // The PM's design-library search, to show references with: what it is for.
        let member_role = session.member.as_ref().map(|m| m.role.clone()).unwrap_or_default();
        let charter = charter.map(|text| match crate::squad_mcp::reference_note(&outside.iter().map(|(server, _)| server).collect::<Vec<_>>(), &member_role) {
            Some(note) => format!("{text} {note}"),
            None => text,
        });
        // What a PM's turn carries, block by block, kept for `equipe usage --pm-context`.
        if !context_parts.is_empty() {
            self.keep_turn_context(id, &context_parts, charter.as_deref(), memory.as_deref(), machine.as_deref());
        }
        watchdog::mark("turn start: engine");
        let (cwd, access, resume, history, ticket, add_dirs) = match &session.work {
            Some(work) => {
                // On a code ticket a member also reads the squad's home; on a task or
                // action, worked in a worktree of the home, it reads every project too.
                let mut read: Vec<PathBuf> = match &session.member {
                    // A light session reads its report's folder (its cwd) and a review's changes' reports, nothing more.
                    Some(_) if work.lite => lite_reads.clone(),
                    Some(_) if work.kind.in_home() => projects.iter().chain(home.iter()).cloned().collect(),
                    Some(_) => home.iter().cloned().collect(),
                    None => Vec::new(),
                };
                // A design QA ticket's precheck report and pictures, kept beside the squad's journal.
                read.extend(squad.as_deref().map(|s| design_qa::precheck_dir(&self.config.home, s, work.ticket)).filter(|d| d.is_dir() && *d != work.cwd));
                let writes = (edits_code || work.kind.in_home()) && work.sandbox.is_some();
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
        let builds = self.build_dir_of(session);
        if let Some(dir) = &builds {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut env: Vec<(String, String)> = builds.iter().map(|d| ("CARGO_TARGET_DIR".to_owned(), d.to_string_lossy().into_owned())).collect();
        // Its own temporary folder too, one per lane: ticket numbers are per squad,
        // so a `/tmp/t84` is as likely another squad's, and nothing could tell
        // whose it was once the ticket ended. This one is exactly the lane's, and
        // goes with its ticket ([`disk::clear_ticket_tmp`]).
        let scratch = squad.as_deref().zip(session.name()).map(|(squad, name)| journal::lane_tmp_dir(&self.config.home, squad, ticket, name));
        if let Some(dir) = &scratch {
            let _ = std::fs::create_dir_all(dir);
            env.extend(crate::disk::lane_tmp_env(dir));
        }
        // No incremental state: it was most of a member's build folder (up to
        // 11 GB of one), for faster rebuilds of what it changes, and the disk
        // ran out for it.
        if builds.is_some() {
            env.push(("CARGO_INCREMENTAL".to_owned(), "0".to_owned()));
        }
        // Leaner Rust builds unless the squad says otherwise: debug info is most of a target's size.
        if builds.is_some()
            && let Some(debug) = squad.as_deref().and_then(|s| self.journals.get(s)).map(Journal::build_debug).and_then(|d| d.cargo_value())
        {
            env.push(("CARGO_PROFILE_DEV_DEBUG".to_owned(), debug.to_owned()));
        }
        // The person's kept values for members' commands (login autopilot):
        // in the environment of this run only, never in a file or a prompt.
        if session.member.is_some()
            && let Some(squad) = squad.as_deref()
        {
            env.extend(self.member_run_env(squad));
            // Keys the member asked its commands to get (`secret_use`), where it may still have them.
            env.extend(self.secret_env_for(squad, id));
        }
        // A member's commands know they are a member's: `equipe browser` refuses there.
        if let Some(m) = &session.member {
            env.push((guard::MEMBER_ENV.to_owned(), m.name.to_string()));
            // And whose run they are, which outlives the shell that started them (`leftovers::orphans`).
            if let Some(squad) = squad.as_deref() {
                env.push((crate::leftovers::RUN_ENV.to_owned(), crate::leftovers::run_tag(squad, id, ticket.map(u64::from))));
            }
            // And on which engine, for what they run outside its sandbox (`equipe design second-look`).
            env.push((crate::design::critique::MEMBER_ENGINE_ENV.to_owned(), crate::design::critique::member_engine_value(session.engine).to_owned()));
            // And who reviews its design work this turn, and why (`equipe design critique`, `second-look`).
            if let Some(squad) = squad.as_deref() {
                let reviewer = self.design_reviewer(squad, session.engine);
                env.push((crate::design::reviewer::REVIEWER_ENV.to_owned(), serde_json::to_string(&reviewer).unwrap_or_default()));
            }
        }
        // Claude Code's cap on subagents at once, where the squad set it;
        // Codex's goes in its thread's config (`codex_app::TurnRequest::subagents`).
        if let (Some(sub), Some(at_once)) = (subagents.filter(|s| s.engine == Engine::Claude), set_at_once) {
            debug_assert_eq!(sub.at_once, at_once);
            env.push((CLAUDE_SUBAGENTS_ENV.to_owned(), at_once.to_string()));
        }
        // Context: the standard window unless allowed larger, and the engine's own compaction at the budget.
        let compact_at = if session.member.is_some() && has_tools(session) {
            let journal = squad.as_deref().and_then(|s| self.journals.get(s));
            let native = journal.is_none_or(Journal::context_native_compact);
            let wants = ticket.and_then(|t| journal.and_then(|j| j.ticket(t).ok().flatten())).is_some_and(|t| hygiene::wants_large_window(&t.body));
            let large = wants || journal.is_some_and(Journal::context_window_large);
            let window = hygiene::window_for(session.engine, ticket.is_some(), native, large);
            env.extend(window.env);
            window.compact_at
        } else {
            None
        };
        let writable: Vec<String> = builds.iter().chain(scratch.iter()).map(|d| d.to_string_lossy().into_owned()).collect();
        let fenced = member_rules && access == claude_code::Access::EditFiles && mode == Mode::Auto;
        // Directories added for reading stay read-only in every mode, and no other
        // squad's home is read at all. Where the guard is asked, it keeps the
        // directories read only — the file tools out, and a command's writes,
        // with the operating system's fence besides — and copying out of one
        // is reading it: Claude Code's own rule for a directory takes `cp`'s
        // source for a write, and refused it.
        let other_homes = squad.as_deref().map(|s| squad_home::others(&self.config.home, s)).unwrap_or_default();
        let deny: Vec<String> = if member_rules {
            let rule = |tool: &str, dir: &Path| format!("{tool}(/{}/**)", dir.display());
            add_dirs
                .iter()
                .filter(|_| !guarded)
                .flat_map(|d| [rule("Edit", d), rule("Write", d)])
                .chain(other_homes.iter().flat_map(|d| [rule("Read", d), rule("Edit", d)]))
                .collect()
        } else {
            Vec::new()
        };
        let mut hooks = guarded.then(|| self.config.equipe_binary.as_deref().map(|binary| guard::hooks(binary, &cwd, &add_dirs))).flatten();
        // A screenshot it already looked at isn't read again: it is in its
        // context, and each model call pays for it again (`image_repeat`).
        if let (Some(hooks), Some(binary), Some(dir)) = (hooks.as_mut(), self.config.equipe_binary.as_deref(), scratch.as_deref()) {
            crate::image_repeat::add_hook(hooks, binary, dir);
        }
        let read_only: Vec<String> = add_dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let prompt_tool = (mcp_config.is_some() && matches!(access, claude_code::Access::EditFiles | claude_code::Access::Ask))
            .then(|| format!("mcp__{MCP_SERVER}__{}", permissions::APPROVE_TOOL));
        let squad_rules = squad.as_deref().and_then(|s| self.journals.get(s)).and_then(|j| j.allowed().ok()).unwrap_or_default();
        let internal = self.internal.clone();
        // What the person says while this turn runs goes into it (Claude Code only).
        let mut steer = None;
        let mut stop = None;
        let mut stop_subagent = None;
        let mut work_over = None;
        let mut wind_down = None;
        let turn = match session.engine {
            // The person's own Codex CLI: an agent with tools, in the worktree.
            Engine::Codex if session.codex.is_none() => {
                let binary = self.config.codex_binary.clone();
                let lang = self.config.lang;
                let mut effort = effort_of(session);
                // NVIDIA's models, and every provider's: Codex's agent with
                // Token Factory — or the provider the model names — as its provider.
                let nebius = session.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
                let open = if nebius { self.open_config(session.model.as_deref()) } else { Ok(None) };
                // The provider is asked for the model by its own id.
                let mut model = session.model.as_deref().map(|m| crate::model_providers::bare_model(m).to_owned());
                let provider = match (open, nebius) {
                    (Ok(Some(cfg)), true) => {
                        env.push(cfg.codex_env());
                        effort = crate::nemotron::agent_effort(effort.as_deref()).map(str::to_owned);
                        Some(cfg.codex_provider())
                    }
                    (Ok(None), true) => return self.error(Some(id), say!(self, "error.nebius_no_key"), Some("equipe login nebius")),
                    (Err(message), true) => return self.error(Some(id), message, None),
                    (_, false) => None,
                };
                let provider_name = provider.as_ref().filter(|p| p.id != crate::nemotron::CODEX_PROVIDER).map(|p| p.name.clone());
                // On the person's own Codex: a model its account refused lately
                // isn't asked for again yet, and one refused now has another to go to.
                let mut fallback = None;
                if provider.is_none() {
                    if let Some(refused) = model.clone().filter(|m| self.codex_refuses(m))
                        && let Some(instead) = self.codex_fallback(&refused, effort.as_deref())
                    {
                        crate::daemon_log!("equipe: {} asks Codex for {} instead of {refused}, which this account refused lately", session.name().unwrap_or("a conversation"), instead.model);
                        (model, effort) = (Some(instead.model), instead.effort);
                    }
                    fallback = model.as_deref().and_then(|m| self.codex_fallback(m, effort.as_deref()));
                }
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
                            if lite && server.is_none() {
                                args.push("--qa-lite".to_owned());
                            }
                            if let Some(run) = &run_arg {
                                args.extend(["--run".to_owned(), run.clone()]);
                            }
                            args
                        };
                        let mut servers = json!({ MCP_SERVER: codex_app::mcp_server(binary, &args(None), true) });
                        // No subagents of its own: equipe's helpers, on the board's server.
                        if helped {
                            helping::offer(&mut servers[MCP_SERVER]);
                        }
                        // A call that waits on the person (`ask_person`, `sign_in`)
                        // holds the turn until they answer, as Claude Code's does.
                        servers[MCP_SERVER]["tool_timeout_sec"] = json!(board_call_timeout_secs());
                        if let Some(port) = browser {
                            servers[browser::SERVER] = codex_app::mcp_http_server(&format!("http://localhost:{port}/mcp"), browses_freely);
                        }
                        for (profile, port) in &profile_browsers {
                            servers[browser::server_name(Some(&profile.name))] = codex_app::mcp_http_server(&format!("http://localhost:{port}/mcp"), browses_freely);
                        }
                        if desktop {
                            let mut server = codex_app::mcp_server(binary, &args(Some(DESKTOP_SERVER)), matches!(mode, Mode::Auto | Mode::Full));
                            // What may send in the person's name waits for their yes.
                            server["tool_timeout_sec"] = json!(person_call_timeout_secs());
                            servers[DESKTOP_SERVER] = server;
                        }
                        if let Some(design) = &design {
                            servers[DESIGN_SERVER] = codex_app::mcp_server(design, &["mcp".to_owned()], matches!(mode, Mode::Auto | Mode::Full));
                        }
                        // Only the ticket's own device: approved in every mode it is offered in.
                        // Booting one takes a minute or two, and installing its tools
                        // waits for the person's yes: past Codex's default wait.
                        if phone {
                            let mut server = codex_app::mcp_server(binary, &args(Some(crate::phone::SERVER)), true);
                            server["tool_timeout_sec"] = json!(person_call_timeout_secs());
                            servers[crate::phone::SERVER] = server;
                        }
                        // The squad's outside servers: equipe's proxy (the fallback: by address, Codex finding its own sign-in).
                        let proxy = crate::squad_mcp::ProxyCommand { binary: binary.clone(), squad: squad.clone(), member: m.name.clone(), home: self.config.home.clone() };
                        for (server, credential) in &outside {
                            servers[server.name.as_str()] = crate::squad_mcp::codex_entry(server, matches!(mode, Mode::Auto | Mode::Full), credential, &proxy);
                        }
                        Some(servers)
                    }
                    _ => None,
                };
                let writable: Vec<PathBuf> = writable.iter().map(PathBuf::from).collect();
                // A member that checks a ticket's work without changing it — QA, a reviewer —
                // runs its tests and builds in a disposable copy of the project: Codex's
                // read-only sandbox would refuse even a test runner's cache, and its
                // workspace-write would open the person's checkout (`crate::check_copy`).
                // Not in the read-only mode, which the person chose so nothing runs.
                let copy_at = scratch
                    .as_deref()
                    .filter(|_| session.member.is_some() && access == claude_code::Access::ReadOnly && mode != Mode::ReadOnly)
                    .filter(|_| session.work.as_ref().is_some_and(|w| !w.lite && !w.kind.in_home()))
                    .map(crate::check_copy::dir_in);
                // Refused by OpenAI's safety check as possibly unintended activity: asked again,
                // once, in a fresh thread that opens with where this one left off — and, refused
                // again, Claude offered when the member could carry on there. Not on another provider.
                let refused_retry = provider.is_none().then(|| self.refused_handoff(id));
                let claude_instead = squad.as_deref().filter(|s| provider.is_none() && self.claude_open(s)).and(session.name()).map(str::to_owned);
                let (steer_tx, steer_rx) = mpsc::unbounded_channel::<String>();
                // What equipe itself says into the turn: a change to a copy's source undone.
                let say_in_turn = steer_tx.clone();
                steer = Some(steer_tx);
                let (stop_tx, stop_rx) = oneshot::channel::<()>();
                stop = Some(stop_tx);
                let (stop_sub_tx, stop_sub_rx) = mpsc::unbounded_channel::<String>();
                stop_subagent = Some(stop_sub_tx);
                // What Codex asks goes to the person as Claude Code's permission prompts do.
                let (ask_tx, mut ask_rx) = mpsc::unbounded_channel::<codex_app::Ask>();
                let asker = session_member(session);
                // Who its engine works for, noted off the loop as it starts (`crate::engine_bind`).
                let bind_as = session.member.as_ref().and(asker.clone());
                let asker_run = run_key;
                let forward = internal.clone();
                tokio::spawn(async move {
                    while let Some(ask) = ask_rx.recv().await {
                        let Some(who) = &asker else {
                            let _ = ask.answer.send(false);
                            continue;
                        };
                        let (tx, rx) = oneshot::channel();
                        let arguments = json!({ "tool_name": ask.tool, "input": ask.input });
                        let call = ToolCall { squad: who.squad.clone(), member: who.member.clone(), name: permissions::APPROVE_TOOL.into(), arguments, run: asker_run };
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
                    let mut retried = false;
                    // equipe's framing of a design session's turn goes as a developer message, the
                    // person's words as theirs: OpenAI's safety check reads instructions in the
                    // person's message that they didn't write as possibly unintended activity.
                    let (asked, framing) = crate::design::session::framing_apart(&prompt);
                    // A checking turn in its copy, told where it is; where no copy can be
                    // made (no git, no copy-on-write), read-only as before.
                    let mut cwd = cwd;
                    let mut copy = None;
                    if let Some(at) = copy_at {
                        let project = cwd.clone();
                        match tokio::task::spawn_blocking(move || crate::check_copy::prepare(&project, &at)).await {
                            Ok(Ok(made)) => copy = Some(made),
                            Ok(Err(why)) => crate::daemon_log!("equipe: session {id} checks read-only in {}: no copy ({why})", cwd.display()),
                            Err(_) => {}
                        }
                    }
                    let framing = match (&copy, framing) {
                        (Some(made), framing) => {
                            let note = crate::check_copy::framing(made, &cwd);
                            Some(framing.map_or_else(|| note.clone(), |f| format!("{f}\n\n{note}")))
                        }
                        (None, framing) => framing,
                    };
                    if let Some(made) = &copy {
                        cwd = made.cwd.clone();
                    }
                    // Its source checked as the turn goes, after each tool that could
                    // write: a change undone at once, and the member told in the turn.
                    let (poke, mut poked) = mpsc::unbounded_channel::<()>();
                    let checker = copy.as_ref().map(|made| {
                        let (root, internal) = (made.root.clone(), internal.clone());
                        tokio::spawn(async move {
                            while poked.recv().await.is_some() {
                                while poked.try_recv().is_ok() {}
                                let at = root.clone();
                                if let Ok(Ok(undone)) = tokio::task::spawn_blocking(move || crate::check_copy::undo_changes(&at)).await
                                    && !undone.is_empty()
                                {
                                    crate::daemon_log!("equipe: session {id} changed its copy's source; undone: {}", undone.join(", "));
                                    let _ = say_in_turn.send(crate::check_copy::undone_note(&undone));
                                    let text = crate::i18n::core(lang).fill("notice.check_copy_undone", &[("files", crate::check_copy::listed(&undone))]);
                                    let _ = internal.send(Internal::Event(Box::new(Event::Notice { undo: None, session: id, text })));
                                }
                            }
                        })
                    });
                    let poke = checker.is_some().then_some(poke);
                    let request = codex_app::TurnRequest {
                        prompt: &asked,
                        model: model.as_deref(),
                        effort: effort.as_deref(),
                        access,
                        resume: resume.as_deref(),
                        fork: false,
                        fresh_prompt: None,
                        web_search: false,
                        instructions: charter.as_deref(),
                        framing: framing.as_deref(),
                        mcp_servers: servers.as_ref(),
                        writable: &writable,
                        checks: copy.is_some(),
                        env: &env,
                        provider: provider.as_ref(),
                        subagents: subagents.and(set_at_once),
                        compact_at,
                    };
                    // The person's changes to the squad's folder saved first (off the loop): what it writes is its own.
                    if let Some(saved) = saved_first {
                        let _ = saved.await;
                    }
                    if let Some(port) = browser {
                        browser::ready(port, Duration::from_secs(90)).await;
                    }
                    let controls = codex_app::Controls { steering: Some(steer_rx), asks: Some(ask_tx), stop: Some(stop_rx), stop_subagent: Some(stop_sub_rx), fallback, refused_retry, ..Default::default() };
                    let mut bound: Option<crate::engine_bind::Bound> = None;
                    let outcome = codex_app::run_turn_with(&binary, &cwd, request, controls, |activity| {
                        if matches!(activity, Activity::Text(_)) {
                            streamed = true;
                        }
                        if let (Some(poke), Activity::ToolDone(done)) = (&poke, &activity)
                            && crate::check_copy::may_write(&done.name)
                        {
                            let _ = poke.send(());
                        }
                        match activity {
                            // Said as it happens, in the person's words: the turn goes on afresh.
                            Activity::RefusedRetried => {
                                retried = true;
                                crate::daemon_log!("equipe: OpenAI refused a turn of session {id} as possibly unintended activity; asked again in a fresh thread");
                                emit(Event::Notice { undo: None, session: id, text: crate::i18n::core(lang).get("notice.codex_refused_retry").to_owned() });
                            }
                            // Token Factory bills per token: no plan, no limits to show.
                            Activity::Limits(_) if nebius => {}
                            Activity::Limits(limits) => {
                                let _ = internal.send(Internal::CodexLimits(Some(limits)));
                            }
                            Activity::Engine { pid } => {
                                if let Some(member) = &bind_as {
                                    bound = Some(crate::engine_bind::bind(pid, member.clone()));
                                }
                                let _ = internal.send(Internal::EngineStarted { session: id, pid });
                            }
                            Activity::SubagentHeard { id: sub } => {
                                let _ = internal.send(Internal::SubagentHeard { session: id, id: sub });
                            }
                            Activity::Settled(why) => {
                                let _ = internal.send(Internal::TurnSettled { session: id, why });
                            }
                            Activity::ModelRefused { model, instead } => {
                                let _ = internal.send(Internal::CodexModelRefused { session: id, model, instead: Some(instead) });
                            }
                            activity => {
                                if let Some(event) = activity_event(id, activity) {
                                    emit(event);
                                }
                            }
                        }
                    })
                    .await;
                    // The engine is gone: nothing is bound to it any more.
                    drop(bound);
                    // Its copy's source looked at once more, the checks under way done first:
                    // a change made at the very end is undone too, and the person told.
                    drop(poke);
                    if let Some(checker) = checker {
                        let _ = checker.await;
                    }
                    if let Some(made) = &copy {
                        let root = made.root.clone();
                        if let Ok(Ok(undone)) = tokio::task::spawn_blocking(move || crate::check_copy::undo_changes(&root)).await
                            && !undone.is_empty()
                        {
                            crate::daemon_log!("equipe: session {id} changed its copy's source at its turn's end; undone: {}", undone.join(", "));
                            let text = crate::i18n::core(lang).fill("notice.check_copy_undone", &[("files", crate::check_copy::listed(&undone))]);
                            emit(Event::Notice { undo: None, session: id, text });
                        }
                    }
                    // Refused with nothing to fall back on: said before the turn's end, so its
                    // failure is never taken for the engine's limit.
                    if let Err(equipe_providers::Error::Backend { message, code }) = &outcome
                        && let Some(refused) = codex_app::refused_model(message, code.as_deref())
                        && !nebius
                    {
                        let refused = if refused.is_empty() { model.clone().unwrap_or_default() } else { refused };
                        let _ = internal.send(Internal::CodexModelRefused { session: id, model: refused, instead: None });
                    }
                    // On Token Factory: what was asked of it, for the log — never the key or the text.
                    if let (true, Ok(turn)) = (nebius, &outcome) {
                        crate::daemon_log!("{}", nebius_log(model.as_deref().unwrap_or_default(), turn));
                    }
                    // Stopped: the turn's end was said already, and what follows it done.
                    if outcome.as_ref().is_ok_and(|turn| turn.result.status == "interrupted") {
                        return;
                    }
                    let thread = outcome.as_ref().ok().and_then(|turn| turn.thread_id.clone());
                    // Refused by OpenAI's safety check, in the person's words — with Claude to carry on
                    // on, when the member could — never the provider's raw line alone.
                    if let Err(equipe_providers::Error::Backend { message, code }) = &outcome
                        && !nebius
                        && codex_app::refused_as_unintended(message, code.as_deref())
                    {
                        let (message, next) = refused_words(lang, message, retried, claude_instead.as_deref());
                        crate::daemon_log!("equipe: OpenAI refused a turn of session {id} as possibly unintended activity{}", if retried { ", again in a fresh thread" } else { "" });
                        emit(Event::Error { session: Some(id), message, next });
                        emit(Event::TurnEnded { session: id, outcome: TurnOutcome::Failed, usage: None });
                        let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply: None, claude_session: thread, ticket }));
                        return;
                    }
                    let result = outcome.map(|turn| turn.result).map_err(|err| if nebius { nebius_failure(err, provider_name.as_deref(), lang) } else { codex_cli_failure(err, lang) });
                    let lost = !nebius && matches!(&result, Err((_, next)) if crate::engine_auth::lost_by(*next) == Some(Engine::Codex));
                    let reply = finish_turn(&emit, id, result, streamed);
                    if lost {
                        let _ = internal.send(Internal::SignInLost { engine: Engine::Codex, session: id, prompt: prompt.clone() });
                    }
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply, claude_session: thread, ticket }));
                })
            }
            Engine::Codex => {
                let client = self.client.clone();
                let Some(cred) = session.codex.clone() else {
                    return self.error(Some(id), say!(self, "error.codex_signed_out"), Some("equipe login codex"));
                };
                let model = session.model.clone().unwrap_or_default();
                let effort = effort_of(session);
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
                let lang = self.config.lang;
                let mut model = session.model.clone();
                // A model on an Anthropic-compatible provider: Claude Code runs on it
                // with the provider's endpoint and key, for this turn's process only.
                let on_provider = match self.claude_provider(model.as_deref()) {
                    Some(Ok((cfg, bare))) => {
                        env.extend(cfg.claude_env(&bare));
                        model = Some(bare);
                        Some(cfg.name().to_owned())
                    }
                    Some(Err(message)) => return self.error(Some(id), message, None),
                    None => None,
                };
                let effort = effort_of(session);
                let memory = session.memory.clone();
                let mut allowed: Vec<String> = mcp_config.iter().map(|_| format!("mcp__{MCP_SERVER}")).collect();
                if browses_freely {
                    allowed.push(format!("mcp__{}", browser::SERVER));
                    allowed.extend(profile_browsers.iter().map(|(p, _)| format!("mcp__{}", browser::server_name(Some(&p.name)))));
                }
                if desktop && matches!(mode, Mode::Auto | Mode::Full) {
                    allowed.push(format!("mcp__{DESKTOP_SERVER}"));
                }
                if design.is_some() && matches!(mode, Mode::Auto | Mode::Full) {
                    allowed.push(format!("mcp__{DESIGN_SERVER}"));
                }
                // An outside server's tools ask in Ask, as the desktop's do; in Auto and Full they run.
                if mcp_config.is_some() && matches!(mode, Mode::Auto | Mode::Full) {
                    allowed.extend(outside.iter().map(|(server, _)| crate::squad_mcp::allowed_rule(server)));
                }
                // The ticket's own simulator or emulator, nothing else: no asking.
                if phone {
                    allowed.push(format!("mcp__{}", crate::phone::SERVER));
                }
                // What the person always allows no longer asks, and reaches its service from inside the fence.
                let outward = permissions::still_asks(&squad_rules);
                let mut domains: Vec<String> = permissions::ALLOWED_DOMAINS.iter().map(|d| (*d).to_owned()).collect();
                domains.extend(permissions::domains_allowed(&squad_rules));
                if member_rules {
                    allowed.extend(squad_rules);
                }
                // Its skills, as a plugin: each opened, and its files read, without asking.
                let plugin_dirs: Vec<PathBuf> = skill_plugin.iter().map(|(dir, _)| dir.clone()).collect();
                if let Some((dir, offered)) = &skill_plugin {
                    allowed.extend(skills::allowed_tools(dir, offered));
                }
                let mut blocked: Vec<String> =
                    if member_rules { permissions::BLOCKED.iter().map(|r| (*r).to_owned()).collect() } else { Vec::new() };
                // Claude Code's own question tool asks a terminal nobody watches:
                // a member with equipe's tools puts decisions with `ask_decision`.
                if member_rules && mcp_config.is_some() {
                    blocked.push(ENGINE_QUESTION_TOOL.to_owned());
                }
                // A window that holds the turn until it is closed: the guard says why first.
                if member_rules {
                    blocked.extend(guard::deny_rules(self.config.equipe_binary.as_deref()));
                }
                // The person's browser sign-ins and equipe's logins: no fenced command reads them.
                // Nor other squads' homes.
                // Who its engine works for, noted off the loop as it starts (`crate::engine_bind`).
                let bind_as = session.member.as_ref().and(session_member(session));
                let (steer_tx, steer_rx) = mpsc::unbounded_channel::<String>();
                steer = Some(steer_tx);
                let over = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                work_over = Some(over.clone());
                let (wind_tx, wind_rx) = oneshot::channel();
                wind_down = Some(wind_tx);
                // The desktop helper's token and signing key too.
                let private: Vec<String> =
                    [self.config.home.join("browser"), self.config.home.join("credentials"), equipe_desktop_control::helper::dir(&self.config.home)]
                    .iter()
                    .chain(&other_homes)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                // A light design QA session looks at pictures and reads its report: nothing else.
                let builtin: Vec<String> = if lite { ["Read", "Glob", "Grep"].map(str::to_owned).to_vec() } else { Vec::new() };
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
                        fork: false,
                        tools: &builtin,
                        charter: charter.as_deref(),
                        plugin_dirs: &plugin_dirs,
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
                        hooks: hooks.as_ref(),
                        compact_at,
                    };
                    // The person's changes to the squad's folder saved first (off the loop): what it writes is its own.
                    if let Some(saved) = saved_first {
                        let _ = saved.await;
                    }
                    // A browser just started may still be loading.
                    if let Some(port) = browser {
                        browser::ready(port, Duration::from_secs(90)).await;
                    }
                    let mut plan = engines::ToldPlan::default();
                    let mut bound: Option<crate::engine_bind::Bound> = None;
                    let controls = claude_code::Controls { steering: Some(steer_rx), work_over: Some(over), wind_down: Some(wind_rx), ..Default::default() };
                    let outcome = claude_code::run_turn_with(&binary, &cwd, request, controls, |activity| {
                        if matches!(activity, Activity::Text(_)) {
                            streamed = true;
                        }
                        if let Activity::Engine { pid } = activity {
                            if let Some(member) = &bind_as {
                                bound = Some(crate::engine_bind::bind(pid, member.clone()));
                            }
                            let _ = internal.send(Internal::EngineStarted { session: id, pid });
                            return;
                        }
                        if let Activity::Settled(why) = activity {
                            let _ = internal.send(Internal::TurnSettled { session: id, why });
                            return;
                        }
                        if let Some(event) = plan.heard(&activity, std::time::Instant::now()).and_then(|again| activity_event(id, again)) {
                            emit(event);
                        }
                        if let Some(event) = activity_event(id, activity) {
                            emit(event);
                        }
                    })
                    .await;
                    // The engine is gone: nothing is bound to it any more.
                    drop(bound);
                    let claude_session = outcome.as_ref().ok().and_then(|turn| turn.session_id.clone());
                    // On a provider's endpoint a refused key is the provider's, not the person's Claude sign-in.
                    let result = outcome.map(|turn| turn.result).map_err(|err| match &on_provider {
                        Some(provider) => provider_claude_failure(err, provider, lang),
                        None => claude_failure(err),
                    });
                    let lost = on_provider.is_none() && matches!(&result, Err((_, next)) if crate::engine_auth::lost_by(*next) == Some(Engine::Claude));
                    let reply = finish_turn(&emit, id, result, streamed);
                    // After the error is told: the screens hear what it was, then why.
                    if lost {
                        let _ = internal.send(Internal::SignInLost { engine: Engine::Claude, session: id, prompt: prompt.clone() });
                    }
                    let _ = internal.send(Internal::Done(TurnDone { session: id, prompt, reply, claude_session, ticket }));
                })
            }
        };
        if let Some(session) = self.sessions.get_mut(&id) {
            session.turn = Some(turn);
            session.steer = steer;
            session.stop = stop;
            session.stop_subagent = stop_subagent;
            session.work_over = work_over;
            session.wind_down = wind_down;
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
    /// Saved off the loop; the answer says when, for the turn's engine to wait on.
    fn save_persons_changes(&mut self, id: SessionId) -> Option<oneshot::Receiver<()>> {
        let session = self.sessions.get(&id).filter(|s| s.member.is_some() && s.work.is_none())?;
        let squad = session.squad.clone()?;
        let busy = self.sessions.values().any(|s| s.squad.as_ref() == Some(&squad) && s.work.is_none() && s.is_working());
        if !busy {
            // Off the loop: a home of tens of thousands of files, and what was
            // just put in it, took git seconds to save — every screen waited.
            // The engine takes longer to start than this takes to begin.
            let dir = squad_home::path(&self.config.home, &squad);
            let (saved, told) = oneshot::channel();
            squad_home::in_order(&dir.clone(), move || {
                if let Err(err) = squad_home::save_persons(&dir) {
                    crate::daemon_log!("equipe: {squad}: the person's changes to the squad's folder weren't saved: {err}");
                }
                let _ = saved.send(());
            });
            return Some(told);
        }
        None
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
        // Saved off the loop (git in a big home takes seconds); told once saved.
        // After the person's changes saved before the turn: each is told as whose it is.
        let (dir, why, internal) = (squad_home::path(&self.config.home, &squad), asked_for(prompt), self.internal.clone());
        squad_home::in_order(&dir.clone(), move || {
            let saved = squad_home::save(&dir, &name, &why);
            let _ = internal.send(Internal::DeskSaved { session: id, name, saved });
        });
    }

    /// A member's desk turn's changes to the squad's folder were saved: the
    /// person is told, with a way to take them back.
    fn desk_saved(&mut self, id: SessionId, name: &str, saved: Result<Option<squad_home::Change>, String>) {
        match saved {
            Ok(Some(change)) => {
                let text = say!(self, "news.desk_saved", name = name, what = change.said(self.config.lang));
                self.publish(Event::Notice { undo: Some(change.id), session: id, text });
            }
            Ok(None) => {}
            Err(err) => self.error(Some(id), say!(self, "error.folder_not_saved", err = err), None),
        }
    }

    /// The person takes back `change` to `squad`'s folder or memory, as
    /// `/home undo` does, and hears it in `session`'s conversation — a change
    /// equipe told of, only (one of its latest): never an arbitrary revision.
    fn undo_home_change(&mut self, squad: &str, session: SessionId, change: &str) {
        if self.sessions.get(&session).is_none_or(|s| s.squad.as_deref() != Some(squad)) {
            return;
        }
        let (dir, internal, squad, change) = (squad_home::path(&self.config.home, squad), self.internal.clone(), squad.to_owned(), change.to_owned());
        // Taken back off the loop, in line with the folder's other saves and landings.
        tokio::task::spawn_blocking(move || {
            let result = squad_home::undo_change(&dir, &change);
            let _ = internal.send(Internal::HomeUndone { squad, session, result });
        });
    }

    /// A change to the squad's folder taken back off the loop ([`Runtime::undo_home_change`]), or why not.
    fn home_undone(&mut self, squad: &str, session: SessionId, result: Result<(squad_home::Change, squad_home::Change), String>) {
        match result {
            Ok((taken, undo)) => {
                let text = say!(self, "notice.taken_back", what = taken.said(self.config.lang));
                self.publish(Event::Notice { session, text, undo: Some(undo.id) });
                // A brief or plan back, or gone again: the screens' lists follow.
                if taken.files.iter().any(|(_, file)| file.starts_with(&format!("{}/", crate::briefs::DOCS))) {
                    self.docs_changed(squad);
                }
            }
            Err(err) => self.error(Some(session), say!(self, "error.undo_failed", err = err), None),
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
        session.watch.stopped = true;
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
        // Nobody is waiting for those answers any more — nor is anyone to be
        // told of them later: the turn that asked was stopped.
        let asked: Vec<u64> = self.pending.iter().filter(|(_, p)| p.approval.session == id).map(|(&a, _)| a).collect();
        for approval in asked {
            self.approvals_turn.remove(&approval);
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
        // A turn that answered: whatever its engine's sign-in stopped of it is over.
        if done.reply.is_some() {
            self.no_longer_stopped(done.session);
        }
        // A member asking the person to run a CLI sign-in equipe may approve itself: run by equipe.
        if let Some(reply) = &done.reply
            && self.sessions.get(&done.session).is_some_and(|s| s.member.is_some())
        {
            self.device_login_from_reply(done.session, reply);
        }
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
            watchdog::mark("turn end: desk changes saved");
            self.save_desk_changes(done.session, &done.prompt);
        }
        watchdog::mark("turn end: told");
        // The answer is already on every screen. Naming it is a separate,
        // tool-less side turn and can never hold up or fail that answer.
        self.start_conversation_title(done.session);
        // The PM said a card is withdrawn or to be ignored: one still up is brought back to it.
        if done.ticket.is_none()
            && let Some(reply) = done.reply.as_deref()
            && let Some(squad) = self.sessions.get(&done.session).filter(|s| s.is_pm()).and_then(|s| s.squad.clone())
        {
            self.check_said_withdrawn(&squad, reply);
        }
        if let Some(reply) = done.reply.filter(|r| !r.trim().is_empty()) {
            let short: String = reply.chars().take(BRIEF_REPLY_CHARS).collect();
            let more = if short.len() < reply.len() { "…" } else { "" };
            self.tell_the_pm(done.session, |name| format!("{name} answered: {short}{more}"), None);
        }
        self.after_turn(done.session).await;
        if answered_person {
            watchdog::mark("turn end: next suggested");
            self.suggest_next(done.session);
        }
    }

    fn start_conversation_title(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id).filter(|s| s.kept && s.title.is_none() && !s.title_started) else { return };
        let Some((user, reply)) = first_exchange(session.transcript.entries()) else { return };
        let (engine, model, codex, squad) = (session.engine, session.model.clone(), session.codex.clone(), session.squad.clone());
        let Some(squad) = squad else { return };
        session.title_started = true;
        let prompt = crate::conversation_title::prompt(&user, &reply);
        let fallback = crate::conversation_title::fallback(&user);
        let internal = self.internal.clone();
        let cwd = self.config.home.join("title");

        match engine {
            Engine::Claude => {
                let binary = self.config.claude_binary.clone();
                tokio::spawn(async move {
                    let _ = std::fs::create_dir_all(&cwd);
                    let generated = claude_code::conversation_title(&binary, &cwd, &prompt).await;
                    let title = crate::conversation_title::resolve(generated.as_deref(), &fallback);
                    let _ = internal.send(Internal::ConversationTitle(ConversationTitleDone { session: id, squad, title }));
                });
            }
            Engine::Codex if codex.is_none() => {
                let binary = self.config.codex_binary.clone();
                let nebius = model.as_deref().is_some_and(crate::nemotron::is_agent_model);
                let configured = if nebius { self.open_config(model.as_deref()).ok().flatten() } else { None };
                let mut env = Vec::new();
                let provider = configured.map(|cfg| {
                    env.push(cfg.codex_env());
                    cfg.codex_provider()
                });
                let model = model.as_deref().map(crate::model_providers::bare_model).map(str::to_owned);
                tokio::spawn(async move {
                    let _ = std::fs::create_dir_all(&cwd);
                    let none = json!({});
                    let request = codex_app::TurnRequest { prompt: &prompt, model: model.as_deref(), effort: Some("low"), access: claude_code::Access::ReadOnly, mcp_servers: Some(&none), env: &env, provider: provider.as_ref(), ..Default::default() };
                    let generated = codex_app::run_turn(&binary, &cwd, request, None, None, None, |_| {}).await.ok().map(|turn| turn.result.text);
                    let title = crate::conversation_title::resolve(generated.as_deref(), &fallback);
                    let _ = internal.send(Internal::ConversationTitle(ConversationTitleDone { session: id, squad, title }));
                });
            }
            Engine::Codex => {
                let client = self.client.clone();
                let version = self.config.client_version.clone();
                let model = model.unwrap_or_default();
                let Some(cred) = codex else { return };
                tokio::spawn(async move {
                    let request = responses::TurnRequest { prompt: &prompt, model: &model, instructions: None, history: &[], effort: Some("low") };
                    let generated = responses::run_turn(&client, &cred, &version, request, |_| {}).await.ok().map(|turn| turn.text);
                    let title = crate::conversation_title::resolve(generated.as_deref(), &fallback);
                    let _ = internal.send(Internal::ConversationTitle(ConversationTitleDone { session: id, squad, title }));
                });
            }
        }
    }

    fn on_conversation_title(&mut self, done: ConversationTitleDone) {
        if self.sessions.get(&done.session).is_some_and(|s| s.title.is_some()) {
            return;
        }
        if self.config.persist
            && let Some(journal) = journal_for(&mut self.journals, &self.config.home, &done.squad)
        {
            let _ = journal.set_title_if_empty(done.session, &done.title);
        }
        if let Some(session) = self.sessions.get_mut(&done.session) {
            if session.title.is_none() {
                session.title = Some(done.title.clone());
                self.publish(Event::ConversationTitle { session: done.session, title: done.title, named: false });
            }
        } else {
            // It may have been put away while the small side turn ran.
            self.publish_put_away(&done.squad);
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
        // A turn that put something in the person's hands waits on them from now.
        if let Some(waits) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).and_then(|w| w.end.with_person.as_mut()) {
            waits.since = now_secs();
        }
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
        // How the turn that just ended went, if one did: one that went well
        // ends its ticket's failures in a row, and passes a breaker's test.
        let ending = self.sessions.get_mut(&id).and_then(|s| s.watch.ended.take());
        if ending.as_ref().is_some_and(|e| !e.limit && !matches!(e.outcome, TurnOutcome::Failed | TurnOutcome::Interrupted)) {
            self.ended_well(id);
        }
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let squad = session.squad.clone();
        let handed_back = session.work.as_ref().is_some_and(|w| w.end.handed_back.is_some());
        // Paused by equipe, or resumed with a reminder: what waits for it waits too.
        watchdog::mark("turn end: its work's end");
        let mut held = false;
        if session.work.as_ref().is_some_and(|w| w.handed_in.is_some()) {
            self.end_work(id, WorkEnd::HandedIn);
        } else if session.work.as_ref().is_some_and(|w| w.waiting.is_some()) {
            self.end_work(id, WorkEnd::Waiting);
        } else if let Some(note) = self.asked_waiting_note(id) {
            // The member put a question (or a secret) to the person and ended
            // its turn, as `ask_decision` tells it to: the ticket waits for
            // the answer. Left as it was, its lease would run out and it
            // would show as stalled, to be resumed by hand and run again.
            if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
                work.waiting = Some(note);
            }
            self.end_work(id, WorkEnd::Waiting);
        } else if handed_back {
            self.end_work(id, WorkEnd::HandedBack);
        } else {
            // No end to the work, or the engine failed or ran out: equipe
            // carries it on rather than leave it for its lease to run out.
            held = self.unfinished_turn(id, ending);
        }
        // Off a ticket, the turn's end is the work's: what was expected is done.
        if self.sessions.get(&id).is_some_and(|s| s.work.is_none()) {
            self.estimate_done(id);
        }
        if !held && let Some(text) = self.sessions.get_mut(&id).and_then(Session::next_memos) {
            watchdog::mark("turn end: memos");
            self.save_waiting(id);
            Box::pin(self.submit(id, text)).await;
        }
        if let Some(squad) = squad {
            watchdog::mark("turn end: dispatch");
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
        // A member leaving takes its lanes and its runs with it; each run's
        // ticket goes back on the board.
        let lanes: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) || s.run_of == Some(id))
            .map(|(&l, _)| l)
            .collect();
        for lane in lanes {
            Box::pin(self.close(lane)).await;
        }
        if self.sessions.get(&id).is_some_and(|s| s.run_of.is_some()) {
            self.stop_turn(id);
            self.end_work(id, WorkEnd::Left);
            return;
        }
        self.stop_turn(id);
        let Some(session) = self.sessions.get(&id) else {
            return self.error(Some(id), say!(self, "error.no_session", id = id), None);
        };
        let squad = session.squad.clone();
        let leaving = session.member.clone().filter(|_| session.is_member_itself());
        if session.work.is_some() {
            self.end_work(id, WorkEnd::Left);
        }
        if let (Some(squad), Some(name)) = (&squad, leaving.as_ref().map(|m| m.name.clone())) {
            self.drop_questions(squad, |set| set.member == name);
            // Letting it go no longer waits on the person: it left.
            self.drop_team_changes(squad, |c| c.member.as_deref() == Some(name.as_str()));
        }
        if let Some(sandbox) = self.sessions.get(&id).and_then(|s| s.sandbox.clone()) {
            self.land_plain_session(id, &sandbox);
        }
        self.close_side_chats_of(id);
        self.publish(Event::SessionClosed { session: id });
        if let Some(who) = self.sessions.remove(&id).as_ref().and_then(session_member)
            && self.desktop.leave(&who)
        {
            self.publish_desktop();
        }
        // What still names it goes to its role — not when it only moves
        // engines, coming back at once under the same name.
        if let (Some(squad), Some(member)) = (&squad, &leaving)
            && !self.engines.keeps_name(&member.name)
        {
            self.hand_to_role(squad, &member.name, &member.role, true);
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
        self.dispatch_to(squad, &[], &[]).await;
    }

    /// Hand ready tickets to `only` (or every member when empty), considering
    /// `first` before the board's normal order: every ready ticket a member
    /// has room for starts now, each as a run of its own beside the member's
    /// others ([`crate::lanes`]). Readiness stays in `board`; how many runs go
    /// at once, and which member of a role takes which, is the lane policy's;
    /// runtime-only engine and room holds are applied here as usual.
    async fn dispatch_to(&mut self, squad: &str, only: &[String], first: &[TicketId]) {
        // A member moving engines moves before it picks up its next ticket.
        watchdog::mark("dispatch: engine moves");
        self.apply_engine_moves(squad).await;
        // Several engines failed at once: nothing new starts until a test turn works.
        if self.turn_ends.paused(squad) {
            return;
        }
        watchdog::mark("dispatch: board");
        // Closed tickets light: what starts next reads only their status and links.
        // Design QA that no longer holds its change goes into the daily review first, unstarted.
        self.release_design_qa(squad);
        // Design QA whose precheck is under way, by the journal too: never a designer's before its result.
        let prechecking = self.prechecks_pending(squad);
        let board = self.tickets(squad);
        let mut tickets: Vec<Ticket> = board
            .iter()
            .filter(|ticket| !self.held_for_room(ticket.kind) && !self.workspaces.busy(squad, ticket.id) && !prechecking.contains(&ticket.id))
            .map(board::light_copy)
            .collect();
        drop(board);
        // Runs on tickets cancelled meanwhile stop; tickets that look obsolete wait for the PM.
        self.stop_obsolete_runs(squad);
        self.hold_obsolete(squad, &mut tickets, first);
        // A ticket whose turns cost its budget already goes to the PM instead (`guards`).
        self.hold_over_budget(squad, &mut tickets);
        let single_project = self.projects(squad).len() == 1;
        // Members equipe once brought in for a rush finish what they hold and take nothing new.
        let helpers = self.journals.get(squad).map(Journal::helpers).unwrap_or_default();
        // So do members the person rested.
        let resting = self.journals.get(squad).map(Journal::resting).unwrap_or_default();
        let disk = self.disk.status.level;
        let parallel = self.journals.get(squad).map(Journal::parallel).unwrap_or_default();
        let used = |engine: Engine| crate::lanes::plan_used(self.plans.iter().find(|p| p.engine == engine).map(|p| &p.usage));
        // Engines whose weekly window is under its reserve (auto only): only the PM's work and urgent tickets start on them.
        let guarded: HashSet<Engine> = [Engine::Claude, Engine::Codex].into_iter().filter(|e| !parallel.is_max() && self.pace_of(squad, *e).held).collect();
        let paced = !guarded.is_empty();
        /// A member as this pass weighs it.
        struct Seat {
            name: String,
            role: String,
            joined: u64,
            engine: Engine,
            runs: usize,
            limit: usize,
            takes_new: bool,
            /// Its engine takes no new ticket now.
            held: bool,
        }
        let mut members: Vec<Seat> = Vec::new();
        let mut at_work: HashMap<Engine, usize> = HashMap::new();
        // How loaded the machine is, as its minute's reading found it (`machine.rs`): short of
        // overloaded, a member's lanes still ease off with it (`flow::machine_cap`).
        let reading = self.machine.watch.last;
        let (load, cpus, free) = (reading.map(|r| r.load), reading.map_or(1, |r| r.cores), reading.and_then(|r| r.memory_left).map(|m| f64::from(m) / 100.0));
        // The places each member's running tickets name: its next lane never overlaps them.
        let mut fit = crate::lanes::Fit::default();
        for s in self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad)) {
            let (Some(of), Some(work)) = (s.run_of, s.work.as_ref()) else { continue };
            let Some(name) = self.sessions.get(&of).and_then(|m| m.name()) else { continue };
            if let Some(t) = tickets.iter().find(|t| t.id == work.ticket) {
                fit.busy.entry(name.to_owned()).or_default().push(crate::flow::paths(&format!("{} {}", t.title, t.body)));
            }
        }
        for (&id, s) in &self.sessions {
            if s.squad.as_deref() != Some(squad) {
                continue;
            }
            if s.run_of.is_some() {
                // A run paused for a limit or a retry holds no slot while its engine's reserve is held.
                if crate::pacing::holds_slot(Runtime::is_parked(s), guarded.contains(&s.engine)) {
                    *at_work.entry(s.engine).or_default() += 1;
                }
                continue;
            }
            let Some(m) = s.member.as_ref().filter(|_| s.is_member_itself()) else { continue };
            if !only.is_empty() && !only.contains(&m.name) {
                continue;
            }
            let runs = self.sessions.values().filter(|r| r.run_of == Some(id)).count();
            // An engine out of its plan, or waited for, takes no new ticket;
            // nor does a member about to move to the other engine.
            let held = self.engines.held(squad, s.engine);
            let moving = self.engines.moving(squad, &m.name);
            // One brought in for a rush, or rested, only finishes what it
            // holds; what it owns by name waits for it rather than going to its role.
            let takes_new = !moving && !helpers.contains(&id) && !resting.contains(&m.name);
            let limit = crate::flow::machine_cap(crate::lanes::member_limit(used(s.engine), disk, &parallel), load, cpus, free);
            members.push(Seat { name: m.name.clone(), role: m.role.clone(), joined: s.joined, engine: s.engine, runs, limit, takes_new, held });
        }
        let room: HashMap<Engine, usize> = [Engine::Claude, Engine::Codex]
            .into_iter()
            .map(|e| (e, crate::lanes::engine_limit(used(e), disk, &parallel).saturating_sub(at_work.get(&e).copied().unwrap_or(0))))
            .collect();
        fn lane<'a>(m: &'a Seat, tickets: &[Ticket], held_too: bool) -> crate::lanes::Member<'a> {
            crate::lanes::Member {
                name: &m.name,
                role: &m.role,
                joined: m.joined,
                engine: m.engine,
                runs: m.runs,
                waiting: crate::lanes::waiting_on_person(tickets, &m.name),
                limit: m.limit,
                takes_new: m.takes_new && (held_too || !m.held),
                pm: m.role == crate::roles::PM,
            }
        }
        let candidates: Vec<crate::lanes::Member> = members.iter().map(|m| lane(m, &tickets, false)).collect();
        // With a reserve held, the little room goes to the most urgent tickets first.
        let urgent = if paced { crate::pacing::urgent_first(&tickets) } else { Vec::new() };
        // What the person asked for goes before the squad's own follow-ups and checks (`requests`) —
        // and a baton passed, work already under way, before both (`flow`).
        let requested = self.journals.get(squad).map(|j| j.requests().serving()).unwrap_or_default();
        let batons: Vec<TicketId> = tickets.iter().filter(|t| t.status == TicketStatus::Todo && t.note.as_deref().is_some_and(|n| n.starts_with(crate::flow::BATON_FROM))).map(|t| t.id).collect();
        let merged: Vec<TicketId> = [urgent.as_slice(), batons.as_slice(), first, requested.as_slice()].concat();
        let first: &[TicketId] = if urgent.is_empty() && requested.is_empty() && batons.is_empty() { first } else { &merged };
        // A queue longer than twice what the squad runs at once finishes before it starts.
        let capacity: usize = members.iter().filter(|m| m.takes_new && !m.held).map(|m| m.limit).sum();
        let ready = tickets.iter().filter(|t| t.status == TicketStatus::Todo && t.owner.is_some()).count();
        fit.finishing = crate::flow::prefer_finishing(ready, capacity, 1);
        let assigned = crate::lanes::assign_fitted(&tickets, &candidates, single_project, first, &room, &guarded, &fit);
        // What a hold keeps from starting: what members would take but for it.
        if members.iter().any(|m| m.held) {
            let unheld: Vec<crate::lanes::Member> = members.iter().map(|m| lane(m, &tickets, true)).collect();
            let mut waiting: Vec<(Engine, TicketId)> = crate::lanes::assign_guarded(&tickets, &unheld, single_project, first, &room, &guarded)
                .into_iter()
                .filter(|(t, _)| !assigned.iter().any(|(a, _)| a == t))
                .filter_map(|(t, name)| members.iter().find(|m| m.name == name).filter(|m| m.held).map(|m| (m.engine, t)))
                .collect();
            waiting.sort_by_key(|(_, t)| *t);
            for engine in [Engine::Claude, Engine::Codex] {
                let ids: Vec<TicketId> = waiting.iter().filter(|(e, _)| *e == engine).map(|(_, t)| *t).collect();
                self.tell_held(squad, engine, &ids);
            }
        }
        // The machine overloaded (`machine.rs`): what the person asked for
        // still starts, and each member idle meanwhile one ticket; the rest
        // waits for it to calm down, which dispatches again.
        let assigned = if self.overloaded() {
            let person: HashSet<TicketId> = tickets.iter().filter(|t| t.created_by == "you").map(|t| t.id).chain(requested.iter().copied()).collect();
            let idle: HashSet<&str> = members.iter().filter(|m| m.runs == 0).map(|m| m.name.as_str()).collect();
            machine::paced(assigned, &person, &idle)
        } else {
            assigned
        };
        watchdog::mark("dispatch: work begun");
        for (ticket, name) in assigned {
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
                    self.publish(Event::Notice { undo: None, session: pm, text });
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
    /// edits, the ticket's project otherwise; then its first turn. On the
    /// loop the ticket is only reserved — its run's generation and lease, the
    /// ticket in progress for the member, the run's session open and counted
    /// by the lane policy — and its worktree is made off the loop
    /// ([`workspace`]); the run's first turn starts once that is ready
    /// ([`Runtime::start_run`]). A run with no worktree to make starts at once.
    fn begin_work(&mut self, id: SessionId, ticket_id: TicketId) {
        let home = self.config.home.clone();
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else {
            return;
        };
        let role_edits = session.edits();
        // The PM, which delegates first, on a code ticket of its own.
        let pm_codes = !role_edits && session.edits_on_code_tickets();
        let board_tools = has_tools(session) && self.config.equipe_binary.is_some();
        let projects = self.projects(&squad);
        let home_dir = squad_home::path(&home, &squad);
        // One start of a ticket at a time: not while its workspace is still
        // being made, nor while a run holds it.
        if self.workspaces.busy(&squad, ticket_id) || self.worker_on(&squad, ticket_id).is_some() {
            return;
        }
        // A light design QA session works in its report's folder, with no checkout.
        let lite = self.lite_dir(&squad, ticket_id);
        if let Some(dir) = &lite {
            let _ = std::fs::create_dir_all(dir);
        }
        let Some(journal) = self.journal(&squad) else {
            return;
        };
        // A member the person rested opens no run, whoever asks.
        if journal.is_resting(&name) {
            return;
        }
        let Some(mut ticket) = journal.ticket(ticket_id).ok().flatten() else {
            return;
        };
        let mut saved = journal.ticket_work(ticket_id).unwrap_or_default();
        let after = work_before(&journal.board().unwrap_or_default(), &ticket);
        let fresh = ticket.branch.is_none() || saved.base.is_none();
        let kind = ticket.kind;
        // A task or action is worked in the squad home; what it is about, if anything, is only read.
        let Some(project) = (if let Some(dir) = &lite {
            Some(dir.clone())
        } else if kind.in_home() {
            ticket.project.clone().or_else(|| Some(home_dir.clone()))
        } else {
            ticket.project.clone().or_else(|| projects.first().cloned())
        }) else {
            return;
        };
        let workplace = if kind.in_home() { home_dir.clone() } else { project.clone() };
        // In the squad home every role writes, in its own worktree; on a code
        // ticket a role that edits does — and the PM, when the project is a
        // repository (looked at off the loop): on a plain folder it reads, as
        // it always did.
        let may_edit = (role_edits || kind.in_home() || pm_codes) && lite.is_none();
        let checkout = may_edit.then(|| match (&saved.worktree, &ticket.branch, &saved.base) {
            // In review and sent back: the worktree is still there.
            (Some(path), Some(branch), Some(base)) if path.exists() => workspace::Checkout::Kept { path: path.clone(), branch: branch.clone(), base: base.clone() },
            // Its worktree went away: check its branch out again.
            (_, Some(branch), Some(base)) => workspace::Checkout::Resume { branch: branch.clone(), base: base.clone() },
            // New: from HEAD, with the finished work it depends on.
            _ => workspace::Checkout::New { label: name.clone(), after: after.clone() },
        });
        // A squad home's copy leaves out other tickets' outputs: gigabytes in a home that has done much work.
        let only = if kind.in_home() {
            worktree::home_patterns(&std::iter::once(&ticket_id).chain(&ticket.depends_on).collect::<Vec<_>>())
        } else {
            Vec::new()
        };
        // A recovery the person approved starts under the operation and
        // generation it reserved; any other start takes a fresh one. Either
        // way the run gets a runtime session of its own, even on the same
        // member and engine as the run before.
        let recovered = ticket.execution.as_ref().filter(|e| e.state == ExecutionState::Queued).map(|e| e.operation.clone());
        let operation = recovered.clone().unwrap_or_else(|| format!("{}{}", journal::START_OPERATION, equipe_protocol::new_session_id()));
        let execution_session = equipe_protocol::new_session_id();
        let Some(execution_generation) = journal.acquire_execution(ticket_id, execution_session, &operation, now_secs(), EXECUTION_LEASE_SECS).ok().flatten()
        else {
            return;
        };
        // Until its session runs, a daemon stopped meanwhile puts the ticket back on the board.
        if checkout.is_some() {
            let _ = journal.set_execution_starting(ticket_id, execution_generation, true);
        }

        // Its model and effort, picked once as its lane starts. A tier
        // changed since its last lane starts a fresh conversation, with a
        // handoff note in place of the one before; branch and worktree stay.
        let mut handoff = None;
        if let Some(choice) = self.choose_model(&squad, id, &ticket) {
            if choice.fresh {
                saved.engine_session = None;
                handoff = Some(self.latest_handoff(&squad, ticket_id).unwrap_or_else(|| {
                    "This ticket's model changed, so you start in a fresh conversation: read its record, the work so far on its branch \
(its log and diff), and what was handed in and sent back before you go on."
                        .to_owned()
                }));
            }
            let role = self.sessions.get(&id).and_then(|s| s.member.as_ref()).map(|m| m.role.clone()).unwrap_or_default();
            if let Some(journal) = self.journal(&squad) {
                let _ = journal.set_ticket_model(ticket_id, Some(&choice.ticket));
                let _ = journal.record_model_choice(ticket_id, "lane", Some((name.as_str(), role.as_str())), &choice.ticket);
            }
            ticket.model = Some(choice.ticket);
        }

        let assignee_before = ticket.assignee.clone();
        ticket.status = TicketStatus::InProgress;
        ticket.assignee = Some(name.clone());
        let reopened = ticket
            .note
            .as_ref()
            .filter(|n| n.starts_with("Reopened") || n.starts_with("Resumed") || n.starts_with("Design QA by ") || n.starts_with(crate::flow::BATON_FROM))
            .cloned();
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.ticket_event(ticket_id, "equipe", "assigned", &name);
        }
        let preparing = checkout.is_some();
        let work = Work {
            ticket: ticket_id,
            execution_session,
            execution_generation,
            kind,
            project: project.clone(),
            // Where its turns run is known once its workspace is ready.
            cwd: project.clone(),
            sandbox: None,
            claude_session: saved.engine_session.clone(),
            history: Vec::new(),
            handed_in: None,
            shown: Vec::new(),
            waiting: None,
            screens: false,
            verdict: None,
            waive: None,
            lite: lite.is_some(),
            refused: None,
            preparing,
            end: RunEnd::default(),
        };
        // The ticket runs in a session of its own, beside the member's
        // conversation and its other runs: from here `id` is the run.
        let Some(id) = self.open_run(id, work) else { return };
        let plan_repo = ticket.plan.as_deref().filter(|plan| !Path::new(plan).is_absolute()).and(ticket.project.clone());
        let start = workspace::Start {
            squad: squad.clone(),
            name,
            ticket: ticket_id,
            run: id,
            generation: execution_generation,
            project,
            after,
            fresh,
            recovered: recovered.is_some(),
            reopened,
            handoff,
            board_tools,
            engine_session: saved.engine_session,
            assignee_before,
            began: std::time::Instant::now(),
        };
        let job = workspace::Job {
            start,
            workplace,
            checkout,
            only_in_repo: !role_edits && !kind.in_home(),
            only,
            home,
            id_part: format!("t{ticket_id}-{:x}-{execution_generation}", now_secs()),
            plan_repo,
        };
        if preparing {
            self.publish(Event::TicketChanged { squad, ticket });
            let text = say!(self, "notice.preparing_workspace");
            self.publish(Event::Notice { undo: None, session: id, text });
            self.prepare_workspace(job);
        } else {
            // Nothing to make: it starts now, its plan's main branch one quick look.
            let main = job.plan_repo.as_deref().and_then(|r| worktree::main_branch(r).ok());
            self.start_run(job.start, None, main);
        }
    }

    /// Reserved run `start` has its workspace — `sandbox`, its worktree when
    /// it edits — and starts: the ticket names its branch and worktree, the
    /// screens are told the work started, and its first turn goes.
    /// `main`: its plan's repository's main branch, when it has one.
    fn start_run(&mut self, start: workspace::Start, sandbox: Option<Sandbox>, main: Option<String>) {
        let workspace::Start { squad, name, ticket: ticket_id, run: id, generation, project, after, fresh, recovered, reopened, handoff, board_tools, engine_session, .. } = start;
        let Some(mut ticket) = self.journal(&squad).and_then(|j| j.ticket(ticket_id).ok().flatten()) else { return };
        let cwd = sandbox.as_ref().map_or_else(|| project.clone(), |s| within(s, &project));
        ticket.branch = sandbox.as_ref().map(|s| s.branch.clone()).or(ticket.branch);
        ticket.worktree = sandbox.as_ref().map(|s| s.path.clone());
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.save_ticket(&ticket);
            let _ = journal.set_ticket_work(
                ticket_id,
                &TicketWork { base: sandbox.as_ref().map(|s| s.base.clone()), worktree: sandbox.as_ref().map(|s| s.path.clone()), engine_session },
            );
            let _ = journal.set_execution_starting(ticket_id, generation, false);
        }
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
            work.cwd = cwd;
            work.sandbox = sandbox.clone();
            work.preparing = false;
        }
        self.publish(Event::TicketChanged { squad: squad.clone(), ticket: ticket.clone() });
        // What follows in its conversation is about the ticket's project.
        self.publish(Event::Thread { session: id, project: ticket.project.as_deref().map(board::project_name) });
        let of = self.sessions.get(&id).and_then(|s| s.run_of);
        self.publish(Event::WorkStarted {
            of,
            session: id,
            ticket: ticket_id,
            title: ticket.title.clone(),
            project: project.clone(),
            workspace: sandbox.as_ref().map(workspace_of),
        });
        self.brief_pm(&squad, format!("{name} picked up T-{ticket_id} ({}).", ticket.title), None);
        let language = self.journal(&squad).and_then(Journal::language);
        let lite = self.sessions.get(&id).and_then(|s| s.work.as_ref()).is_some_and(|w| w.lite);
        let batch = if lite { self.journal(&squad).and_then(|j| j.design_qa_batch(ticket_id)) } else { None };
        let checked = self.checked_by(&squad, ticket_id);
        let mut prompt = match (&batch, &checked) {
            // A light session's brief is the whole of it: no checkout to be told about.
            (Some(batch), _) => self.batch_prompt(&squad, batch),
            (None, Some(checked)) if lite => self.lite_prompt(&squad, ticket_id, checked),
            _ => work_prompt(&ticket, &project, sandbox.as_ref(), reopened.as_deref(), recovered, board_tools, language.as_deref()),
        };
        if let (Some(checked), false) = (&checked, lite) {
            let precheck = self.precheck_brief(&squad, ticket_id);
            prompt.push_str(&design_qa::qa_prompt(ticket_id, checked, precheck.as_ref()));
        }
        // Brand work, and only it, takes the designer's full reference pass.
        let designer = self.sessions.get(&id).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role == design_qa::DESIGNER);
        let branding = crate::design::brand::is_branding(&ticket.title, &ticket.body);
        if designer && branding {
            prompt.push_str(&crate::design::brand::note(board_tools));
        }
        // The kinds of work the person stopped: nothing of them is filed from this work either.
        if let Some(stops) = self.stops_context(&squad) {
            prompt.push_str(&format!("\n\n{stops}\n"));
        }
        // A project that inherits a brand: its system is the brand's, changed only on a brand ticket.
        let member = self.sessions.get(&id).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role != crate::roles::PM);
        if member && let Some(note) = crate::design::inherit::prompt_note(&self.config.home.join(crate::design::inherit::DIR), &sandbox.as_ref().map_or_else(|| project.clone(), |s| within(s, &project)).join(crate::design::DIR), branding) {
            prompt.push_str(&note);
        }
        if let Some(plan) = &ticket.plan {
            let repo = ticket.project.as_deref().filter(|_| !Path::new(plan).is_absolute());
            prompt.push_str(&crate::plan::work_note(plan, repo.zip(main.as_deref())));
        }
        if let Some(note) = &handoff {
            prompt.push_str(&format!("\n{note}"));
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

    /// Run `id`'s turn ended with something in the person's hands — its
    /// browser for a sign-in, an approval it asked for: as of unix time `at`
    /// it is still on its ticket, waiting for them to come back, not stalled.
    /// A hand-off holds it for [`turn_end::WITH_PERSON_GRACE`]; an approval,
    /// on its card in front of them, for as long as it is open.
    fn run_waits_on_person(&self, id: SessionId, session: &Session, at: u64) -> bool {
        session.work.as_ref().and_then(|w| w.end.with_person.as_ref()).is_some_and(|w| at.saturating_sub(w.since) < turn_end::WITH_PERSON_GRACE)
            || self.pending.values().any(|p| p.approval.session == id)
    }

    /// Renew leases only for turns that this runtime can still observe. Late
    /// heartbeats are fenced by the stored generation and session id.
    fn renew_execution_leases(&mut self) {
        let now = now_secs();
        let active: Vec<(String, SessionId, TicketId, u64)> = self
            .sessions
            .iter()
            // A run equipe paused holds its lease while it waits to go on, and
            // so does one waiting on the person (its browser in their hands).
            .filter(|(id, session)| session.is_working() || session.is_preparing() || Runtime::is_parked(session) || self.run_waits_on_person(**id, session, now))
            .filter_map(|(_, session)| {
                let work = session.work.as_ref()?;
                Some((session.squad.clone()?, work.execution_session, work.ticket, work.execution_generation))
            })
            .collect();
        let at = now_secs();
        for (squad, session, ticket, generation) in active {
            let Some(journal) = self.journal(&squad) else { continue };
            let _ = journal.heartbeat_execution(ticket, generation, session, at, EXECUTION_LEASE_SECS);
        }
    }

    /// The minute's check, as of unix time `at`: a run whose turn ended
    /// without a hand-in or a wait, and that nothing renewed before its lease
    /// ran out, is stalled. A turn still running is never stalled, however
    /// long it takes. The stalled ticket's member lets go of it and is free
    /// for other work. One a member with the board's tools left that way —
    /// not stopped by the person, not waiting on them — comes back once by
    /// itself when its member is free and its engine has room
    /// ([`Self::may_resume_by_itself`]); stalled so again, it waits for the
    /// person.
    async fn recover_execution_leases(&mut self, at: u64) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let live: Vec<SessionId> = self
                .sessions
                .iter()
                .filter(|(id, session)| {
                    session.squad.as_deref() == Some(squad.as_str())
                        && (session.is_working() || session.is_preparing() || Runtime::is_parked(session) || self.run_waits_on_person(**id, session, at))
                })
                .map(|(_, session)| session)
                .filter_map(|session| session.work.as_ref().map(|work| work.execution_session))
                .collect();
            let stalled = self.journal(&squad).and_then(|j| j.recover_executions(&live, at).ok()).unwrap_or_default();
            if stalled.is_empty() {
                continue;
            }
            for ticket in stalled {
                let run = self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad.as_str())).find(|s| s.work.as_ref().is_some_and(|w| w.ticket == ticket));
                // One that waited on the person past its grace says what for.
                let waited = run.and_then(|s| s.work.as_ref()).and_then(|w| w.end.with_person.as_ref().map(|p| p.what.clone()));
                // One whose turn just ended with nothing holding it — not the
                // person's stop, not a wait on them — may come back once by itself.
                let again = run
                    .filter(|s| waited.is_none() && has_tools(s) && self.config.equipe_binary.is_some())
                    .filter(|s| s.work.as_ref().is_none_or(|w| !w.end.stopped))
                    .and_then(|s| Some((s.name()?.to_owned(), s.engine)));
                if let Some(what) = waited
                    && let Some(journal) = self.journal(&squad)
                {
                    let _ = journal.set_waited_on_person(ticket, &what);
                }
                let again = again.filter(|(name, engine)| self.may_resume_by_itself(&squad, ticket, name, *engine));
                self.stalled(&squad, ticket, again.is_none());
                if let Some((name, _)) = again
                    && !self.resume_by_itself(&squad, ticket, &name)
                {
                    self.tell_stalled(&squad, ticket);
                }
            }
            Box::pin(self.dispatch(&squad)).await;
        }
    }

    /// `ticket`'s run stalled. Its member, if still on it, lets go — in
    /// memory only: nothing is committed, and the ticket stays in progress
    /// with its branch, worktree, commit and uncommitted files exactly as they
    /// were, for the person to resume, requeue or reassign. What its checkout
    /// holds is kept for their card; the PM hears of it when `tell`.
    fn stalled(&mut self, squad: &str, ticket_id: TicketId, tell: bool) {
        let Some(ticket) = self.journal(squad).and_then(|j| j.ticket(ticket_id).ok().flatten()) else { return };
        let generation = ticket.execution.as_ref().map(|e| e.generation);
        // Only the session on that very run lets go.
        let holder = self
            .sessions
            .iter()
            .find(|(_, s)| s.squad.as_deref() == Some(squad) && s.work.as_ref().is_some_and(|w| w.ticket == ticket_id && Some(w.execution_generation) == generation))
            .map(|(&id, _)| id);
        if let Some(id) = holder {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.work = None;
            }
            let of = self.sessions.get(&id).and_then(|s| s.run_of);
        self.publish(Event::WorkEnded { session: id, ticket: ticket_id, of });
            let desk = self.sessions.get(&id).and_then(|s| s.thread.clone());
            self.publish(Event::Thread { session: id, project: desk });
            self.close_run(id);
        }
        // A run whose ticket moved on without it (back on the board, say) only
        // frees its lease: the board starts it again as usual.
        if ticket.status != TicketStatus::InProgress {
            return;
        }
        let Some(journal) = self.journal(squad) else { return };
        let (head, dirty) = ticket.worktree.as_deref().map(worktree::checkout_facts).unwrap_or_default();
        let _ = journal.set_stalled_checkout(ticket_id, head.as_deref(), dirty);
        let reason = ticket.execution.as_ref().and_then(|e| e.reason.clone()).unwrap_or_default();
        let _ = journal.ticket_event(ticket_id, "equipe", "stalled", &reason);
        let Some(ticket) = journal.ticket(ticket_id).ok().flatten() else { return };
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        if tell {
            self.tell_stalled(squad, ticket_id);
        }
    }

    /// The PM and the person hear that `ticket_id`'s run stalled, and what it
    /// waits for: the person's resume, requeue or reassign on its card.
    fn tell_stalled(&mut self, squad: &str, ticket_id: TicketId) {
        let Some(ticket) = self.journal(squad).and_then(|j| j.ticket(ticket_id).ok().flatten()) else { return };
        let (title, who) = (ticket.title.clone(), ticket.assignee.clone().unwrap_or_default());
        let waited = ticket.execution.as_ref().and_then(|e| e.waited_for.clone());
        let (line, shown) = match waited {
            Some(what) => (
                format!(
                    "T-{ticket_id}'s run stopped: {who} waited on the person for {} minutes ({what}) and they haven't come \
back to it. It stays in progress with its branch and worktree as they are, in the person's Needs-you with what it waited \
for: they do it and resume it on its card — you can't, and don't open a new ticket for it.",
                    turn_end::WITH_PERSON_GRACE / 60
                ),
                say!(self, "news.stalled_on_person", id = ticket_id, name = who, what = what, title = title),
            ),
            None => (
                format!(
                    "T-{ticket_id}'s run stopped: {who}'s turn ended without a hand-in or a wait, and nothing ran it since. It \
stays in progress with its branch and worktree as they are. The person resumes, requeues or reassigns it on its card — you \
can't, and don't open a new ticket for it."
                ),
                say!(self, "news.stalled", id = ticket_id, name = who, title = title),
            ),
        };
        self.brief_pm(squad, line, Some(shown));
    }

    /// Whether `ticket`'s run, stalled as its lease ran out with nothing
    /// holding it, comes back once by itself: its member `name` is on the
    /// squad and free to take work — not resting, not moving engines — its
    /// `engine` has room and isn't held, the squad's starts aren't paused, and
    /// equipe hasn't brought this very ticket back already since its last run
    /// began otherwise.
    fn may_resume_by_itself(&self, squad: &str, ticket: TicketId, name: &str, engine: Engine) -> bool {
        let Some(journal) = self.journals.get(squad) else { return false };
        self.member_id(squad, name).is_some()
            && !journal.is_resting(name)
            && !self.engines.moving(squad, name)
            && !self.engines.held(squad, engine)
            && self.engine_out(squad, engine).is_none()
            && !self.turn_ends.paused(squad)
            && journal.resumed_by_itself_last(ticket).is_ok_and(|again| !again)
    }

    /// Bring stalled `ticket` back once, on its own, to `name`: a fresh run on
    /// the same ticket, branch and worktree, as the person's resume does; the
    /// PM and the person told, and the log. Whether it was.
    fn resume_by_itself(&mut self, squad: &str, ticket: TicketId, name: &str) -> bool {
        let (title, changed) = {
            let Some(journal) = self.journal(squad) else { return false };
            let Some(found) = journal.ticket(ticket).ok().flatten() else { return false };
            let generation = found.execution.as_ref().map_or(0, |e| e.generation);
            let operation = format!("{}{ticket}-{generation}", journal::AUTO_RESUME_OPERATION);
            if !matches!(journal.reserve_requeue(ticket, &operation, Some(name), now_secs()), Ok(Requeued::Reserved)) {
                return false;
            }
            let _ = journal.ticket_event(ticket, "equipe", "resumed", &format!("by itself, once, to {name}, on the same branch and worktree: its run stopped without a hand-in"));
            (found.title, journal.ticket(ticket).ok().flatten())
        };
        if let Some(changed) = changed {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: changed });
        }
        crate::daemon_log!("equipe: {squad}: T-{ticket}'s run stalled (lease expired) with {name} free: brought back once by itself");
        let line = format!(
            "T-{ticket}'s run stopped: {name}'s turn ended without a hand-in or a wait, and nothing ran it since. equipe brought it \
back once by itself — a fresh run for {name} on the same ticket, branch and worktree, nothing reset. If it stops so again, it \
waits for the person."
        );
        let shown = say!(self, "news.stalled_resumed", id = ticket, to = name, title = title);
        self.brief_pm(squad, line, Some(shown));
        true
    }

    /// A ticket that can't start (its project isn't a repository, say) goes back
    /// to a proposal with the reason, for the PM or the person to sort out.
    fn cannot_start(&mut self, squad: &str, mut ticket: Ticket, name: &str, reason: &str) {
        let reason = &output_excerpt(reason, OUTPUT_NOTE_MAX);
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
    /// back, while it waits, and when its member left, for the next one; it
    /// goes otherwise — the branch stays to merge.
    fn end_work(&mut self, id: SessionId, how: WorkEnd) {
        // A run is its ticket's: put down, it closes — once its work is in
        // the squad's folder, when it goes there ([`Runtime::home_put_down`]).
        if self.put_down(id, how) {
            self.close_run(id);
        }
    }

    /// A ticket's pictures saved in a worktree or a temp folder, copied where
    /// they outlast it (`crate::visuals::keep`).
    fn keep_pictures(&self, squad: &str, ticket: &Ticket) {
        if !ticket.images.is_empty() {
            crate::visuals::keep::ticket(&self.config.home, squad, ticket.id, &ticket.images);
        }
    }

    /// `remember` / `forget`: the squad's memory changed in its folder off the
    /// loop, in line with the folder's other saves and landings — the loop
    /// never waits on its git — and the member answered once it is in.
    fn remember(&mut self, call: &ToolCall, reply: oneshot::Sender<ToolResult>) {
        let fail = |reply: oneshot::Sender<ToolResult>, text: String| {
            let _ = reply.send(ToolResult { text, is_error: true, image: None });
        };
        let Some(session) = self.caller_id(call) else {
            return fail(reply, format!("{} isn't on the squad {}.", call.member, call.squad));
        };
        let (folder, member, args) = (squad_home::path(&self.config.home, &call.squad), call.member.clone(), call.arguments.clone());
        let language = self.journals.get(&call.squad).and_then(Journal::language);
        let forget = call.name == board::FORGET;
        // What its conversation is shown, worded here, where the words are.
        let text = if forget {
            let Some(fact) = board::arg_str(&args, "name") else {
                return fail(reply, "Which fact? Give its name.".into());
            };
            (say!(self, "notice.forgot", name = member, fact = fact), String::new())
        } else {
            let what = clip(board::arg_str(&args, "description").unwrap_or_default().trim(), 80);
            (say!(self, "notice.remembered", name = member, what = what), say!(self, "notice.memory_updated", name = member, what = what))
        };
        let internal = self.internal.clone();
        tokio::task::spawn_blocking(move || {
            let saved = if forget {
                squad_memory::forget(&folder, &member, board::arg_str(&args, "name").unwrap_or_default(), language.as_deref()).map(|change| (change, text.0))
            } else {
                let fact = squad_memory::Remember {
                    name: board::arg_str(&args, "name").unwrap_or_default(),
                    description: board::arg_str(&args, "description").unwrap_or_default(),
                    kind: board::arg_str(&args, "kind").unwrap_or_default(),
                    body: board::arg_str(&args, "body").unwrap_or_default(),
                    confirmed: args.get("confirmed").and_then(Value::as_bool).unwrap_or(false),
                };
                squad_memory::remember(&folder, &member, &fact, &squad_memory::date(now_secs()), language.as_deref()).map(|(change, new)| (change, if new { text.0 } else { text.1 }))
            };
            let _ = reply.send(match saved {
                Ok((change, text)) => {
                    let what = change.what();
                    let _ = internal.send(Internal::Remembered { session, undo: change.id, text });
                    ToolResult { text: format!("Saved in the squad's memory ({what}). Every member has it from their next turn."), is_error: false, image: None }
                }
                Err(text) => ToolResult { text, is_error: true, image: None },
            });
        });
    }

    /// Put `id`'s ticket down as `how` says. `false` while its work goes into
    /// the squad's folder off the loop: [`Runtime::home_put_down`] finishes it.
    fn put_down(&mut self, id: SessionId, how: WorkEnd) -> bool {
        let Some(session) = self.sessions.get_mut(&id) else {
            return true;
        };
        let Some(work) = session.work.take() else {
            return true;
        };
        // A design QA handed in is finished by its verdict, not the person's review.
        let how = if how == WorkEnd::HandedIn && work.verdict.is_some() { WorkEnd::Checked } else { how };
        let (Some(squad), Some(name)) = (session.squad.clone(), session.name().map(str::to_owned)) else {
            return true;
        };
        self.put_down_as(id, how, work, squad, name, None)
    }

    /// A task's or action's work, accepted, went into the squad's folder off
    /// the loop ([`Runtime::land_put_down`]), or didn't: the rest of putting
    /// it down, then its run closes.
    fn home_put_down(&mut self, put: HomePutDown) {
        let HomePutDown { id, how, work, squad, name, committed, landed } = put;
        if let (Err(err), Some(sandbox)) = (committed, &work.sandbox) {
            let message = say!(self, "error.ticket_not_committed", id = work.ticket, err = err, path = sandbox.path.display());
            self.error(Some(id), message, None);
        }
        self.put_down_as(id, how, work, squad, name, Some(landed));
        self.close_run(id);
    }

    /// [`Runtime::put_down`], with `home` its work's landing in the squad's
    /// folder once made off the loop. Without it, a task or action accepted
    /// goes there off the loop first, and `false` says so: the loop never
    /// waits on the folder's git, nor on its other landings.
    fn put_down_as(&mut self, id: SessionId, how: WorkEnd, work: Work, squad: String, name: String, home: Option<HomeLanded>) -> bool {
        let Some(mut ticket) = self.journal(&squad).and_then(|j| j.ticket(work.ticket).ok().flatten()) else {
            return true;
        };
        let summary = work.handed_in.clone().or_else(|| work.waiting.clone()).or_else(|| work.end.handed_back.as_ref().map(|h| h.note.clone()));
        let mut kept_worktree = None;
        // A task or action accepted goes into the squad's folder now; `Err` when the folder changed under it.
        let mut landed: Option<Result<(), String>> = None;
        // What it showed so far, kept before its worktree may go below.
        if home.is_none() {
            self.keep_pictures(&squad, &ticket);
        }
        if let Some(sandbox) = &work.sandbox {
            let body = summary.clone().unwrap_or_else(|| ticket.body.clone());
            let message =
                format!("equipe: T-{} {}\n\n{body}\n\nRun by {name} on equipe through Claude Code.", ticket.id, ticket.title);
            if matches!(how, WorkEnd::Accepted | WorkEnd::Checked) && ticket.kind.in_home() {
                // Committed and landed off the loop, then back here.
                let Some(home) = home else {
                    self.land_put_down(id, how, work, squad, name, &ticket, message);
                    return false;
                };
                if how == WorkEnd::Checked {
                    ticket.images = work.shown.clone();
                }
                let result = self.home_landed(&squad, &mut ticket, home);
                if result.is_err() {
                    kept_worktree = Some(sandbox.path.clone());
                }
                landed = Some(result);
            } else {
                watchdog::mark("work's end: commit");
                if let Err(err) = sandbox.commit_all(&message) {
                    let message = say!(self, "error.ticket_not_committed", id = ticket.id, err = err, path = sandbox.path.display());
                    self.error(Some(id), message, None);
                }
                // Work its member left stays checked out for whoever takes it
                // next: removing gigabytes of worktree, and checking the branch
                // out again for the next member, held the daemon's loop for
                // seconds per ticket.
                if matches!(how, WorkEnd::HandedIn | WorkEnd::Waiting | WorkEnd::Left | WorkEnd::HandedBack) {
                    kept_worktree = Some(sandbox.path.clone());
                } else {
                    ticket.branch = release(sandbox);
                }
            }
            ticket.worktree = kept_worktree.clone();
        }
        // Code handed in, committed on its branch: Nemotron reads its diff, off the loop.
        if how == WorkEnd::HandedIn
            && !ticket.kind.in_home()
            && let Some(sandbox) = &work.sandbox
        {
            // Its word on the review goes to the member: the run closes now.
            let member = self.member_of(id);
            self.ask_for_review(&squad, &ticket, sandbox.clone(), member);
        }
        let handed_in = match (&how, &summary) {
            (WorkEnd::HandedIn, Some(summary)) if ticket.kind.in_home() => Some(self.home_hand_in(&squad, &ticket, work.sandbox.as_ref(), summary)),
            _ => summary.clone(),
        };
        // What the hand-in shows: the pictures named, then the images its branch changed.
        let images = match &work.sandbox {
            Some(sandbox) if how == WorkEnd::HandedIn => {
                let roots = [squad_home::path(&self.config.home, &squad), sandbox.path.clone()];
                {
                    watchdog::mark("work's end: hand-in pictures");
                    // The design captures it made are on disk, never committed: its manifest names them.
                    let mut changed = sandbox.changed_files().unwrap_or_default();
                    // What the branch changed decides whether its member may land it (`runtime::flow`).
                    self.handed_in_files.insert((squad.clone(), ticket.id), changed.clone());
                    // …and what its SQL migrations do, read where it was made: one that only adds lands like code.
                    let sql = crate::person_gate::read_sql(&sandbox.path, &changed);
                    self.handed_in_sql.insert((squad.clone(), ticket.id), sql);
                    changed.extend(crate::design::captures::changed(&sandbox.path, &sandbox.base));
                    crate::visuals::hand_in(&work.shown, &changed, &sandbox.path, &roots)
                }
            }
            _ => work.shown.clone(),
        };
        if !matches!(how, WorkEnd::HandedIn | WorkEnd::Accepted | WorkEnd::Checked) {
            ticket.images.clear();
        }
        // What the hand-in's news is about: the ticket, or the one a design QA checked.
        let (mut about, mut to): (Option<(TicketId, String)>, Option<String>) = (None, None);
        // Handed back: why, and what the PM is told to do about it.
        let mut handed_back: Option<(turn_end::Reason, String)> = None;
        // What the PM is told, in English, and what the person reads (`news`).
        let (by, event, line, news) = match how {
            WorkEnd::HandedIn => {
                self.tell(&squad, crate::telemetry::Happened::WorkHandedIn { kind: ticket.kind });
                ticket.status = TicketStatus::Review;
                ticket.note = handed_in;
                ticket.images = images;
                match work.screens.then(|| self.ask_design_qa(&squad, &mut ticket, &name, work.waive.as_deref())).flatten() {
                    // Held on itself while its screens are prechecked: no design QA ticket yet.
                    Some(qa) if qa == ticket.id => {
                        let line = format!("{name} handed in T-{}; equipe prechecks its screens first — a designer checks it only if the precheck finds it needs one, else it goes into the daily design review", ticket.id);
                        (name.as_str(), "handed in", line, "news.design_qa_prechecking")
                    }
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
                let checked = self.design_qa_checked(&squad, &ticket, &name, &verdict, true);
                let (id, engineer) = checked.as_ref().map_or((ticket.id, String::new()), |(t, e)| (t.id, e.clone()));
                let reviewed = checked.is_none() && self.design_batch_kind(&squad, ticket.id).is_some();
                if let Some((checked, engineer)) = checked {
                    about = Some((checked.id, checked.title));
                    to = Some(engineer);
                }
                if reviewed {
                    (name.as_str(), "design review done", format!("{name} finished the design review T-{id}: {}", verdict.findings.lines().last().unwrap_or_default()), "news.design_review_done")
                } else if verdict.passed {
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
            WorkEnd::HandedBack => {
                let hand = work.end.handed_back.clone().unwrap_or(turn_end::HandBack { reason: turn_end::Reason::Cannot, tickets: Vec::new(), suggest: None, note: String::new(), blocked_on: None });
                let (event, line, news) = self.handed_back(&squad, &mut ticket, &name, &hand);
                // Waiting on missing work: whose, for the person's line.
                if event == crate::blocked_on::BLOCKED_ON {
                    to = hand.blocked_on.as_ref().map(|m| m.role.clone());
                }
                handed_back = Some((hand.reason, line.clone()));
                (name.as_str(), event, line, news)
            }
            WorkEnd::Reassigned { ref to } => {
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
        // Passed on by the baton: who has it now, for its record; the next member starts a conversation of its own.
        let baton_to = handed_back.as_ref().filter(|(r, _)| *r == turn_end::Reason::Baton).map(|_| ticket.owner.clone().unwrap_or_default());
        // Its pictures saved in its worktree or a temp folder outlast them:
        // the worktree goes once the work lands, and they were never committed.
        watchdog::mark("work's end: keep pictures");
        self.keep_pictures(&squad, &ticket);
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.save_ticket(&ticket);
            let mut saved = journal.ticket_work(ticket.id).unwrap_or_default();
            saved.worktree = kept_worktree;
            if baton_to.is_some() {
                saved.engine_session = None;
            }
            let _ = journal.set_ticket_work(ticket.id, &saved);
            let text = match &baton_to {
                Some(to) => format!("{to} · {}", summary.as_deref().unwrap_or("")),
                None => summary.clone().unwrap_or_default(),
            };
            let _ = journal.ticket_event(ticket.id, &by, event, &text);
            if work.execution_generation > 0 {
                let state = if how == WorkEnd::Waiting { ExecutionState::Waiting } else { ExecutionState::Stopped };
                let reason = match how {
                    WorkEnd::Waiting => "waiting_for_person",
                    WorkEnd::HandedIn => "handed_in",
                    WorkEnd::Accepted => "accepted",
                    WorkEnd::Cancelled => "cancelled",
                    WorkEnd::Checked => "design_qa_ended",
                    WorkEnd::Left => "member_left",
                    WorkEnd::Reassigned { .. } => "reassigned",
                    WorkEnd::HandedBack => "handed_back",
                };
                let _ = journal.finish_execution(ticket.id, work.execution_generation, state, Some(reason));
            }
        }
        // Work now ready for review — this ticket handed in, or the one a
        // designer's QA just passed — when the PM accepts it. Code the PM
        // changed itself isn't: it waits for the person.
        let pm_name = self.pm_of(&squad).and_then(|pm| self.sessions.get(&pm)).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
        let the_pms = |t: &Ticket| crate::accept::own_code(t, &pm_name).is_some();
        let ready_for_pm = if !self.journal(&squad).is_some_and(|j| j.pm_accepts()) {
            None
        } else if ticket.ready_for_review() {
            (!the_pms(&ticket)).then_some(ticket.id)
        } else {
            about.as_ref().map(|(id, _)| *id).filter(|id| self.journal(&squad).and_then(|j| j.ticket(*id).ok().flatten()).is_some_and(|t| t.ready_for_review() && !the_pms(&t)))
        };
        let ticket_id = ticket.id;
        let (about_id, about_title) = about.unwrap_or_else(|| (ticket.id, ticket.title.clone()));
        let to = to.or_else(|| ticket.owner.clone()).unwrap_or_default();
        let shown = self.said(news, &[("id", about_id.to_string()), ("name", name.clone()), ("title", about_title), ("to", to)]);
        self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
        let of = self.sessions.get(&id).and_then(|s| s.run_of);
        self.publish(Event::WorkEnded { session: id, ticket: ticket_id, of });
        // A member that says it cannot do the work, or ended its turns twice without handing in:
        // the next lane, once the PM has settled it, starts one tier up.
        if let Some((reason, _)) = &handed_back
            && matches!(reason, turn_end::Reason::Cannot | turn_end::Reason::NoEnd)
            && self.journal(&squad).is_some_and(|j| j.model_choice() != equipe_protocol::tiers::ModelChoice::Off)
        {
            let why = if *reason == turn_end::Reason::Cannot { "its member handed it back as beyond it" } else { "its member's turns ended twice without a hand-in" };
            self.escalate(&squad, ticket_id, why);
        }
        // Back at its desk: the thread it was in there.
        let desk = self.sessions.get(&id).and_then(|s| s.thread.clone());
        self.publish(Event::Thread { session: id, project: desk });
        self.publish(Event::Notice { undo: None, session: id, text: shown.clone() });
        let briefing = match &summary {
            // A hand-back's line says its note already.
            Some(_) if handed_back.is_some() => line.clone(),
            Some(summary) => format!("{line}: {summary}"),
            None => format!("{line}."),
        };
        if self.pm_of(&squad) != Some(self.member_of(id)) {
            self.brief_pm(&squad, briefing, Some(shown));
            // Where the PM accepts, work ready for review is its to act on now —
            // a note it would read only when next spoken to leaves it waiting.
            // Its member lands it when nothing in it needs a person (`runtime::flow`); otherwise the PM is woken.
            if let Some(ready) = ready_for_pm {
                let prompt = format!(
                    "[equipe: T-{ready} is handed in and ready for you. Check it and accept it with `accept_work` (with \
`push` when the person asked for it to go live), send it back with `comment` and `review: changes`, or — if it needs the \
person — put it to them with `propose_accept`. A branch only behind main is no reason to send it back. Then carry on \
with what else waits.]"
                );
                self.work_ready(&squad, ready, prompt);
            }
            // A ticket handed back to the PM is its to settle now, not when next spoken to; a baton isn't the PM's.
            if let Some((_, line)) = handed_back.filter(|(r, _)| !matches!(r, turn_end::Reason::WaitingOn | turn_end::Reason::Baton))
                && let Some(pm) = self.pm_of(&squad)
            {
                let prompt = format!("[equipe: {line} Settle it now, then carry on with what else waits.]");
                let _ = self.internal.send(Internal::Wake { session: pm, prompt });
            }
        }
        true
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
                    self.publish(Event::Notice { undo: None, session, text });
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

    /// A task's or action's work, accepted, into the squad home's main, off
    /// the loop: committed in its worktree, the person's own changes in the
    /// folder saved, then the branch merges — no question, no git words — and
    /// its worktree and branch go. A conflict merges nothing and keeps both.
    /// [`Runtime::home_put_down`] takes it from there.
    #[allow(clippy::too_many_arguments)]
    fn land_put_down(&mut self, id: SessionId, how: WorkEnd, work: Work, squad: String, name: String, ticket: &Ticket, commit: String) {
        let Some(sandbox) = work.sandbox.clone() else {
            return;
        };
        let (merge, save_persons) = (home_merge_message(ticket), !self.desk_busy(&squad));
        crate::daemon_log!("equipe: {squad}: T-{} goes into the squad's folder", ticket.id);
        let internal = self.internal.clone();
        tokio::task::spawn_blocking(move || {
            let committed = sandbox.commit_all(&commit).map(drop).map_err(|err| err.to_string());
            let landed = land_home(&sandbox, &merge, save_persons);
            let _ = internal.send(Internal::HomePutDown(Box::new(HomePutDown { id, how, work, squad, name, committed, landed })));
        });
    }

    /// Whether a member of `squad` writes at its desk (in the squad's folder,
    /// on no ticket) right now: what changed there otherwise is the person's.
    fn desk_busy(&self, squad: &str) -> bool {
        self.sessions.values().any(|s| s.squad.as_deref() == Some(squad) && s.work.is_none() && s.is_working())
    }

    /// The git side of [`Runtime::land_in_home`] is through, wherever it ran:
    /// what it kept out of git said, the ticket's pictures shown from the
    /// folder they landed in — or why nothing merged, on its record.
    fn home_landed(&mut self, squad: &str, ticket: &mut Ticket, landed: HomeLanded) -> Result<(), String> {
        let kept = match landed.result {
            Ok(kept) => kept,
            Err(err) => {
                if let Some(journal) = self.journal(squad) {
                    let _ = journal.ticket_event(ticket.id, "equipe", "not merged", &err);
                }
                return Err(err);
            }
        };
        if let Some(kept) = kept {
            self.captures_kept_out(squad, ticket, &kept);
        }
        let folder = squad_home::path(&self.config.home, squad);
        ticket.images = crate::visuals::moved(&ticket.images, &landed.from, &folder.canonicalize().unwrap_or(folder));
        ticket.branch = None;
        ticket.worktree = None;
        Ok(())
    }

    /// The git work closing `ticket` as `status` takes — its worktree
    /// released, or a task's work landed in the squad's folder — read from
    /// what is cheap to read here; [`CloseGit::run`] does it, on the loop or off it.
    pub(super) fn close_git(&self, squad: &str, ticket: &Ticket, saved: &TicketWork, status: TicketStatus) -> Option<CloseGit> {
        let workplace = if ticket.kind.in_home() { Some(squad_home::path(&self.config.home, squad)) } else { ticket.project.clone() };
        let (path, branch, base, workplace) = (saved.worktree.clone()?, ticket.branch.clone()?, saved.base.clone()?, workplace?);
        let home = (status == TicketStatus::Done && ticket.kind.in_home()).then(|| HomeClose {
            commit: format!("equipe: T-{} {}", ticket.id, ticket.title),
            merge: home_merge_message(ticket),
            save_persons: !self.desk_busy(squad),
        });
        Some(CloseGit { workplace, path, branch, base, home })
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
    fn close_ticket(&mut self, squad: &str, ticket: Ticket, status: TicketStatus) {
        let saved = self.journal(squad).and_then(|j| j.ticket_work(ticket.id).ok()).unwrap_or_default();
        // Its git work, done off the loop already (the PM's `accept_work`), or
        // now: a task's work lands in the squad's folder off the loop, in line
        // with its other landings — the loop never waits on them — and
        // [`Runtime::closed_git`] closes the ticket once it is through.
        let closed = match self.closing.remove(&(squad.to_owned(), ticket.id)) {
            Some(closed) => Some(closed),
            None => match self.close_git(squad, &ticket, &saved, status) {
                // A worktree only released: no landing to wait on, done here as before.
                Some(git) if !git.lands() => {
                    watchdog::mark("closing a ticket: its worktree");
                    git.run()
                }
                Some(git) => {
                    let key = (squad.to_owned(), ticket.id);
                    // Asked twice: the git work under way closes it once.
                    if self.closing_git.insert(key.clone()) {
                        let internal = self.internal.clone();
                        tokio::task::spawn_blocking(move || {
                            let closed = git.run();
                            let _ = internal.send(Internal::ClosedGit { squad: key.0, ticket: key.1, status, closed });
                        });
                    }
                    return;
                }
                None => None,
            },
        };
        self.close_ticket_with(squad, ticket, status, saved, closed);
    }

    /// A ticket's git work as it closed is through, off the loop
    /// ([`Runtime::close_ticket`]): it closes now, as it stands now.
    fn closed_git(&mut self, squad: &str, ticket: TicketId, status: TicketStatus, closed: Option<Closed>) {
        self.closing_git.remove(&(squad.to_owned(), ticket));
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        // Closed meanwhile some other way: nothing more to do.
        if found.status.is_closed() {
            return;
        }
        let saved = self.journal(squad).and_then(|j| j.ticket_work(ticket).ok()).unwrap_or_default();
        self.close_ticket_with(squad, found, status, saved, closed);
    }

    fn close_ticket_with(&mut self, squad: &str, mut ticket: Ticket, status: TicketStatus, saved: TicketWork, closed: Option<Closed>) {
        match closed {
            Some(Closed::Home(landed)) => {
                if self.home_landed(squad, &mut ticket, landed).is_err() {
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
            }
            Some(Closed::Released(branch)) => ticket.branch = branch,
            None => {}
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
        // Its lane has ended: publishing it closed clears the temporary folder it worked in, not the merge.
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
        self.brief_pm(squad, line, Some(shown));
    }

    /// A ticket asked to become what it already is — cancelled again, done
    /// again, from a card or a screen that hadn't caught up: no banner (the
    /// person asked for what is so), only the ticket sent again so every
    /// screen shows it as it is.
    fn already_so(&mut self, squad: &str, ticket: Ticket) {
        crate::daemon_log!("equipe: {squad}: T-{} is already {:?}; nothing to do", ticket.id, ticket.status);
        self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket });
    }

    /// The person accepts a ticket: it is done, and its branch stays to merge.
    async fn accept(&mut self, squad: &str, ticket: TicketId, by: crate::telemetry::Accepter) {
        // The person's word comes before a designer's.
        self.drop_design_qa(squad, ticket);
        let Some(found) = self.find_ticket(squad, ticket) else {
            return;
        };
        // A proposal accepted is taken on: ready for its owner, not done.
        if found.status == TicketStatus::Proposed {
            return self.take_on(squad, found).await;
        }
        // Already as asked (a card or a screen behind): nothing to say, the screens mend.
        if found.status == TicketStatus::Done {
            return self.already_so(squad, found);
        }
        if found.status.is_closed() {
            let message = say!(self, "error.already_closed", id = ticket, status = crate::i18n::label::status(self.config.lang, found.status));
            return self.squad_error(squad, message);
        }
        if found.status == TicketStatus::Review {
            self.tell(squad, crate::telemetry::Happened::WorkAccepted { kind: found.kind, by });
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

    /// Whether `ticket`, accepted and not on main, is on its way there now:
    /// its landing is in line or under way ([`Runtime::merge`]).
    pub(super) fn landing(&self, squad: &str, ticket: &Ticket) -> bool {
        ticket.not_on_main() && self.merging.contains(&(squad.to_owned(), ticket.id))
    }

    /// Its landing just went in line: every screen shows it on its way to main.
    pub(super) fn show_landing(&mut self, squad: &str, ticket: TicketId) {
        if let Some(t) = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()) {
            self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: t });
        }
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
        self.show_landing(squad, ticket);
        let message = format!("equipe: merge T-{ticket} {}\n\nAccepted; merged from {branch}.", found.title);
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            // In line with the repository's other landings: merged and pushed as one.
            let result = landing_tries(|| {
                worktree::one_landing_at_a_time(&repo, || {
                    worktree::merge_into_main(&repo, &branch, &message).map(|main| (main, push.then(|| worktree::push_main(&repo).map(|_| ()))))
                })
            });
            let kept = crate::design::captures::take_report(&repo, &branch);
            let _ = internal.send(Internal::Merged { squad, ticket, result, kept, push });
        });
    }

    /// A merge the person asked for is through, or merged nothing.
    fn on_merged(&mut self, squad: &str, ticket: TicketId, result: MergeResult, kept: Option<crate::design::captures::Report>, push: bool) {
        self.merging.remove(&(squad.to_owned(), ticket));
        let retried = self.merge_retries.remove(&(squad.to_owned(), ticket));
        let Some(mut found) = self.find_ticket(squad, ticket) else {
            return;
        };
        if let Some(kept) = kept {
            self.captures_kept_out(squad, &found, &kept);
        }
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
                // Anything but a conflict may pass (main's checkout busy, main
                // moved on meanwhile): accepted isn't merged, the PM hears it,
                // and equipe tries again on its own a few times.
                if !err.contains(" conflicts with ") {
                    let again = retried.as_ref().is_some_and(|r| r.running);
                    self.merge_failed(squad, &found, &err, push, retried);
                    // Said once, when it first failed: a try again failing says nothing more.
                    if again {
                        return;
                    }
                }
                // A conflict is work: its owner merges main in and resolves it,
                // and the person merges again from the board.
                let owner = found.owner.clone().or_else(|| found.assignee.clone()).filter(|_| err.contains(" conflicts with "));
                let message = match owner {
                    Some(owner) => {
                        let branch = found.branch.clone().unwrap_or_else(|| "its branch".into());
                        let line = format!(
                            "T-{ticket} couldn't be merged into main: {err}. In the ticket's worktree, merge main into {branch}, resolve the conflict keeping what both sides meant, run the tests and commit. Then tell the PM it's ready: it is accepted already, and lands again with `accept_work` — nobody accepts it twice."
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
        let mut went_out = false;
        match pushed {
            Some(Ok(())) => {
                line.push_str(" and pushed it");
                news = "news.merged_pushed";
                went_out = true;
            }
            // A project kept on this machine only: merged is all there is to do.
            Some(Err(err)) if err == worktree::NO_ORIGIN => {
                line.push_str(" (it has no remote to push to)");
                news = "news.merged_local";
            }
            Some(Err(err)) => {
                let message = match worktree::conflicts_with_origin(&err) {
                    // Origin's main moved on elsewhere and the two conflict: said
                    // plainly, and the PM hears which files, to have them merged by hand.
                    Some(files) => {
                        let (count, files) = (files.len(), files.join(", "));
                        line.push_str(&format!(", but it wasn't pushed: origin's {main} has commits this one doesn't, and they conflict in {files} — someone has to merge origin's {main} in by hand"));
                        say!(self, "error.push_conflicts", branch = main, count = count, files = files)
                    }
                    None => say!(self, "error.not_pushed", branch = main, err = err),
                };
                self.squad_error(squad, message);
            }
            None => {}
        }
        let shown = self.said(news, &[("id", ticket.to_string()), ("title", found.title.clone()), ("branch", main.clone())]);
        let project = found.project.as_deref().map(PathBuf::from);
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
        // Pushed is on main, not live: the project's go-live command, if it has one, runs next.
        if went_out && let Some(project) = project {
            self.go_live_landed(squad, project, Some(ticket));
        }
    }

    /// A merge failed for another reason than a conflict: the ticket is
    /// accepted but not on main. Tried again in a while, up to
    /// [`MERGE_TRIES`] times; the PM hears it the first time and when equipe
    /// stops trying, so nobody takes it for merged.
    fn merge_failed(&mut self, squad: &str, t: &Ticket, err: &str, push: bool, before: Option<MergeRetry>) {
        let failed = before.map_or(1, |b| b.failed + 1);
        let ticket = t.id;
        if failed > MERGE_TRIES {
            // Said once; tried again every half hour from here, so it lands once cleared.
            if failed == MERGE_TRIES + 1 {
                crate::daemon_log!("equipe: {squad}: T-{ticket} still isn't merged after {MERGE_TRIES} tries: {err}");
                self.brief_pm(
                    squad,
                    format!(
                        "T-{ticket} · {} is accepted but still not on main after {MERGE_TRIES} tries ({err}). It stays \"accepted, not on main\" \
on the board. Find out what is in the way — usually uncommitted changes in the project's main checkout — and get it cleared. It is accepted \
already: equipe keeps trying on its own and lands it once it can, or land it at once with `accept_work` — never put it to the person again.",
                        t.title
                    ),
                    None,
                );
            }
            let next = std::time::Instant::now() + MERGE_RETRY_SLOW;
            self.merge_retries.insert((squad.to_owned(), ticket), MergeRetry { push, failed, next, running: false });
            return;
        }
        if failed == 1 {
            self.brief_pm(
                squad,
                format!(
                    "T-{ticket} · {} was accepted but NOT merged into main: {err}. It shows \"accepted, not on main\" on the board, not merged. \
equipe tries the merge again on its own every few minutes; you hear if it still can't.",
                    t.title
                ),
                None,
            );
        }
        let next = std::time::Instant::now() + MERGE_RETRY_AFTER;
        self.merge_retries.insert((squad.to_owned(), ticket), MergeRetry { push, failed, next, running: false });
    }

    /// Failed merges whose time has come are tried again — and, after a
    /// restart, done tickets a recent merge failed on are put back on that
    /// list. A ticket merged meanwhile, kept on its branch, or moved, isn't.
    fn retry_merges(&mut self) {
        let now = std::time::Instant::now();
        // What a run before this one left failed lately: tried again from here.
        if !self.merge_retries_seeded {
            self.merge_retries_seeded = true;
            let squads: Vec<String> = self.journals.keys().cloned().collect();
            for squad in squads {
                for t in self.tickets(&squad).iter() {
                    if t.not_on_main() && matches!(t.off_main, Some(OffMain::Failed { .. })) && t.updated_at.is_some_and(|at| now_secs().saturating_sub(at) < MERGE_RETRY_WITHIN) {
                        self.merge_retries.entry((squad.clone(), t.id)).or_insert(MergeRetry { push: false, failed: 1, next: now, running: false });
                    }
                }
            }
        }
        let due: Vec<((String, TicketId), bool)> =
            self.merge_retries.iter().filter(|(key, r)| r.next <= now && !self.merging.contains(*key)).map(|(key, r)| (key.clone(), r.push)).collect();
        for ((squad, ticket), push) in due {
            let still = self.find_ticket(&squad, ticket).is_some_and(|t| t.not_on_main() && matches!(t.off_main, Some(OffMain::Failed { .. })));
            if !still {
                self.merge_retries.remove(&(squad, ticket));
                continue;
            }
            // Its next try waits while this one runs; how it went sets the one after.
            if let Some(r) = self.merge_retries.get_mut(&(squad.clone(), ticket)) {
                r.next = now + MERGE_RETRY_AFTER;
                r.running = true;
            }
            crate::daemon_log!("equipe: {squad}: trying T-{ticket}'s merge again");
            self.merge(&squad, ticket, push);
        }
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
            for t in self.tickets(&squad).iter().filter(|t| t.not_on_main() && !t.kind.in_home()) {
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
        // Design work sent back with a reason: the person's taste, in their words.
        let by_designer = [found.assignee.as_deref(), found.owner.as_deref()].into_iter().flatten().any(|who| who == "designer" || self.member_id(squad, who).and_then(|id| self.sessions.get(&id)).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role == "designer"));
        if let Some(entry) = by_designer.then(|| crate::taste::sent_back(found.id, &found.title, &said, &squad_memory::date(now_secs()))).flatten() {
            self.learn_taste(squad, &entry);
        }
        let note = if note.trim().is_empty() { "Reopened.".to_owned() } else { format!("Reopened: {}", note.trim()) };
        found.owner = found.assignee.take().or(found.owner);
        found.status = TicketStatus::Todo;
        found.note = Some(note.clone());
        if let Some(journal) = self.journal(squad) {
            let _ = journal.save_ticket(&found);
            let _ = journal.ticket_event(ticket, "you", "reopened", &note);
        }
        // What it asked for under the old direction no longer waits on the person.
        self.withdraw_ticket_cards(squad, ticket, withdraw::TicketTurn::SentBack);
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

    /// The person recovers a stalled ticket (`Command::Requeue`, theirs
    /// only): its same member goes on, whoever of its role is free next
    /// starts, or another member takes it over. The recovery and the move
    /// back to the board are one journal transaction; the start follows at
    /// once when its owner is free, in a fresh runtime session on the same
    /// ticket, branch and worktree. The same `operation` twice is one
    /// recovery. What happened is said: started or queued, done already, or
    /// why nothing changed.
    async fn requeue(&mut self, squad: &str, ticket: TicketId, operation: &str, how: equipe_protocol::Recovery) {
        use equipe_protocol::Recovery;
        if operation.trim().is_empty() {
            return self.squad_error(squad, say!(self, "error.recovery_no_operation", id = ticket));
        }
        let Some(found) = self.find_ticket(squad, ticket) else { return };
        let team = self.team(squad);
        let on_team = |name: &str| team.iter().any(|(_, member, _)| member == name);
        // The run's member: who held it, or — queued already — who it waits for.
        let member = found.assignee.clone().or_else(|| found.owner.clone().filter(|o| on_team(o))).unwrap_or_default();
        let owner = match &how {
            Recovery::Resume if on_team(&member) => member.clone(),
            Recovery::Resume => return self.squad_error(squad, say!(self, "error.recovery_member_gone", id = ticket, name = member)),
            Recovery::Requeue => team
                .iter()
                .find(|(_, name, _)| *name == member)
                .map(|(_, _, role)| role.clone())
                .unwrap_or_else(|| board::role_of(found.owner.as_deref().unwrap_or(&member)).to_owned()),
            Recovery::Reassign(to) if on_team(to) => to.clone(),
            Recovery::Reassign(to) => return self.squad_error(squad, say!(self, "error.no_member", name = to)),
        };
        let Some(journal) = self.journal(squad) else { return };
        match journal.reserve_requeue(ticket, operation, Some(&owner), now_secs()) {
            Ok(Requeued::Reserved) => {
                let (event, key) = match how {
                    Recovery::Resume => ("resumed", "news.recovery_resume"),
                    Recovery::Requeue => ("requeued", "news.recovery_requeue"),
                    Recovery::Reassign(_) => ("reassigned", "news.recovery_reassign"),
                };
                let _ = journal.ticket_event(ticket, "you", event, &format!("to {owner}, on the same branch and worktree"));
                if let Ok(Some(changed)) = journal.ticket(ticket) {
                    self.publish(Event::TicketChanged { squad: squad.to_owned(), ticket: changed });
                }
                // A fresh run asks afresh: what the stalled one asked for doesn't wait on the person.
                self.withdraw_ticket_cards(squad, ticket, withdraw::TicketTurn::Restarted);
                let line = format!(
                    "The person brought T-{ticket} back for {owner}: a fresh run on the same ticket, branch and worktree, nothing reset. \
It starts when {owner} is free."
                );
                let shown = self.said(key, &[("id", ticket.to_string()), ("title", found.title.clone()), ("to", owner.clone())]);
                self.brief_pm(squad, line, Some(shown));
                self.dispatch(squad).await;
            }
            Ok(Requeued::AlreadyApplied) => {
                let text = say!(self, "notice.recovery_applied", id = ticket);
                if let Some(pm) = self.pm_of(squad) {
                    self.publish(Event::Notice { undo: None, session: pm, text });
                }
            }
            Ok(Requeued::NotStalled) => self.squad_error(squad, say!(self, "error.recovery_not_stalled", id = ticket)),
            Err(err) => self.squad_error(squad, say!(self, "error.recovery_failed", id = ticket, err = err)),
        }
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
        if found.status == TicketStatus::Cancelled {
            return self.already_so(squad, found);
        }
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
    async fn tool(&mut self, mut call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        // Kept keys and what looks like one cut out of what the member wrote;
        // told to keep one it let slip, with the answer to this very call.
        let reply = if self.guard_call(&mut call) {
            let (inner, answered) = oneshot::channel::<ToolResult>();
            tokio::spawn(async move {
                if let Ok(mut result) = answered.await {
                    result.text = format!("{}\n\n{}", result.text, crate::squad_secrets::SAVE_NUDGE);
                    let _ = reply.send(result);
                }
            });
            inner
        } else {
            reply
        };
        // A run that no longer holds its ticket changes nothing: no ticket,
        // no question or permission to the person, no estimate, no hand-in.
        // Reading the board is still fine.
        if let Err(why) = self.caller(&call)
            && call.run.is_some()
            && !crate::lanes::reads_only(&call.name)
        {
            if let Some(run) = call.run
                && let Some(journal) = self.journal(&call.squad)
            {
                let _ = journal.ticket_event(run.ticket, &call.member, "stale call", &format!("{} from generation {}", call.name, run.generation));
            }
            let _ = reply.send(if call.name == permissions::APPROVE_TOOL { verdict(Decision::Deny, &json!({}), &why) } else { ToolResult::error(why) });
            return;
        }
        if call.name == permissions::APPROVE_TOOL {
            return self.ask_permission(call, reply);
        }
        // Never the person in front of a page that changes with every deploy.
        if let Some(why) = self.per_deploy_refusal(&call) {
            let _ = reply.send(ToolResult::error(why));
            return;
        }
        if call.name == board::ASK_PERSON {
            return self.ask_person(call, reply);
        }
        if call.name == crate::controls::CHANGE_SETTING {
            return self.change_setting(call, reply).await;
        }
        if call.name == crate::stops::STOP_WORK {
            return self.stop_work_tool(call, reply).await;
        }
        if call.name == board::UNBLOCK {
            return self.unblock_tool(call, reply).await;
        }
        if call.name == board::CLOSE_REQUEST {
            return self.close_request_tool(call, reply);
        }
        if call.name == crate::signals::PROPOSE_SIGNAL {
            return self.propose_signal(call, reply);
        }
        if call.name == board::SIGN_IN {
            self.with_person(&call);
            return self.ask_sign_in(call, reply);
        }
        if call.name == crate::helpers::SPAWN_HELPER {
            return self.spawn_helper(call, reply);
        }
        if call.name == crate::plan_use::TOOL {
            return self.plan_use_answer(&call, reply);
        }
        if call.name == crate::metrics::tool::QUERY {
            return self.metrics_query(&call, reply);
        }
        if call.name == crate::helpers::HELPER_RESULT {
            return self.helper_result(call, reply);
        }
        if call.name == crate::references::TOOL {
            return self.show_references(call, reply);
        }
        if call.name == board::SHOW_BROWSER {
            self.with_person(&call);
            let url = call.arguments["url"].as_str().map(str::to_owned);
            let why = call.arguments["why"].as_str().unwrap_or_default().to_owned();
            // Where a machine credential or a desktop app does it, the person isn't asked for the browser.
            if let Some(text) = self.sign_in_elsewhere(&call, url.as_deref(), &why, false) {
                let _ = reply.send(ToolResult::error(text));
                return;
            }
            // Its own page on that site shows no sign-in: it looks before anyone is asked.
            if let Some(text) = self.look_first(&call, url.as_deref(), &why) {
                let _ = reply.send(ToolResult { text, is_error: false, image: None });
                return;
            }
            // The person was just asked for this sign-in and hasn't answered: one ask, not another.
            if let Some(text) = self.ask_still_open(&call, url.as_deref(), &why) {
                let _ = reply.send(ToolResult { text, is_error: false, image: None });
                return;
            }
            // First what equipe can do by itself: the login autopilot, or the
            // sign-in the person made before put back. Only then the person.
            if let Some(text) = self.try_kept_sign_in(&call, url.as_deref(), &why) {
                let _ = reply.send(ToolResult { text, is_error: false, image: None });
                return;
            }
            // Kept with its cause — or, the person signing in to the same site
            // in another member's browser now, this one waits on that.
            self.asking_cause = None;
            if let Some(text) = self.handoff_asked(&call, url.as_deref(), &why) {
                let _ = reply.send(ToolResult { text, is_error: false, image: None });
                return;
            }
            // A site's own second step (a code, "verify it's you" before a sensitive page):
            // the person is told it is the site's, once, and how to make it rarer.
            let why = match self.asking_cause.take() {
                Some(crate::logins::Cause::TwoFactor) => format!("{why} {}", say!(self, "notice.browser_reauth_hint")),
                _ => why,
            };
            // A sign-in the person does in a normal Chrome window of their own, the
            // automated browser stepping aside — a site's human check lets a person
            // through there. Not possible (no Chrome): the panel, with the reason.
            let mut window_note = None;
            if self.window_mode() {
                match self.begin_window_handoff(&call, url.as_deref(), &why).await {
                    Ok(view) => {
                        // A window on this Mac while the person is on another: said where, and how to reach it.
                        let far = (!view).then(|| self.on_this_screen(&call.squad)).flatten();
                        if let Some(id) = self.caller_id(&call) {
                            let text = if view { say!(self, "notice.browser_view", name = call.member, why = why) } else { self.window_notice(&call.member, &why) };
                            let text = match &far {
                                Some((_, person)) => format!("{text}\n{person}"),
                                None => text,
                            };
                            self.publish(Event::Notice { undo: None, session: id, text });
                        }
                        let far = far.map(|(member, _)| member).unwrap_or_default();
                        let _ = reply.send(ToolResult {
                            text: if view {
                                "A sign-in view is opening on the person's screen at that page: an ordinary web view, not your browser — they sign in there \
by hand, and when they press \"Done\" the sign-in is handed to your browser, which loads the page again. Say in one line what they \
should do there, then end your turn. When they're done you get a message and carry on by yourself — don't ask them to tell you."
                            } else {
                                "A normal Chrome window is opening on the person's Mac at that page, with your browser's own sign-ins — \
a real browser of theirs, not yours: your browser tools are off until they press \"Done\" (or close that window), and the page there \
isn't yours to look at. Say in one line what they should do there, then end your turn. When they're done you get a message and \
carry on by yourself — don't ask them to tell you."
                            }
                            .to_owned()
                                + &far,
                            is_error: false,
                            image: None,
                        });
                        return;
                    }
                    Err(why) => window_note = Some(why),
                }
            }
            if self.ask_in_panel(&call.squad, &call.member, url.as_deref(), &why) {
                if let Some(id) = self.caller_id(&call) {
                    let text = match window_note {
                        Some(handoff_why) => self.window_fallback_note(&call.member, &why, handoff_why),
                        None => say!(self, "notice.browser_in_panel", name = call.member, why = why),
                    };
                    self.publish(Event::Notice { undo: None, session: id, text });
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
            // On this Mac's screen while the person is on another: said where, and how to reach it.
            let far = self.on_this_screen(&call.squad);
            if let Some(id) = self.caller_id(&call) {
                let text = say!(self, "notice.browser_on_screen", name = call.member, why = why);
                let text = match &far {
                    Some((_, person)) => format!("{text}\n{person}"),
                    None => text,
                };
                self.publish(Event::Notice { undo: None, session: id, text });
            }
            let _ = reply.send(ToolResult {
                text: "Your browser comes up on the person's screen when this turn ends, at that page. Tell them what to do \
there, end your turn, and carry on when they say they're done."
                    .to_owned()
                    + &far.map(|(member, _)| member).unwrap_or_default(),
                is_error: false,
                image: None,
            });
            return;
        }
        // A fact remembered or forgotten, committed off the loop: answered once it is in.
        if call.name == board::REMEMBER || call.name == board::FORGET {
            return self.remember(&call, reply);
        }
        // A brief, committed off the loop: answered once it is in.
        if call.name == crate::briefs::WRITE_BRIEF {
            return self.write_brief(&call, reply);
        }
        // A plan, committed off the loop; a ticket cut from one reads it first.
        if call.name == board::WRITE_PLAN {
            return self.write_plan(&call, reply);
        }
        if call.name == board::CREATE_TICKET && board::arg_str(&call.arguments, "plan").is_some_and(|p| !p.trim().is_empty()) {
            return self.cut_from_plan(call, reply);
        }
        let squad = call.squad.clone();
        // The PM accepting its team's work: it waits for the acceptance itself,
        // and the loop doesn't — the git side runs off it (`accept.rs`).
        if call.name == crate::accept::ACCEPT_WORK {
            match self.caller_id(&call) {
                Some(id) => self.accept_work(&squad, &call.member, id, &call.arguments, reply).await,
                None => {
                    self.dispatch(&squad).await;
                    let _ = reply.send(ToolResult { text: "You're not on the squad.".into(), is_error: true, image: None });
                }
            }
            return;
        }
        // A sprint proposed: its small items may start at once, as the person's autonomy setting says.
        if call.name == crate::sprint::PLAN {
            let result = match self.caller_id(&call) {
                Some(id) => Box::pin(self.sprint_plan(&squad, &call.member, id, &call.arguments)).await,
                None => Err("You're not on the squad.".into()),
            };
            self.dispatch(&squad).await;
            let _ = reply.send(match result {
                Ok(text) => ToolResult { text, is_error: false, image: None },
                Err(text) => ToolResult { text, is_error: true, image: None },
            });
            return;
        }
        // A design session proposed, or started at once: it may send the designer its kickoff.
        if call.name == crate::design::session::PROPOSE {
            let result = match self.caller_id(&call) {
                Some(id) => self.propose_design_session(&squad, &call.member, id, &call.arguments).await,
                None => Err("You're not on the squad.".into()),
            };
            let _ = reply.send(match result {
                Ok(text) => ToolResult { text, is_error: false, image: None },
                Err(text) => ToolResult { text, is_error: true, image: None },
            });
            return;
        }
        let caller = self.caller_id(&call);
        let result = self.run_tool(call);
        // A hand-in, a wait, a hand-back: the turn only winds down now.
        if let Some(id) = caller {
            self.tell_work_over(id);
            // Handed in with its turn already over: taken now, not at a turn end that won't come.
            if result.is_ok() {
                self.take_late_hand_in(id).await;
            }
        }
        // Whatever changed, the rules look again.
        self.dispatch(&squad).await;
        let _ = reply.send(match result {
            Ok(text) => ToolResult { text, is_error: false, image: None },
            Err(text) => ToolResult { text, is_error: true, image: None },
        });
    }

    /// The member that made `call` put something in the person's hands —
    /// its browser, a sign-in: its turn may end waiting on them.
    fn with_person(&mut self, call: &ToolCall) {
        let said = |key: &str| call.arguments.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
        let what = match (said("why"), said("site").or_else(|| said("url"))) {
            (Some(why), Some(site)) => format!("{why} ({site})"),
            (Some(what), None) | (None, Some(what)) => what,
            (None, None) => call.name.clone(),
        };
        if let Some(work) = self.caller_id(call).and_then(|id| self.sessions.get_mut(&id)).and_then(|s| s.work.as_mut()) {
            work.end.with_person = Some(turn_end::WithPerson { since: now_secs(), what: crate::signals::one_line(&what) });
        }
    }

    // ── permission ──

    /// The engine asks whether a member may use a tool its mode doesn't allow on
    /// its own. Blocked things are refused, "always allow" rules answered, and
    /// everything else put to the person.
    fn ask_permission(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let tool = permissions::canonical(call.arguments.get("tool_name").and_then(Value::as_str).unwrap_or("?")).to_owned();
        let input = call.arguments.get("input").cloned().unwrap_or_else(|| json!({}));
        let (action, reason) = permissions::describe_in(self.config.lang, &tool, &input);
        let Some(id) = self.caller_id(&call) else {
            let _ = reply.send(verdict(Decision::Deny, &input, "Unknown member."));
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        if permissions::blocked(&tool, &action) {
            let _ = reply.send(verdict(Decision::Deny, &input, "equipe never allows this, in any mode."));
            let text = say!(self, "notice.blocked", action = action);
            self.publish(Event::Notice { undo: None, session: id, text });
            self.audit(&call.squad, ticket, "equipe", "blocked", &action);
            return;
        }
        if let Some(why) = self.off_limits(id, &call.squad, &tool, &action, &input) {
            let _ = reply.send(verdict(Decision::Deny, &input, why));
            let text = say!(self, "notice.refused", why = why, action = action);
            self.publish(Event::Notice { undo: None, session: id, text });
            self.audit(&call.squad, ticket, "equipe", "refused", &action);
            return;
        }
        let rules = self.journal(&call.squad).and_then(|j| j.allowed().ok()).unwrap_or_default();
        if rules.iter().any(|rule| permissions::matches(rule, &tool, &action)) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        // A standing allowance the person granted for this kind of command
        // ("Allow for this kind"): allowed, and its use logged.
        // The squad's own working areas under `.equipe` (a ticket's scratch folder, the
        // build cache, the desk) hold no secrets: only its config, credentials,
        // journals and other squads' folders do.
        let scratch = permissions::scratch_areas(&self.config.home, &call.squad);
        let kind = crate::autonomy::engine_kind_in(&tool, &action, &scratch);
        let mut ask = crate::protocol::autonomy::Ask { kind, plain: None, standing: None, later: false, op: None };
        if kind == crate::protocol::autonomy::Kind::Other && self.allowed_standing(&call.squad, id, &call.member, ticket, &ask, (&tool, &action)).is_some() {
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
        // A squad project checked out beside its repository into the member's own
        // temporary folder: it writes only the project's `.git/worktrees/` and that
        // folder, which the fence can't open without opening the project, so it runs
        // outside it, exactly as given back (hooks off), without asking.
        // A Codex member asks the same through Codex's own approval (its escalation
        // out of the sandbox): its engine runs its own command once allowed, so
        // it passes where no hook can run on it ([`permissions::codex_project_checkout`]).
        let codex = self.sessions.get(&id).is_some_and(|s| s.engine == Engine::Codex);
        if auto && tool == "Bash" && codex && self.codex_project_checkout(id, &call.squad, &input) {
            self.audit(&call.squad, ticket, "equipe", "allowed", &action);
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        if auto && tool == "Bash" && !codex && let Some(command) = self.project_checkout(id, &call.squad, &input) {
            let mut input = input.clone();
            input["command"] = json!(command);
            input["dangerouslyDisableSandbox"] = json!(true);
            self.audit(&call.squad, ticket, "equipe", "allowed", &command);
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        if auto && permissions::only_reads_in(&tool, &action, &input, &scratch) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        // Writing in the squad's own scratch areas is part of the work, as it is inside the fence.
        if auto && permissions::writes_in_scratch(&tool, &action, &scratch) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        // A squad on full autonomy (non-developers): its internal work goes ahead
        // without the person; outward and irreversible things still ask.
        if self.autonomy_mode(&call.squad) == crate::protocol::autonomy::Autonomy::Full {
            let mut roots: Vec<String> = self.projects_cached(&call.squad).iter().map(|p| p.to_string_lossy().into_owned()).collect();
            roots.push(squad_home::path(&self.config.home, &call.squad).to_string_lossy().into_owned());
            if let Some(session) = self.sessions.get(&id) {
                roots.push(session.cwd.to_string_lossy().into_owned());
                roots.push(session.project.to_string_lossy().into_owned());
                roots.extend(session.work.as_ref().map(|w| w.cwd.to_string_lossy().into_owned()));
            }
            if crate::autonomy::hands_off_allows(&tool, &action, &input, kind, &roots, &scratch) {
                self.audit(&call.squad, ticket, "equipe", "allowed (full autonomy)", &action);
                let _ = reply.send(verdict(Decision::Allow, &input, ""));
                return;
            }
        }
        if fenced && permissions::stays_in_sandbox(&tool, &input) && !permissions::outward(&tool, &action) {
            let _ = reply.send(verdict(Decision::Allow, &input, ""));
            return;
        }
        let approval_id = self.next_approval;
        self.next_approval += 1;
        // What "Allow for this kind" would grant: the rule "always" would keep, for a while.
        ask.standing = (kind == crate::protocol::autonomy::Kind::Other).then(|| permissions::rule_for(&tool, &action)).flatten().map(|rule| crate::protocol::autonomy::Scope {
            words: say!(self, "autonomy.scope_rule", rule = crate::autonomy::rule_label(&rule)),
            rule: Some(rule),
            ..Default::default()
        });
        let approval = Approval { id: approval_id, session: id, member: call.member.clone(), ticket, tool, action, reason, preview: None, ask: Some(ask) };
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

    /// The command to run for a member's shell call that checks out one of
    /// its squad's projects beside the repository into its own temporary
    /// folder ([`permissions::project_checkout`]); `None` for anything else.
    fn project_checkout(&self, id: SessionId, squad: &str, input: &Value) -> Option<String> {
        let session = self.sessions.get(&id)?;
        let command = input.get("command").and_then(Value::as_str)?;
        let tmp = journal::lane_tmp_dir(&self.config.home, squad, session.work.as_ref().map(|w| w.ticket), session.name()?);
        permissions::project_checkout(command, &self.projects_cached(squad), &tmp)
    }

    /// Whether a Codex member's shell call is such a checkout, one its engine
    /// may run as asked ([`permissions::codex_project_checkout`]).
    fn codex_project_checkout(&self, id: SessionId, squad: &str, input: &Value) -> bool {
        let Some(session) = self.sessions.get(&id) else { return false };
        let (Some(command), Some(name)) = (input.get("command").and_then(Value::as_str), session.name()) else { return false };
        let tmp = journal::lane_tmp_dir(&self.config.home, squad, session.work.as_ref().map(|w| w.ticket), name);
        let configs = permissions::git_configs(std::env::var_os("HOME").map(PathBuf::from).as_deref());
        permissions::codex_project_checkout(command, &self.projects_cached(squad), &tmp, &configs)
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
        let Some(id) = self.caller_id(&call).filter(|_| !action.is_empty()) else {
            let _ = reply.send(ToolResult { text: "Say what you'll do in `action`.".into(), is_error: true, image: None });
            return;
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        // What the squad's autonomy policy makes of it: its kind (the floor
        // read in its words whatever it says it is), its plain words, and the
        // standing allowance its card may offer.
        let system = text("system");
        let standing = crate::autonomy::offered_scope(&call.arguments, Some(system.as_str()).filter(|s| !s.is_empty()));
        let own_deletes = standing.as_ref().is_some_and(|s| s.ops.contains(&crate::protocol::autonomy::Op::Delete) && s.undo_days > 0);
        let declared = crate::protocol::autonomy::Kind::parse(&text("kind"));
        let kind = crate::autonomy::person_kind(declared, &action, own_deletes);
        let ask = crate::protocol::autonomy::Ask {
            kind,
            plain: crate::autonomy::plain_of(&call.arguments),
            standing: standing.filter(|_| kind.allowable()),
            later: false,
            op: autonomy::op_of(&call.arguments),
        };
        if let Some(text) = self.allowed_standing(&call.squad, id, &call.member, ticket, &ask, (CONFIRM, &action)) {
            let _ = reply.send(ToolResult::ok(text));
            return;
        }
        // A schema migration that only adds lands like code: nobody's yes is needed for it (`crate::migrations`).
        if matches!(kind, crate::protocol::autonomy::Kind::ProductionData | crate::protocol::autonomy::Kind::Other)
            && let Some(root) = self.sessions.get(&id).and_then(|s| s.work.as_ref()).and_then(|w| w.sandbox.as_ref()).map(|s| s.path.clone())
            && let Some(files) = crate::person_gate::additive_migrations_named(&root, &action)
        {
            if let Some(journal) = self.journal(&call.squad)
                && let Some(ticket) = ticket
            {
                let _ = journal.ticket_event(ticket, "equipe", "additive migration", &files.join(", "));
            }
            let _ = reply.send(ToolResult::ok(format!(
                "Not asked: {} only add to the database (new tables, indexes, or columns existing rows can take — nothing dropped, \
deleted, rewritten, retyped or renamed), so they land like code: no one's yes is needed. Commit them with your work and hand it in; \
the PM hears of them in the digest of landed work. A migration that drops, deletes, updates existing rows, changes a type or \
renames still needs the person — equipe holds that one at landing by itself.",
                files.join(", ")
            )));
            return;
        }
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
            ask: Some(ask),
        };
        // The member waits a few minutes for a quick answer — most come
        // within them — then is let go to work on what doesn't depend on it,
        // the card staying up and the answer reaching it as a message.
        self.pending.insert(approval_id, Pending { squad: call.squad.clone(), approval: approval.clone(), input: json!({}), reply });
        self.publish(Event::ApprovalRequested { squad: call.squad.clone(), approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(autonomy::HOLD).await;
            let _ = internal.send(Internal::ReleaseAsk(approval_id));
            tokio::time::sleep(autonomy::LATER_WAIT.saturating_sub(autonomy::HOLD)).await;
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
                self.publish(Event::Notice { undo: None, session: id, text });
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
            ask: None,
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
        // A request its member didn't wait for: the answer goes to it as a message, below.
        let later = approval.ask.as_ref().is_some_and(|a| a.later);
        let reply = if later { reply } else { self.late_reply(approval_id, &approval, decision, reply) };
        // How long it waited on the person, for the week's retro.
        if let Some(asked) = self.approvals_asked.remove(&approval_id) {
            let outcome = match decision {
                _ if why == "Nobody answered in time." => "timed out",
                Decision::Allow => "allowed",
                Decision::Always => "always allowed",
                Decision::Deny => "declined",
            };
            let wait = crate::retro::Wait { kind: "permission".into(), member: approval.member.clone(), ticket: approval.ticket, what: approval.action.clone(), asked, answered: Some(now_secs()), outcome: outcome.into() };
            if let Some(journal) = self.journal(&squad) {
                let _ = journal.record_wait(&wait);
            }
        }
        if approval.tool == SIGN_IN_TOOL {
            self.publish(Event::ApprovalResolved { squad: squad.clone(), approval: approval_id, decision });
            let joined = self.sign_in_joined.remove(&approval_id).unwrap_or_default();
            return self.sign_in_answered(squad, approval, input, decision, reply, true, joined);
        }
        if approval.tool == crate::controls::SETTING_TOOL || approval.tool == crate::controls::ENGINE_TOOL {
            self.publish(Event::ApprovalResolved { squad: squad.clone(), approval: approval_id, decision });
            return self.setting_answered(squad, approval, input, decision, reply);
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
        // A signal the PM proposed: kept on a yes, and the PM told so.
        let signal = if approval.tool == CONFIRM { self.signal_answered(&squad, &input, decision != Decision::Deny) } else { None };
        let result = if let Some(text) = signal {
            ToolResult { text, is_error: false, image: None }
        } else if approval.tool == crate::phone::INSTALL {
            // The phone tool waits on this: AXe is installed only on a yes.
            Self::phone_install_answered(decision, why)
        } else if approval.tool == desktop_gate::SEND {
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
        };
        if later {
            let _ = self.internal.send(Internal::LaterAnswered { squad: squad.clone(), approval: Box::new(approval.clone()), said: result.text.clone() });
        } else {
            let _ = reply.send(result);
        }
        let line = match (decision, &rule, words) {
            (Decision::Deny, _, Some(w)) => say!(self, "notice.declined_said", words = w, action = approval.action),
            (_, _, Some(w)) => say!(self, "notice.allowed_said", words = w, action = approval.action),
            (Decision::Always, Some(rule), _) => say!(self, "notice.always_allowed", rule = rule, action = approval.action),
            (Decision::Allow | Decision::Always, _, _) => say!(self, "notice.allowed", action = approval.action),
            (Decision::Deny, _, _) if why == "The person declined." => say!(self, "notice.declined", action = approval.action),
            (Decision::Deny, _, _) => say!(self, "notice.declined_why", why = why, action = approval.action),
        };
        self.publish(Event::Notice { undo: None, session: approval.session, text: line });
        let verb = match decision {
            Decision::Allow => "allowed",
            Decision::Always => "always allowed",
            Decision::Deny => "declined",
        };
        self.audit(&squad, approval.ticket, "you", verb, &approval.action);
        self.publish(Event::ApprovalResolved { squad, approval: approval_id, decision });
    }

    /// Where the answer to request `id` goes. Asked by `approval.session`
    /// during a turn that has ended since — its engine stopped waiting for the
    /// call (Codex gives a call up after its timeout) and the turn went on to
    /// its end — it would reach nobody, and a run on a ticket would sit with
    /// nothing to hold it until its lease ran out. So it goes to the member as
    /// a turn of its own (or after the one running now), with what it asked;
    /// otherwise to `reply`, the call still waiting for it.
    fn late_reply(&mut self, id: u64, approval: &Approval, decision: Decision, reply: oneshot::Sender<ToolResult>) -> oneshot::Sender<ToolResult> {
        let Some((asker, began)) = self.approvals_turn.remove(&id) else { return reply };
        let Some(s) = self.sessions.get(&asker) else { return reply };
        if s.member.is_none() || (s.is_working() && s.turn_began == Some(began)) {
            return reply;
        }
        let on_ticket = s.work.is_some();
        let who = format!("{}{}", s.squad.as_deref().map(|q| format!("{q}: ")).unwrap_or_default(), s.name().unwrap_or("agent"));
        let at = s.work.as_ref().map(|w| format!(" on T-{}", w.ticket)).unwrap_or_default();
        crate::daemon_log!("equipe: {who}'s request{at} was answered after the turn that asked had ended: it goes to it as a turn of its own");
        let action = clip(approval.action.lines().next().unwrap_or_default(), 300);
        // A question the member put, or a sign-in: its answer is in words already.
        let in_words = approval.tool == CONFIRM || approval.tool == SIGN_IN_TOOL;
        let (tx, rx) = oneshot::channel::<ToolResult>();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let Ok(answer) = rx.await else { return };
            let said = match decision {
                _ if in_words && !answer.text.trim().is_empty() => answer.text,
                Decision::Allow | Decision::Always => "The person allowed it.".to_owned(),
                Decision::Deny => "The person declined it.".to_owned(),
            };
            let prompt = late_answer_prompt(&action, &said, on_ticket);
            let _ = internal.send(Internal::Wake { session: asker, prompt });
        });
        tx
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
        let Some(id) = self.caller_id(&call) else {
            let _ = reply.send(ToolResult::error("Unknown member."));
            return;
        };
        if self.config.npx.is_none() {
            let _ = reply.send(ToolResult::error("You have no browser on this machine (Node.js isn't installed), so there is nothing to sign in with."));
            return;
        }
        let sign_in_site = format!("https://{}", vault::site_of(&site));
        if let Some(text) = self.sign_in_elsewhere(&call, Some(&sign_in_site), &why, true) {
            let _ = reply.send(ToolResult::error(text));
            return;
        }
        // Another member met this site's sign-in lately — still with the person, or blocked for
        // automation: told so, no second card.
        if let Some(wait) = self.site_waits.other(&vault::site_of(&site), &call.member, now_secs()) {
            let _ = reply.send(ToolResult::error(crate::logins::site_wait_told(&wait, now_secs())));
            return;
        }
        let kept: Vec<&crate::protocol::vault::Login> = self.logins.iter().filter(|l| vault::serves(&l.site, &site)).collect();
        let login = match (kept.as_slice(), account.is_empty()) {
            ([], _) => {
                self.login_record(&call.squad, crate::logins::Entry::new(now_secs(), &site, &call.member, crate::logins::Kind::NoLogin, Some(crate::logins::Cause::NoLoginKept), None));
                let _ = reply.send(ToolResult::error(format!(
                    "The person keeps no login for {}. First look whether you are signed in already: every member shares one browser and the \
person's sign-ins — open the page you need and take a snapshot. Only if it asks you to sign in, hand them your browser with \
`show_my_browser` to sign in themselves (or ask them to add a login in equipe, Settings › Accounts).",
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
            ask: None,
        };
        // Let through already, in this squad or every squad: no card, straight to the form.
        if vault::allows(&login, &call.squad) {
            return self.sign_in_answered(call.squad, approval, input, Decision::Allow, reply, false, Vec::new());
        }
        // A card asks the person about this login already: this member waits
        // on that one — one question, and every member waiting carries on.
        let asking = self.pending.iter().find(|(_, p)| p.squad == call.squad && p.approval.tool == SIGN_IN_TOOL && p.input["login"].as_u64() == Some(u64::from(login.id))).map(|(id, _)| *id);
        if let Some(card) = asking {
            self.sign_in_joined.entry(card).or_default().push(Pending { squad: call.squad.clone(), approval, input, reply });
            return;
        }
        self.login_record(&call.squad, crate::logins::Entry::new(now_secs(), &login.site, &call.member, crate::logins::Kind::Card, Some(crate::logins::Cause::SessionExpired), None));
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
    /// in itself — the member hears only how it went. `asked`: a card asked
    /// the person; `joined`: members that waited on the same card, told as
    /// the first one is (their browsers share its sign-in).
    #[allow(clippy::too_many_arguments)]
    fn sign_in_answered(&mut self, squad: String, approval: Approval, input: Value, decision: Decision, reply: oneshot::Sender<ToolResult>, asked: bool, joined: Vec<Pending>) {
        let id = input["login"].as_u64().unwrap_or_default() as u32;
        let member = approval.member.clone();
        let login = self.logins.iter().find(|l| l.id == id).cloned();
        let Some(login) = login.filter(|_| decision != Decision::Deny) else {
            let no = "The person said no: don't sign in there; carry on without it, or say what you need.";
            let _ = reply.send(ToolResult { text: no.into(), is_error: false, image: None });
            for other in joined {
                let _ = other.reply.send(ToolResult { text: no.into(), is_error: false, image: None });
            }
            let declined_in = squad.clone();
            self.with_vault(move |v| {
                v.used(id, now_secs(), &declined_in, &member, "declined");
                Ok(())
            });
            self.audit(&squad, approval.ticket, "you", "declined", &approval.action);
            return;
        };
        // "Always": the login, for the whole squad — every member and lane of it.
        if decision == Decision::Always {
            let squad = squad.clone();
            self.with_vault(move |v| v.allow(id, &squad));
        }
        self.audit(&squad, approval.ticket, "you", if decision == Decision::Always { "always allowed" } else { "allowed" }, &approval.action);
        let Some(port) = self.browser_key(approval.session).and_then(|(sq, key)| self.open_browser(&sq, &key, false)) else {
            let _ = reply.send(ToolResult::error("Your browser couldn't be started."));
            for other in joined {
                let _ = other.reply.send(ToolResult::error("Your browser couldn't be started."));
            }
            return;
        };
        let (client, vault, internal) = (self.client.clone(), self.vault.clone(), self.internal.clone());
        let start = login.url.clone().unwrap_or_else(|| format!("https://{}", login.site));
        tokio::spawn(async move {
            let (site, username) = (login.site.clone(), login.username.clone());
            let outcome = sign_in_page(&client, port, &vault, &login, &start).await;
            let (text, is_error, kept) = match &outcome {
                Ok(step) => (format!("Signed in to {site} as {username} — the form was filled and submitted ({step}). Look at the page to confirm, then carry on."), false, "signed_in"),
                Err(SignIn::NoForm) => (format!("No sign-in form on the page equipe opened for {site}. Open the page with the form yourself, then call `sign_in` again."), true, "no_form"),
                Err(SignIn::Failed(why)) => (format!("Couldn't sign in to {site}: {why}"), true, "failed"),
            };
            let _ = reply.send(ToolResult { text: text.clone(), is_error, image: None });
            for other in joined {
                let text = if is_error {
                    text.clone()
                } else {
                    format!("Signed in to {site} as {username} in another member's browser, on the same yes from the person — every member shares the browser's sign-ins. Reload your page there and carry on.")
                };
                let _ = other.reply.send(ToolResult { text, is_error, image: None });
            }
            let entry = match outcome {
                Ok(_) => crate::logins::Entry::new(now_secs(), &site, &member, crate::logins::Kind::SignedIn, None, Some(if asked { crate::logins::BY_PERSON } else { crate::logins::BY_EQUIPE })),
                Err(_) => crate::logins::Entry::new(now_secs(), &site, &member, crate::logins::Kind::FillFailed, None, Some(crate::logins::BY_EQUIPE)),
            };
            let _ = internal.send(Internal::LoginRecord { squad: squad.clone(), entry });
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

    /// Whether the person's Accessibility switch for equipe's desktop helper is on.
    fn desktop_granted(&self) -> bool {
        self.desktop_auto.as_ref().is_some_and(desktop::auto::Auto::granted)
    }

    /// Whether a member may use the desktop: everyone once the switch is on,
    /// but whoever the person turned it off for.
    fn desktop_allowed(&self, call: &ToolCall) -> Result<(PathBuf, Vec<String>), ToolResult> {
        let journal = self.journals.get(&call.squad);
        let granted = self.desktop_granted();
        let on = self.team(&call.squad).iter().any(|(_, name, _)| *name == call.member) && journal.is_some_and(|j| j.desktop_enabled(&call.member, granted));
        if on {
            // Still being set up, or waiting on the person's one switch: said
            // short, so the member neither retries nor guesses; the person is shown the card.
            if let Some(refused) = self.desktop_auto.as_ref().and_then(|auto| auto.refusal(&call.name)) {
                return Err(ToolResult::error(refused));
            }
            Ok((self.config.home.clone(), journal.map(Journal::messaging_apps).unwrap_or_default()))
        } else {
            Err(ToolResult::error(format!(
                "The Mac's desktop is off for {}. Only the person turns it on (`equipe desktop on {}`).",
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
            // A run drives a browser of its own: never its member's, whose
            // conversation goes on beside it, nor another run's.
            None if s.run_of.is_some() => browser::lane_key(name, Some(&format!("run{}", s.slot + 1))),
            None => name.to_owned(),
        };
        Some((s.squad.clone()?, key))
    }

    /// Start a conversation's browser now — on screen when `shown` — and take
    /// it to the page asked for, or where it last was. `name` is its key.
    fn open_browser(&mut self, squad: &str, name: &str, shown: bool) -> Option<u16> {
        let npx = self.config.npx.clone()?;
        // Its profile is the person's, in a window of their own, just now.
        if self.windows.handoffs.paused(browser::split_profile(name).1.unwrap_or_default()) {
            return None;
        }
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
            let lane = running.is_lane();
            self.browsers.insert(key.clone(), running);
            // The squad's kept sign-ins: whatever its profile's Chrome lacks of them goes in.
            if lane {
                self.restore_sign_ins(squad, name);
            }
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
            self.tell_profile_separation();
            // A new Chrome: the kept sign-ins go in again as lanes open.
            self.sign_ins_restart(None);
            self.suspend_again();
        }
        self.host.clone()
    }

    /// The person signs in to a site in equipe's own browser — the one way
    /// in for `equipe browser <url>` and the app's "Sign in to a site": `url`
    /// is a site or an address ([`crate::signin::resolve`]). The page opens
    /// where the person is: a tab of the panel of an attached screen — shown
    /// there, never a window on a Mac nobody may be looking at — else (or with
    /// `window`) the separate Chrome window on this Mac. The page is followed
    /// until the sign-in is seen through ([`Self::follow_person_tabs`]).
    /// Says how it went to every screen ([`Event::BrowserSignIn`]).
    fn start_sign_in(&mut self, input: &str, window: bool) -> equipe_protocol::rpc::BrowserSignInOpened {
        use equipe_protocol::BrowserSignInState as State;
        let Some(url) = crate::signin::resolve(input) else {
            self.publish(Event::BrowserSignIn { site: String::new(), state: State::Failed, window });
            return equipe_protocol::rpc::BrowserSignInOpened { opened: false, window };
        };
        let window = window || !self.hub.has_panel();
        let site = if url.starts_with("data:") { String::new() } else { vault::site_of(&url) };
        let host = self.config.npx.clone().and_then(|npx| self.browser_host(&npx));
        let opened = host.is_some_and(|host| {
            host.lock()
                .map(|mut h| {
                    if window {
                        h.person(&url);
                    } else {
                        h.op(&json!({ "op": "newtab", "url": url }));
                    }
                })
                .is_ok()
        });
        if opened {
            let open: Vec<u64> = match self.tabs.get("") {
                Some(Event::BrowserTabs { pages, .. }) => pages.iter().map(|p| p.id).collect(),
                _ => Vec::new(),
            };
            self.person_sign_in.start(crate::signin::Flow::new(&url, window, open, now_secs()));
        }
        let state = if opened { State::Opened } else { State::Failed };
        self.publish(Event::BrowserSignIn { site, state, window });
        equipe_protocol::rpc::BrowserSignInOpened { opened, window }
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
            HostEvent::Tabs { lane, member, color, pages, agent, touched, .. } => {
                let (squad, session) = self.lane_session(&lane);
                // What login autopilot reads off the pages; a sign-in that ran out, signed back in.
                self.see_pages(&lane, &pages);
                // A page waiting on a code the site emailed: the person's card.
                self.see_verification_pages(&lane, &pages);
                // A sign-in page that turned into its site: a sign-in finished.
                self.follow_sign_ins(&lane, &pages, touched).await;
                // Back from the person: whether the sign-in they were handed it for went through.
                self.verify_back(&lane, &pages, agent).await;
                if !lane.is_empty() {
                    self.host_pages.insert(lane.clone(), (pages.clone(), agent));
                }
                // The person's own tabs, in the panel or the Chrome window: a sign-in they started, or any.
                if lane.is_empty() {
                    self.follow_person_tabs(&pages);
                }
                let pages: Vec<equipe_protocol::BrowserPage> = pages.into_iter().map(browser::HostPage::page).collect();
                // The person's window, or the browser coming back from it: no check to ask about.
                let follow = if !self.windows.handoffs.busy_lane(&lane) && !self.windows.views.holds(&lane) { self.watch_checks(&lane, session, &pages, agent) } else { None };
                let event = Event::BrowserTabs { squad, lane: lane.clone(), session, member, color, pages, agent_page: agent };
                self.tabs.insert(lane, event.clone());
                self.publish(event);
                // After its tabs are told: a window that follows closes the browser they show.
                if let Some(follow) = follow {
                    self.check_follow(follow).await;
                }
            }
            HostEvent::Control { lane, by, refused } => {
                let (squad, session) = self.lane_session(&lane);
                let asked = if by.is_none() { self.asked_in.remove(&lane) } else { None };
                // Handed back: the conversation that asked hears it — a ticket's run,
                // whose browser shares the member's sign-ins — not the member's own
                // conversation, whose browser it was and which knows nothing of it.
                let asker = if by.is_none() { self.asked_by.remove(&lane).filter(|id| self.sessions.contains_key(id)) } else { None };
                // Handed back: its member's next ask for a sign-in is a new one.
                if by.is_none()
                    && let Some((squad, key)) = lane.split_once('/')
                {
                    self.recent_asks.answered(squad, browser::member_of(key));
                    self.site_waits.answered(browser::member_of(key));
                }
                let session = if asked.is_some() { asker.or(session) } else { session };
                // Handed back from a page that waited for them: passed — or, still
                // waiting, left to the member, and not taken from it again.
                let checked = if by.is_none() { self.checking.remove(&lane) } else { None };
                if let Some((page, url, _)) = &checked
                    && self.waits_for_person(&lane, *page)
                {
                    self.let_go.insert((lane.clone(), *page, url.clone()));
                }
                // Back from the person: whoever waited on a sign-in in it looks
                // again — its pages on that site reloaded, its members told.
                if by.is_none() {
                    let site = checked.as_ref().map(|(_, url, _)| vault::site_of(url)).or_else(|| self.asked_site.get(&lane).cloned());
                    if let Some(site) = &site {
                        self.reload_waiting(site, &lane);
                    }
                    // Its finish not seen in their hands (their own window, a sign-in view, a page
                    // reloaded after): its page is looked at as it comes back.
                    // Only for a sign-in or the step after it: a CAPTCHA passed is no sign-in.
                    let seen = self.finished_in_hands.remove(&lane);
                    let signing_in = match &checked {
                        Some((_, _, check)) => matches!(check, equipe_protocol::HumanCheck::SignIn | equipe_protocol::HumanCheck::TwoFactor),
                        None => asked.is_some() && self.asked_site.contains_key(&lane),
                    };
                    if let Some(site) = site.filter(|s| !s.is_empty() && !seen && signing_in) {
                        self.verify_back.insert(lane.clone(), (site, std::time::Instant::now()));
                        // Loaded already (a sign-in view's page waits for that): looked at now.
                        if let Some((pages, agent)) = self.host_pages.get(&lane).cloned() {
                            self.verify_back(&lane, &pages, agent).await;
                        }
                    }
                    self.release_login_waiters(&lane).await;
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
                // Handed back to a member that had asked for it, or whose page needed the person.
                if by.is_none()
                    && (asked.is_some() || checked.is_some())
                    && let Some(name) = self.sessions.get(&session).and_then(|s| s.name().map(str::to_owned))
                {
                    let squad = lane.split_once('/').map(|(q, _)| q.to_owned()).unwrap_or_default();
                    self.browser_back.insert((squad, name), now_secs());
                }
                let working = self.sessions.get(&session).is_some_and(Session::is_working);
                let back_at = self.tabs.get(&lane).and_then(current_page);
                // Its tools were turned away at a page that waited for the person,
                // and its turn ended: it carries on now.
                if checked.is_some() && asked.is_none() {
                    // equipe filled its emailed code on the person's click: "they confirmed access".
                    if self.verification_handed_back(&lane).await {
                        return;
                    }
                    if refused && !working {
                        let text = say!(self, "notice.browser_given_back");
                        let prompt = handed_back("a human check or a sign-in on the page", false, back_at.as_ref());
                        self.carry_on(session, text, prompt).await;
                    } else if working {
                        let text = say!(self, "notice.browser_given_back");
                        self.publish(Event::Notice { undo: None, session, text });
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
                        self.publish(Event::Notice { undo: None, session, text });
                    }
                    // Done with what it asked of them: it carries on.
                    (false, true) => {
                        let why = asked.unwrap_or_default();
                        let text = say!(self, "notice.browser_given_back");
                        self.carry_on(session, text, handed_back(&why, true, back_at.as_ref())).await;
                    }
                    (false, false) if working => {
                        let text = say!(self, "notice.browser_given_back");
                        self.publish(Event::Notice { undo: None, session, text });
                    }
                    _ => {}
                }
            }
            HostEvent::Dialog { page, lane, kind, message, default } => {
                let (squad, _) = self.lane_session(&lane);
                self.publish(Event::BrowserDialog { squad, lane, page, kind, message, default });
            }
            HostEvent::DialogClosed { page } => self.publish(Event::BrowserDialogClosed { page }),
            HostEvent::Suspended { profile } => self.window_suspended(&profile),
            HostEvent::LaneFailed { lane } => self.lane_failed(&lane),
            HostEvent::Filled { token, answer, .. } => self.filled(token, answer),
            HostEvent::Visited { token, page } => self.on_visited(token, page),
            HostEvent::Passkey { token, credential } => self.on_passkey(token, credential),
            HostEvent::Snap { token, png } => self.on_snap(token, png),
            HostEvent::Cookies { lane, profile, added, failed, restore } => self.cookies_went_in(lane.as_deref(), &profile, added, failed, restore),
            HostEvent::Collected { token, cookies } => self.on_collected(token, cookies.0),
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
                self.signing_in.clear();
                self.relogins.clear();
                self.fill_waits.lock().map(|mut w| w.clear()).unwrap_or_default();
                let waiting: Vec<String> = self.login_waiters.keys().cloned().collect();
                for lane in waiting {
                    self.release_login_waiters(&lane).await;
                }
                self.asked_site.clear();
                self.site_waits = Default::default();
                self.verify_back.clear();
                self.finished_in_hands.clear();
                self.host_pages.clear();
            }
        }
    }

    /// A member's page waits for a person — a "verify you are human" check, a
    /// sign-in, a code from their phone: its browser goes into the person's
    /// hands (its tools wait, and say why), and the person is told. Passed
    /// before they put a hand on it, it goes back to its member by itself.
    ///
    /// A check the person answers by clicking goes to a normal Chrome window
    /// once they reach for it (`window::check_to_window`); what follows here:
    /// that window again at once when it came back right after they passed it
    /// there, and no more rounds after that.
    fn watch_checks(&mut self, lane: &str, session: Option<SessionId>, pages: &[equipe_protocol::BrowserPage], agent: Option<u64>) -> Option<window::CheckFollow> {
        if lane.is_empty() {
            return None;
        }
        // equipe is signing it in by itself: what its pages say waits for that.
        if self.relogins.contains_key(lane) {
            return None;
        }
        let let_go = |page: u64, url: &str| self.let_go.contains(&(lane.to_owned(), page, url.to_owned()));
        match browser::check_step(pages, agent, self.checking.contains_key(lane), self.held.contains_key(lane), let_go) {
            browser::CheckStep::Pause { page, url, check } => {
                // A sign-in, a code or a passkey the person lets equipe do by itself: no one is asked.
                if self.try_autopilot(lane, page, &url, check) {
                    return None;
                }
                let round = self.check_round(&url, check);
                // Passed twice in the person's window and back again: the site refuses this
                // browser. Not the person's again — its member does without the site.
                if round == crate::window_handoff::CheckRound::GiveUp {
                    let site = vault::site_of(&url);
                    self.let_go.insert((lane.to_owned(), page, url));
                    return Some(window::CheckFollow::GiveUp(lane.to_owned(), site));
                }
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": "person", "why": "check", "page": page }));
                // Kept with its cause — or, the person on the same sign-in for
                // another member already, waiting on that one.
                let joined = self.check_paused(lane, &url, check);
                if let Some(session) = session {
                    let site = browser::site_of(&url).to_owned();
                    let text = if joined { say!(self, "notice.browser_check_joined", site = site) } else { say!(self, "notice.browser_check", site = site) };
                    self.publish(Event::Notice { undo: None, session, text });
                }
                self.checking.insert(lane.to_owned(), (page, url, check));
                (round == crate::window_handoff::CheckRound::Again).then(|| window::CheckFollow::Window(lane.to_owned()))
            }
            browser::CheckStep::Release => {
                self.hub.op(&json!({ "op": "control", "lane": lane, "by": null, "ifUntouched": true }));
                None
            }
            browser::CheckStep::Nothing => None,
        }
    }

    /// The member lane a tab of the shared Chrome is in, as last told.
    fn lane_of_page(&self, page: u64) -> Option<String> {
        self.tabs.iter().find_map(|(lane, tabs)| match tabs {
            Event::BrowserTabs { pages, .. } if !lane.is_empty() && pages.iter().any(|p| p.id == page) => Some(lane.clone()),
            _ => None,
        })
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
            self.publish(Event::Notice { undo: None, session, text });
        }
    }

    /// equipe tells a member something on the person's behalf — their browser
    /// came back, a tab of it was closed: a notice in the conversation
    /// (`notice`, in the person's language), never a message of theirs, and
    /// `prompt` to the member — read at its next step if it is at work, else
    /// starting a turn of its own.
    async fn carry_on(&mut self, id: SessionId, notice: String, prompt: String) {
        self.publish(Event::Notice { undo: None, session: id, text: notice });
        self.set_going(id, prompt).await;
    }

    /// Set conversation `id` going on `prompt`: read at its next step when at
    /// work, else a turn of its own — with what it missed meanwhile.
    async fn set_going(&mut self, id: SessionId, prompt: String) {
        let Some(session) = self.sessions.get(&id) else { return };
        if session.is_working() {
            // An engine that can't take it mid-turn gets it after: never dropped.
            match session.steer.clone() {
                Some(steer) => {
                    let _ = steer.send(prompt);
                }
                None => self.leave_memo(id, prompt),
            }
            return;
        }
        if !self.ensure_codex(id).await {
            return;
        }
        let previewing = self.preview_note(id);
        let Some(session) = self.sessions.get_mut(&id) else { return };
        // What it missed travels with it, as with the person's own messages.
        let prompt = match std::mem::take(&mut session.briefing) {
            briefing if briefing.is_empty() => prompt,
            briefing => format!("[equipe: since your last turn —\n{}]\n\n{prompt}", briefing.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")),
        };
        let prompt = match previewing {
            Some(previewing) => format!("[equipe: {previewing}]\n\n{prompt}"),
            None => prompt,
        };
        self.save_waiting(id);
        self.start_turn(id, prompt);
    }

    /// Whether `page` of `lane` waits for a person, as last told.
    fn waits_for_person(&self, lane: &str, page: u64) -> bool {
        matches!(self.tabs.get(lane), Some(Event::BrowserTabs { pages, .. }) if pages.iter().any(|p| p.id == page && p.check.is_some()))
    }

    /// A member's lane couldn't open in the browser host: its port is
    /// forgotten, so its next turn asks for the lane again rather than
    /// handing its engine a browser no server answers for — an engine
    /// that gives up on a server that doesn't come up (Codex) would start
    /// with no browser tools at all.
    fn lane_failed(&mut self, lane: &str) {
        let Some((squad, key)) = lane.split_once('/') else { return };
        let key = (squad.to_owned(), key.to_owned());
        if !self.browsers.get(&key).is_some_and(|b| b.is_lane()) {
            return;
        }
        self.browsers.remove(&key);
        if let Some(host) = &self.host
            && let Ok(mut host) = host.lock()
        {
            host.forget_lane(lane);
        }
        crate::daemon_log!("equipe: {lane}'s browser couldn't open; it opens again on its next turn");
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
                // Its runs too: how far a member may go is the member's, on every ticket it runs.
                let runs = self.runs_of(id);
                for at in std::iter::once(id).chain(runs) {
                    if let Some(session) = self.sessions.get_mut(&at) {
                        session.mode = mode;
                    }
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
        let id = self.caller_id(&call).ok_or_else(|| format!("{} isn't on the squad {}.", call.member, call.squad))?;
        let ToolCall { squad, member, name, arguments: args, .. } = call;
        let is_pm = self.sessions.get(&id).is_some_and(Session::is_pm);
        let projects = self.projects(&squad);
        let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
        let no_project = |arg: &str| format!("No project called {arg}. The squad's projects: {}.", names.join(", "));
        match name.as_str() {
            board::BOARD => {
                // A long board comes in pages (`status`, `after`), one answer at most `tool_cap`'s ceiling.
                let paged = self.journal(&squad).is_none_or(Journal::context_tool_caps);
                // A member's page is a third of the PM's: the whole board at every look
                // (38k characters on a busy squad) filled a designer's conversation within
                // the hour, and it rarely needs more than its own corner — the rest is a page away.
                let page = if is_pm { crate::tool_cap::MAX_TOOL_CHARS } else { crate::tool_cap::MAX_TOOL_CHARS / 3 };
                let limit = if paged { page } else { usize::MAX };
                let (status, after) = (board::arg_str(&args, "status"), board::arg_id(&args, "after"));
                let mut text = board::render_page(&self.tickets(&squad), status, after, limit);
                let resting = self.journal(&squad).map(|j| j.resting()).unwrap_or_default();
                if let Some(line) = board::resting_line(&resting) {
                    text = format!("{}\n\n{line}", text.trim_end());
                }
                Ok(text)
            }
            board::CREATE_TICKET => {
                let title = board::arg_str(&args, "title").ok_or("A ticket needs a title.")?.trim();
                let written = self.squad_language(&squad);
                glance::check_title(written, title)?;
                let label = glance::label_arg(&args)?;
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
                let idea = args.get("idea").and_then(Value::as_bool).unwrap_or(false);
                // The person's request it serves, when the member names one — and which it serves:
                // whenever more than one of theirs could be it, the member is asked to name it.
                let named_request = self.request_arg(&squad, &args)?;
                let serving = if idea { None } else { self.serving(&squad, id, named_request)? };
                let named_request = named_request.filter(|r| *r != crate::requests::NO_REQUEST);
                // A kind of work the person stopped is turned away — unless they asked for it again since.
                if !idea {
                    let working = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
                    let mut lineage: Vec<TicketId> = working.into_iter().collect();
                    let of_work = working.and_then(|w| self.journal(&squad).and_then(|j| j.follow_up(w)));
                    lineage.extend(of_work.iter().flat_map(|f| [f.parent, f.root]));
                    let serves = named_request.or_else(|| working.and_then(|w| self.journal(&squad).and_then(|j| j.requests().of_ticket(w))));
                    let stops = self.stops_in_force(&squad);
                    if let Some(stop) = crate::stops::stopping(&stops, title, body, &lineage, serves).cloned()
                        && (working.is_some() || !self.asked_again(id, &stop))
                    {
                        self.stop_refused(&squad, &stop, title);
                        return Err(crate::stops::refusal(&stop));
                    }
                }
                // Its checklist, when given; and no tickets for meta work, or for the next part
                // of a request that goes on the same ticket by the baton (`flow`).
                let steps = crate::flow::steps_arg(&args, "steps")?;
                if !idea {
                    let working = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
                    let serves = working.and_then(|w| self.journal(&squad).and_then(|j| j.requests().of_ticket(w)));
                    let role = self.sessions.get(&id).and_then(|s| s.member.as_ref()).map(|m| m.role.clone()).unwrap_or_default();
                    let open: Vec<TicketId> = tickets.iter().filter(|t| !t.status.is_closed() && !matches!(t.status, TicketStatus::Idea)).map(|t| t.id).collect();
                    let asked = crate::flow::Asked {
                        title,
                        body,
                        label,
                        severity: board::arg_str(&args, "severity").map(str::trim),
                        depends_on: &depends_on,
                        separate_why: board::arg_str(&args, "separate_why"),
                        idea,
                    };
                    let maker = crate::flow::Maker {
                        role: &role,
                        pm: is_pm,
                        working,
                        working_serves_request: serves.is_some(),
                        same_request: serves.is_some() && named_request.is_none_or(|r| Some(r) == serves),
                    };
                    // A designer's finding from design QA or a review is placed by its own rules (`follow_ups`).
                    let finding = role == crate::flow::DESIGNER && board::arg_str(&args, "severity").is_some();
                    if !finding && let Some(why) = crate::flow::meta_refusal(&asked, &maker, &open) {
                        crate::daemon_log!("equipe: {squad}: {member}'s ticket “{title}” turned away: meta work or the next part of a request");
                        return Err(why);
                    }
                }
                // A request's ticket isn't chained behind other work without a reason (`requests`).
                let depends_note = if idea { String::new() } else { self.check_depends(&squad, None, serving.map(|(r, _, _)| Some(r)), title, body, &depends_on, &args)? };
                // Where the PM runs the work, a member's proposal is simply done:
                // on the board, its own unless it names someone else.
                let just_do = !is_pm && self.journal(&squad).is_some_and(|j| j.pm_accepts());
                let owner = board::arg_str(&args, "owner").map(str::to_owned).or_else(|| just_do.then(|| member.clone()));
                // A designer's ticket from the ticket it is on keeps where it came from (`follow_ups`).
                let filing = if idea { None } else { self.follow_up_filing(&squad, id, &args)? };
                let mut ticket = Ticket {
                    id: 0,
                    title: title.to_owned(),
                    body: body.to_owned(),
                    // The PM's are ready, unless it puts one up for the person to take on.
                    status: if idea {
                        TicketStatus::Idea
                    } else if (is_pm && !proposal) || just_do {
                        TicketStatus::Todo
                    } else {
                        TicketStatus::Proposed
                    },
                    kind,
                    owner: owner.clone(),
                    assignee: None,
                    project,
                    depends_on,
                    branch: None,
                    worktree: None,
                    execution: None,
                    merged: None,
                    off_main: None,
                    design_qa: None, landing: false,
                    // Read and checked off the loop before it gets here (`cut_from_plan`).
                    plan: board::arg_str(&args, "plan").map(str::to_owned),
                    model: None,
                    flow: None,
                    created_by: member.clone(),
                    note: None,
                    asks: vec![],
                    label, remaining: Vec::new(), images: Vec::new(),
                    updated_at: None,
                    priority: Default::default(), rank: 0,
                };
                if let Some(filing) = &filing
                    && let Some(reply) = self.file_follow_up(&squad, &member, filing, &mut ticket)?
                {
                    return Ok(reply);
                }
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let new_id = journal.create_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(new_id, &member, "created", title);
                if let Some(filing) = &filing {
                    self.keep_follow_up(&squad, new_id, filing);
                }
                let by = if is_pm { crate::telemetry::Maker::Pm } else { crate::telemetry::Maker::Member };
                self.tell(&squad, crate::telemetry::Happened::TicketCreated { kind, by });
                let made = Ticket { id: new_id, ..ticket };
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket: made.clone() });
                // The person's request it serves: theirs to follow without asking.
                let serves = if idea { None } else { self.ticket_made(&squad, new_id, serving) };
                // The ticket answering the person drives their request: its driver and checklist (`flow`).
                let flowing = if idea { None } else { self.start_flow(&squad, id, &made, steps) };
                let mut reply = format!("Created T-{new_id}.");
                if kind.in_home() {
                    reply = format!("Created T-{new_id} ({}).", kind.label());
                }
                if equipe_protocol::title::is_long(title) {
                    reply.push_str(&format!(" The title is longer than reads at a glance: {} characters or {} words is the most — shorten it with update_ticket if you can.", equipe_protocol::title::GOOD_CHARS, equipe_protocol::title::GOOD_WORDS));
                }
                if idea {
                    reply = format!("Kept T-{new_id} as an idea — nothing starts it. Tell the person in one line it's kept; when the board is light you'll be asked to look at the ideas again.");
                    if !is_pm {
                        let line = format!("{member} kept the person's idea T-{new_id}: {title}");
                        self.brief_pm(&squad, line, None);
                    }
                } else if is_pm && proposal {
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
                if let Some(serves) = serves {
                    reply.push_str(&serves);
                }
                if let Some(flowing) = flowing {
                    reply.push_str(&flowing);
                }
                reply.push_str(&depends_note);
                Ok(reply)
            }
            board::UPDATE_TICKET => {
                if !is_pm {
                    return Err("Only the PM changes tickets; propose a new one, or comment.".into());
                }
                let ticket_id = board::arg_id(&args, "id").ok_or("Which ticket? Give its id.")?;
                // Yes to a relay's card: the missing piece made or found, and the ticket waits on it.
                if let Some(missing) = crate::blocked_on::parse(&args)? {
                    let said = self.pm_blocked_on(&squad, &member, ticket_id, &missing)?;
                    let other = args.as_object().is_some_and(|o| o.keys().any(|k| k != "id" && k != "blocked_on"));
                    return Ok(if other { format!("{said} Only `blocked_on` was applied: change the rest in another call.") } else { said });
                }
                // Its checklist grows at any status still open (`flow`).
                let steps = crate::flow::steps_arg(&args, "steps")?;
                if !steps.is_empty() {
                    let said = self.change_steps(&squad, ticket_id, &[], steps, &member)?;
                    let other = args.as_object().is_some_and(|o| o.keys().any(|k| k != "id" && k != "steps"));
                    if !other {
                        return Ok(said);
                    }
                }
                // Put with one of the person's requests: whatever its status.
                if let Some(request) = self.request_arg(&squad, &args)? {
                    self.link_to_request(&squad, ticket_id, request)?;
                    let other = args.as_object().is_some_and(|o| o.keys().any(|k| k != "id" && k != "request"));
                    if !other {
                        return Ok(format!("T-{ticket_id} now serves the person's request R-{request}."));
                    }
                }
                let lang = self.squad_language(&squad);
                let present: Vec<String> = self
                    .sessions
                    .values()
                    .filter(|s| s.squad.as_deref() == Some(squad.as_str()))
                    .filter_map(|s| s.name().map(str::to_owned))
                    .collect();
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let mut ticket = journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
                if !matches!(ticket.status, TicketStatus::Proposed | TicketStatus::Idea | TicketStatus::Todo | TicketStatus::Waiting) {
                    let status = ticket.status.label();
                    return Err(format!("T-{ticket_id} is {status} — from here it's the person's to change."));
                }
                if let Some(title) = board::arg_str(&args, "title").map(str::trim).filter(|t| *t != ticket.title) {
                    glance::check_title(lang, title)?;
                    // The old one stays in its history.
                    let _ = journal.ticket_event(ticket_id, &member, glance::RETITLED, &ticket.title);
                    ticket.title = title.to_owned();
                }
                if let Some(label) = glance::label_arg(&args)? {
                    ticket.label = Some(label);
                }
                let replanned = board::arg_str(&args, "body").is_some_and(|b| b.trim() != ticket.body.trim());
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
                let mut depends_note = String::new();
                if let Some(ids) = board::arg_ids(&args, "depends_on") {
                    let added: Vec<TicketId> = ids.iter().copied().filter(|d| !ticket.depends_on.contains(d)).collect();
                    let text = format!("{} {}", ticket.title, ticket.body);
                    depends_note = self.check_depends(&squad, Some(ticket_id), None, &ticket.title, &text, &added, &args)?;
                    ticket.depends_on = ids;
                }
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let waiting = ticket.status == TicketStatus::Waiting;
                let was_idea = ticket.status == TicketStatus::Idea;
                match board::arg_str(&args, "status") {
                    // An idea's time has come: the person is asked, on its card.
                    Some("proposed") if was_idea => {
                        let why = board::arg_str(&args, "why").map(str::trim).filter(|w| !w.is_empty()).ok_or("Say `why` now — what makes this idea timely — in the person's language; it shows on the card that asks them.")?;
                        ticket.status = TicketStatus::Proposed;
                        ticket.note = Some(format!("{}{why}", equipe_protocol::WHY_NOW));
                    }
                    Some("proposed") => return Err("Only an idea is put up as a proposal; a ready ticket goes back to one by the person's hand.".into()),
                    Some("idea") if waiting => return Err(format!("T-{ticket_id} waits on the person; it can't be kept for later until that's settled.")),
                    Some("idea") => {
                        ticket.status = TicketStatus::Idea;
                        ticket.note = None;
                    }
                    // Back to the member that put it down, where it left off.
                    Some("todo") if waiting => {
                        back_to_its_member(&mut ticket, &present, "Resumed: the PM says what you were waiting for is done.");
                    }
                    Some("todo") => {
                        ticket.status = TicketStatus::Todo;
                        ticket.note = None;
                    }
                    Some("cancelled") => ticket.status = TicketStatus::Cancelled,
                    Some(other) => return Err(format!("A ticket's status can be set to todo, cancelled, proposed (an idea) or idea, not {other}.")),
                    None => {}
                }
                journal.save_ticket(&ticket).map_err(|e| e.to_string())?;
                let _ = journal.ticket_event(ticket_id, &member, "updated", &args.to_string());
                let pitched = was_idea && ticket.status == TicketStatus::Proposed;
                self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
                // A new plan: what its member asked the person for under the old one is taken away.
                let withdrawn = if replanned { self.withdraw_ticket_cards(&squad, ticket_id, withdraw::TicketTurn::Replanned) } else { 0 };
                let withdrawn = match withdrawn {
                    0 => String::new(),
                    n => format!(" Its plan changed, so {n} card(s) its member raised under the old one (a secret, a sign-in, an ask) were taken away; its member asks again if the new plan still needs them."),
                };
                if pitched {
                    return Ok(format!("Put idea T-{ticket_id} to the person: its card asks whether to start it now. Tell them in a line or two why now; their click starts it.{withdrawn}"));
                }
                Ok(format!("Updated T-{ticket_id}.{depends_note}{withdrawn}"))
            }
            board::COMMENT => {
                let ticket_id = board::arg_id(&args, "id").ok_or("Which ticket? Give its id.")?;
                let text = board::arg_str(&args, "text").ok_or("Say something.")?;
                let word = crate::accept::review_word(&args)?;
                let why = crate::accept::send_back_reason(&args)?;
                let journal = self.journal(&squad).ok_or("The board can't be opened.")?;
                let ticket = journal.ticket(ticket_id).ok().flatten().ok_or_else(|| format!("There is no T-{ticket_id}."))?;
                // The PM's word on work in review sends it back or only notes it: said which.
                if is_pm {
                    return self.pm_comment(&squad, &member, &ticket, text, word, why);
                }
                journal.ticket_event(ticket_id, &member, board::COMMENT, text).map_err(|e| e.to_string())?;
                // Its last word on its own ticket: what the PM reads if its turns end with no end to the work.
                if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).filter(|w| w.ticket == ticket_id) {
                    work.end.last_comment = Some(text.to_owned());
                }
                Ok(format!("Noted on T-{ticket_id}."))
            }
            board::HAND_BACK => self.hand_back_tool(&squad, id, &args),
            crate::flow::PASS_BATON => self.pass_baton(&squad, id, &args),
            crate::flow::CHECK_STEPS => self.check_steps(&squad, id, &args),
            crate::flow::REVERT_LANDED if is_pm => self.revert_landed(&squad, &member, &args),
            crate::flow::REVERT_LANDED => Err("Only the PM reverts landed work; tell the PM what is wrong.".into()),
            board::SUBMIT_FOR_REVIEW => {
                let summary = board::arg_str(&args, "summary").ok_or("Summarize what you did.")?.to_owned();
                // Its pictures, found where it works; one that isn't there is said, to fix.
                let (cwd, roots) = self.picture_places(&squad, id);
                let shown = crate::visuals::resolve_all(&board::arg_strs(&args, "images"), &cwd, &roots, crate::visuals::HAND_IN)
                    .map_err(|e| format!("{e}. Save the pictures in your worktree and give their paths, then hand in again."))?;
                // Their thumbnails made now, off the loop: the hand-in's card shows them at once.
                crate::visuals::thumbs::warm(&self.config.home, shown.clone());
                let screens = self.screens_handed_in(&squad, id, &args, shown.len())?;
                // A designer's screen change says what changed and why it is better, under its summary.
                let summary = match self.designer_change_note(id, &args)? {
                    Some(note) => format!("{summary}\n\nWhat changed and why it is better:\n{note}"),
                    None => summary,
                };
                let gap = self.designer_hand_in(id)?;
                let session = self.sessions.get_mut(&id).ok_or("You're not on the squad.")?;
                let work = session.work.as_mut().ok_or("You're not on a ticket — there is nothing to hand in.")?;
                let ticket = work.ticket;
                let summary = match &gap {
                    Some(gap) => format!("{summary}\n\nNot cleared before hand-in — for the person to see (the design checks couldn't be cleared by going round again):\n{}", gap.iter().map(|g| format!("- {g}")).collect::<Vec<_>>().join("\n")),
                    None => summary,
                };
                // Checks that fail on main just the same: pre-existing, not this change's — noted for whoever reads it.
                let failing_on_main = board::arg_strs(&args, "failing_on_main");
                let summary = if failing_on_main.iter().any(|f| !f.trim().is_empty()) {
                    format!("{summary}\n\nPre-existing on main (fails there too, not this change's): {}", failing_on_main.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).collect::<Vec<_>>().join("; "))
                } else {
                    summary
                };
                work.handed_in = Some(summary);
                work.shown = shown;
                work.waiting = None;
                work.screens = screens;
                // Findings of its self-check it says aren't real: a designer judges them.
                work.waive = board::arg_str(&args, "waive_findings").map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
                let red_note = self.handed_in_failing_on_main(&squad, ticket, &member, &failing_on_main);
                let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) else { return Err("You're not on a ticket — there is nothing to hand in.".into()) };
                if let Some(gap) = &gap {
                    let n = gap.len();
                    let line = move |name: &str| format!("{name} handed in T-{ticket} with {n} design check(s) it couldn't clear, marked in its hand-in for the person: look at why before they review it.");
                    let shown = format!("T-{ticket}: handed in with {n} design check(s) not cleared, marked for the person.");
                    self.tell_the_pm(id, line, Some(&shown));
                    return Ok(format!(
                        "Handed in T-{ticket}, with the {n} check(s) it couldn't clear marked for the person; the PM is told. Your changes are committed when this turn ends.{red_note} End your turn now with a short summary."
                    ));
                }
                Ok(if screens {
                    format!(
                        "Handed in T-{}. Your changes are committed when this turn ends; the squad's designer checks \
the screens, then the person reviews them.{red_note} End your turn now with a short summary.",
                        work.ticket
                    )
                } else {
                    format!(
                        "Handed in T-{}. Your changes are committed when this turn ends and the person reviews them.{red_note} \
End your turn now with a short summary.",
                        work.ticket
                    )
                })
            }
            board::BLOCK_ON_PERSON => {
                // A code the site emailed: the person's card fills it from their mailbox on their click.
                if let Some(verification) = args.get("verification").filter(|v| v.is_object()) {
                    return self.verification_asked(&squad, id, verification);
                }
                let asked = board::arg_strs(&args, "needs");
                let permissions: Vec<crate::protocol::asks::Permission> =
                    board::arg_strs(&args, "permission").iter().filter_map(|p| crate::protocol::asks::Permission::parse(p)).collect();
                if asked.is_empty() && permissions.is_empty() {
                    return Err("List what the person has to do, one thing per item.".into());
                }
                // A question or a choice doesn't put a ticket down by itself: it goes through ask_decision and the squad's autonomy policy.
                if let Some(question) = asked.iter().find(|a| board::is_question(a)) {
                    return Err(format!(
                        "\"{}\" is a question, not something only the person can do. Ask it with `ask_decision`: a choice with your \
`recommended` option marked — decide-and-tell, so you carry on with it and your ticket doesn't wait — or, for what only they \
know, a question without options. `block_on_person` is for what only they can do: a token to issue, an account or a setting to change.",
                        clip(question, 120)
                    ));
                }
                // Steps asking the person to change macOS privacy permissions: refused when
                // equipe's desktop tools already hold them, else one card with one button.
                let ready = self.desktop_auto.as_ref().and_then(desktop::auto::Auto::latest);
                let judged = crate::permission_chore::judge(&asked, &permissions, ready.as_ref())?;
                let (asked, chore_told) = (judged.asks, judged.told);
                // A permission card is its own headline (one button, no `ask`/`effort_minutes`); every
                // other waiting ticket needs the one-line ask the person reads in their list.
                let headline = if asked.iter().any(|a| crate::protocol::asks::permission_kinds(a).is_some()) {
                    None
                } else {
                    Some(self.ask_headline(&squad, &args)?)
                };
                // What the person left open last time (not yet, later) is asked again with these.
                let on = self.sessions.get(&id).and_then(|s| s.work.as_ref()).map(|w| w.ticket);
                let open = on.and_then(|t| self.journal(&squad).and_then(|j| j.ticket(t).ok().flatten())).map(|t| t.remaining).unwrap_or_default();
                let needs = crate::protocol::asks::with_open(&asked, &open);
                let again = needs.len() - asked.len();
                let note = equipe_protocol::asks::waiting_note_headed(&needs, board::arg_str(&args, "why"), headline.as_deref());
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
                let chore_told = chore_told.map(|t| format!(" {t}")).unwrap_or_default();
                Ok(format!(
                    "T-{} waits for the person when this turn ends; your changes are kept in your worktree and you get \
it back there once they have done it.{again}{chore_told} End your turn now with a short line on what you need.",
                    work.ticket
                ))
            }
            board::ASK_DECISION => self.ask_decision(&squad, &member, id, &args),
            crate::growth::PROPOSE => self.propose_growth(&squad, &member, id, &args),
            crate::sprint::DEFINE => self.define_experiment(&squad, &member, id, &args),
            crate::withdraw::WITHDRAW_CARD => self.withdraw_card(&squad, &member, id, &args, is_pm, false),
            // From before `withdraw_card`: a conversation started then still has it.
            board::WITHDRAW_DECISION => self.withdraw_card(&squad, &member, id, &args, is_pm, true),
            board::REVISE_TICKET => self.revise_ticket(&squad, &member, id, &args),
            crate::secrets::ASK_SECRET => self.ask_secret(&squad, &member, id, &args),
            crate::cli_signin::CLI_SIGN_IN => self.cli_sign_in(&squad, &member, id, &args),
            crate::connections::REQUEST => self.request_secret(&squad, &member, id, &args),
            crate::squad_secrets::SAVE => self.secret_save(&squad, &member, id, &args),
            crate::squad_secrets::LIST => self.secret_list(&squad),
            crate::squad_secrets::USE => self.secret_use(&squad, &member, id, &args),
            crate::squad_secrets::WRITE => self.secret_write(&squad, &member, id, &args),
            board::DESIGN_QA => self.finish_design_qa(&squad, id, &args),
            board::DESIGN_REVIEW => self.finish_design_review(&squad, id, &args),
            board::QA_PICTURES => self.qa_pictures(&squad, id, &args),
            // Asked by a designer's or frontend engineer's board server, which runs `system_tools` itself, in this folder.
            board::WORKDIR => {
                let session = self.sessions.get(&id).ok_or("You're not on the squad.")?;
                if session.member.as_ref().is_none_or(|m| !board::builds_from_system(&m.role)) {
                    return Err("Only a designer's or a frontend engineer's board server asks where it works.".into());
                }
                Ok(session.work.as_ref().map_or_else(|| session.cwd.clone(), |w| w.cwd.clone()).to_string_lossy().into_owned())
            }
            board::SYSTEM_TOOLS => Err("system_tools is run by your board server; this equipe's server is older than its daemon — restart your conversation.".into()),
            board::DESIGN_TOOLS => Err("design_tools is run by your board server; this equipe's server is older than its daemon — restart your conversation.".into()),
            crate::team::PROPOSE_TEAM_CHANGE => self.propose_team_change(&squad, &member, id, &args),
            crate::heartbeat::PROPOSE_ROUTINE => self.propose_routine(&squad, &member, id, &args),
            board::RETITLE_OPEN_TICKETS => {
                if !is_pm {
                    return Err("Only the PM retitles tickets.".into());
                }
                self.retitle_open_tickets(&squad, &member, &args)
            }
            board::ADD_PROJECT => {
                if !is_pm {
                    return Err("Only the PM adds projects; tell the PM which folder the person wants added.".into());
                }
                let typed = board::arg_str(&args, "path").ok_or("Which folder? Give its `path`.")?;
                let user_home = std::env::var_os("HOME").map(PathBuf::from);
                let path = crate::squads::add_folder_asked(&self.config.home, &squad, typed, user_home.as_deref())?;
                let name = board::project_name(&path);
                crate::daemon_log!("equipe: {squad} has a new project the PM added, {name} ({})", path.display());
                self.publish_projects(&squad);
                let shown = say!(self, "news.project_added", name = name, path = path.display().to_string());
                self.show_news(id, shown);
                Ok(format!("Added {name} at {} — tickets can name it now. Tell the person in one line; they can take it off from the sidebar.", path.display()))
            }
            crate::signals::SIGNALS => self.read_signals(&squad, id, &args),
            crate::controls::SETTINGS if is_pm => Ok(self.settings_text(&squad)),
            crate::controls::SETTINGS => Err("Only the PM reads the squad's settings; ask the PM.".into()),
            crate::metrics::tool::METRICS => {
                let today = crate::metrics::day_of(&crate::routines::Local, now_secs());
                crate::metrics::tool::answer(&self.config.home, &squad, &args, today)
            }
            crate::accept::PROPOSE_ACCEPT => self.propose_accept(&squad, &member, id, &args),
            crate::go_live::SET_DEPLOY if is_pm => self.set_deploy(&squad, id, &args),
            crate::go_live::SET_DEPLOY => Err("Only the PM says how a project goes live; tell the PM.".into()),
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
                let at_once = self.subagents_at_once(Some(&squad));
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
                let reply = board::estimate_reply(done, !session.subagents.is_empty(), subagents_of(session, at_once));
                // When it said a ticket would be done, on the ticket's record: the retro tells a miss.
                let said = session.work.as_ref().map(|w| w.ticket).zip(done.and(estimate.as_ref().and_then(|e| e.done_at)));
                if let (Some((ticket, at)), Some(journal)) = (said, self.journal(&squad)) {
                    let _ = journal.ticket_event(ticket, &member, crate::retro::ESTIMATED, &at.to_string());
                }
                self.publish(Event::Estimated { session: id, estimate });
                Ok(reply)
            }
            other => Err(format!("equipe has no tool called {other}.")),
        }
    }

    // ── attach and restart ──

    /// The Mermaid blocks and HTML cards of replies just ended, drawn as PNGs
    /// off the loop and told as `visuals_rendered` (docs/VISUALS.md); one that
    /// won't draw is not told. Cards only while they are on
    /// (`visuals::card::enabled`), each reply within its own budget. Then the
    /// squad's `visuals/` is brought within its quota, keeping what its open
    /// conversations show (what a snapshot would list) and what was just drawn.
    fn draw_visuals(&self, session: SessionId, squad: &str, finished: &[Entry]) {
        let texts: Vec<String> = finished.iter().filter_map(|line| match line {
            Entry::Reply { text } if has_visuals(text) => Some(text.clone()),
            _ => None,
        }).collect();
        if texts.is_empty() {
            return;
        }
        let mut shown: std::collections::HashSet<String> = self
            .sessions
            .values()
            .filter(|s| s.squad.as_deref() == Some(squad))
            .flat_map(|s| {
                let entries = s.transcript.entries();
                visual_keys(&entries[entries.len().saturating_sub(SNAPSHOT_LINES)..]).collect::<Vec<_>>()
            })
            .collect();
        let (home, squad, internal) = (self.config.home.clone(), squad.to_owned(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            for text in texts {
                let mut visuals = crate::visuals::diagram::draw(&home, &squad, &text);
                visuals.extend(crate::visuals::card::draw(&home, &squad, &text));
                shown.extend(visuals.iter().map(|v| v.key.clone()));
                if !visuals.is_empty() {
                    let _ = internal.send(Internal::Event(Box::new(Event::VisualsRendered { session, visuals })));
                }
            }
            crate::visuals::cache::prune(&crate::visuals::diagram::dir(&home, &squad), &shown, crate::visuals::cache::Quota::default());
        });
    }

    fn snapshot(&mut self, squad: &str) -> Snapshot {
        watchdog::mark("snapshot: board");
        let projects = self.projects(squad);
        let board = self.tickets(squad);
        // Open tickets, and the latest closed ones: only those copied.
        let recent_closed: Vec<TicketId> =
            board.iter().rev().filter(|t| t.status.is_closed()).map(|t| t.id).take(CLOSED_TICKETS_SHOWN).collect();
        let mut tickets: Vec<Ticket> = board.iter().filter(|t| !t.status.is_closed() || recent_closed.contains(&t.id)).cloned().collect();
        drop(board);
        for t in &mut tickets {
            t.landing = self.landing(squad, t);
        }
        watchdog::mark("snapshot: members");
        let marks = self.journals.get(squad).and_then(|j| j.conversation_marks().ok()).unwrap_or_default();
        let pins = self.journals.get(squad).and_then(|j| j.conversation_pins().ok()).unwrap_or_default();
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .map(|(&id, s)| {
                let entries = s.transcript.entries();
                let (recent, recent_at) = scope::from_at(entries, s.transcript.times(), entries.len().saturating_sub(SNAPSHOT_LINES));
                let mut visuals = crate::visuals::diagram::kept(&self.config.home, squad, &recent);
                visuals.extend(crate::visuals::card::kept(&self.config.home, squad, &recent));
                SessionInfo {
                    session: id,
                    project: s.project.clone(),
                    member: s.member.clone(),
                    engine: s.engine,
                    model: s.ran.clone().or_else(|| s.model.clone()),
                    asked_model: s.model.clone().filter(|m| s.ran.as_ref().is_some_and(|r| r != m)),
                    access: s.access,
                    workspace: s.sandbox.as_ref().map(workspace_of),
                    state: if s.is_working() || s.is_preparing() { SessionState::Working } else { SessionState::Ready },
                    title: s.title.clone(),
                    tokens: s.tokens,
                    context: s.context,
                    recent,
                    recent_at,
                    queued: s.queued.clone(),
                    queued_project: s.memo_threads.first().cloned().flatten(),
                    queued_in: s.memo_threads.clone(),
                    steering: s.steering.clone(),
                    steering_in: s.steer_threads.clone(),
                    lane: s.lane.clone().map(|(of, project)| equipe_protocol::Lane { of, project, kept: s.kept, active: marks.get(&id).map_or(0, |m| m.0), pinned: marks.get(&id).is_some_and(|m| m.1), pinned_at: pins.get(&id).copied().unwrap_or(0) }),
                    run: s.run_of.zip(s.work.as_ref()).map(|(of, w)| equipe_protocol::RunOf { of, ticket: w.ticket, generation: w.execution_generation }),
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
                        .map(|s| equipe_protocol::SubagentInfo {
                            id: s.id.clone(),
                            what: s.what.clone(),
                            step: s.step.clone(),
                            secs: s.since.elapsed().as_secs(),
                            brief: s.brief.clone(),
                            entries: s.entries.iter().cloned().collect(),
                            stuck_secs: s.stuck.then(|| s.heard.elapsed().as_secs()),
                        })
                        .collect(),
                    compacting: s.compacting.as_ref().map(|(info, ..)| info.clone()),
                    visuals,
                }
            })
            .collect();
        // The PM first, then everyone in the order they joined.
        sessions.sort_by_key(|info| {
            let s = &self.sessions[&info.session];
            (!s.is_pm(), s.joined)
        });
        watchdog::mark("snapshot: settings and cards");
        let mode = self.journal(squad).map(Journal::mode).unwrap_or_default();
        let mut approvals: Vec<Approval> =
            self.pending.values().filter(|p| p.squad == squad).map(|p| p.approval.clone()).collect();
        approvals.sort_by_key(|a| a.id);
        let scale = self.journals.get(squad).map(Journal::scale);
        let suggestions_off = self.journals.get(squad).is_some_and(Journal::suggestions_off);
        let allowed = self.journals.get(squad).and_then(|j| j.allowed().ok()).unwrap_or_default();
        let autonomy = self.journals.contains_key(squad).then(|| self.autonomy_state(squad));
        let browsers_shown = self.journals.get(squad).is_some_and(Journal::browsers_all_shown);
        let questions = self.question_sets(squad).clone();
        let team_changes = self.team_changes(squad).clone();
        let accepts = self.accepts(squad).clone();
        let hq = self.hq(squad);
        let routines = self.routines(squad);
        let routine_proposals = self.routine_proposals(squad).clone();
        let idle_wake_hours = self.journals.get(squad).map(Journal::idle_wake_hours);
        let subagents_at_once = self.subagents_at_once(Some(squad));
        let signals = self.signal_views(squad);
        let deploys = self.deploy_views(squad);
        // Nemotron's reviews of the tickets shown.
        let shown: Vec<TicketId> = tickets.iter().map(|t| t.id).collect();
        let reviews = self.journals.get(squad).and_then(|j| j.reviews(&shown).ok()).unwrap_or_default();
        let put_away = self.journals.get(squad).and_then(|j| j.put_away(PUT_AWAY_SHOWN).ok()).unwrap_or_default();
        // Which closed session each open member carries on from: its moves on record.
        let hand_overs = self.hand_overs(squad);
        Snapshot {
            signals,
            deploys,
            reviews,
            put_away,
            routines,
            routine_proposals,
            idle_wake_hours,
            notify: self.journals.contains_key(squad).then(|| self.notify_settings(squad)),
            subagents_at_once,
            design_reviewer: self.journals.get(squad).map(Journal::design_reviewer).unwrap_or_default(),
            team_changes,
            accepts,
            requests: self.requests_listed(squad),
            stops: self.stops_in_force(squad),
            decided: self.decided_for_person(squad),
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
            parallel: self.journals.get(squad).map(Journal::parallel).unwrap_or_default(),
            pacing: self.pacing_now(squad),
            model_choice: self.journals.get(squad).map(Journal::model_choice).unwrap_or_default(),
            pm_accepts: self.journals.get(squad).is_none_or(Journal::pm_accepts),
            resting: self.journals.get(squad).map(Journal::resting).unwrap_or_default(),
            role_models: self.journals.get(squad).map(Journal::role_models).unwrap_or_default(),
            secret_asks: self.secret_asks(squad).clone(),
            cli_sign_ins: self.cli_sign_in_cards(squad),
            mcp_servers: self.mcp_servers_of(squad),
            mcp_status: self.mcp_status_of(squad),
            verifications: self.verification_cards(squad),
            login_offers: self.login_offers_now(),
            engine_proposals: self.engines.proposals(squad),
            hand_overs,
            desktop: self.desktop.state(),
            allowed,
            autonomy,
            browsers_shown,
            disk: self.disk_for_snapshot(),
            words: Some(self.words_live(squad)),
            words_all: self.journals.get(squad).is_some_and(Journal::words_all),
            auto_put_away_off: self.journals.get(squad).is_some_and(|j| !j.auto_put_away()),
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
            // A run still making its workspace when the daemon stopped never
            // ran: its ticket goes back on the board, to start afresh. What it
            // left half-made, tidy clears as it always did.
            let unstarted = journal.release_starting_executions(now_secs()).unwrap_or_default();
            if !unstarted.is_empty() {
                let ids: Vec<String> = unstarted.iter().map(|t| format!("T-{t}")).collect();
                crate::daemon_log!("equipe: {squad}: {} still preparing a workspace when equipe stopped; back on the board", ids.join(", "));
                let _ = self.internal.send(Internal::Dispatch);
            }
            // No runtime session outlives the process that ran it: every run
            // still active when the daemon stopped — and in-progress work from
            // before runs were kept — is stalled now. A member brought back
            // below on the ticket it ran, its checkout still there and its
            // engine able to resume here, carries it on under a new generation
            // (`resume_work`); what nothing carries on stays stalled for the
            // person, its ticket, work and files untouched.
            let carried: Vec<TicketId> = journal
                .tickets()
                .unwrap_or_default()
                .iter()
                .filter(|t| t.status == TicketStatus::InProgress && t.execution.as_ref().is_none_or(|e| e.state == ExecutionState::Active))
                .map(|t| t.id)
                .collect();
            // The run session each was in, for what was said there.
            let ran_in: HashMap<TicketId, SessionId> = journal
                .tickets()
                .unwrap_or_default()
                .iter()
                .filter_map(|t| t.execution.as_ref().filter(|e| e.state == ExecutionState::Active).map(|e| (t.id, e.session)))
                .collect();
            // Each member's runs to carry on, opened once the squad is back.
            let mut runs: Vec<(SessionId, Work)> = Vec::new();
            let _ = journal.stall_all_executions(now_secs());
            let codex_cli = codex_app::find(&self.config.codex_binary).is_some();
            let mut carried_on: Vec<TicketId> = Vec::new();
            // A squad from before squad homes gets its home now.
            let home = squad_home::path(&self.config.home, &squad);
            if let Err(err) = squad_home::ensure(&self.config.home, &squad, journal.language().as_deref()) {
                crate::daemon_log!("equipe: {squad}'s folder couldn't be made: {err}");
            }
            // Members of one role once each kept their own lessons; now they share them.
            if let Err(err) = journal::fold_member_memories(&self.config.home, &squad) {
                crate::daemon_log!("equipe: {squad}'s members' memories couldn't be gathered: {err}");
            }
            // Attachments sent before they were kept: what is still there stays, with its words.
            match crate::kept::rescue(&self.config.home, &journal) {
                Ok(Some(r)) if r != crate::kept::Rescued::default() => crate::daemon_log!(
                    "equipe: {squad}: sent attachments rescued: {} files kept, the words of {} text files kept with their messages, {} gone already",
                    r.kept,
                    r.words,
                    r.gone
                ),
                Ok(_) => {}
                Err(err) => crate::daemon_log!("equipe: {squad}'s sent attachments couldn't be looked through: {err}"),
            }
            let projects = journal.projects().unwrap_or_default();
            keep_docs_from_repos(&dir.path(), &home, &projects);
            let tickets = journal.tickets().unwrap_or_default();
            for mut record in journal.open_sessions().unwrap_or_default() {
                let Some(member) = record.member.clone() else {
                    // Only members live in a squad; anything else can't be resumed.
                    let _ = journal.close_session(record.id);
                    continue;
                };
                let (conversation, said_at) = journal.conversation_at(record.id).unwrap_or_default();
                let history = conversation
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::User { text, kept } => harness::words(text).map(|text| responses::Message { role: responses::Role::User, text: tags::read_back(&text, kept).into_owned() }),
                        Entry::Reply { text } => {
                            Some(responses::Message { role: responses::Role::Assistant, text: text.clone() })
                        }
                        _ => None,
                    })
                    .collect();
                let edits = record.access == Access::EditFiles;
                let resumes = record.engine == Engine::Claude || codex_cli;
                // Every ticket it ran comes back as a run of its own, each
                // exactly once, under a new generation.
                let works: Vec<Work> = tickets
                    .iter()
                    .filter(|_| resumes)
                    .filter(|t| t.status == TicketStatus::InProgress && t.assignee.as_deref() == Some(member.name.as_str()) && carried.contains(&t.id))
                    .filter_map(|ticket| restore_work(&journal, ticket, &projects, &home, edits))
                    .filter_map(|mut work| {
                        let session = equipe_protocol::new_session_id();
                        let operation = format!("restart:{session}");
                        work.execution_generation = journal.acquire_execution(work.ticket, session, &operation, now_secs(), EXECUTION_LEASE_SECS).ok().flatten()?;
                        work.execution_session = session;
                        carried_on.push(work.ticket);
                        Some(work)
                    })
                    .collect();
                runs.extend(works.into_iter().map(|work| (record.id, work)));
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
                        ran: None,
                        access: record.access,
                        cwd: record.cwd,
                        sandbox: None,
                        codex: None,
                        history,
                        claude_session: record.engine_session,
                        work: None,
                        prompts: Vec::new(),
                        title: record.title,
                        title_started: false,
                        tokens: record.tokens,
                        context: None,
                        desk_context: None,
                        last_turn: None,
                        refresh,
                        transcript: Transcript::with_times(conversation, said_at),
                        turn: None,
                        queued,
                        memo_threads,
                        memo_thread: None,
                        steer: None,
                        steering: Vec::new(),
                        steer_threads: Vec::new(),
                        stop: None,
                        stop_subagent: None,
                        work_over: None,
                        wind_down: None,
                        taken_back: Vec::new(),
                        send_now_prompt: None,
                told: Default::default(),
                        subagents: Vec::new(),
                        compacting: None,
                        briefing: journal.waiting(record.id, "briefing").unwrap_or_default(),
                        memory,
                        mode: record.mode,
                        thread,
                        estimate: None,
                        turn_began: None,
                        watch: TurnWatch::default(),
                        threads,
                        lane: None,
                        kept: false,
                        run_of: None,
                        slot: 0,
                        answers_person: false,
                        on_phone: false,
                        effort: None,
                    },
                );
            }
            // The conversations the person opened with the PM, as they were left.
            let pm = self.sessions.iter().find(|(_, s)| s.squad.as_deref() == Some(squad.as_str()) && s.is_member_itself() && s.is_pm()).map(|(&id, _)| id);
            if let Some(pm) = pm {
                for c in journal.conversations().unwrap_or_default() {
                    let thread = DeskThread { claude_session: c.engine_session, ..DeskThread::default() };
                    let mut lane = lane_of(pm, &self.sessions[&pm], c.project, thread, true);
                    let (lines, lines_at) = journal.conversation_at(c.id).unwrap_or_default();
                    lane.title = c.title;
                    lane.transcript = Transcript::with_times(lines, lines_at);
                    self.sessions.insert(c.id, lane);
                }
            }
            self.journals.insert(squad.clone(), journal);
            // The runs carried on, each beside its member, with what was said in it.
            for (member, work) in runs {
                let before = ran_in.get(&work.ticket).copied();
                let Some(run) = self.open_run(member, work) else { continue };
                let lines = before.and_then(|old| self.journals.get(&squad).and_then(|j| j.conversation_at(old).ok()));
                if let (Some((lines, at)), Some(session)) = (lines, self.sessions.get_mut(&run)) {
                    session.transcript = Transcript::with_times(lines, at);
                }
            }
            // What the restart couldn't carry on: stalled, for the person.
            for ticket in carried.into_iter().filter(|t| !carried_on.contains(t)) {
                if self.journals.get(&squad).and_then(|j| j.ticket(ticket).ok().flatten()).is_some_and(|t| t.status == TicketStatus::InProgress) {
                    self.stalled(&squad, ticket, true);
                }
            }
            // A squad whose PM joined before the weekly retro was: it comes now, once.
            if pm.is_some() && self.config.persist {
                self.add_joining_routine(&squad, crate::retro::ROLE);
            }
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
            self.publish(Event::Notice { undo: None, session: id, text });
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
        let asked = session.refresh.take();
        let summary = match asked {
            Some(summary) => summary,
            None if full || cold => true,
            None => return None,
        };
        // Handed on with a summary: a compaction of equipe's own, the person's
        // (`/compact`) or for room. Starting clean (`/clear`) is none.
        if summary {
            let trigger = if asked.is_some() { CompactTrigger::Manual } else { CompactTrigger::Auto };
            self.compaction_began(id, compact::By::Equipe, trigger);
        }
        let session = self.sessions.get_mut(&id)?;
        session.claude_session = None;
        session.desk_context = None;
        session.context = None;
        let tail: Vec<String> = session
            .transcript
            .entries()
            .iter()
            .filter_map(|e| match e {
                Entry::User { text, kept } => harness::words(text).map(|text| format!("- the person: {}", clip(&tags::read_back(&text, kept), 600))),
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
        self.publish(Event::Notice { undo: None, session: id, text: shown });
        self.compaction_ended(id, None);
        Some(if summary && !tail.is_empty() {
            format!(
                "[equipe: this is a fresh conversation — {why}. Where the last one left off, latest last:\n{tail}\nYour memory and the board hold the rest.]"
            )
        } else {
            "[equipe: this is a fresh conversation. Your memory and the board hold what lasts.]".to_owned()
        })
    }

    /// The note a fresh Codex thread opens with when OpenAI's safety check
    /// refused a turn of `id` as possibly unintended activity and the turn is
    /// asked again there ([`codex_app::Controls::refused_retry`]): where the
    /// conversation left off, latest last — equipe's framing of a design
    /// session kept out of it, the message being answered left to follow.
    fn refused_handoff(&self, id: SessionId) -> String {
        let Some(session) = self.sessions.get(&id) else { return String::new() };
        let mut entries = session.transcript.entries();
        if let Some((Entry::User { .. }, before)) = entries.split_last() {
            entries = before;
        }
        let lines: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::User { text, kept } => harness::words(text)
                    .map(|text| crate::design::session::framing_apart(&text).0)
                    .map(|text| format!("- the person: {}", clip(&tags::read_back(&text, kept), 600))),
                Entry::Reply { text } => Some(format!("- you: {}", clip(text, 600))),
                _ => None,
            })
            .collect();
        let tail = lines[lines.len().saturating_sub(REFRESH_TAIL)..].join("\n");
        let mut note = "This is a fresh conversation: OpenAI's safety check refused a turn of your last one as possibly unintended \
activity, so equipe moved you here."
            .to_owned();
        if !tail.is_empty() {
            note.push_str(&format!(" Where the last one left off, latest last:\n{tail}\n"));
        }
        note.push_str(&match &session.work {
            Some(work) => {
                let place = work.sandbox.as_ref().map_or_else(|| work.cwd.clone(), |sb| sb.path.clone());
                format!(" You are on T-{}, in {}: the ticket's notes on the board and your work there hold the rest.", work.ticket, place.display())
            }
            None => " Your memory and the board hold the rest.".to_owned(),
        });
        note
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
        let mut busy: Vec<PathBuf> = self
            .sessions
            .values()
            .filter(|s| s.is_working())
            .filter_map(|s| Some(journal::build_dir(&self.config.home, s.squad.as_deref()?, s.name()?)))
            .collect();
        // And where accepted work's checks are building.
        busy.extend(self.gate_builds());
        let home = self.config.home.clone();
        std::thread::spawn(move || {
            for line in worktree::trim_builds(&home, &busy, BUILD_CAP, BUILDS_BUDGET) {
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

    /// Lanes' temporary folders of tickets that are not open go, off the loop:
    /// the safety net for a close that was missed (a crash, an older daemon).
    /// At start, daily by the prune, and hourly.
    fn tidy_lane_tmp(&mut self) {
        self.lane_tmp_tidied = std::time::Instant::now();
        let home = self.config.home.clone();
        // Lanes' temporary folders: each squad's own open tickets say which stay.
        let open: Vec<(String, Vec<TicketId>)> = self
            .journals
            .iter()
            .filter_map(|(squad, j)| Some((squad.clone(), j.board().ok()?.iter().filter(|t| !t.status.is_closed()).map(|t| t.id).collect())))
            .collect();
        let keep: Vec<u32> = self.commands.engine_pids();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        let looked: Vec<crate::disk::LaneSquad> = squads
            .iter()
            .map(|s| crate::disk::LaneSquad {
                squad: s.clone(),
                projects: self.projects(s),
                open: open.iter().find(|(o, _)| o == s).map(|(_, t)| t.clone()).unwrap_or_default(),
            })
            .collect();
        let mut repos: Vec<PathBuf> = looked.iter().flat_map(|s| s.projects.iter().cloned()).collect();
        repos.sort();
        repos.dedup();
        let homes: Vec<PathBuf> = squads.iter().map(|s| squad_home::path(&home, s)).filter(|p| p.join(".git").exists()).collect();
        // Each squad's design QA pictures (`<squad>/design-qa`), with the tickets they may still serve.
        let design_qa: Vec<(PathBuf, Vec<TicketId>)> =
            open.iter().map(|(squad, tickets)| (crate::journal::squad_dir(&home, squad).join("design-qa"), tickets.clone())).collect();
        // Each squad's pictures kept past their worktrees (`visuals::keep`): those
        // of the tickets that keep them and of the waiting decisions, kept now
        // where they weren't yet (a hand-in from before equipe kept them).
        let days = crate::visuals::keep::days();
        let now = now_secs();
        // By squad: its tickets' pictures, by ticket, and its waiting decisions'.
        type Pictures = (String, Vec<(TicketId, Vec<String>)>, Vec<String>);
        let pictures: Vec<Pictures> = self
            .journals
            .iter()
            .map(|(squad, j)| {
                let tickets = j.tickets().unwrap_or_default().into_iter().filter(|t| crate::visuals::keep::stays(t.status.is_closed(), t.updated_at, now, days)).map(|t| (t.id, t.images)).collect();
                let waiting = j.questions().iter().flat_map(|q| q.images().map(str::to_owned).collect::<Vec<_>>()).collect();
                (squad.clone(), tickets, waiting)
            })
            .collect();
        let internal = self.internal.clone();
        std::thread::spawn(move || {
            for (squad, tickets, waiting) in &pictures {
                let kept: usize = tickets.iter().map(|(id, images)| crate::visuals::keep::ticket(&home, squad, *id, images)).sum::<usize>() + crate::visuals::keep::decision(&home, squad, waiting);
                if kept > 0 {
                    crate::daemon_log!("equipe: {squad}: kept {kept} pictures saved in worktrees or temp folders");
                }
                let ids: Vec<TicketId> = tickets.iter().map(|(id, _)| *id).collect();
                for line in crate::visuals::keep::sweep(&home, squad, &ids, waiting, days) {
                    crate::daemon_log!("equipe: {line}");
                }
            }
            for line in crate::disk::tidy_lane_tmp(&home, &open, &keep) {
                crate::daemon_log!("equipe: {line}");
            }
            // A copy of a project in a ticket's or a desk's temporary folder (a
            // `cp -R`, a clone): its member is told to use a worktree, and while
            // the disk is low an untouched one goes. A folder past its budget is told.
            let look = crate::disk::look_at_lane_tmp(&home, &looked, &keep);
            for line in &look.log {
                crate::daemon_log!("equipe: {line}");
            }
            let _ = internal.send(Internal::LaneTmpLooked(Box::new(look)));
            // Packs git left unfinished, in the projects and in the squads' own folders.
            let all: Vec<PathBuf> = repos.iter().chain(&homes).cloned().collect();
            for line in crate::disk::clear_git_garbage(&all) {
                crate::daemon_log!("equipe: {line}");
            }
            // Design captures — and in a squad's folder, what its tasks hand over — out
            // of git (an upgrade's one commit on main), and the store kept small.
            for repo in &all {
                for scope in crate::design::captures::SCOPES {
                    match crate::design::captures::migrate_in(scope, repo) {
                        Ok(Some(m)) => crate::daemon_log!(
                            "equipe: {} stopped tracking {} {} ({}) in {}",
                            repo.display(),
                            m.untracked,
                            scope.what,
                            crate::design::captures::size_words(m.bytes),
                            m.commit
                        ),
                        Ok(None) => {}
                        Err(err) => crate::daemon_log!("equipe: {} still tracks {}: {err}", repo.display(), scope.what),
                    }
                }
                for line in crate::design::captures::gc(repo) {
                    crate::daemon_log!("equipe: {line}");
                }
            }
            // A squad's folder never runs git's own gc: repacked here, only while there is room.
            for home in &homes {
                if let Some(line) = crate::disk::repack_if_room(home) {
                    crate::daemon_log!("equipe: {line}");
                }
            }
            for (dir, open) in &design_qa {
                for line in crate::disk::sweep_design_qa(dir, open, crate::disk::DESIGN_QA_KEPT) {
                    crate::daemon_log!("equipe: {line}");
                }
            }
        });
    }

    /// What a look at the lanes' temporary folders found, told: a copy of a
    /// project to the member whose folder it is in (the ticket's owner, or
    /// the desk's member), once; a ticket's folder past its budget to its
    /// owner and the PM, once. Logged.
    fn lane_tmp_looked(&mut self, look: crate::disk::LaneTmpLook) {
        use crate::disk::LaneTmp;
        for copy in look.copies {
            if !self.tmp_copies_told.insert(copy.path.clone()) && !copy.removed {
                continue;
            }
            let (path, project) = (copy.path.display().to_string(), copy.project.display().to_string());
            let (line, shown) = if copy.removed {
                (
                    format!("equipe removed {path}, a copy of {project} in your temporary folder that stood untouched while the disk was low. A copy is gigabytes; to build, test or deploy the project outside your worktree, check it out with `git -C {project} worktree add --detach \"$TMPDIR/<name>\" <ref>` (run outside the fence), and remove it with `git -C {project} worktree remove --force <path>` when done — never `cp -R` or clone it."),
                    say!(self, "news.tmp_copy_removed", path = path),
                )
            } else {
                (
                    format!("{path} in your temporary folder is a copy of {project}: gigabytes the disk can't spare. Switch to a worktree, which shares the repository — `git -C {project} worktree add --detach \"$TMPDIR/<name>\" <ref>` (run outside the fence) — remove the copy, and never `cp -R` or clone a project again. While the disk is low, a copy left untouched for half an hour is removed."),
                    say!(self, "news.tmp_copy", path = path),
                )
            };
            let removed = if copy.removed { ", removed" } else { "" };
            crate::daemon_log!("equipe: {path} is a copy of {project} in {}'s temporary folder{removed}", copy.squad);
            self.brief_lane(&copy.squad, &copy.lane, &line, &shown);
        }
        for over in look.over_budget {
            if !self.tmp_budget_told.insert((over.squad.clone(), over.ticket)) {
                continue;
            }
            let (ticket, size, budget) = (over.ticket, crate::disk::size(over.bytes), crate::disk::size(crate::disk::TICKET_TMP_BUDGET));
            let dir = journal::lane_tmp_dir(&self.config.home, &over.squad, Some(ticket), "");
            crate::daemon_log!("equipe: T-{ticket}'s temporary folder in {} holds {size}, past its {budget}", over.squad);
            let line = format!(
                "T-{ticket}'s temporary folder ({}) holds {size}, past the {budget} a ticket's should: remove what it no longer needs — checkouts with `git -C <project> worktree remove --force <path>`, builds and installs it can make again — and keep at most one checkout of a project per ticket.",
                dir.display()
            );
            let shown = say!(self, "news.tmp_over_budget", id = ticket, size = size);
            if !self.brief_lane(&over.squad, &LaneTmp::Ticket(ticket), &line, &shown) {
                self.brief_pm(&over.squad, line, Some(shown));
            }
        }
    }

    /// Tell whoever a lane's temporary folder is: the member working its
    /// ticket now, else the ticket's owner; a desk's member. A line in its
    /// conversation and words with its next message, as [`Self::brief_member`]
    /// tells. Whether the one told was the PM.
    fn brief_lane(&mut self, squad: &str, lane: &crate::disk::LaneTmp, line: &str, shown: &str) -> bool {
        use crate::disk::LaneTmp;
        let (working, named) = match lane {
            LaneTmp::Ticket(id) => (self.worker_on(squad, *id), self.tickets(squad).iter().find(|t| t.id == *id).and_then(|t| t.owner.clone().or_else(|| t.assignee.clone()))),
            LaneTmp::Desk(name) => (None, Some(name.clone())),
        };
        match working.or_else(|| named.as_deref().and_then(|name| self.member_id(squad, name))) {
            Some(id) if self.pm_of(squad) == Some(id) => {
                self.brief_pm(squad, line.to_owned(), Some(shown.to_owned()));
                true
            }
            Some(id) => {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.briefing.push(line.to_owned());
                }
                self.save_waiting(id);
                self.publish(Event::Notice { undo: None, session: id, text: shown.to_owned() });
                false
            }
            None => {
                if let Some(name) = named {
                    self.brief_member(squad, &name, line, shown);
                }
                false
            }
        }
    }

    /// Prune conversations past the retention window in every journal.
    fn prune(&mut self) {
        let cutoff = now_secs() as i64 - (self.config.retention_days as i64) * 86_400;
        for journal in self.journals.values() {
            let _ = journal.prune(cutoff);
        }
        self.tidy_lane_tmp();
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
        // What checks that never finished left (the daemon stopped mid-run): only while none run.
        let checks_left = self.gates.is_empty().then(|| gate::checks_scratch(&home));
        std::thread::spawn(move || {
            for line in worktree::tidy(&home, &in_use, &kept) {
                crate::daemon_log!("equipe: {line}");
            }
            for line in checks_left.map(|dir| crate::checks::sweep(&dir)).unwrap_or_default() {
                crate::daemon_log!("equipe: {line}");
            }
            // Deleted squads kept past their week.
            for line in crate::squads::purge_deleted(&home, now_secs()) {
                crate::daemon_log!("equipe: {line}");
            }
        });
    }
}

/// A member's ticket as it was when the daemon stopped. A ticket whose worktree
/// was removed in the meantime goes back on the board instead.
/// Where in a worktree `project` is: the worktree itself for a repository,
/// the same folder in it for a folder of a repository.
/// Marks a squad whose projects' repositories were looked through once for
/// the briefs and plans equipe used to commit into them.
const DOCS_KEPT: &str = ".docs-kept";

/// Once per squad, off the loop: the briefs and plans equipe once committed
/// into its `projects`' repositories copied into its folder `home`, where
/// they are kept now, so the PM's history stays. The repositories are only
/// read; a copy the person later deletes isn't brought back.
fn keep_docs_from_repos(squad_dir: &Path, home: &Path, projects: &[PathBuf]) {
    let mark = squad_dir.join(DOCS_KEPT);
    if mark.exists() || projects.is_empty() {
        return;
    }
    let (home, projects) = (home.to_owned(), projects.to_vec());
    std::thread::spawn(move || match crate::briefs::keep_from_repos(&home, &projects) {
        Ok(copied) => {
            if !copied.is_empty() {
                crate::daemon_log!("equipe: kept {} briefs and plans from the projects' repositories in {}", copied.len(), home.display());
            }
            let _ = std::fs::write(&mark, format!("{}\n", copied.len()));
        }
        Err(why) => crate::daemon_log!("equipe: the briefs and plans in the projects' repositories weren't kept in {}: {why}", home.display()),
    });
}

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
        // Bound to a new run by the restart that brings it back.
        execution_session: 0,
        execution_generation: 0,
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
        waive: None,
        lite: false,
        refused: None,
        preparing: false,
        end: RunEnd::default(),
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
    recovered: bool,
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
            if worktree::is_sparse(&sandbox.path) {
                out.push_str(&format!(
                    " Other tickets' outputs aren't in this copy: read them in the squad's folder, {}.",
                    sandbox.repo.join("outputs").display()
                ));
            }
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
    if let Some(note) = reopened.filter(|n| n.starts_with(crate::flow::BATON_FROM)) {
        // Passed on by the baton: the next holder carries on where it is.
        let holder = ticket.assignee.as_deref().or(ticket.owner.as_deref()).unwrap_or_default();
        out.push_str(&crate::flow::picked_up(note, ticket.flow.as_deref(), holder));
    } else if let Some(note) = reopened.and_then(|n| n.strip_prefix("Resumed: ")) {
        out.push_str(&format!(
            "You put this ticket down to wait for the person; it is back. {note}\nCarry on from where you left off — \
check what is already done in your worktree first.\n"
        ));
        if crate::protocol::asks::read(note).is_some_and(|r| r.whole) {
            out.push_str(crate::roles::ANSWERS_GUIDE);
            out.push('\n');
        }
    } else if let Some(note) = reopened.and_then(|n| n.strip_prefix(board::WAITED_ON)) {
        out.push_str(&format!(
            "You handed this ticket back to wait on {note}\nCarry on from where you left off — check what is already done in \
your worktree first, and what those tickets changed.\n"
        ));
    } else if let Some(note) = reopened.filter(|n| n.starts_with("Design QA by ")) {
        out.push_str(&format!(
            "The squad's designer checked its screens and sent it back. {note}\nFix each difference it lists in your \
worktree, then hand in again with fresh screenshots of the screens you changed.\n"
        ));
    } else if let Some(note) = reopened.and_then(|n| n.strip_prefix(crate::accept::PM_SENT_BACK)) {
        out.push_str(&format!(
            "The PM reviewed your hand-in and sent it back: {note}\nFix each point in your worktree, on the same branch, \
then hand in again.\n"
        ));
    } else if let Some(note) = reopened {
        out.push_str(&format!(
            "The person sent it back. {note}\nWhat they asked for says something about how they want work done: save that \
lesson in your memory before you start, so the next ticket begins with it.\n"
        ));
    }
    if reopened.is_some_and(|n| !n.starts_with(crate::flow::BATON_FROM)) {
        out.push_str("This latest reopen state supersedes any older message saying the work was complete or handed in.\n");
    }
    // A ticket carrying the person's request: its driver, checklist and baton (`flow`).
    if let Some(flow) = ticket.flow.as_deref().filter(|f| board_tools && (f.driver.is_some() || !f.steps.is_empty())) {
        let holder = ticket.assignee.as_deref().or(ticket.owner.as_deref()).unwrap_or_default();
        out.push_str(&crate::flow::brief(flow, holder, ticket.id));
        out.push('\n');
    }
    if !ticket.remaining.is_empty() {
        out.push_str("The latest unresolved blockers take priority:\n");
        for blocker in &ticket.remaining {
            out.push_str(&format!("- {blocker}\n"));
        }
    }
    if recovered {
        // Brought back by equipe on its own, once, or by the person.
        let by_itself = ticket.execution.as_ref().is_some_and(|e| e.operation.starts_with(journal::AUTO_RESUME_OPERATION));
        out.push_str(if by_itself {
            "The last run on this ticket stopped without a hand-in, and equipe brought it back once by itself: this is a fresh \
session on the same ticket."
        } else {
            "The last run on this ticket stopped without a hand-in, and the person brought it back: this is a fresh session on the same ticket."
        });
        if sandbox.is_some() {
            out.push_str(
                " Same branch and worktree, nothing reset, committed or cleaned — the commit and the uncommitted and untracked files \
are as the last run left them. Look at `git status` and what is already done first, then carry on from there.",
            );
        }
        out.push('\n');
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
of questions in your reply. If it can't start until other tickets are done, duplicates another, is another role's \
work, or can't be done as asked, call hand_back with that reason and a note, and end your turn. Every turn on this \
ticket ends in exactly one of these ways, or goes on working: a turn that ends otherwise leaves it unfinished."
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
fn handed_back(why: &str, asked: bool, page: Option<&equipe_protocol::BrowserPage>) -> String {
    let why = why.trim();
    let what = match (asked, why.is_empty()) {
        (_, true) => String::new(),
        (true, false) => format!(" (you asked: {why})"),
        (false, false) => format!(" ({why})"),
    };
    let at = page.map_or_else(String::new, |p| {
        let title = p.title.trim();
        if title.is_empty() { format!(" The page is at {}.", p.url) } else { format!(" The page is at {} (title: \"{title}\").", p.url) }
    });
    // A page that still waits on a person: said, not tried again by itself.
    let still = if page.is_some_and(|p| p.check.is_some()) {
        " The page still shows a human check or a sign-in: tell the person plainly that it does (a site may not carry a sign-in over \
to another browser), and don't try it again by yourself."
    } else {
        ""
    };
    format!(
        "[equipe: the person has finished{what} and pressed Done: your browser is yours again.{at}{still} Check the page yourself now \
(take a snapshot or read the page and its URL) and carry on where you left off. Don't ask the person what they saw or whether \
it worked — they did their part; you can see the result.]"
    )
}

/// The tab a member's tools last worked in, else the first, of a lane's tabs as last told.
fn current_page(tabs: &Event) -> Option<equipe_protocol::BrowserPage> {
    let Event::BrowserTabs { pages, agent_page, .. } = tabs else { return None };
    agent_page.and_then(|id| pages.iter().find(|p| p.id == id)).or(pages.first()).cloned()
}

/// How many of the last messages a fresh conversation is handed.
const REFRESH_TAIL: usize = 12;

/// `text` cut to about `max` characters, on a character boundary.
/// The line a PM's message carries in place of the list of what waits on
/// the person when that list is as it last heard it: how many, and how many
/// went stale — the counts it acts on — read off the note
/// ([`importance::pm_note`]).
fn waiting_still(note: &str) -> String {
    let number_before = |marker: &str| note.find(marker).and_then(|at| note[..at].rsplit(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse::<usize>().ok());
    let all = number_before(" in all)").unwrap_or(0);
    let stale = match number_before(" of them waited") {
        Some(n) if n > 0 => format!(", {n} of them STALE"),
        _ => String::new(),
    };
    format!("{all} things wait on the person{stale} — the list you were given earlier in this conversation still stands; lead with its most blocking when you remind them.")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// How often lanes' temporary folders of tickets that are not open are tidied.
const LANE_TMP_TIDY_EVERY: Duration = Duration::from_secs(60 * 60);
/// … and while the disk is short of room.
const LANE_TMP_TIDY_EVERY_SHORT: Duration = Duration::from_secs(10 * 60);

/// The most a note made of a command's output keeps.
const OUTPUT_NOTE_MAX: usize = 4096;

/// A command's `output` for a note: as it is when it is short, else its first
/// lines, how many were left out, and its last ones. A failing `git worktree
/// add` can print a line for each of thousands of files.
fn output_excerpt(output: &str, max: usize) -> String {
    const HEAD: usize = 5;
    const TAIL: usize = 2;
    const LINE: usize = 400;
    if output.len() <= max {
        return output.to_owned();
    }
    let lines: Vec<&str> = output.lines().collect();
    let mut kept = Vec::new();
    if lines.len() > HEAD + TAIL {
        kept.extend(lines[..HEAD].iter().map(|l| clip(l, LINE)));
        kept.push(format!("[{} lines left out]", lines.len() - HEAD - TAIL));
        kept.extend(lines[lines.len() - TAIL..].iter().map(|l| clip(l, LINE)));
    } else {
        kept.extend(lines.iter().map(|l| clip(l, LINE)));
    }
    let text = kept.join("\n");
    if text.len() <= max {
        return text;
    }
    // A few very long lines: cut on a character boundary.
    let mut at = max;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}…", &text[..at])
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

/// How many times a landing that failed only for a moment — another git
/// holding the main checkout's lock, main moved on — is tried again at once,
/// and how long apart: in seconds, not at the next retry minutes away.
const LANDING_TRIES: u32 = 6;
const LANDING_AGAIN_AFTER: Duration = Duration::from_secs(10);

/// `land` (a landing, merged and pushed), tried again after a moment while
/// it fails for a passing cause ([`worktree::passing`]) — the ticket stays
/// "landing" meanwhile. What still fails is for someone to see to: the
/// retries minutes apart and the PM take it from there. Blocking.
pub(super) fn landing_tries(mut land: impl FnMut() -> MergeResult) -> MergeResult {
    let mut tries = 1;
    loop {
        match land() {
            Err(err) if worktree::passing(&err) && tries < LANDING_TRIES => {
                crate::daemon_log!("equipe: a landing waits a moment and goes again: {err}");
                std::thread::sleep(LANDING_AGAIN_AFTER);
                tries += 1;
            }
            landed => return landed,
        }
    }
}

/// The message a task's or action's work lands in the squad's folder with.
fn home_merge_message(ticket: &Ticket) -> String {
    format!("{}: T-{} {}", ticket.assignee.as_deref().unwrap_or("equipe"), ticket.id, ticket.title)
}

/// How a task's or action's work landed in the squad home's main: the
/// worktree it came from (its pictures are shown from the folder they landed
/// in), and what landing kept out of git — or why nothing merged.
pub(super) struct HomeLanded {
    from: PathBuf,
    result: Result<Option<crate::design::captures::Report>, String>,
}

/// The git side of landing a task's work in the squad home's main: the
/// person's own changes there saved first (`save_persons`), then the branch
/// merges and its worktree and branch go — all in line with the folder's
/// other landings and saves. A conflict merges nothing and keeps both.
/// Blocking, and waiting its turn: never on the loop.
fn land_home(sandbox: &Sandbox, message: &str, save_persons: bool) -> HomeLanded {
    // The worktree goes once merged: read where it is first.
    let from = sandbox.path.canonicalize().unwrap_or_else(|_| sandbox.path.clone());
    let result = worktree::one_landing_at_a_time(&sandbox.repo, || {
        if save_persons {
            let _ = squad_home::save_persons(&sandbox.repo);
        }
        worktree::merge_into_main(&sandbox.repo, &sandbox.branch, message)?;
        Ok(crate::design::captures::take_report(&sandbox.repo, &sandbox.branch))
    });
    if result.is_ok() {
        let _ = sandbox.discard();
    }
    HomeLanded { from, result }
}

/// A ticket put down whose work went into the squad's folder off the loop
/// ([`Runtime::land_put_down`]): what putting it down goes on with.
pub(super) struct HomePutDown {
    id: SessionId,
    how: WorkEnd,
    work: Work,
    squad: String,
    name: String,
    /// Its worktree's commit, or why it wasn't made.
    committed: Result<(), String>,
    landed: HomeLanded,
}

/// A ticket's closing git work ([`Runtime::close_git`]).
pub(super) struct CloseGit {
    workplace: PathBuf,
    path: PathBuf,
    branch: String,
    base: String,
    /// A task's or action's work accepted: committed and landed in the squad's folder.
    home: Option<HomeClose>,
}

struct HomeClose {
    commit: String,
    merge: String,
    save_persons: bool,
}

/// How a ticket's closing git work went.
pub(super) enum Closed {
    /// Its worktree went; its branch stays when it has commits of its own.
    Released(Option<String>),
    /// A task's or action's work landed in the squad's folder, or didn't.
    Home(HomeLanded),
}

impl CloseGit {
    /// Whether it lands a task's work in the squad's folder — in line with
    /// the folder's other landings, so never on the loop.
    pub(super) fn lands(&self) -> bool {
        self.home.is_some()
    }

    /// Do it: blocking, and fine off the loop. `None` when its workplace is
    /// no git repository any more.
    pub(super) fn run(self) -> Option<Closed> {
        let repo = worktree::repo_root(&self.workplace).ok()?;
        let sandbox = Sandbox { repo, path: self.path, branch: self.branch, base: self.base };
        Some(match self.home {
            Some(home) => {
                let _ = sandbox.commit_all(&home.commit);
                Closed::Home(land_home(&sandbox, &home.merge, home.save_persons))
            }
            None => Closed::Released(release(&sandbox)),
        })
    }
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

/// Whether a reply has a Mermaid block or an HTML card in it.
fn has_visuals(text: &str) -> bool {
    crate::visuals::diagram::has_blocks(text) || crate::visuals::card::has_blocks(text)
}

/// The keys of the blocks in these lines' replies, Mermaid and cards.
fn visual_keys(lines: &[Entry]) -> impl Iterator<Item = String> + '_ {
    lines.iter().filter_map(|line| match line {
        Entry::Reply { text } if has_visuals(text) => Some(text),
        _ => None,
    }).flat_map(|text| crate::visuals::diagram::blocks(text).into_iter().chain(crate::visuals::card::blocks(text))).map(|source| crate::visuals::diagram::key(&source))
}

/// The journal of `squad`, opened on first use.
fn journal_for<'a>(journals: &'a mut HashMap<String, Journal>, home: &Path, squad: &str) -> Option<&'a Journal> {
    if !journals.contains_key(squad) {
        // Opening a deleted squad's journal would make the squad anew.
        if crate::squads::is_gone(home, squad) {
            return None;
        }
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
        ran: m.ran.clone(),
        access: m.access,
        cwd: m.cwd.clone(),
        sandbox: None,
        codex: m.codex.clone(),
        history: thread.history,
        claude_session: thread.claude_session,
        work: None,
        prompts: Vec::new(),
        title: None,
        title_started: false,
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
        stop_subagent: None,
        work_over: None,
        wind_down: None,
        taken_back: Vec::new(),
        send_now_prompt: None,
                told: Default::default(),
        subagents: Vec::new(),
        compacting: None,
        answers_person: false,
        on_phone: false,
        effort: None,
        briefing: Vec::new(),
        memory: m.memory.clone(),
        mode: m.mode,
        thread: project.clone(),
        threads: HashMap::new(),
        estimate: None,
        turn_began: None,
        watch: TurnWatch::default(),
        lane: Some((main, project)),
        kept,
        run_of: None,
        slot: 0,
    }
}

/// `false` when the disk had no room for it. Nothing else stops for a write
/// that failed: the screens still get the event.
fn record(journal: &Journal, id: SessionId, event: &Event, finished: &[Entry]) -> bool {
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
        Event::UserMessage { .. } => {}
        // What the member is set to, so an alias stays one; what runs when nothing was asked.
        Event::ModelKnown { model, asked, .. } => saw(journal.set_model(id, asked.as_deref().unwrap_or(model))),
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
fn first_exchange(entries: &[Entry]) -> Option<(String, String)> {
    let (at, user) = entries.iter().enumerate().find_map(|(at, entry)| match entry {
        Entry::User { text, .. } if !text.trim().is_empty() && !harness::only_harness(text) => Some((at, text.clone())),
        _ => None,
    })?;
    let reply = entries[at + 1..]
        .iter()
        .take_while(|entry| !matches!(entry, Entry::User { .. }))
        .find_map(|entry| match entry {
            Entry::Reply { text } if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })?;
    Some((user, reply))
}

fn suggestion_context(member: &str, entries: &[Entry]) -> Option<String> {
    let said: Vec<(bool, &str)> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::User { text, .. } if !harness::only_harness(text) => Some((true, text.as_str())),
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
                cache_read_tokens: u.cache_read_tokens,
                cache_write_tokens: u.cache_write_tokens,
                cache_write_long_tokens: u.cache_write_long_tokens,
                calls: u.calls,
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

/// What a member is told when the person answered what it asked (`action`)
/// only after the turn that asked had ended: their answer (`said`), and — on a
/// ticket — to carry on and end the turn in one of the ways to finish.
fn late_answer_prompt(action: &str, said: &str, on_ticket: bool) -> String {
    let mut prompt = format!(
        "[equipe] Your last turn ended before the person answered what you asked them (\u{201c}{action}\u{201d}): the call \
stopped waiting for them. They have answered now: {said}\n"
    );
    prompt.push_str(if on_ticket {
        "Carry on from where you left off in light of their answer — look at where things stand first, and don't redo what \
is done. Then end the turn in one of the ways to finish: `submit_for_review` when it's done, `block_on_person` when only \
the person can do what's left, or `hand_back`."
    } else {
        "Carry on from where you left off in light of their answer."
    });
    prompt
}

/// Whether a session's engine runs an agent with tools: Claude Code, or the
/// person's Codex CLI — not equipe's own Codex sign-in, which only answers.
fn has_tools(session: &Session) -> bool {
    session.engine == Engine::Claude || session.codex.is_none()
}

/// The environment variable Claude Code reads its cap on subagents running at once from.
const CLAUDE_SUBAGENTS_ENV: &str = "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS";

/// The subagents a session's engine runs, `set` at a time (`None`: as many
/// as it runs on its own): none for Codex on Token Factory, whose turns go
/// to another provider.
fn subagents_of(session: &Session, set: Option<u32>) -> Option<roles::Subagents> {
    let other_provider = session.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
    roles::Subagents::of(session.engine, has_tools(session), other_provider, set)
}

/// A subagent at work for a member: what it was handed, its latest step,
/// since when, and its latest steps and words, kept for a screen that
/// attaches while it works.
struct SubRun {
    id: String,
    what: String,
    brief: String,
    step: Option<String>,
    since: std::time::Instant,
    entries: std::collections::VecDeque<Entry>,
    /// When it was last heard from: a step, a word, its engine telling of it.
    heard: std::time::Instant,
    /// Watched for going quiet ([`crate::subagent_watch`]): Codex's own, and
    /// equipe's helpers — those the person can stop alone.
    watched: bool,
    /// The screens were told it looks stuck, and not yet that it moves.
    stuck: bool,
}

impl SubRun {
    /// One more step or word: the oldest goes past [`SUBAGENT_KEPT`].
    fn keep(&mut self, entry: Entry) {
        if self.entries.len() == SUBAGENT_KEPT {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

/// How much of a subagent's tool output goes to the screens, from its end.
const SUBAGENT_OUTPUT: usize = 4000;
/// How many of a subagent's latest steps and words the daemon keeps for a
/// screen that attaches while it works.
const SUBAGENT_KEPT: usize = 40;

/// `text`'s last `most` characters.
fn tail_chars(text: &str, most: usize) -> String {
    let n = text.chars().count();
    if n <= most { text.to_owned() } else { text.chars().skip(n - most).collect() }
}

/// One step of an engine's plan, as the protocol has it.
fn plan_step(step: equipe_providers::turn::PlanStep) -> equipe_protocol::PlanStep {
    use equipe_providers::turn::StepState;
    let state = match step.state {
        StepState::Pending => equipe_protocol::PlanState::Pending,
        StepState::Active => equipe_protocol::PlanState::Active,
        StepState::Done => equipe_protocol::PlanState::Done,
    };
    equipe_protocol::PlanStep { text: step.text, state }
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
        Activity::PlanSteps(steps) => Event::Plan { session: id, steps: steps.into_iter().map(plan_step).collect() },
        Activity::Thinking(text) => Event::Thinking { session: id, text },
        Activity::Model(model) => Event::ModelKnown { session: id, model, asked: None },
        Activity::Output(tokens) => Event::Output { session: id, tokens },
        Activity::SubagentStarted { id: sub, what, brief } => Event::SubagentStarted { session: id, id: sub, what, brief },
        Activity::SubagentStep { id: sub, step } => Event::SubagentStep { session: id, id: sub, step },
        Activity::SubagentTool { id: sub, done } => {
            // A subagent's outcome travels lighter than the member's own: a look inside, not its record.
            let (output, skipped) = equipe_protocol::ToolOutcome::tail(&tail_chars(&done.output, SUBAGENT_OUTPUT));
            let outcome = equipe_protocol::ToolOutcome { ok: done.ok, exit: done.exit, ms: Some(done.ms), output, skipped, diff: done.diff };
            let entry = Entry::Tool { tool: done.name, target: done.target, outcome: Some(Box::new(outcome)), label: None, background: false };
            Event::SubagentEntry { session: id, id: sub, entry }
        }
        Activity::SubagentText { id: sub, text } => Event::SubagentEntry { session: id, id: sub, entry: Entry::Reply { text } },
        Activity::SubagentEnded { id: sub, result, failed } => Event::SubagentEnded { session: id, id: sub, result, failed },
        Activity::Steered(text) => Event::Steered { session: id, text, kept: Vec::new() },
        // Filled in on the loop, which knows who it is and how long the last ones took.
        Activity::Compacting => {
            Event::Compacting { session: id, member: None, project: None, trigger: CompactTrigger::Auto, started_at: 0, estimate_ms: 0 }
        }
        Activity::Compacted => Event::Compacted { session: id, member: None, project: None, trigger: CompactTrigger::Auto, took_ms: 0 },
        Activity::CompactFailed(message) => Event::CompactFailed { session: id, member: None, project: None, trigger: CompactTrigger::Auto, message },
        Activity::Plan { five_hour, seven_day } => Event::PlanUsage { usage: claude_plan(five_hour, seven_day), engine: Engine::Claude },
        Activity::Limits(_) => return None,
        // The loop's to know, never a screen's: sent apart by the turn.
        Activity::Engine { .. } | Activity::Spent(_) | Activity::SubagentHeard { .. } | Activity::Settled(_) | Activity::ModelRefused { .. } | Activity::RefusedRetried => return None,
    })
}

/// The Claude plan's windows, each as the share used (0–1) and when it
/// resets, as plan usage.
fn claude_plan(five_hour: Option<(f64, i64)>, seven_day: Option<(f64, i64)>) -> equipe_protocol::PlanUsage {
    let window = |w: Option<(f64, i64)>| {
        w.map(|(used, resets_at)| equipe_protocol::PlanWindow { used_percent: (used * 100.0).round().clamp(0.0, 100.0) as u8, resets_at })
    };
    equipe_protocol::PlanUsage { five_hour: window(five_hour), seven_day: window(seven_day) }
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
        equipe_providers::Error::CodexSignedOut => (crate::i18n::core(lang).get("error.codex_cli_signed_out").to_owned(), Some(crate::engine_auth::CODEX_HINT)),
        equipe_providers::Error::Login(_) => (codex_app::plain_message(&err.to_string()), Some("equipe status")),
        // A model the account refused: a line in the person's words, never the provider's JSON.
        equipe_providers::Error::Backend { message, code } if codex_app::refused_model(message, code.as_deref()).is_some() => {
            let model = codex_app::refused_model(message, code.as_deref()).filter(|m| !m.is_empty()).unwrap_or_else(|| "?".to_owned());
            (crate::i18n::core(lang).fill("error.codex_model_refused", &[("model", model)]), None)
        }
        _ => (codex_app::plain_message(&err.to_string()), None),
    }
}

/// A turn OpenAI's safety check refused as possibly unintended activity, in
/// the person's words: whether equipe asked it again in a fresh thread,
/// OpenAI's own explanation when it gave one (the line after its message),
/// and — when `claude` names the member and Claude could take it — the
/// command that moves it there, as the next step.
fn refused_words(lang: crate::i18n::Lang, message: &str, retried: bool, claude: Option<&str>) -> (String, Option<String>) {
    let words = crate::i18n::core(lang);
    let mut text = words.get(if retried { "error.codex_refused_again" } else { "error.codex_refused" }).to_owned();
    if let Some(why) = message.split_once('\n').map(|(_, why)| why.trim()).filter(|why| !why.is_empty()) {
        text.push_str(&format!(" \u{201c}{why}\u{201d}"));
    }
    match claude {
        Some(name) => {
            text.push(' ');
            text.push_str(&words.fill("error.codex_refused_claude", &[("name", name.to_owned())]));
            (text, Some(format!("/engine {name} claude now")))
        }
        None => (text, None),
    }
}

/// A member's turn on Token Factory, for the daemon's log: the model, each
/// request's status and Token Factory's id for it, the time, the tokens.
fn nebius_log(model: &str, turn: &codex_app::Turn) -> String {
    let requests = &turn.provider_requests;
    let statuses: Vec<String> = requests.iter().map(|r| r.status.to_string()).collect();
    let ids: Vec<&str> = requests.iter().filter_map(|r| r.request_id.as_deref()).collect();
    let ms: u64 = requests.iter().map(|r| r.ms).sum();
    let usage = turn.result.usage.as_ref();
    let tokens = |n: Option<u64>| n.map_or_else(|| "?".to_owned(), |n| n.to_string());
    format!(
        "equipe: turn on Nemotron via Nebius Token Factory: model {model}, {} request(s) [{}], request ids [{}], {ms} ms on requests, tokens in {} (cached {}) out {}, {}",
        requests.len(),
        statuses.join(" "),
        ids.join(" "),
        tokens(usage.and_then(|u| u.input_tokens)),
        tokens(usage.and_then(|u| u.cache_tokens)),
        tokens(usage.and_then(|u| u.output_tokens)),
        turn.result.status,
    )
}

/// A turn on Token Factory — or another provider, by its name — that failed:
/// a refused key is the provider's, not Codex's sign-in.
fn nebius_failure(err: equipe_providers::Error, provider: Option<&str>, lang: crate::i18n::Lang) -> (String, Option<&'static str>) {
    let words = crate::i18n::core(lang);
    let message = err.to_string();
    let lower = message.to_ascii_lowercase();
    let refused = matches!(err, equipe_providers::Error::CodexSignedOut) || lower.contains("401") || lower.contains("403") || lower.contains("unauthorized");
    match provider {
        Some(provider) if refused => return (words.fill("error.provider_unauthorized", &[("provider", provider.to_owned())]), None),
        None if refused => return (words.get("error.nebius_unauthorized").to_owned(), Some("https://tokenfactory.nebius.com")),
        _ => {}
    }
    codex_cli_failure(err, lang)
}

/// Of the models `usable`, the one a turn refused on `model` goes to — never
/// a dearer one ([`Runtime::codex_fallback`]).
fn codex_fallback_of<'a>(model: &str, usable: &[&'a codex_app::ModelInfo]) -> Option<&'a codex_app::ModelInfo> {
    const FAMILIES: [&str; 3] = ["luna", "sol", "astra"];
    let ids = |keep: &dyn Fn(Option<u8>) -> bool| -> Vec<String> { usable.iter().filter(|m| keep(crate::tiers::model_rank(Engine::Codex, &m.id))).map(|m| m.id.clone()).collect() };
    let find = |id: Option<String>| id.and_then(|id| usable.iter().find(|m| m.id == id).copied());
    match crate::tiers::model_rank(Engine::Codex, model) {
        Some(rank) => {
            let cheaper: Vec<&str> = FAMILIES[..usize::from(rank)].iter().rev().copied().collect();
            find(roles::codex_family(&ids(&|r| r.is_some_and(|r| r < rank)), &cheaper)).or_else(|| find(roles::codex_family(&ids(&|r| r == Some(rank)), &[FAMILIES[usize::from(rank)]])))
        }
        None => {
            let top = FAMILIES.len() as u8 - 1;
            usable.iter().copied().filter(|m| crate::tiers::model_rank(Engine::Codex, &m.id) != Some(top)).find(|m| m.default).or_else(|| usable.iter().copied().find(|m| crate::tiers::model_rank(Engine::Codex, &m.id).is_some_and(|r| r < top)))
        }
    }
}

/// An engine whose lost sign-in stopped turns here, and its fix.
#[derive(Debug)]
struct AuthFix {
    /// Why it can't serve members, as last looked at.
    state: AuthState,
    detail: String,
    /// The turns to carry on once it is signed in again.
    stopped: Vec<crate::engine_auth::Stopped>,
    /// A look under way.
    checking: bool,
    /// When stopped turns were last carried on (unix seconds): failing again
    /// soon after, the sign-in its tool lists doesn't work.
    carried_on_at: Option<u64>,
    /// Since when turns have waited (unix seconds): looked at again for [`AUTH_WATCH`].
    since: Option<u64>,
    /// equipe started the engine's own sign-in itself for this sign-out:
    /// once, for every member and lane it stopped; again only when the
    /// person asks, or after it signed in and was lost again.
    auto_started: bool,
}

impl Default for AuthFix {
    fn default() -> Self {
        Self { state: AuthState::Checking, detail: String::new(), stopped: Vec::new(), checking: false, carried_on_at: None, since: None, auto_started: false }
    }
}

impl AuthFix {
    fn event(&self, engine: Engine) -> Event {
        Event::EngineAuth { engine, state: self.state, detail: self.detail.clone(), stopped: self.stopped.iter().map(|s| s.session).collect() }
    }
}

/// While turns wait on a sign-in, it is looked at again this often…
const AUTH_RECHECK: Duration = Duration::from_secs(15);
/// …for this long.
const AUTH_WATCH: Duration = Duration::from_secs(2 * 60 * 60);
/// Turns carried on that fail again within this long (seconds) mean the sign-in doesn't work.
const CARRIED_ON_FOR: u64 = 15 * 60;

/// An engine's name, as the person knows it.
fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude Code",
        Engine::Codex => "Codex",
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
        // The sign-in Claude Code reads no longer works here: expired, kept
        // in a Keychain that is locked, or out of reach of a daemon started
        // over SSH. equipe looks at which (`engine_auth`); the hint is what an
        // older screen shows.
        Some(crate::engine_auth::CLAUDE_HINT)
    } else if lower.contains("login") || lower.contains("logged in") {
        Some("equipe login claude")
    } else if refuses_the_model(&lower) {
        // A pinned model this Claude Code is older than.
        Some("claude update")
    } else {
        None
    };
    (message, next)
}

/// A Claude Code turn's failure on an Anthropic-compatible provider's
/// endpoint: a refused key is said as the provider's, and nothing points the
/// person at Claude's own sign-in, which the turn never used.
fn provider_claude_failure(err: equipe_providers::Error, provider: &str, lang: crate::i18n::Lang) -> (String, Option<&'static str>) {
    let lower = err.to_string().to_ascii_lowercase();
    let refused = ["401", "403", "unauthorized", "invalid x-api-key", "invalid api key", "authentication_error", "failed to authenticate"].iter().any(|w| lower.contains(w));
    if refused {
        return (crate::i18n::core(lang).fill("error.provider_unauthorized", &[("provider", provider.to_owned())]), None);
    }
    (claude_failure(err).0, None)
}

/// Whether an engine's error says it can't run the model asked for (Claude
/// Code: "There's an issue with the selected model … It may not exist").
fn refuses_the_model(lower: &str) -> bool {
    lower.contains("model")
        && ["selected model", "not exist", "not found", "not_found", "invalid model", "unknown model", "not available", "not supported"].iter().any(|w| lower.contains(w))
}

/// A model the Codex CLI listed, as screens are told of it.
fn model_choice(m: &codex_app::ModelInfo) -> ModelChoice {
    ModelChoice { id: m.id.clone(), name: m.name.clone(), efforts: m.efforts.clone(), default_effort: m.default_effort.clone(), description: m.description.clone(), default: m.default }
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

/// The effort a member's turn runs at: the person's choice, else its role's
/// ([`roles::default_effort`]), else its engine's default.
fn effort_of(session: &Session) -> Option<String> {
    session.effort.clone().or_else(|| session.member.as_ref().and_then(|m| roles::default_effort(&m.role)).map(str::to_owned))
}

/// How big a member's build cache may grow before it is trimmed; how big all
/// of them together, every squad's — each member building on its own made it
/// members × the cap, and filled the disk within the hour; and how often
/// that is looked at.
const BUILD_CAP: u64 = 10 * 1024 * 1024 * 1024;
const BUILDS_BUDGET: u64 = 20 * 1024 * 1024 * 1024;
const BUILDS_TRIM_EVERY: Duration = Duration::from_secs(5 * 60);

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
/// It types only on the login's site (or a subdomain of it), checked in the
/// page as it types. The password is fetched off the loop and goes nowhere
/// but the page.
async fn sign_in_page(client: &reqwest::Client, port: u16, vault: &std::sync::Arc<std::sync::Mutex<vault::Vault>>, login: &crate::protocol::vault::Login, start: &str) -> Result<&'static str, SignIn> {
    let text_of = |items: Option<Vec<Value>>| -> String { items.unwrap_or_default().iter().filter_map(|i| i.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n") };
    let here = text_of(browser::call(client, port, "browser_evaluate", json!({ "function": "() => location.href" })).await);
    if !here.lines().any(|l| vault::serves(&login.site, l.trim().trim_matches('"'))) {
        browser::navigate(client, port, start).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    let (id, vault) = (login.id, vault.clone());
    let password = tokio::task::spawn_blocking(move || vault.lock().map_err(|_| "the logins are unreadable".to_owned()).and_then(|v| v.password(id)))
        .await
        .map_err(|_| SignIn::Failed("stopped".into()))?
        .map_err(|why| SignIn::Failed(format!("the password couldn't be read from the secret store ({why})")))?;
    let guard = vault::FillGuard::serving(login);
    let mut last = "";
    for _ in 0..2 {
        let answer = vault::fill_answer(&text_of(browser::call(client, port, "browser_evaluate", json!({ "function": vault::fill_script(&login.username, &password, &guard) })).await));
        if let Some(refused) = answer.as_ref().and_then(|a| a.refused.as_deref()) {
            return Err(SignIn::Failed(format!("the page isn't on {} (or isn't secure: {refused}), so nothing was typed", login.site)));
        }
        let step = answer.as_ref().filter(|a| !a.filled.is_empty()).and_then(|a| a.step.as_deref()).unwrap_or_default();
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
    fn a_refused_codex_model_falls_back_to_a_cheaper_one_never_a_dearer_one() {
        let info = |id: &str, default: bool| codex_app::ModelInfo { id: id.into(), default, ..Default::default() };
        let listed = [info("gpt-6.1-sol", true), info("gpt-6-astra", false), info("gpt-6.1-terra", false), info("gpt-6-sol", false), info("gpt-6-luna", false), info("gpt-5.6-luna", false)];
        let usable = |refused: &str| -> Vec<&codex_app::ModelInfo> { listed.iter().filter(|m| m.id != refused).collect() };
        let pick = |refused: &str| codex_fallback_of(refused, &usable(refused)).map(|m| m.id.clone());
        assert_eq!(pick("gpt-6.1-sol").as_deref(), Some("gpt-6-luna"), "Sol refused: the newest Luna, never Astra");
        assert_eq!(pick("gpt-6-astra").as_deref(), Some("gpt-6.1-sol"), "Astra refused: the strongest cheaper family");
        let no_luna: Vec<&codex_app::ModelInfo> = listed.iter().filter(|m| !m.id.contains("luna") && m.id != "gpt-6.1-sol").collect();
        assert_eq!(codex_fallback_of("gpt-6.1-sol", &no_luna).map(|m| m.id.as_str()), Some("gpt-6-sol"), "no cheaper one: the same family's other version");
        let only_astra = [&listed[1]];
        assert!(codex_fallback_of("gpt-6.1-sol", &only_astra).is_none(), "never up to a dearer model");
        assert_eq!(pick("gpt-6.1-terra").as_deref(), Some("gpt-6.1-sol"), "no known family: the account's default, unless it is the dearest");
    }

    #[test]
    fn a_browser_handed_back_says_the_person_finished_where_the_page_is_and_to_look_itself() {
        let page = equipe_protocol::BrowserPage { id: 3, url: "https://app.example.com/dashboard".into(), title: "Dashboard".into(), ..Default::default() };
        let text = handed_back("sign in to the app", true, Some(&page));
        for wanted in ["the person has finished", "(you asked: sign in to the app)", "pressed Done", "https://app.example.com/dashboard", "title: \"Dashboard\"", "Check the page yourself", "Don't ask the person what they saw"] {
            assert!(text.contains(wanted), "{wanted} in: {text}");
        }
        let bare = handed_back("", true, None);
        assert!(bare.contains("Check the page yourself") && !bare.contains("The page is at"), "{bare}");
        // The lane's tabs as last told: the tab its tools worked in, else the first.
        let tab = |id, url: &str| equipe_protocol::BrowserPage { id, url: url.into(), ..Default::default() };
        let tabs = |agent_page| Event::BrowserTabs { squad: Some("acme".into()), lane: "acme/qa".into(), session: Some(1), member: None, color: None, pages: vec![tab(1, "https://a.test/"), tab(2, "https://b.test/")], agent_page };
        assert_eq!(current_page(&tabs(Some(2))).map(|p| p.url), Some("https://b.test/".into()));
        assert_eq!(current_page(&tabs(None)).map(|p| p.url), Some("https://a.test/".into()));
    }

    #[test]
    fn a_failure_made_of_thousands_of_lines_is_kept_as_an_excerpt() {
        let lines: Vec<String> = (0..5000).map(|n| format!("error: failed to truncate file 'f{n}': No space left on device")).collect();
        let out = format!("git worktree add failed:\n{}", lines.join("\n"));
        let note = output_excerpt(&out, OUTPUT_NOTE_MAX);
        assert!(note.len() <= OUTPUT_NOTE_MAX, "{}", note.len());
        assert!(note.starts_with("git worktree add failed:") && note.contains("[4994 lines left out]") && note.ends_with("f4999': No space left on device"), "{note}");
        // Short output stays whole, and one very long line is cut on a boundary.
        assert_eq!(output_excerpt("a\nb", OUTPUT_NOTE_MAX), "a\nb");
        let long = "é".repeat(20_000);
        let note = output_excerpt(&long, OUTPUT_NOTE_MAX);
        assert!(note.len() <= OUTPUT_NOTE_MAX + 3 && note.ends_with('…'));
    }

    #[test]
    fn only_the_first_completed_exchange_can_name_a_conversation() {
        let first = [
            Entry::User { text: "첫 질문 원문".into(), kept: Vec::new() },
            Entry::Reply { text: "첫 답변".into() },
            Entry::User { text: "둘째 질문".into(), kept: Vec::new() },
            Entry::Reply { text: "둘째 답변".into() },
        ];
        assert_eq!(first_exchange(&first), Some(("첫 질문 원문".into(), "첫 답변".into())));

        let unanswered = [
            Entry::User { text: "답하지 못한 첫 질문".into(), kept: Vec::new() },
            Entry::User { text: "둘째 질문".into(), kept: Vec::new() },
            Entry::Reply { text: "둘째 답변".into() },
        ];
        assert_eq!(first_exchange(&unanswered), None);
    }

    #[test]
    fn a_background_task_ending_is_never_the_persons_last_word() {
        let done = "<task-notification><task-id>b1</task-id><status>completed</status><summary>built</summary></task-notification>";
        let talk = [Entry::User { text: "go".into(), kept: Vec::new() }, Entry::Reply { text: "Started the build.".into() }, Entry::User { text: done.into(), kept: Vec::new() }];
        let context = suggestion_context("frontend", &talk).expect("the member answered last");
        assert!(!context.contains("task-notification"), "{context}");
    }

    #[test]
    fn a_turn_on_token_factory_logs_its_requests_never_its_text() {
        let turn = codex_app::Turn {
            result: TurnResult { text: "secret answer".into(), status: "completed".into(), usage: Some(equipe_providers::turn::TurnUsage { input_tokens: Some(1200), output_tokens: Some(40), ..Default::default() }) },
            thread_id: None,
            provider_requests: vec![
                equipe_providers::relay::Exchange { path: "/responses".into(), status: 200, request_id: Some("req-1".into()), ms: 900 },
                equipe_providers::relay::Exchange { path: "/responses".into(), status: 200, request_id: Some("req-2".into()), ms: 600 },
            ],
        };
        let line = nebius_log("nvidia/x", &turn);
        assert_eq!(line, "equipe: turn on Nemotron via Nebius Token Factory: model nvidia/x, 2 request(s) [200 200], request ids [req-1 req-2], 1500 ms on requests, tokens in 1200 (cached ?) out 40, completed");
        assert!(!line.contains("secret"));
    }

    #[test]
    fn a_turn_on_token_factory_with_its_key_refused_says_whose_key() {
        let lang = crate::i18n::Lang::En;
        // As codex 0.154 reports a 401 from the provider (mock-verified).
        let refused = || equipe_providers::Error::Backend { message: "unexpected status 401 Unauthorized: Invalid API key, url: http://127.0.0.1:1/responses".into(), code: Some("other".into()) };
        let (said, next) = nebius_failure(refused(), None, lang);
        assert!(said.contains("Nebius Token Factory refused the key"), "{said}");
        assert_eq!(next, Some("https://tokenfactory.nebius.com"));
        assert!(nebius_failure(equipe_providers::Error::CodexSignedOut, None, lang).0.contains("NEBIUS_API_KEY"));
        let other = nebius_failure(equipe_providers::Error::Protocol("stream ended".into()), None, lang);
        assert_eq!(other, ("stream ended".to_owned(), None));
        // Another provider's key: its own name, and where to replace it.
        let (said, next) = nebius_failure(refused(), Some("OpenRouter"), lang);
        assert!(said.contains("OpenRouter refused the key") && said.contains("Settings") && !said.contains("NEBIUS"), "{said}");
        assert_eq!(next, None);
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

/// What tests elsewhere in the crate need of the runtime's insides.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A handle whose loop never answers — as a loop busy with a long step
    /// (a merge, a worktree) doesn't — kept so while the second half lives:
    /// for tests of what must not wait on the loop.
    pub(crate) fn unanswered() -> (Handle, impl Sized) {
        let (requests, held) = mpsc::unbounded_channel::<Request>();
        (Handle { requests, pulse: watchdog::Pulse::new(), screens: std::sync::Arc::new(crate::screens::Screens::default()) }, held)
    }
}
