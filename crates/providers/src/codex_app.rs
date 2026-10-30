//! "Bring your own Codex": run the person's installed `codex` CLI as an
//! agentic engine, over `codex app-server` — the JSON-RPC protocol OpenAI's
//! own IDE extension speaks.
//!
//! As with Claude Code, equipe never touches the credential: the person signs
//! the CLI in with `codex login`, equipe asks it `codex login status`, and every
//! request authenticates inside Codex itself. equipe never reads
//! `~/.codex/auth.json`, never stores a token and never injects one.
//!
//! One `codex app-server` runs per turn, in the turn's working directory, the
//! way one `claude -p` does: it starts (or resumes) the member's thread, runs
//! one turn and exits. The thread lives on in Codex's own store, so its id
//! continues the conversation — across daemon restarts too. What the turn does
//! comes back as the same [`Activity`] Claude Code's does, tools named as
//! Claude Code names them (`Bash`, `Edit`, `Write`), so every screen shows both
//! engines alike. Codex's requests for approval go to the caller as [`Ask`]s:
//! the person answers them, never a model.
//!
//! Wire shapes come from `codex app-server generate-ts` (codex-cli 0.154.0).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use crate::claude_code::{Access, cap_diff, tool_target};
use crate::error::{Error, Result};
use crate::turn::{Activity, LimitWindow, Limits, ToolDone, TurnResult, TurnUsage};

/// The executable equipe runs unless told otherwise.
pub const DEFAULT_BINARY: &str = "codex";

/// How equipe introduces itself to the app-server: as itself.
const CLIENT_NAME: &str = "equipe";

/// How long the app-server may take to answer a plain request. Starting a
/// thread starts its MCP servers too, so it gets longer.
const ANSWER_WAIT: Duration = Duration::from_secs(30);
const THREAD_WAIT: Duration = Duration::from_secs(120);
/// After an interrupt, how long the turn has to wind down before it is cut.
const INTERRUPT_WAIT: Duration = Duration::from_secs(5);

// ── the binary and its sign-in ──

/// Where `binary` is: itself when it names a path, else the first on `PATH`.
pub fn find(binary: &str) -> Option<PathBuf> {
    if binary.contains('/') {
        let path = PathBuf::from(binary);
        return path.is_file().then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(binary)).find(|p| p.is_file())
}

/// How the person's Codex CLI is signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// With their ChatGPT plan: turns draw on it.
    ChatGpt,
    /// With an OpenAI API key: turns are billed per token.
    ApiKey,
    SignedOut,
}

impl SignIn {
    pub fn signed_in(self) -> bool {
        self != SignIn::SignedOut
    }
}

/// What `codex login status` says. Only the method is read: its output may
/// carry a masked key, which equipe neither keeps nor shows.
pub fn parse_login_status(success: bool, said: &str) -> SignIn {
    let said = said.to_ascii_lowercase();
    if !success || said.contains("not logged in") {
        SignIn::SignedOut
    } else if said.contains("chatgpt") {
        SignIn::ChatGpt
    } else if said.contains("api key") {
        SignIn::ApiKey
    } else {
        SignIn::SignedOut
    }
}

/// Ask the installed Codex CLI whether it is signed in. `Ok(None)`: it isn't installed.
pub async fn login_status(binary: &str) -> Result<Option<SignIn>> {
    let asked = Command::new(binary).args(["login", "status"]).stdin(Stdio::null()).kill_on_drop(true).output();
    let Ok(output) = tokio::time::timeout(ANSWER_WAIT, asked).await else {
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("`{binary} login status` didn't answer")).into());
    };
    let output = match output {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    // It says so on stderr in some versions, stdout in others.
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    Ok(Some(parse_login_status(output.status.success(), &said)))
}

// ── what a turn may do ──

/// The sandbox a thread starts in, by what its turns may do.
pub fn sandbox_mode(access: Access) -> &'static str {
    match access {
        Access::ReadOnly => "read-only",
        Access::EditFiles | Access::Ask => "workspace-write",
        Access::Full => "danger-full-access",
    }
}

/// When Codex asks before acting:
/// - read-only: never — what the sandbox refuses is refused, as `dontAsk` does;
/// - edit: only to leave the sandbox (`on-request`);
/// - ask: before every command it doesn't know to be safe, and every change
///   (`untrusted`);
/// - full: never.
pub fn approval_policy(access: Access) -> &'static str {
    match access {
        Access::ReadOnly | Access::Full => "never",
        Access::EditFiles => "on-request",
        Access::Ask => "untrusted",
    }
}

/// The sandbox for one turn. Writing reaches the working directory — the
/// member's worktree — and `writable` (its build cache) only; the network
/// stays off, so whatever needs it asks the person.
pub fn sandbox_policy(access: Access, writable: &[PathBuf]) -> Value {
    match access {
        Access::ReadOnly => json!({ "type": "readOnly", "networkAccess": false }),
        Access::EditFiles | Access::Ask => json!({
            "type": "workspaceWrite",
            "writableRoots": writable,
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false,
        }),
        Access::Full => json!({ "type": "dangerFullAccess" }),
    }
}

/// An MCP server for a thread, run as a command, as Codex's `mcp_servers`
/// config takes it. `approved`: its tools run without asking (equipe's own
/// board tools); otherwise each call asks the person.
pub fn mcp_server(command: &Path, args: &[String], approved: bool) -> Value {
    json!({ "command": command, "args": args, "startup_timeout_sec": 30, "default_tools_approval_mode": tools_approval(approved) })
}

/// One reached at `url` (streamable HTTP): a member's browser.
pub fn mcp_http_server(url: &str, approved: bool) -> Value {
    json!({ "url": url, "startup_timeout_sec": 30, "default_tools_approval_mode": tools_approval(approved) })
}

fn tools_approval(approved: bool) -> &'static str {
    if approved { "approve" } else { "prompt" }
}

/// The config a thread starts with: the project's `CLAUDE.md` read where it
/// has no `AGENTS.md`, so a project written for Claude Code tells Codex its
/// conventions too; and, given MCP servers, only those — `others` is what
/// [`others_off`] made of the person's own.
pub fn thread_config(mcp_servers: Option<&Value>, others: Option<&Value>) -> Value {
    let mut config = others.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    config["project_doc_fallback_filenames"] = json!(["CLAUDE.md"]);
    if let Some(Value::Object(servers)) = mcp_servers {
        let mut all = config.get("mcp_servers").and_then(Value::as_object).cloned().unwrap_or_default();
        all.extend(servers.clone());
        config["mcp_servers"] = Value::Object(all);
    }
    config
}

/// The person's own MCP servers turned off for a thread, from what
/// `mcpServerStatus/list` names — never their settings: config overrides add
/// to `~/.codex/config.toml` rather than replace it, so each server it
/// configures is switched off by name, each plugin's likewise, and the apps'.
/// Claude Code's `--strict-mcp-config`, as Codex takes it.
pub fn others_off(listing: &Value, ours: &Value) -> Value {
    let mut servers = serde_json::Map::new();
    let mut plugins = serde_json::Map::new();
    for server in listing.get("data").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = server.get("name").and_then(Value::as_str) else { continue };
        match server.get("pluginId").and_then(Value::as_str) {
            Some(plugin) => {
                plugins.insert(plugin.to_owned(), json!({ "enabled": false }));
            }
            // The apps' server is Codex's own, off with its feature.
            None if name == APPS_SERVER || ours.get(name).is_some() => {}
            None => {
                servers.insert(name.to_owned(), json!({ "enabled": false }));
            }
        }
    }
    json!({ "mcp_servers": servers, "plugins": plugins, "features": { "apps": false } })
}

/// Codex's built-in server for ChatGPT apps.
const APPS_SERVER: &str = "codex_apps";

// ── another provider ──

/// What Codex can't send a provider that only speaks the Responses API's
/// function tools: tools of other types (the web search, the namespaces
/// subagents and apps come in, image generation), images, compaction on
/// OpenAI's own endpoint, and compressed bodies. Turned off for such a
/// provider's turns, so each request carries function tools only
/// (`exec_command`, `write_stdin`, `request_user_input`), and MCP servers'
/// tools, which Codex sends in a `namespace`, flattened on the way
/// ([`crate::relay`]). Its edits go through the shell: `apply_patch` comes
/// only as a freeform tool (codex-cli 0.154.0 has no function form of it).
pub const PLAIN_FEATURES_OFF: [&str; 11] = [
    "multi_agent",
    "goals",
    "image_generation",
    "apps",
    "plugins",
    "remote_compaction_v2",
    "view_image",
    "browser_use",
    "computer_use",
    "tool_suggest",
    "enable_request_compression",
];

/// Another provider speaking OpenAI's Responses API, for a turn to run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The id Codex knows it by.
    pub id: String,
    /// Its name, as Codex shows it.
    pub name: String,
    /// Its API's root, before `/responses`.
    pub base_url: String,
    /// The environment variable Codex reads its key from.
    pub env_key: String,
    /// More config overrides for its turns (`key=value`, as `-c` takes them).
    pub config: Vec<String>,
}

/// The app-server's command line for a turn on another provider that speaks
/// OpenAI's Responses API at `base_url` (`wire_api = "responses"`; Codex
/// takes no other), as config overrides on top of the person's own: the
/// provider by its id, its key read by Codex itself from the environment
/// variable it names (never passed on the command line, never seen by
/// equipe), and the requests kept to what such a provider serves
/// ([`PLAIN_FEATURES_OFF`]). The ChatGPT sign-in plays no part: Codex asks for
/// none when the provider doesn't require OpenAI's auth. The key is kept out
/// of the commands the agent runs (`shell_environment_policy.exclude`, which
/// for these turns stands in for the person's own list).
pub fn provider_args(provider: &Provider, base_url: &str) -> Vec<String> {
    let toml = |value: &str| serde_json::to_string(value).unwrap_or_default();
    let id = &provider.id;
    let mut config = vec![format!("model_provider={}", toml(id))];
    let at = format!("model_providers.{id}");
    config.push(format!("{at}.name={}", toml(&provider.name)));
    config.push(format!("{at}.base_url={}", toml(base_url)));
    config.push(format!("{at}.env_key={}", toml(&provider.env_key)));
    config.push(format!("{at}.wire_api={}", toml("responses")));
    config.push(format!("{at}.requires_openai_auth=false"));
    config.push(format!("web_search={}", toml("disabled")));
    config.push(format!("model_reasoning_summary={}", toml("none")));
    config.push(format!("shell_environment_policy.exclude=[{}]", toml(&provider.env_key)));
    config.extend(PLAIN_FEATURES_OFF.iter().map(|feature| format!("features.{feature}=false")));
    config.extend(provider.config.iter().cloned());
    config.into_iter().flat_map(|c| ["-c".to_owned(), c]).collect()
}

// ── one turn ──

/// One turn's input.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnRequest<'a> {
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    /// How hard the model thinks: `low` … `ultra`; `None`, the model's default.
    pub effort: Option<&'a str>,
    pub access: Access,
    /// The thread an earlier turn ran in, to continue it.
    pub resume: Option<&'a str>,
    /// Who the agent is on the team: Codex's developer instructions, beside
    /// its own and the project's `AGENTS.md`.
    pub instructions: Option<&'a str>,
    /// MCP servers for the thread, by name ([`mcp_server`]). Replace the person's own.
    pub mcp_servers: Option<&'a Value>,
    /// Directories outside the working directory the turn may write: a build cache.
    pub writable: &'a [PathBuf],
    /// More environment for the app-server, and so for its commands.
    pub env: &'a [(String, String)],
    /// Another provider to run the turn on ([`Provider`]); `None`: the
    /// person's own Codex, as they set it up.
    pub provider: Option<&'a Provider>,
}

/// `thread/start`'s parameters; `others`, the person's own MCP servers off.
pub fn start_params(request: &TurnRequest<'_>, cwd: &Path, others: Option<&Value>) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "approvalPolicy": approval_policy(request.access),
        "sandbox": sandbox_mode(request.access),
        "config": thread_config(request.mcp_servers, others),
        "serviceName": CLIENT_NAME,
    });
    // A thread's `features` stand in for the command line's: on another
    // provider, what it can't serve stays off in them too.
    if request.provider.is_some() {
        for feature in PLAIN_FEATURES_OFF {
            params["config"]["features"][feature] = json!(false);
        }
    }
    if let Some(model) = request.model {
        params["model"] = json!(model);
    }
    if let Some(instructions) = request.instructions {
        params["developerInstructions"] = json!(instructions);
    }
    params
}

/// `thread/resume`'s: the same settings, which may have changed since — the
/// team, the mode — for the thread `id`.
pub fn resume_params(request: &TurnRequest<'_>, cwd: &Path, id: &str, others: Option<&Value>) -> Value {
    let mut params = start_params(request, cwd, others);
    if let Some(map) = params.as_object_mut() {
        map.remove("serviceName");
        map.insert("threadId".into(), json!(id));
        map.insert("excludeTurns".into(), json!(true));
    }
    params
}

/// `turn/start`'s: the prompt, and the sandbox for this turn.
pub fn turn_params(request: &TurnRequest<'_>, cwd: &Path, thread: &str) -> Value {
    let mut params = json!({
        "threadId": thread,
        "input": [text_input(request.prompt)],
        "cwd": cwd,
        "approvalPolicy": approval_policy(request.access),
        "sandboxPolicy": sandbox_policy(request.access, request.writable),
        "summary": "auto",
        // Standard speed for a member, whatever the person's own Codex is set
        // to (Fast, `service_tier = "priority"`, spends the plan 2–2.5× as
        // fast for ~1.5× the speed: worth it to someone waiting, not to a
        // member working in the background). Their own config is left alone.
        "serviceTierForTurn": "default",
    });
    if let Some(model) = request.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = request.effort {
        params["effort"] = json!(effort);
    }
    params
}

fn text_input(text: &str) -> Value {
    json!({ "type": "text", "text": text, "text_elements": [] })
}

/// One turn's outcome, with the thread that continues the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub result: TurnResult,
    pub thread_id: Option<String>,
    /// On another provider: each request relayed to it ([`crate::relay`]).
    pub provider_requests: Vec<crate::relay::Exchange>,
}

/// Codex asks before it acts: `tool` and `input` say what, as Claude Code's
/// permission prompts say it (`Bash` with its `command`, `Edit` with its
/// `file_path`). Send `true` to allow it once, `false` to decline; dropping
/// `answer` declines.
#[derive(Debug)]
pub struct Ask {
    pub tool: String,
    pub input: Value,
    pub answer: oneshot::Sender<bool>,
}

/// What a request from the app-server asks the person, if anything: `None`
/// answers it at once ([`reply`] with `false`). `access` tells whether an
/// allowed command leaves the sandbox.
pub fn ask_of(method: &str, params: &Value, collector: &TurnCollector, access: Access) -> Option<(String, Value)> {
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_owned);
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => {
            let command = match params.get("command") {
                Some(Value::Array(argv)) => argv.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
                Some(Value::String(command)) => command.clone(),
                _ => String::new(),
            };
            let network = params.get("networkApprovalContext").and_then(|c| c.get("host")).and_then(Value::as_str).map(|h| format!("network access to {h}"));
            let reason = [text("reason"), network].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let mut input = json!({ "command": unwrap_shell(&command) });
            if !reason.is_empty() {
                input["description"] = json!(reason);
            }
            // Asked in edit access, the command is asking to run outside the
            // sandbox: that is what allowing it allows.
            if access == Access::EditFiles {
                input["dangerouslyDisableSandbox"] = json!(true);
            }
            Some(("Bash".into(), input))
        }
        "item/fileChange/requestApproval" | "applyPatchApproval" => {
            let paths: Vec<(String, bool)> = match params.get("fileChanges").and_then(Value::as_object) {
                Some(changes) => changes.iter().map(|(path, c)| (path.clone(), c.get("type").and_then(Value::as_str) == Some("add"))).collect(),
                None => text("itemId").and_then(|id| collector.changes.get(&id).cloned()).unwrap_or_default(),
            };
            let (first, _) = paths.first()?.clone();
            let tool = if paths.iter().all(|(_, added)| *added) { "Write" } else { "Edit" };
            let more = match paths.len() {
                1 => None,
                n => Some(format!("and {} more file{}", n - 1, if n == 2 { "" } else { "s" })),
            };
            let reason = [text("reason"), text("grantRoot").map(|root| format!("write under {root}")), more].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let mut input = json!({ "file_path": first });
            if !reason.is_empty() {
                input["description"] = json!(reason);
            }
            Some((tool.into(), input))
        }
        "item/permissions/requestApproval" => {
            let mut input = params.get("permissions").cloned().unwrap_or_else(|| json!({}));
            if let (Some(map), Some(reason)) = (input.as_object_mut(), text("reason")) {
                map.insert("description".into(), json!(reason));
            }
            Some(("Permissions".into(), input))
        }
        // An MCP server asking for a yes (a tool call that wants approval).
        "mcpServer/elicitation/request" if params.get("mode").and_then(Value::as_str) != Some("url") => {
            let server = text("serverName").unwrap_or_else(|| "mcp".into());
            Some((format!("mcp__{server}"), json!({ "description": text("message").unwrap_or_default() })))
        }
        _ => None,
    }
}

/// The answer to a request from the app-server, allowed or not.
pub fn reply(method: &str, params: &Value, allowed: bool) -> std::result::Result<Value, (i64, String)> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Ok(json!({ "decision": if allowed { "accept" } else { "decline" } }))
        }
        "execCommandApproval" | "applyPatchApproval" => {
            Ok(json!({ "decision": if allowed { json!("approved") } else { json!({ "denied": { "rejection": "The person declined." } }) } }))
        }
        "item/permissions/requestApproval" => {
            let granted = if allowed {
                let asked = params.get("permissions").and_then(Value::as_object).cloned().unwrap_or_default();
                Value::Object(asked.into_iter().filter(|(_, v)| !v.is_null()).collect())
            } else {
                json!({})
            };
            Ok(json!({ "permissions": granted, "scope": "turn" }))
        }
        "mcpServer/elicitation/request" => Ok(if allowed {
            json!({ "action": "accept", "content": {}, "_meta": null })
        } else {
            json!({ "action": "decline", "content": null, "_meta": null })
        }),
        // A member puts its questions with equipe's own `ask_decision`.
        "item/tool/requestUserInput" => Ok(json!({ "answers": {} })),
        other => Err((-32601, format!("equipe doesn't answer {other}"))),
    }
}

/// A shell command as the person reads it: `/bin/zsh -lc 'cat a'` is `cat a`.
pub fn unwrap_shell(command: &str) -> String {
    let command = command.trim();
    for flag in [" -lc ", " -c "] {
        if let Some((shell, rest)) = command.split_once(flag)
            && !shell.contains(' ')
            && shell.ends_with("sh")
        {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                return inner.replace("'\\''", "'");
            }
            if let Some(inner) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
                return inner.replace("\\\"", "\"");
            }
            return rest.to_owned();
        }
    }
    command.to_owned()
}

/// A tool use under way, waiting for its outcome.
struct Pending {
    name: String,
    target: Option<String>,
    since: Instant,
}

/// Folds the app-server's notifications about one thread's turn into what it
/// shows, and its outcome.
#[derive(Default)]
pub struct TurnCollector {
    thread: Option<String>,
    turn: Option<String>,
    /// Tool uses under way, by item id: a file change has one per file.
    pending: HashMap<String, Vec<Pending>>,
    /// Each file change's files, and whether each is new: what an approval
    /// of it is about.
    changes: HashMap<String, Vec<(String, bool)>>,
    /// Answer text streamed so far, and the latest message whole.
    streamed: String,
    last_message: String,
    /// Messages streamed as they were written, by item id.
    streaming: HashSet<String>,
    /// A message ended, and text streamed since needs a paragraph between.
    after_message: bool,
    /// The last thing shown was answer text.
    text_open: bool,
    /// The person's messages read so far: the prompt first, then steers.
    read: usize,
    /// Tokens of the turn's requests, summed: input not cached, cached, output.
    input: u64,
    cached: u64,
    output: u64,
    /// The context as of the latest request, and the window.
    context: Option<u64>,
    window: Option<u64>,
    limits: Limits,
    /// The plan last shown, so an unchanged one isn't shown again.
    plan: Option<String>,
    subagents: HashSet<String>,
    /// An error the app-server reported on the way.
    error: Option<String>,
    result: Option<Result<TurnResult>>,
}

impl TurnCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread the turn runs in: notifications about any other (a
    /// subagent's) are its own.
    pub fn set_thread(&mut self, id: &str) {
        self.thread = Some(id.to_owned());
    }

    pub fn set_turn(&mut self, id: &str) {
        self.turn = Some(id.to_owned());
    }

    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    /// Whether the turn has ended.
    pub fn done(&self) -> bool {
        self.result.is_some()
    }

    /// Apply one notification. Returns what it shows.
    pub fn on_notification(&mut self, method: &str, params: &Value) -> Vec<Activity> {
        // The plan's limits are the account's, whatever thread they came with.
        if method == "account/rateLimits/updated" {
            return match params.get("rateLimits").and_then(parse_limits) {
                Some(update) => {
                    self.limits.merge(update);
                    vec![Activity::Limits(self.limits.clone())]
                }
                None => Vec::new(),
            };
        }
        let thread = params.get("threadId").and_then(Value::as_str);
        if let (Some(ours), Some(thread)) = (&self.thread, thread)
            && ours != thread
        {
            return Vec::new();
        }
        let out = match method {
            "turn/started" => {
                if let Some(id) = params.get("turn").and_then(|t| t.get("id")).and_then(Value::as_str) {
                    self.turn = Some(id.to_owned());
                }
                Vec::new()
            }
            "item/agentMessage/delta" => {
                let Some(delta) = params.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty()) else {
                    return Vec::new();
                };
                if let Some(id) = params.get("itemId").and_then(Value::as_str) {
                    self.streaming.insert(id.to_owned());
                }
                self.text(delta)
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => match params.get("delta").and_then(Value::as_str) {
                Some(delta) if !delta.is_empty() => vec![Activity::Thinking(delta.to_owned())],
                _ => Vec::new(),
            },
            "item/started" => params.get("item").map(|item| self.item_started(item)).unwrap_or_default(),
            "item/completed" => params.get("item").map(|item| self.item_completed(item)).unwrap_or_default(),
            "turn/plan/updated" => self.plan(params),
            "thread/tokenUsage/updated" => {
                let usage = params.get("tokenUsage");
                let last = usage.and_then(|u| u.get("last"));
                let n = |k: &str| last.and_then(|l| l.get(k)).and_then(Value::as_u64).unwrap_or(0);
                let cached = n("cachedInputTokens");
                self.input += n("inputTokens").saturating_sub(cached);
                self.cached += cached;
                self.output += n("outputTokens");
                if last.is_some() {
                    self.context = Some(n("totalTokens"));
                }
                if let Some(window) = usage.and_then(|u| u.get("modelContextWindow")).and_then(Value::as_u64) {
                    self.window = Some(window);
                }
                vec![Activity::Output(self.output)]
            }
            "model/rerouted" => match params.get("toModel").and_then(Value::as_str) {
                Some(model) => vec![Activity::Model(model.to_owned())],
                None => Vec::new(),
            },
            "error" => {
                if params.get("willRetry").and_then(Value::as_bool) != Some(true) {
                    self.error = params.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).map(str::to_owned);
                }
                Vec::new()
            }
            "turn/completed" => {
                let turn = params.get("turn");
                let id = turn.and_then(|t| t.get("id")).and_then(Value::as_str);
                if self.turn.is_some() && id.is_some() && id != self.turn.as_deref() {
                    return Vec::new();
                }
                self.result = Some(self.outcome(turn));
                Vec::new()
            }
            _ => Vec::new(),
        };
        if out.iter().any(|a| !matches!(a, Activity::Text(_) | Activity::Thinking(_) | Activity::Output(_) | Activity::Limits(_))) {
            self.text_open = false;
        }
        out
    }

    fn text(&mut self, delta: &str) -> Vec<Activity> {
        let text = if self.after_message && self.text_open { format!("\n\n{delta}") } else { delta.to_owned() };
        self.after_message = false;
        self.text_open = true;
        self.streamed.push_str(&text);
        vec![Activity::Text(text)]
    }

    fn item_started(&mut self, item: &Value) -> Vec<Activity> {
        let Some(id) = item.get("id").and_then(Value::as_str) else { return Vec::new() };
        let uses: Vec<(String, Option<String>)> = match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "commandExecution" => vec![command_tool(item)],
            "fileChange" => {
                let changes = file_changes(item);
                self.changes.insert(id.to_owned(), changes.iter().map(|(path, kind, _)| (path.clone(), kind == "add")).collect());
                changes.into_iter().map(|(path, kind, _)| (change_tool(&kind).to_owned(), Some(path))).collect()
            }
            "mcpToolCall" => vec![mcp_tool(item)],
            _ => return Vec::new(),
        };
        let now = Instant::now();
        self.pending.insert(id.to_owned(), uses.iter().map(|(name, target)| Pending { name: name.clone(), target: target.clone(), since: now }).collect());
        uses.into_iter().map(|(name, target)| Activity::Tool { name, target, label: None, background: false }).collect()
    }

    fn item_completed(&mut self, item: &Value) -> Vec<Activity> {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let status = item.get("status").and_then(Value::as_str).unwrap_or("completed");
        let ms = item.get("durationMs").and_then(Value::as_u64);
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "agentMessage" => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                let mut out = Vec::new();
                // A message that came whole, without deltas, shows whole.
                if !text.is_empty() && !self.streaming.contains(id) {
                    out = self.text(text);
                }
                if !text.is_empty() {
                    self.last_message = text.to_owned();
                    self.after_message = true;
                }
                out
            }
            // The person's words, read: the prompt, then what they said while it worked.
            "userMessage" => {
                self.read += 1;
                let text: String = item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("");
                if self.read > 1 && !text.is_empty() { vec![Activity::Steered(text)] } else { Vec::new() }
            }
            "commandExecution" => {
                let (name, target) = command_tool(item);
                let pending = self.take(id).into_iter().next().unwrap_or(Pending { name, target, since: Instant::now() });
                let exit = item.get("exitCode").and_then(Value::as_i64).and_then(|c| i32::try_from(c).ok());
                let ok = status == "completed" && exit.is_none_or(|c| c == 0);
                let output = match (status, item.get("aggregatedOutput").and_then(Value::as_str)) {
                    ("declined", _) => "Declined.".to_owned(),
                    (_, Some(out)) if !ok || pending.name == "Bash" => out.to_owned(),
                    _ => String::new(),
                };
                vec![Activity::ToolDone(done(pending, ok, exit, ms, output, None))]
            }
            "fileChange" => {
                let changes = file_changes(item);
                let mut pending = self.take(id);
                let ok = status == "completed";
                changes
                    .into_iter()
                    .enumerate()
                    .map(|(i, (path, kind, diff))| {
                        let p = if i < pending.len() { pending.remove(0) } else { Pending { name: change_tool(&kind).into(), target: Some(path.clone()), since: Instant::now() } };
                        let diff = ok.then(|| change_diff(&path, &kind, &diff));
                        let output = if ok { String::new() } else { format!("{} {status}.", if status == "declined" { "Change" } else { "Patch" }) };
                        Activity::ToolDone(done(p, ok, None, ms, output, diff))
                    })
                    .collect()
            }
            "mcpToolCall" => {
                let (name, target) = mcp_tool(item);
                let pending = self.take(id).into_iter().next().unwrap_or(Pending { name, target, since: Instant::now() });
                let error = item.get("error").and_then(|e| e.get("message")).and_then(Value::as_str);
                let result = item.get("result");
                let failed = status != "completed" || error.is_some() || result.and_then(|r| r.get("isError")).and_then(Value::as_bool) == Some(true);
                let output = match error {
                    Some(error) => error.to_owned(),
                    None if failed => mcp_text(result),
                    None => String::new(),
                };
                vec![Activity::ToolDone(done(pending, !failed, None, ms, output, None))]
            }
            "webSearch" => {
                let query = item
                    .get("query")
                    .and_then(Value::as_str)
                    .filter(|q| !q.is_empty())
                    .or_else(|| item.get("action").and_then(|a| a.get("query").or_else(|| a.get("url"))).and_then(Value::as_str))
                    .map(|q| q.chars().take(120).collect::<String>());
                let pending = Pending { name: "WebSearch".into(), target: query.clone(), since: Instant::now() };
                vec![Activity::Tool { name: "WebSearch".into(), target: query, label: None, background: false }, Activity::ToolDone(done(pending, true, None, ms, String::new(), None))]
            }
            "imageView" => {
                let path = item.get("path").and_then(Value::as_str).map(str::to_owned);
                let pending = Pending { name: "Read".into(), target: path.clone(), since: Instant::now() };
                vec![Activity::Tool { name: "Read".into(), target: path, label: None, background: false }, Activity::ToolDone(done(pending, true, None, ms, String::new(), None))]
            }
            // Work handed to a subagent, and back.
            "collabAgentToolCall" => {
                let receivers: Vec<String> = item.get("receiverThreadIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
                match item.get("tool").and_then(Value::as_str) {
                    Some("spawnAgent") => receivers
                        .into_iter()
                        .filter(|r| self.subagents.insert(r.clone()))
                        .map(|id| {
                            let what = item.get("prompt").and_then(Value::as_str).unwrap_or("a subagent");
                            Activity::SubagentStarted { id, what: what.lines().next().unwrap_or("").chars().take(80).collect() }
                        })
                        .collect(),
                    Some(_) => {
                        let states = item.get("agentsStates");
                        receivers
                            .into_iter()
                            .filter(|r| {
                                let state = states.and_then(|s| s.get(r)).and_then(|s| s.get("status")).and_then(Value::as_str);
                                matches!(state, Some("completed" | "errored" | "shutdown" | "interrupted" | "notFound"))
                                    || item.get("tool").and_then(Value::as_str) == Some("closeAgent")
                            })
                            .filter(|r| self.subagents.remove(r))
                            .map(|id| Activity::SubagentEnded { id })
                            .collect()
                    }
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn take(&mut self, id: &str) -> Vec<Pending> {
        self.pending.remove(id).unwrap_or_default()
    }

    /// The turn's plan, as a to-do list's latest state: how far along, and the step at hand.
    fn plan(&mut self, params: &Value) -> Vec<Activity> {
        let steps: Vec<(&str, &str)> = params
            .get("plan")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s.get("step")?.as_str()?, s.get("status").and_then(Value::as_str).unwrap_or("pending"))))
            .collect();
        if steps.is_empty() {
            return Vec::new();
        }
        let finished = steps.iter().filter(|(_, s)| *s == "completed").count();
        let at = steps.iter().find(|(_, s)| *s == "inProgress").or_else(|| steps.iter().find(|(_, s)| *s == "pending")).map(|(step, _)| *step);
        let line = match at {
            Some(step) => format!("{finished}/{} · {}", steps.len(), step.lines().next().unwrap_or("").chars().take(100).collect::<String>()),
            None => format!("{finished}/{}", steps.len()),
        };
        if self.plan.as_deref() == Some(line.as_str()) {
            return Vec::new();
        }
        self.plan = Some(line.clone());
        let pending = Pending { name: "TodoWrite".into(), target: Some(line.clone()), since: Instant::now() };
        vec![Activity::Tool { name: "TodoWrite".into(), target: Some(line), label: None, background: false }, Activity::ToolDone(done(pending, true, None, Some(0), String::new(), None))]
    }

    fn usage(&self) -> Option<TurnUsage> {
        (self.input + self.cached + self.output > 0).then_some(TurnUsage {
            input_tokens: Some(self.input),
            output_tokens: Some(self.output),
            cache_tokens: (self.cached > 0).then_some(self.cached),
            context_tokens: self.context,
            context_window: self.window,
        })
    }

    fn answer(&self) -> String {
        if self.last_message.is_empty() { self.streamed.clone() } else { self.last_message.clone() }
    }

    fn outcome(&self, turn: Option<&Value>) -> Result<TurnResult> {
        match turn.and_then(|t| t.get("status")).and_then(Value::as_str).unwrap_or("completed") {
            "failed" => {
                let error = turn.and_then(|t| t.get("error"));
                let message = error.and_then(|e| e.get("message")).and_then(Value::as_str).map(str::to_owned).or_else(|| self.error.clone());
                let code = error.and_then(|e| e.get("codexErrorInfo")).and_then(|c| match c {
                    Value::String(code) => Some(code.clone()),
                    Value::Object(map) => map.keys().next().cloned(),
                    _ => None,
                });
                if code.as_deref() == Some("unauthorized") {
                    return Err(Error::CodexSignedOut);
                }
                Err(Error::Backend { message: message.unwrap_or_else(|| "Codex's turn failed".into()), code })
            }
            status => Ok(TurnResult {
                text: self.answer(),
                status: if status == "interrupted" { "interrupted".into() } else { "completed".into() },
                usage: self.usage(),
            }),
        }
    }

    /// The turn as far as it went, stopped.
    pub fn interrupted(&self) -> TurnResult {
        TurnResult { text: self.answer(), status: "interrupted".into(), usage: self.usage() }
    }

    /// The turn, once the app-server is done with it. `diagnostics` is what
    /// to show when it ended without saying how the turn went.
    pub fn finish(self, diagnostics: &str) -> Result<TurnResult> {
        match self.result {
            Some(result) => result,
            None if !self.streamed.is_empty() => Ok(TurnResult { text: self.answer(), status: "incomplete".into(), usage: self.usage() }),
            None => Err(Error::Protocol(format!(
                "codex app-server ended without finishing the turn{}",
                match (self.error.as_deref(), diagnostics) {
                    (Some(error), _) => format!(": {error}"),
                    (None, "") => String::new(),
                    (None, more) => format!(": {more}"),
                }
            ))),
        }
    }
}

fn done(pending: Pending, ok: bool, exit: Option<i32>, ms: Option<u64>, output: String, diff: Option<String>) -> ToolDone {
    let ms = ms.unwrap_or_else(|| u64::try_from(pending.since.elapsed().as_millis()).unwrap_or(u64::MAX));
    ToolDone { name: pending.name, target: pending.target, ok, exit, ms, output, diff }
}

/// A command as a tool: one that only reads a file, lists or searches shows
/// as Claude Code's `Read`, `Glob`, `Grep` do; anything else is `Bash`.
fn command_tool(item: &Value) -> (String, Option<String>) {
    let actions: Vec<&Value> = item.get("commandActions").and_then(Value::as_array).into_iter().flatten().collect();
    let text = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_owned);
    if let [action] = actions[..] {
        let found = match action.get("type").and_then(Value::as_str) {
            Some("read") => text(action, "path").map(|p| ("Read", p)),
            Some("listFiles") => text(action, "path").map(|p| ("Glob", p)),
            Some("search") => text(action, "query").or_else(|| text(action, "path")).map(|q| ("Grep", q)),
            _ => None,
        };
        if let Some((name, target)) = found {
            return (name.into(), Some(target));
        }
    }
    let command = unwrap_shell(item.get("command").and_then(Value::as_str).unwrap_or_default());
    ("Bash".into(), Some(command.lines().next().unwrap_or("").chars().take(120).collect()))
}

fn mcp_tool(item: &Value) -> (String, Option<String>) {
    let server = item.get("server").and_then(Value::as_str).unwrap_or("mcp");
    let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
    let name = format!("mcp__{server}__{tool}");
    let target = tool_target(&name, item.get("arguments"));
    (name, target)
}

fn mcp_text(result: Option<&Value>) -> String {
    result
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A file change's files: each one's path, kind (`add`, `delete`, `update`) and diff.
fn file_changes(item: &Value) -> Vec<(String, String, String)> {
    item.get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let path = c.get("path")?.as_str()?.to_owned();
            let kind = c.get("kind").and_then(|k| k.get("type").or(Some(k))).and_then(Value::as_str).unwrap_or("update").to_owned();
            let diff = c.get("diff").and_then(Value::as_str).unwrap_or_default().to_owned();
            Some((path, kind, diff))
        })
        .collect()
}

fn change_tool(kind: &str) -> &'static str {
    if kind == "add" { "Write" } else { "Edit" }
}

/// A file change as a unified diff headed `diff --git`, as Claude Code's edits
/// show. Codex gives a new file's content, a deleted one's, or an update's hunks.
pub fn change_diff(path: &str, kind: &str, diff: &str) -> String {
    let lines = |prefix: char| diff.lines().map(|l| format!("{prefix}{l}\n")).collect::<String>();
    let n = diff.lines().count();
    let out = match kind {
        "add" => format!("diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{n} @@\n{}", lines('+')),
        "delete" => format!("diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1,{n} +0,0 @@\n{}", lines('-')),
        _ if diff.starts_with("diff --git") => diff.to_owned(),
        _ if diff.starts_with("--- ") => format!("diff --git a/{path} b/{path}\n{diff}"),
        _ => format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{diff}"),
    };
    let out = if out.ends_with('\n') { out } else { format!("{out}\n") };
    cap_diff(out)
}

// ── the plan's usage, and the models ──

/// A `RateLimitSnapshot` as [`Limits`]: the plan's own bucket only (`codex`,
/// or none named); a model's own bucket is left out.
pub fn parse_limits(snapshot: &Value) -> Option<Limits> {
    let id = snapshot.get("limitId").and_then(Value::as_str);
    if id.is_some_and(|id| id != "codex") {
        return None;
    }
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|key| {
            let w = snapshot.get(*key).filter(|w| !w.is_null())?;
            Some(LimitWindow {
                used_percent: w.get("usedPercent")?.as_f64()?,
                minutes: w.get("windowDurationMins").and_then(Value::as_i64),
                resets_at: w.get("resetsAt").and_then(Value::as_i64),
            })
        })
        .collect();
    let credits = snapshot.get("credits").filter(|c| !c.is_null()).map(|c| {
        let has = c.get("hasCredits").and_then(Value::as_bool).unwrap_or(false) || c.get("unlimited").and_then(Value::as_bool).unwrap_or(false);
        (has, c.get("balance").and_then(Value::as_str).map(str::to_owned))
    });
    Some(Limits {
        windows,
        plan_type: snapshot.get("planType").and_then(Value::as_str).map(str::to_owned),
        reached: snapshot.get("rateLimitReachedType").and_then(Value::as_str).map(str::to_owned),
        credits,
    })
}

/// Usable model ids from `model/list`, the account's default first; hidden ones left out.
pub fn model_ids(listing: &Value) -> Vec<String> {
    let models: Vec<&Value> = listing
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| m.get("hidden").and_then(Value::as_bool) != Some(true))
        .collect();
    let id = |m: &&Value| m.get("id").or_else(|| m.get("model")).and_then(Value::as_str).map(str::to_owned);
    let default = models.iter().filter(|m| m.get("isDefault").and_then(Value::as_bool) == Some(true)).filter_map(id);
    let rest = models.iter().filter(|m| m.get("isDefault").and_then(Value::as_bool) != Some(true)).filter_map(id);
    let mut seen = HashSet::new();
    default.chain(rest).filter(|m| seen.insert(m.clone())).collect()
}

/// The plan's usage now, without running a turn.
pub async fn read_limits(binary: &str, cwd: &Path) -> Result<Limits> {
    let mut server = Server::start(binary, cwd, &[], &[]).await?;
    let read = server.call("account/rateLimits/read", json!({}), ANSWER_WAIT).await?;
    server.close().await;
    read.get("rateLimits").and_then(parse_limits).ok_or_else(|| Error::Protocol("codex reported no usage limits".into()))
}

/// Who the person's Codex is signed in as: the ChatGPT account's email and
/// plan (`free`, `plus`, `pro`…), and whether it is an API key instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub email: Option<String>,
    pub plan: Option<String>,
    pub api_key: bool,
}

/// Read who the person's Codex is signed in as (`account/read`); `None`: signed out.
pub async fn read_account(binary: &str, cwd: &Path) -> Result<Option<Account>> {
    let mut server = Server::start(binary, cwd, &[], &[]).await?;
    let read = server.call("account/read", json!({}), ANSWER_WAIT).await?;
    server.close().await;
    Ok(account_of(&read))
}

fn account_of(read: &Value) -> Option<Account> {
    let account = read.get("account").filter(|a| !a.is_null())?;
    let text = |k: &str| account.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Account { email: text("email"), plan: text("planType"), api_key: text("type").as_deref() == Some("apiKey") })
}

/// The models the person's Codex offers, its default first.
pub async fn list_models(binary: &str, cwd: &Path) -> Result<Vec<String>> {
    let mut server = Server::start(binary, cwd, &[], &[]).await?;
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    // Pages, as many as there are — a few at most.
    for _ in 0..10 {
        let listing = server.call("model/list", json!({ "cursor": cursor }), ANSWER_WAIT).await?;
        ids.extend(model_ids(&listing));
        cursor = listing.get("nextCursor").and_then(Value::as_str).map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    server.close().await;
    Ok(ids)
}

// ── the app-server ──

/// A message from the app-server.
enum Incoming {
    /// The answer to a request of ours.
    Answer { id: u64, result: std::result::Result<Value, String> },
    /// A request of its: asking for approval, mostly.
    Request { id: Value, method: String, params: Value },
    Notification { method: String, params: Value },
}

fn incoming(message: Value) -> Option<Incoming> {
    let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match (message.get("id").filter(|id| !id.is_null()).cloned(), method) {
        (Some(id), Some(method)) => Some(Incoming::Request { id, method, params }),
        (None, Some(method)) => Some(Incoming::Notification { method, params }),
        (Some(id), None) => {
            let result = match message.get("error") {
                Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("error").to_owned()),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Answer { id: id.as_u64()?, result })
        }
        (None, None) => None,
    }
}

/// One `codex app-server`, spoken to over its standard input and output.
struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    stderr: tokio::task::JoinHandle<String>,
    next_id: u64,
    /// Notifications that came while waiting for an answer.
    early: Vec<(String, Value)>,
    stray: Vec<String>,
}

impl Server {
    /// Start it in `cwd` and introduce equipe.
    async fn start(binary: &str, cwd: &Path, args: &[String], env: &[(String, String)]) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg("app-server")
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => Error::Login(format!(
                    "Codex (`{binary}`) is not installed or not on PATH. Install it (https://learn.chatgpt.com/docs), then run `codex login`."
                )),
                _ => err.into(),
            })?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let stderr = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });
        let mut server = Server { child, stdin, lines, stderr, next_id: 1, early: Vec::new(), stray: Vec::new() };
        let hello = json!({
            "clientInfo": { "name": CLIENT_NAME, "title": "equipe", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": false, "requestAttestation": false },
        });
        server.call("initialize", hello, ANSWER_WAIT).await?;
        server.send(&json!({ "method": "initialized" })).await?;
        Ok(server)
    }

    async fn send(&mut self, message: &Value) -> Result<()> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Send a request; its id.
    async fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "id": id, "method": method, "params": params })).await?;
        Ok(id)
    }

    async fn answer(&mut self, id: Value, result: std::result::Result<Value, (i64, String)>) -> Result<()> {
        let message = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err((code, message)) => json!({ "id": id, "error": { "code": code, "message": message } }),
        };
        self.send(&message).await
    }

    /// The next message; `None` once the app-server is gone.
    async fn next(&mut self) -> Result<Option<Incoming>> {
        loop {
            let Some(line) = self.lines.next_line().await? else { return Ok(None) };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line).ok().and_then(incoming) {
                Some(message) => return Ok(Some(message)),
                None if self.stray.len() < 5 => self.stray.push(line.chars().take(200).collect()),
                None => {}
            }
        }
    }

    /// A request and its answer. Notifications meanwhile are kept for later;
    /// requests meanwhile are declined — nothing asks before a turn runs.
    async fn call(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value> {
        let id = self.request(method, params).await?;
        let answered = async {
            loop {
                match self.next().await? {
                    Some(Incoming::Answer { id: got, result }) if got == id => {
                        return result.map_err(|message| Error::Backend { message: format!("codex {method}: {message}"), code: None });
                    }
                    Some(Incoming::Answer { .. }) => {}
                    Some(Incoming::Notification { method, params }) => self.early.push((method, params)),
                    Some(Incoming::Request { id, method, params }) => {
                        let declined = reply(&method, &params, false);
                        self.answer(id, declined).await?;
                    }
                    None => return Err(Error::Protocol(format!("codex app-server ended before answering {method}{}", self.diagnostics()))),
                }
            }
        };
        match tokio::time::timeout(wait, answered).await {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("codex app-server didn't answer {method} in {}s", wait.as_secs())).into()),
        }
    }

    fn diagnostics(&self) -> String {
        if self.stray.is_empty() { String::new() } else { format!(": {}", self.stray.join(" · ")) }
    }

    /// Let it go: its input ends, and it exits (or is stopped).
    async fn close(self) -> String {
        let Server { mut child, stdin, stderr, stray, .. } = self;
        drop(stdin);
        if tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        let stderr = tokio::time::timeout(Duration::from_secs(1), stderr).await.ok().and_then(|r| r.ok()).unwrap_or_default();
        let mut said = stray;
        if !stderr.trim().is_empty() {
            said.push(stderr.trim().chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect());
        }
        said.join(" · ")
    }
}

/// A request of the app-server's waiting for the person: its id, method and
/// parameters, and whether they allowed it.
type Answering = std::pin::Pin<Box<dyn std::future::Future<Output = (Value, String, Value, bool)> + Send>>;

/// Run one turn through the person's Codex, in `cwd`, reporting what it does
/// as it happens. What the person says meanwhile comes on `steering`; what
/// Codex asks before acting goes out on `asks` (none: declined); a word on
/// `stop` interrupts the turn, which then winds down.
pub async fn run_turn(
    binary: &str,
    cwd: &Path,
    request: TurnRequest<'_>,
    mut steering: Option<mpsc::UnboundedReceiver<String>>,
    asks: Option<mpsc::UnboundedSender<Ask>>,
    stop: Option<oneshot::Receiver<()>>,
    mut on_activity: impl FnMut(Activity),
) -> Result<Turn> {
    // On another provider, through a relay on the loopback that keeps its tools plain.
    let relay = match request.provider {
        Some(provider) => Some(crate::relay::start(&provider.base_url).await?),
        None => None,
    };
    let args = match (request.provider, &relay) {
        (Some(provider), Some(relay)) => provider_args(provider, &relay.url),
        _ => Vec::new(),
    };
    let mut server = Server::start(binary, cwd, &args, request.env).await?;
    // Signed out, the turn would fail somewhere inside: say so first.
    let account = server.call("account/read", json!({}), ANSWER_WAIT).await?;
    if account.get("account").is_none_or(Value::is_null) && account.get("requiresOpenaiAuth").and_then(Value::as_bool) != Some(false) {
        server.close().await;
        return Err(Error::CodexSignedOut);
    }

    // Given servers of its own, the person's are left out: named, to be switched off.
    let others = match request.mcp_servers {
        Some(ours) => server
            .call("mcpServerStatus/list", json!({ "detail": "toolsAndAuthOnly", "limit": 200 }), THREAD_WAIT)
            .await
            .ok()
            .map(|listing| others_off(&listing, ours)),
        None => None,
    };
    // The member's thread, continued; a fresh one when there is none, or it is gone.
    let resumed = match request.resume {
        Some(id) => server.call("thread/resume", resume_params(&request, cwd, id, others.as_ref()), THREAD_WAIT).await.ok(),
        None => None,
    };
    let opened = match resumed {
        Some(opened) => opened,
        None => server.call("thread/start", start_params(&request, cwd, others.as_ref()), THREAD_WAIT).await?,
    };
    let Some(thread) = opened.get("thread").and_then(|t| t.get("id")).and_then(Value::as_str).map(str::to_owned) else {
        return Err(Error::Protocol("codex app-server started no thread".into()));
    };
    if let Some(model) = opened.get("model").and_then(Value::as_str) {
        on_activity(Activity::Model(model.to_owned()));
    }
    let mut collector = TurnCollector::new();
    collector.set_thread(&thread);
    let turn_request = server.request("turn/start", turn_params(&request, cwd, &thread)).await?;
    for (method, params) in std::mem::take(&mut server.early) {
        collector.on_notification(&method, &params).into_iter().for_each(&mut on_activity);
    }

    // Steers said before the turn had an id wait for it.
    let mut unsent: Vec<String> = Vec::new();
    let mut answers: FuturesUnordered<Answering> = FuturesUnordered::new();
    let mut stop = stop;
    let mut stopping: Option<tokio::time::Instant> = None;
    let mut interrupt_sent = false;
    let looped: Result<()> = async {
        loop {
            let steer = async {
                match &mut steering {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            };
            let stopped = async {
                match &mut stop {
                    Some(rx) => {
                        let _ = rx.await;
                    }
                    None => std::future::pending().await,
                }
            };
            let deadline = async {
                match stopping {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                message = server.next() => {
                    let Some(message) = message? else { break };
                    match message {
                        Incoming::Answer { id, result } if id == turn_request => match result {
                            Ok(started) => {
                                if let Some(id) = started.get("turn").and_then(|t| t.get("id")).and_then(Value::as_str) {
                                    collector.set_turn(id);
                                }
                            }
                            Err(message) => {
                                return Err(Error::Backend { message: format!("codex couldn't start the turn: {message}"), code: None });
                            }
                        },
                        Incoming::Answer { .. } => {}
                        Incoming::Notification { method, params } => {
                            collector.on_notification(&method, &params).into_iter().for_each(&mut on_activity);
                            if collector.done() {
                                break;
                            }
                        }
                        Incoming::Request { id, method, params } => match (ask_of(&method, &params, &collector, request.access), &asks) {
                            (Some((tool, input)), Some(asks)) => {
                                let (tx, rx) = oneshot::channel();
                                if asks.send(Ask { tool, input, answer: tx }).is_err() {
                                    server.answer(id, reply(&method, &params, false)).await?;
                                } else {
                                    answers.push(Box::pin(async move { (id, method, params, rx.await.unwrap_or(false)) }));
                                }
                            }
                            _ => server.answer(id, reply(&method, &params, false)).await?,
                        },
                    }
                }
                Some((id, method, params, allowed)) = answers.next(), if !answers.is_empty() => {
                    server.answer(id, reply(&method, &params, allowed)).await?;
                }
                text = steer => match text {
                    Some(text) => unsent.push(text),
                    None => steering = None,
                },
                () = stopped, if stopping.is_none() => {
                    stop = None;
                    stopping = Some(tokio::time::Instant::now() + INTERRUPT_WAIT);
                }
                () = deadline => break,
            }
            // Into the turn, once it has an id to aim at: steers, or the interrupt.
            if let Some(turn) = collector.turn().map(str::to_owned) {
                if stopping.is_some() && !interrupt_sent {
                    interrupt_sent = true;
                    unsent.clear();
                    server.request("turn/interrupt", json!({ "threadId": thread, "turnId": turn })).await?;
                }
                for text in unsent.drain(..) {
                    server.request("turn/steer", json!({ "threadId": thread, "input": [text_input(&text)], "expectedTurnId": turn })).await?;
                }
            }
        }
        Ok(())
    }
    .await;
    let diagnostics = server.close().await;
    let requests = relay.as_ref().map(crate::relay::Relay::exchanges).unwrap_or_default();
    // Stopped: however it wound down, it ended because it was told to.
    if stopping.is_some() {
        return Ok(Turn { result: collector.interrupted(), thread_id: Some(thread), provider_requests: requests });
    }
    looped?;
    collector.finish(&diagnostics).map(|result| Turn { result, thread_id: Some(thread), provider_requests: requests })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(method: &str, params: Value) -> (String, Value) {
        (method.to_owned(), params)
    }

    fn feed(collector: &mut TurnCollector, notes: &[(String, Value)]) -> Vec<Activity> {
        notes.iter().flat_map(|(m, p)| collector.on_notification(m, p)).collect()
    }

    fn item(kind: &str, fields: Value) -> Value {
        let mut item = fields;
        item["type"] = json!(kind);
        json!({ "threadId": "t", "turnId": "u", "item": item })
    }

    #[test]
    fn login_status_reads_only_how_it_is_signed_in() {
        assert_eq!(parse_login_status(true, "Logged in using ChatGPT\n"), SignIn::ChatGpt);
        assert_eq!(parse_login_status(true, "Logged in using an API key - sk-proj-***ABCDE"), SignIn::ApiKey);
        assert_eq!(parse_login_status(false, "Not logged in"), SignIn::SignedOut);
        assert_eq!(parse_login_status(true, "Not logged in"), SignIn::SignedOut);
        assert!(!SignIn::SignedOut.signed_in() && SignIn::ChatGpt.signed_in());
    }

    #[test]
    fn each_access_maps_to_a_sandbox_and_an_approval_policy() {
        let build = [PathBuf::from("/h/builds/fe")];
        assert_eq!((sandbox_mode(Access::ReadOnly), approval_policy(Access::ReadOnly)), ("read-only", "never"));
        assert_eq!((sandbox_mode(Access::EditFiles), approval_policy(Access::EditFiles)), ("workspace-write", "on-request"));
        assert_eq!((sandbox_mode(Access::Ask), approval_policy(Access::Ask)), ("workspace-write", "untrusted"));
        assert_eq!((sandbox_mode(Access::Full), approval_policy(Access::Full)), ("danger-full-access", "never"));
        assert_eq!(sandbox_policy(Access::ReadOnly, &build), json!({ "type": "readOnly", "networkAccess": false }));
        let edit = sandbox_policy(Access::EditFiles, &build);
        assert_eq!(edit["type"], "workspaceWrite");
        assert_eq!(edit["writableRoots"], json!(["/h/builds/fe"]), "the worktree (its cwd) and the build cache, nothing else");
        assert_eq!(edit["networkAccess"], false, "reaching out asks the person");
        assert_eq!(sandbox_policy(Access::Full, &build), json!({ "type": "dangerFullAccess" }));
    }

    #[test]
    fn a_thread_starts_in_the_worktree_with_the_charter_and_only_equipes_servers() {
        let servers = json!({ "equipe": mcp_server(Path::new("/opt/equipe"), &["mcp".into(), "--squad".into(), "acme".into()], true) });
        let request = TurnRequest { prompt: "hi", model: Some("gpt-6.1-sol"), access: Access::EditFiles, instructions: Some("You are qa."), mcp_servers: Some(&servers), ..Default::default() };
        let listing = json!({ "data": [
            { "name": "codex_apps", "pluginId": null },
            { "name": "shadcn", "pluginId": null },
            { "name": "cua_repl", "pluginId": "unified-computer-use@openai-bundled" },
            { "name": "equipe", "pluginId": null },
        ] });
        let others = others_off(&listing, &servers);
        let params = start_params(&request, Path::new("/w/t-3"), Some(&others));
        assert_eq!(params["cwd"], "/w/t-3");
        assert_eq!(params["sandbox"], "workspace-write");
        assert_eq!(params["approvalPolicy"], "on-request");
        assert_eq!(params["developerInstructions"], "You are qa.");
        assert_eq!(params["model"], "gpt-6.1-sol");
        assert_eq!(params["config"]["mcp_servers"]["equipe"]["command"], "/opt/equipe");
        assert_eq!(params["config"]["mcp_servers"]["equipe"]["default_tools_approval_mode"], "approve", "board tools never ask");
        assert_eq!(params["config"]["project_doc_fallback_filenames"], json!(["CLAUDE.md"]), "a CLAUDE.md-only project still tells Codex its conventions");
        // The person's own servers off, by name: their settings never read.
        assert_eq!(params["config"]["mcp_servers"]["shadcn"], json!({ "enabled": false }));
        assert!(params["config"]["mcp_servers"].get("codex_apps").is_none(), "Codex's own, off with its feature");
        assert_eq!(params["config"]["plugins"]["unified-computer-use@openai-bundled"], json!({ "enabled": false }));
        assert_eq!(params["config"]["features"]["apps"], false);
        assert!(params.get("ephemeral").is_none(), "kept, so the next turn continues it");
        let resume = resume_params(&request, Path::new("/w/t-3"), "thr-1", None);
        assert_eq!((resume["threadId"].as_str(), resume["developerInstructions"].as_str()), (Some("thr-1"), Some("You are qa.")));
        let turn = turn_params(&TurnRequest { effort: Some("high"), ..request }, Path::new("/w/t-3"), "thr-1");
        assert_eq!(turn["input"][0]["text"], "hi");
        assert_eq!(turn["effort"], "high");
        assert_eq!(turn["sandboxPolicy"]["type"], "workspaceWrite");
        // A member works at standard speed, whatever Fast the person's own Codex is set to.
        assert_eq!(turn["serviceTierForTurn"], "default");
        let plain = start_params(&TurnRequest { prompt: "hi", ..Default::default() }, Path::new("/r"), None);
        assert!(plain["config"].get("mcp_servers").is_none(), "without servers of equipe's, the person's own");
        assert!(plain.get("model").is_none() && plain.get("developerInstructions").is_none());
    }

    #[test]
    fn a_command_shows_as_what_it_does() {
        let cmd = |command: &str, actions: Value| command_tool(&json!({ "command": command, "commandActions": actions }));
        assert_eq!(cmd("/bin/zsh -lc 'cat a.rs'", json!([{ "type": "read", "command": "cat a.rs", "name": "a.rs", "path": "/w/a.rs" }])), ("Read".into(), Some("/w/a.rs".into())));
        assert_eq!(cmd("/bin/zsh -lc 'rg foo'", json!([{ "type": "search", "command": "rg foo", "query": "foo", "path": null }])), ("Grep".into(), Some("foo".into())));
        assert_eq!(cmd("/bin/zsh -lc 'ls src'", json!([{ "type": "listFiles", "command": "ls src", "path": "src" }])), ("Glob".into(), Some("src".into())));
        assert_eq!(cmd("/bin/bash -lc 'cargo test && echo '\\''ok'\\'''", json!([{ "type": "unknown" }, { "type": "unknown" }])), ("Bash".into(), Some("cargo test && echo 'ok'".into())));
        assert_eq!(unwrap_shell("npm test"), "npm test");
        assert_eq!(unwrap_shell("bash -c \"echo \\\"hi\\\"\""), "echo \"hi\"");
    }

    #[test]
    fn a_turn_streams_text_tools_and_its_outcome() {
        let mut c = TurnCollector::new();
        c.set_thread("t");
        let out = feed(&mut c, &[
            note("turn/started", json!({ "threadId": "t", "turn": { "id": "u" } })),
            note("item/completed", item("userMessage", json!({ "id": "m0", "content": [{ "type": "text", "text": "do it" }] }))),
            note("item/reasoning/summaryTextDelta", json!({ "threadId": "t", "delta": "Looking" })),
            note("item/agentMessage/delta", json!({ "threadId": "t", "itemId": "a1", "delta": "On it." })),
            note("item/completed", item("agentMessage", json!({ "id": "a1", "text": "On it." }))),
            note("item/started", item("commandExecution", json!({ "id": "c1", "command": "/bin/zsh -lc 'cargo test'", "commandActions": [{ "type": "unknown" }], "status": "inProgress" }))),
            note("item/completed", item("commandExecution", json!({ "id": "c1", "command": "/bin/zsh -lc 'cargo test'", "commandActions": [{ "type": "unknown" }], "status": "failed", "exitCode": 101, "aggregatedOutput": "1 failed", "durationMs": 900 }))),
            note("item/agentMessage/delta", json!({ "threadId": "t", "itemId": "a2", "delta": "Fixed." })),
            note("item/completed", item("agentMessage", json!({ "id": "a2", "text": "Fixed." }))),
            note("item/agentMessage/delta", json!({ "threadId": "t", "itemId": "a3", "delta": "DONE" })),
            note("item/completed", item("agentMessage", json!({ "id": "a3", "text": "DONE", "phase": "final_answer" }))),
            // A subagent's thread is its own.
            note("item/agentMessage/delta", json!({ "threadId": "other", "itemId": "x", "delta": "not mine" })),
            note("thread/tokenUsage/updated", json!({ "threadId": "t", "tokenUsage": { "last": { "totalTokens": 1500, "inputTokens": 1400, "cachedInputTokens": 1000, "outputTokens": 100 }, "modelContextWindow": 258400 } })),
            note("turn/completed", json!({ "threadId": "t", "turn": { "id": "u", "status": "completed" } })),
        ]);
        assert!(c.done());
        assert_eq!(out[0], Activity::Thinking("Looking".into()));
        let text: String = out.iter().filter_map(|a| if let Activity::Text(t) = a { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "On it.Fixed.\n\nDONE", "a tool between messages parts them already; back to back they get a paragraph");
        assert!(out.contains(&Activity::Tool { name: "Bash".into(), target: Some("cargo test".into()), label: None, background: false }));
        let Some(Activity::ToolDone(failed)) = out.iter().find(|a| matches!(a, Activity::ToolDone(_))) else { panic!("{out:?}") };
        assert_eq!((failed.ok, failed.exit, failed.ms, failed.output.as_str()), (false, Some(101), 900, "1 failed"));
        assert!(out.contains(&Activity::Output(100)));
        let result = c.finish("").unwrap();
        assert_eq!((result.text.as_str(), result.status.as_str()), ("DONE", "completed"));
        let usage = result.usage.unwrap();
        assert_eq!((usage.input_tokens, usage.cache_tokens, usage.output_tokens), (Some(400), Some(1000), Some(100)));
        assert_eq!((usage.context_tokens, usage.context_window), (Some(1500), Some(258400)));
    }

    #[test]
    fn a_patch_shows_each_file_with_its_change() {
        let mut c = TurnCollector::new();
        let changes = json!([
            { "path": "/w/hello.txt", "kind": { "type": "add" }, "diff": "hi\n" },
            { "path": "/w/README.md", "kind": { "type": "update", "move_path": null }, "diff": "@@ -1 +1 @@\n-hello\n+hello, world\n" },
        ]);
        let started = c.on_notification("item/started", &item("fileChange", json!({ "id": "p1", "changes": changes, "status": "inProgress" })));
        assert_eq!(started, vec![
            Activity::Tool { name: "Write".into(), target: Some("/w/hello.txt".into()), label: None, background: false },
            Activity::Tool { name: "Edit".into(), target: Some("/w/README.md".into()), label: None, background: false },
        ]);
        let done: Vec<ToolDone> = c
            .on_notification("item/completed", &item("fileChange", json!({ "id": "p1", "changes": changes, "status": "completed" })))
            .into_iter()
            .filter_map(|a| if let Activity::ToolDone(d) = a { Some(d) } else { None })
            .collect();
        assert_eq!(done.len(), 2);
        assert_eq!(done[0].diff.as_deref(), Some("diff --git a//w/hello.txt b//w/hello.txt\nnew file mode 100644\n--- /dev/null\n+++ b//w/hello.txt\n@@ -0,0 +1,1 @@\n+hi\n"));
        assert_eq!(done[1].diff.as_deref(), Some("diff --git a//w/README.md b//w/README.md\n--- a//w/README.md\n+++ b//w/README.md\n@@ -1 +1 @@\n-hello\n+hello, world\n"));
        assert!(done.iter().all(|d| d.ok && d.output.is_empty()));
    }

    #[test]
    fn board_tools_web_searches_and_plans_read_as_claude_codes_do() {
        let mut c = TurnCollector::new();
        let call = json!({ "id": "m1", "server": "equipe", "tool": "update_ticket", "arguments": { "id": 4, "status": "cancelled" }, "status": "inProgress" });
        assert_eq!(c.on_notification("item/started", &item("mcpToolCall", call.clone())), vec![Activity::Tool { name: "mcp__equipe__update_ticket".into(), target: Some("T-4 · cancelled".into()), label: None, background: false }]);
        let mut finished = call;
        finished["status"] = json!("completed");
        finished["result"] = json!({ "content": [{ "type": "text", "text": "T-4 cancelled." }] });
        assert!(matches!(&c.on_notification("item/completed", &item("mcpToolCall", finished))[..], [Activity::ToolDone(d)] if d.ok && d.name == "mcp__equipe__update_ticket"));
        let search = c.on_notification("item/completed", &item("webSearch", json!({ "id": "w1", "query": "codex app-server" })));
        assert_eq!(search[0], Activity::Tool { name: "WebSearch".into(), target: Some("codex app-server".into()), label: None, background: false });
        let plan = json!({ "threadId": "t", "plan": [{ "step": "Read the code", "status": "completed" }, { "step": "Write the test", "status": "inProgress" }, { "step": "Fix", "status": "pending" }] });
        assert_eq!(c.on_notification("turn/plan/updated", &plan)[0], Activity::Tool { name: "TodoWrite".into(), target: Some("1/3 · Write the test".into()), label: None, background: false });
        assert!(c.on_notification("turn/plan/updated", &plan).is_empty(), "the same plan again shows nothing new");
    }

    #[test]
    fn a_steer_read_mid_turn_is_echoed_and_the_prompt_is_not() {
        let mut c = TurnCollector::new();
        let said = |text: &str| item("userMessage", json!({ "id": text, "content": [{ "type": "text", "text": text }] }));
        assert!(c.on_notification("item/completed", &said("the prompt")).is_empty());
        assert_eq!(c.on_notification("item/completed", &said("not the footer")), vec![Activity::Steered("not the footer".into())]);
    }

    #[test]
    fn usage_windows_are_known_by_their_length_and_updates_merge() {
        // Pro has no five-hour window: the first one listed is the weekly one.
        let read = json!({ "limitId": "codex", "primary": { "usedPercent": 18, "windowDurationMins": 10080, "resetsAt": 1761000000 }, "secondary": null, "planType": "pro", "credits": { "hasCredits": true, "unlimited": false, "balance": "12.50" } });
        let mut limits = parse_limits(&read).unwrap();
        assert_eq!(limits.window(crate::turn::SEVEN_DAYS).map(|w| w.used_percent), Some(18.0));
        assert!(limits.window(crate::turn::FIVE_HOURS).is_none());
        assert_eq!((limits.plan_type.as_deref(), limits.credits.clone()), (Some("pro"), Some((true, Some("12.50".into())))));
        // A sparse update: a five-hour window appears, the plan isn't said again.
        let update = json!({ "limitId": "codex", "primary": { "usedPercent": 42, "windowDurationMins": 300, "resetsAt": 1760630400 }, "secondary": null, "planType": null, "credits": null });
        limits.merge(parse_limits(&update).unwrap());
        assert_eq!(limits.window(crate::turn::FIVE_HOURS).map(|w| (w.used_percent, w.resets_at)), Some((42.0, Some(1760630400))));
        assert_eq!(limits.window(crate::turn::SEVEN_DAYS).map(|w| w.used_percent), Some(18.0), "what an update leaves out stays");
        assert_eq!(limits.plan_type.as_deref(), Some("pro"));
        assert!(parse_limits(&json!({ "limitId": "gpt-6-astra", "primary": { "usedPercent": 99 } })).is_none(), "a model's own bucket is not the plan's");
        let mut c = TurnCollector::new();
        assert!(matches!(&c.on_notification("account/rateLimits/updated", &json!({ "rateLimits": read }))[..], [Activity::Limits(l)] if l.plan_type.as_deref() == Some("pro")));
    }

    #[test]
    fn the_catalog_is_the_accounts_default_first_without_hidden_models() {
        let listing = json!({ "data": [
            { "id": "gpt-6-luna", "hidden": false, "isDefault": false },
            { "id": "gpt-6.1-sol", "hidden": false, "isDefault": true },
            { "id": "codex-auto-review", "hidden": true, "isDefault": false },
            { "id": "gpt-6-astra", "hidden": false, "isDefault": false },
        ], "nextCursor": null });
        assert_eq!(model_ids(&listing), ["gpt-6.1-sol", "gpt-6-luna", "gpt-6-astra"]);
    }

    #[test]
    fn approvals_ask_in_claude_codes_words_and_answer_in_codexs() {
        let mut c = TurnCollector::new();
        c.on_notification("item/started", &item("fileChange", json!({ "id": "p1", "changes": [{ "path": "/etc/hosts", "kind": { "type": "update" }, "diff": "" }] })));
        let exec = json!({ "itemId": "c1", "command": "/bin/zsh -lc 'git push origin main'", "reason": "needs the network", "kind": "command" });
        let (tool, input) = ask_of("item/commandExecution/requestApproval", &exec, &c, Access::EditFiles).unwrap();
        assert_eq!(tool, "Bash");
        assert_eq!(input, json!({ "command": "git push origin main", "description": "needs the network", "dangerouslyDisableSandbox": true }));
        let (_, asked) = ask_of("item/commandExecution/requestApproval", &exec, &c, Access::Ask).unwrap();
        assert!(asked.get("dangerouslyDisableSandbox").is_none(), "asked in ask access, it still runs fenced in");
        let patch = json!({ "itemId": "p1", "reason": null });
        assert_eq!(ask_of("item/fileChange/requestApproval", &patch, &c, Access::Ask), Some(("Edit".into(), json!({ "file_path": "/etc/hosts" }))));
        assert_eq!(reply("item/commandExecution/requestApproval", &exec, true).unwrap(), json!({ "decision": "accept" }));
        assert_eq!(reply("item/fileChange/requestApproval", &patch, false).unwrap(), json!({ "decision": "decline" }));
        assert!(ask_of("item/tool/call", &json!({}), &c, Access::Ask).is_none());
        assert!(reply("item/tool/call", &json!({}), false).is_err(), "equipe registers no tools of its own there");
        assert_eq!(reply("item/tool/requestUserInput", &json!({}), false).unwrap(), json!({ "answers": {} }));
    }

    #[test]
    fn a_failed_turn_says_why_and_an_unauthorized_one_how_to_sign_in() {
        let failed = |info: Value| {
            let mut c = TurnCollector::new();
            c.on_notification("turn/completed", &json!({ "turn": { "id": "u", "status": "failed", "error": { "message": "You've hit your usage limit.", "codexErrorInfo": info } } }));
            c.finish("")
        };
        assert!(matches!(failed(json!("usageLimitExceeded")), Err(Error::Backend { message, code: Some(code) }) if message.contains("usage limit") && code == "usageLimitExceeded"));
        assert!(matches!(failed(json!("unauthorized")), Err(Error::CodexSignedOut)));
        assert!(matches!(failed(json!({ "httpConnectionFailed": { "httpStatusCode": 502 } })), Err(Error::Backend { code: Some(code), .. }) if code == "httpConnectionFailed"));
        assert!(matches!(TurnCollector::new().finish("boom"), Err(Error::Protocol(m)) if m.contains("boom")));
    }

    #[test]
    fn a_change_reads_as_a_git_diff_whatever_codex_sent() {
        assert!(change_diff("/w/a", "delete", "x\ny\n").contains("--- a//w/a\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n"));
        assert_eq!(change_diff("/w/a", "update", "--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y"), "diff --git a//w/a b//w/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n");
    }

    #[test]
    fn the_account_says_who_and_on_what_plan() {
        let chatgpt = json!({ "account": { "type": "chatgpt", "email": "abel@example.com", "planType": "free" }, "requiresOpenaiAuth": true });
        assert_eq!(super::account_of(&chatgpt), Some(super::Account { email: Some("abel@example.com".into()), plan: Some("free".into()), api_key: false }));
        assert_eq!(super::account_of(&json!({ "account": { "type": "apiKey" } })), Some(super::Account { api_key: true, ..Default::default() }));
        assert_eq!(super::account_of(&json!({ "account": null, "requiresOpenaiAuth": true })), None);
    }
}
