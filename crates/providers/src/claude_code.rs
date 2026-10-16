//! "Bring your own Claude Code": run the user's installed, unmodified `claude`
//! binary as a `cli` engine.
//!
//! Anthropic's terms forbid third-party tools from offering Claude.ai login or
//! collecting, storing or forwarding Claude credentials, but allow an end user to
//! sign in to the unmodified Claude Code binary with their own subscription. So
//! equipe never touches Claude credentials: the user signs in through Anthropic's
//! own flow (`claude auth login`), equipe asks Claude Code for its status, and
//! every request authenticates inside Claude Code itself.
//!
//! The environment is passed through unchanged. Stripping auth variables would
//! restrict the user's own choice of sign-in method, which the terms also forbid;
//! instead `status` reports which credential Claude Code will use.

use std::path::Path;
use std::process::Stdio;

use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::error::{Error, Result};
use crate::turn::{TurnResult, TurnUsage};

/// The executable equipe runs unless told otherwise.
pub const DEFAULT_BINARY: &str = "claude";

/// What `claude auth status --json` reports. Only non-secret fields are read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub logged_in: bool,
    /// e.g. `claude.ai` for a subscription login.
    #[serde(default)]
    pub auth_method: Option<String>,
    /// e.g. `firstParty`, or a cloud provider.
    #[serde(default)]
    pub api_provider: Option<String>,
    /// e.g. `pro` or `max`.
    #[serde(default)]
    pub subscription_type: Option<String>,
}

impl AuthStatus {
    /// Whether requests will draw on a Claude subscription rather than per-token billing.
    pub fn uses_subscription(&self) -> bool {
        self.logged_in && self.auth_method.as_deref() == Some("claude.ai")
    }
}

pub fn parse_auth_status(json: &str) -> Result<AuthStatus> {
    serde_json::from_str(json)
        .map_err(|_| Error::Protocol("`claude auth status --json` returned something unexpected".into()))
}

/// How long `claude auth status` may take.
const AUTH_STATUS_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Ask the installed Claude Code whether it is signed in.
/// `Ok(None)` means the binary isn't installed (or not on PATH).
pub async fn auth_status(binary: &str) -> Result<Option<AuthStatus>> {
    let asked = Command::new(binary).args(["auth", "status", "--json"]).stdin(Stdio::null()).kill_on_drop(true).output();
    // A sign-in check that never answers must not hold up whoever asked (the daemon's loop).
    let Ok(output) = tokio::time::timeout(AUTH_STATUS_WAIT, asked).await else {
        let secs = AUTH_STATUS_WAIT.as_secs();
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("`{binary} auth status` didn't answer in {secs}s")).into());
    };
    let output = match output {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    // A signed-out Claude Code may exit non-zero yet still print its status.
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(Some(AuthStatus {
            logged_in: false,
            auth_method: None,
            api_provider: None,
            subscription_type: None,
        }));
    }
    parse_auth_status(&stdout).map(Some)
}

/// What a turn may do to the files in its working directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Access {
    /// Read and answer; anything that would need approval is denied (`dontAsk`).
    #[default]
    ReadOnly,
    /// Also write files inside the working directory without asking
    /// (`acceptEdits`). Anything else needs approval: from the prompt tool when
    /// there is one, otherwise it is denied.
    EditFiles,
    /// Ask before every change and command (`manual`).
    Ask,
    /// Ask nothing (`bypassPermissions`); only disallowed tools stay out.
    Full,
}

impl Access {
    fn permission_mode(self) -> &'static str {
        match self {
            Access::ReadOnly => "dontAsk",
            Access::EditFiles => "acceptEdits",
            Access::Ask => "manual",
            Access::Full => "bypassPermissions",
        }
    }
}

/// Claude Code's own sandbox: shell commands run fenced in by the operating
/// system — writing only inside the working directory, reaching only the
/// network hosts named — and need no approval while they stay inside. A command
/// that must run outside the fence asks first (through the prompt tool), and runs
/// outside only when allowed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sandbox<'a> {
    pub allowed_domains: &'a [String],
    /// Commands that ask anyway, sandboxed or not — whatever reaches outside
    /// (`Bash(git push:*)`).
    pub ask: &'a [String],
    /// Paths no fenced command may read: the person's sign-ins and logins.
    pub deny_read: &'a [String],
    /// Paths no fenced command may write: directories added for reading only.
    pub deny_write: &'a [String],
    /// Paths outside the working directory fenced commands may write: a build cache.
    pub allow_write: &'a [String],
}

/// Arguments for one headless turn.
///
/// `--bare` is deliberately absent: bare mode never reads the subscription login,
/// which is the whole point of bringing your own Claude Code.
pub fn turn_args(request: &TurnRequest<'_>) -> Vec<String> {
    // The prompt goes in on stdin, as the first of the turn's messages: more
    // can follow while it runs — the person steering it — and Claude Code reads
    // each at its next step, echoing it back as it does.
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--replay-user-messages",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        request.access.permission_mode(),
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(model) = request.model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(effort) = request.effort {
        args.extend(["--effort".to_owned(), effort.to_owned()]);
    }
    // Continue the conversation Claude Code already holds, rather than resending it.
    if let Some(id) = request.resume {
        args.extend(["--resume".to_owned(), id.to_owned()]);
    }
    // Added to Claude Code's own system prompt, never replacing it.
    if let Some(charter) = request.charter {
        args.extend(["--append-system-prompt".to_owned(), charter.to_owned()]);
    }
    // Settings of the unmodified binary, not changes to it: Claude Code's auto
    // memory moved to a directory of the caller's choosing, and its sandbox.
    let mut settings = serde_json::Map::new();
    if let Some(dir) = request.memory {
        settings.insert("autoMemoryDirectory".into(), dir.to_string_lossy().into());
    }
    if let Some(sandbox) = &request.sandbox {
        settings.insert(
            "sandbox".into(),
            serde_json::json!({
                "enabled": true,
                "autoAllowBashIfSandboxed": true,
                "allowUnsandboxedCommands": true,
                // Serving a build and pointing a browser or a test at it is
                // everyday work; on macOS that needs local ports, else every
                // preview leaves the sandbox and asks the person.
                "network": { "allowedDomains": sandbox.allowed_domains, "allowLocalBinding": true },
                "filesystem": { "denyRead": sandbox.deny_read, "denyWrite": sandbox.deny_write, "allowWrite": sandbox.allow_write },
            }),
        );
    }
    // What asks each time: whatever reaches outward from the sandbox, and tools
    // asked about by name. An ask rule outranks an allow rule.
    let ask: Vec<&String> = request.sandbox.iter().flat_map(|s| s.ask.iter()).chain(request.ask_tools).collect();
    let mut permissions = serde_json::Map::new();
    if !ask.is_empty() {
        permissions.insert("ask".into(), serde_json::json!(ask));
    }
    // What is refused in every mode, bypass included: a deny rule outranks all.
    if !request.deny.is_empty() {
        permissions.insert("deny".into(), serde_json::json!(request.deny));
    }
    if !permissions.is_empty() {
        settings.insert("permissions".into(), Value::Object(permissions));
    }
    if !settings.is_empty() {
        args.extend(["--settings".to_owned(), Value::Object(settings).to_string()]);
    }
    // Questions a headless run can't put to anyone go to this tool instead.
    if let Some(tool) = request.prompt_tool {
        args.extend(["--permission-prompts".to_owned(), "host".to_owned(), "--permission-prompt-tool".to_owned(), tool.to_owned()]);
    }
    if !request.disallowed_tools.is_empty() {
        args.push("--disallowedTools".to_owned());
        args.extend(request.disallowed_tools.iter().cloned());
    }
    for dir in request.add_dirs {
        args.extend(["--add-dir".to_owned(), dir.to_string_lossy().into_owned()]);
    }
    // Only the MCP servers given here — never the person's own (their mail, their
    // calendar) — and the tools named, which a headless run could not otherwise use.
    if let Some(config) = request.mcp_config {
        args.extend(["--mcp-config".to_owned(), config.to_owned(), "--strict-mcp-config".to_owned()]);
    }
    if !request.allowed_tools.is_empty() {
        args.push("--allowedTools".to_owned());
        args.extend(request.allowed_tools.iter().cloned());
    }
    args
}

/// One turn's input.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnRequest<'a> {
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    pub access: Access,
    /// Claude Code's session id from an earlier turn, to continue that conversation.
    pub resume: Option<&'a str>,
    /// Who the agent is on the team, appended to Claude Code's system prompt.
    pub charter: Option<&'a str>,
    /// Where Claude Code keeps what it learns (its auto memory). `None` leaves it
    /// where Claude Code would put it — which is per directory and shared with the
    /// person's own sessions there.
    pub memory: Option<&'a Path>,
    /// More directories the turn may read (and, with edit access, write).
    pub add_dirs: &'a [std::path::PathBuf],
    /// MCP servers for this turn, as Claude Code's `--mcp-config` JSON. Replaces
    /// the person's own servers for the turn.
    pub mcp_config: Option<&'a str>,
    /// Tools allowed without asking, e.g. `mcp__equipe` for a whole server.
    pub allowed_tools: &'a [String],
    /// Tools never allowed, whatever the mode: `Bash(sudo:*)`.
    pub disallowed_tools: &'a [String],
    /// Tools that ask each time even where a wider rule allows them.
    pub ask_tools: &'a [String],
    /// Rules refused in every mode: `Edit(//code/app/**)` for a directory added
    /// for reading only.
    pub deny: &'a [String],
    pub sandbox: Option<Sandbox<'a>>,
    /// The MCP tool that answers permission prompts (`mcp__equipe__approve`).
    pub prompt_tool: Option<&'a str>,
    /// More environment for the turn's process, and so for its commands.
    pub env: &'a [(String, String)],
    /// How hard the model thinks: `--effort low|medium|high|xhigh|max`; `None`, its default.
    pub effort: Option<&'a str>,
}

/// One turn's outcome, with the id that continues the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub result: TurnResult,
    pub session_id: Option<String>,
}

/// Something worth showing while a turn runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Activity {
    /// A fragment of the answer.
    Text(String),
    /// Claude Code is using a tool, e.g. `Edit` on a file.
    Tool { name: String, target: Option<String> },
    /// A tool use came back (the `Tool` with this `name` and `target`).
    ToolDone(ToolDone),
    /// A fragment of the model's reasoning, as the engine shows it — not
    /// part of the answer.
    Thinking(String),
    /// The model Claude Code is running, reported at startup.
    Model(String),
    /// The plan's usage windows: five-hour and seven-day, each as the share
    /// used (0–1) and when it resets (Unix seconds).
    Plan { five_hour: Option<(f64, i64)>, seven_day: Option<(f64, i64)> },
    /// Output the turn has produced so far, in tokens.
    Output(u64),
    /// The member handed `what` to a subagent (Claude Code's Agent tool).
    SubagentStarted { id: String, what: String },
    /// A subagent's latest step: a tool and what it acts on.
    SubagentStep { id: String, step: String },
    /// A subagent finished and handed back what it found.
    SubagentEnded { id: String },
    /// A message sent while the turn ran was read, at this point of it.
    Steered(String),
}

/// How a tool use ended, as Claude Code answered it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDone {
    pub name: String,
    pub target: Option<String>,
    pub ok: bool,
    /// A command's exit code, when it said one.
    pub exit: Option<i32>,
    pub ms: u64,
    /// What it printed (or answered), whole.
    pub output: String,
    /// An edit's or a write's change, as a unified diff headed `diff --git`.
    pub diff: Option<String>,
}

/// A tool use waiting for its answer.
struct Pending {
    name: String,
    target: Option<String>,
    since: std::time::Instant,
    /// The change the call asks for, worked out from its input.
    diff: Option<String>,
}

/// Past this many lines a diff carried with a tool's outcome is cut.
const DIFF_LINES: usize = 400;

/// Folds Claude Code's `stream-json` lines into one turn.
#[derive(Default)]
pub struct TurnCollector {
    /// The member's own tool uses not answered yet, by id.
    pending: std::collections::HashMap<String, Pending>,
    streamed: String,
    last_message: String,
    model: Option<String>,
    session_id: Option<String>,
    /// How full the context was at the latest request: its input, cached or not, and output.
    context: Option<u64>,
    /// Output tokens of the turn's finished requests.
    output: u64,
    result: Option<Result<TurnResult>>,
    /// Subagents handed work this turn and not yet back.
    subagents: std::collections::HashSet<String>,
    /// The person's messages Claude Code has read so far — the prompt first.
    replayed: usize,
}

/// A subagent run in the background answers its tool call at once with this;
/// it hands back later, in a task notification.
const LAUNCHED_IN_BACKGROUND: &str = "Async agent launched";

impl TurnCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The model Claude Code reported at startup, once it has.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The id that continues this conversation with `--resume`, once reported.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Apply one event. Returns what it shows: answer text and tool use.
    pub fn on_event(&mut self, event: &Value) -> Vec<Activity> {
        if let Some(id) = event.get("session_id").and_then(Value::as_str).filter(|id| !id.is_empty()) {
            self.session_id = Some(id.to_owned());
        }
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                self.model = event.get("model").and_then(Value::as_str).map(str::to_owned);
                self.model.clone().map(Activity::Model).into_iter().collect()
            }
            // A subagent at work in the background: where it is, and when it is done.
            "system" => {
                let id = event.get("tool_use_id").and_then(Value::as_str).filter(|id| self.subagents.contains(*id));
                match (event.get("subtype").and_then(Value::as_str), id) {
                    (Some("task_progress"), Some(id)) => event
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|step| Activity::SubagentStep { id: id.to_owned(), step: step.chars().take(80).collect() })
                        .into_iter()
                        .collect(),
                    (Some("task_notification"), Some(id)) => {
                        let id = id.to_owned();
                        self.subagents.remove(&id);
                        vec![Activity::SubagentEnded { id }]
                    }
                    _ => Vec::new(),
                }
            }
            "rate_limit_event" => {
                let windows = event.get("rate_limit_info").and_then(|i| i.get("unifiedWindows"));
                let window = |name: &str| {
                    let w = windows?.get(name)?;
                    Some((w.get("utilization")?.as_f64()?, w.get("resetsAt")?.as_i64()?))
                };
                let (five_hour, seven_day) = (window("five_hour"), window("seven_day"));
                if five_hour.is_none() && seven_day.is_none() {
                    return Vec::new();
                }
                vec![Activity::Plan { five_hour, seven_day }]
            }
            // A request finished: its output counts toward the turn's.
            "stream_event"
                if event.get("event").and_then(|e| e.get("type")).and_then(Value::as_str) == Some("message_delta") =>
            {
                let tokens = event.get("event").and_then(|e| e.get("usage")).and_then(|u| u.get("output_tokens")).and_then(Value::as_u64);
                match tokens {
                    Some(n) => {
                        self.output += n;
                        vec![Activity::Output(self.output)]
                    }
                    None => Vec::new(),
                }
            }
            // A subagent's words are its own: the member's answer is the member's.
            "stream_event" if parent(event).is_some() => Vec::new(),
            "stream_event" => match (text_delta(event), thinking_delta(event)) {
                (Some(text), _) => {
                    self.streamed.push_str(text);
                    vec![Activity::Text(text.to_owned())]
                }
                (None, Some(thought)) => vec![Activity::Thinking(thought.to_owned())],
                (None, None) => Vec::new(),
            },
            "assistant" => {
                if let Some(u) = event.get("message").and_then(|m| m.get("usage")) {
                    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    self.context = Some(
                        n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens") + n("output_tokens"),
                    );
                }
                // Inside a subagent: its steps, as the subagent's.
                if let Some(id) = parent(event) {
                    return tool_uses(event)
                        .into_iter()
                        .filter_map(|a| match a {
                            Activity::Tool { name, target } => {
                                Some(Activity::SubagentStep { id: id.to_owned(), step: format!("{name} {}", target.unwrap_or_default()).trim().to_owned() })
                            }
                            _ => None,
                        })
                        .collect();
                }
                let text = message_text(event);
                if !text.is_empty() {
                    self.last_message = text;
                }
                let mut out = tool_uses(event);
                // Each call waits for its answer, timed from now.
                let now = std::time::Instant::now();
                for block in content_blocks(event).filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")) {
                    if let (Some(id), Some(name)) = (block.get("id").and_then(Value::as_str), block.get("name").and_then(Value::as_str)) {
                        let input = block.get("input");
                        let pending = Pending { name: name.to_owned(), target: tool_target(name, input), since: now, diff: input.and_then(|i| input_diff(name, i)) };
                        self.pending.insert(id.to_owned(), pending);
                    }
                }
                // Handing work to a subagent: followed until it hands back.
                for block in content_blocks(event) {
                    if let (Some("tool_use"), Some(name), Some(id)) = (
                        block.get("type").and_then(Value::as_str),
                        block.get("name").and_then(Value::as_str),
                        block.get("id").and_then(Value::as_str),
                    ) && matches!(name, "Agent" | "Task")
                    {
                        let what = block.get("input").and_then(|i| i.get("description")).and_then(Value::as_str).unwrap_or("a subagent");
                        self.subagents.insert(id.to_owned());
                        out.push(Activity::SubagentStarted { id: id.to_owned(), what: what.chars().take(80).collect() });
                    }
                }
                out
            }
            // The person's own message, read: the prompt first, then whatever
            // they sent while the turn ran.
            "user" if event.get("isReplay").and_then(Value::as_bool) == Some(true) => {
                self.replayed += 1;
                let text = match event.get("message").and_then(|m| m.get("content")) {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
                    _ => String::new(),
                };
                if self.replayed > 1 && !text.is_empty() { vec![Activity::Steered(text)] } else { Vec::new() }
            }
            // Tools answering: each call's outcome; and a subagent handing back —
            // unless the answer is that it went off to work in the background.
            "user" => {
                let mut out = Vec::new();
                let structured = event.get("tool_use_result");
                for block in content_blocks(event).filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                    let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else { continue };
                    let text = result_text(block);
                    if let Some(pending) = self.pending.remove(id) {
                        out.push(Activity::ToolDone(tool_done(pending, block, &text, structured)));
                    }
                    if !text.starts_with(LAUNCHED_IN_BACKGROUND) && self.subagents.remove(id) {
                        out.push(Activity::SubagentEnded { id: id.to_owned() });
                    }
                }
                out
            }
            "result" => {
                let mut result = result_from(event, &self.streamed, &self.last_message);
                if let Ok(TurnResult { usage: Some(usage), .. }) = &mut result {
                    usage.context_tokens = self.context;
                }
                self.result = Some(result);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The turn, once the stream has ended. `diagnostics` is what to show when
    /// Claude Code exited without ever reporting a result.
    pub fn finish(self, diagnostics: &str) -> Result<Turn> {
        let session_id = self.session_id;
        let result = match self.result {
            Some(result) => result,
            None if !self.streamed.is_empty() => Ok(TurnResult {
                text: self.streamed,
                status: "incomplete".into(),
                usage: None,
            }),
            None => Err(Error::Protocol(format!(
                "Claude Code ended without a result{}",
                if diagnostics.is_empty() { String::new() } else { format!(": {diagnostics}") }
            ))),
        };
        result.map(|result| Turn { result, session_id })
    }
}

/// The subagent an event comes from, when it comes from one.
/// A tool result's text, whether it came as a string or as blocks.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn parent(event: &Value) -> Option<&str> {
    event.get("parent_tool_use_id").and_then(Value::as_str).filter(|p| !p.is_empty())
}

fn content_blocks(event: &Value) -> impl Iterator<Item = &Value> {
    event.get("message").and_then(|m| m.get("content")).and_then(Value::as_array).into_iter().flatten()
}

/// The text of a `text_delta`; thinking and signature deltas are not output.
fn text_delta(event: &Value) -> Option<&str> {
    let delta = event.get("event")?.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta.get("text").and_then(Value::as_str).filter(|t| !t.is_empty())
}

/// The text of a `thinking_delta`: the model's reasoning as the engine shows it.
fn thinking_delta(event: &Value) -> Option<&str> {
    let delta = event.get("event")?.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("thinking_delta") {
        return None;
    }
    delta.get("thinking").and_then(Value::as_str).filter(|t| !t.is_empty())
}

/// `tool_use` blocks of an assistant message, with the thing each one acts on.
fn tool_uses(event: &Value) -> Vec<Activity> {
    event
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|block| {
            let name = block.get("name").and_then(Value::as_str)?.to_owned();
            let target = tool_target(&name, block.get("input"));
            Some(Activity::Tool { name, target })
        })
        .collect()
}

/// The thing a tool use acts on, in one line.
fn tool_target(name: &str, input: Option<&Value>) -> Option<String> {
    board_target(name, input).or_else(|| {
        // `element` and `text`: what a browser step clicks or types into, in words;
        // `app`, `keys`, `window`: a desktop step's.
        ["file_path", "path", "notebook_path", "pattern", "command", "url", "action", "element", "text", "description", "app", "keys", "window"]
            .iter()
            .find_map(|key| input?.get(*key).and_then(Value::as_str))
            .map(|t| t.lines().next().unwrap_or("").chars().take(120).collect())
    })
}

/// A call's answer: whether it worked, a command's exit code, what it printed,
/// and an edit's change — Claude Code's own patch when it sent one.
fn tool_done(pending: Pending, block: &Value, text: &str, structured: Option<&Value>) -> ToolDone {
    let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
    let (exit, output) = exit_code(text);
    let path = pending.target.clone().unwrap_or_default();
    let patch = structured.and_then(|s| s.get("structuredPatch")).and_then(|p| structured_diff(&path, p));
    let ok = !failed && exit.is_none_or(|code| code == 0);
    ToolDone {
        ok,
        exit,
        ms: u64::try_from(pending.since.elapsed().as_millis()).unwrap_or(u64::MAX),
        // What a command printed, and why anything failed. A file read or an
        // edit answers with the file, which the person doesn't need again.
        output: if !ok || pending.name == "Bash" { output.to_owned() } else { String::new() },
        diff: if failed { None } else { patch.or(pending.diff) },
        name: pending.name,
        target: pending.target,
    }
}

/// `Exit code 2` heading a command's answer: the code, and the rest.
fn exit_code(text: &str) -> (Option<i32>, &str) {
    let first = text.lines().next().unwrap_or_default();
    let line = first.trim().trim_start_matches("Error: ");
    match line.strip_prefix("Exit code ").and_then(|n| n.trim().parse().ok()) {
        Some(code) => (Some(code), text[first.len()..].trim_start_matches(['\r', '\n'])),
        None => (None, text),
    }
}

/// Claude Code's `structuredPatch` (hunks with their lines) as a unified diff.
fn structured_diff(path: &str, patch: &Value) -> Option<String> {
    let hunks = patch.as_array().filter(|h| !h.is_empty())?;
    let mut out = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    for hunk in hunks {
        let n = |k: &str| hunk.get(k).and_then(Value::as_u64).unwrap_or(0);
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", n("oldStart"), n("oldLines"), n("newStart"), n("newLines")));
        for line in hunk.get("lines").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out.push_str(line);
            out.push('\n');
        }
    }
    Some(cap_diff(out))
}

/// The change an edit or a write asks for, from its input: what is replaced
/// and with what (line numbers count from the snippet, not the file).
fn input_diff(name: &str, input: &Value) -> Option<String> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let path = text(input, "file_path")?;
    let pairs: Vec<(String, String)> = match name {
        "Edit" => vec![(text(input, "old_string")?, text(input, "new_string")?)],
        "MultiEdit" => input.get("edits")?.as_array()?.iter().filter_map(|e| Some((text(e, "old_string")?, text(e, "new_string")?))).collect(),
        "Write" => vec![(String::new(), text(input, "content")?)],
        _ => return None,
    };
    let mut out = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    for (old, new) in &pairs {
        let (old, new) = (with_newline(old), with_newline(new));
        let diff = similar::TextDiff::from_lines(old.as_str(), new.as_str());
        for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
            out.push_str(&hunk.to_string());
        }
    }
    Some(cap_diff(out))
}

fn with_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') { text.to_owned() } else { format!("{text}\n") }
}

/// A diff no longer than [`DIFF_LINES`].
fn cap_diff(diff: String) -> String {
    match diff.match_indices('\n').nth(DIFF_LINES - 1) {
        Some((at, _)) => diff[..=at].to_owned(),
        None => diff,
    }
}

/// What one of equipe's board tools acts on, as the person reads it: the
/// ticket's number and title, an owner, a new status.
fn board_target(name: &str, input: Option<&Value>) -> Option<String> {
    let tool = name.strip_prefix("mcp__equipe__")?;
    let input = input?;
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(|t| t.lines().next().unwrap_or("").chars().take(100).collect::<String>());
    let id = input.get("id").and_then(Value::as_u64).map(|id| format!("T-{id}"));
    let parts: Vec<String> = match tool {
        "create_ticket" => [text("title"), text("owner").map(|o| format!("→ {o}"))].into_iter().flatten().collect(),
        "update_ticket" => [id, text("status"), text("owner").map(|o| format!("→ {o}")), text("title")].into_iter().flatten().collect(),
        "comment" | "submit_for_review" => [id, text("text").or_else(|| text("summary"))].into_iter().flatten().collect(),
        "block_on_person" => {
            let needs = input.get("needs").and_then(Value::as_array).map(|items| {
                items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ").chars().take(100).collect::<String>()
            });
            needs.into_iter().collect()
        }
        _ => Vec::new(),
    };
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn message_text(event: &Value) -> String {
    event
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect()
}

fn result_from(event: &Value, streamed: &str, last_message: &str) -> Result<TurnResult> {
    let text = event.get("result").and_then(Value::as_str).unwrap_or("");
    let subtype = event.get("subtype").and_then(Value::as_str).unwrap_or("success");
    // Failures inside the run, such as a missing login, arrive as an error result.
    if event.get("is_error").and_then(Value::as_bool).unwrap_or(false) || subtype.starts_with("error") {
        return Err(Error::Backend {
            message: if text.is_empty() { format!("Claude Code reported {subtype}") } else { text.to_owned() },
            code: Some(subtype.to_owned()),
        });
    }
    let text = [text, last_message, streamed]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or("")
        .to_owned();
    let usage = event.get("usage").and_then(|u| {
        let input_tokens = u.get("input_tokens").and_then(Value::as_u64);
        let output_tokens = u.get("output_tokens").and_then(Value::as_u64);
        let cache = ["cache_read_input_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| u.get(*k).and_then(Value::as_u64)).sum::<u64>();
        // The window of the model that did the work (the first listed).
        let context_window = event
            .get("modelUsage")
            .and_then(Value::as_object)
            .and_then(|models| models.values().find_map(|m| m.get("contextWindow").and_then(Value::as_u64)));
        (input_tokens.is_some() || output_tokens.is_some()).then_some(TurnUsage {
            input_tokens,
            output_tokens,
            cache_tokens: (cache > 0).then_some(cache),
            context_tokens: None,
            context_window,
        })
    });
    Ok(TurnResult {
        text,
        status: "completed".into(),
        usage,
    })
}

/// Run one headless turn through the user's Claude Code, reporting activity as it happens.
pub async fn run_turn(
    binary: &str,
    cwd: &Path,
    request: TurnRequest<'_>,
    mut steering: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    mut on_activity: impl FnMut(Activity),
) -> Result<Turn> {
    let mut command = Command::new(binary);
    // A directory added for reading brings its own CLAUDE.md, so the project's
    // conventions are known from wherever the turn runs.
    if !request.add_dirs.is_empty() {
        command.env("CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD", "1");
    }
    command.envs(request.env.iter().map(|(k, v)| (k, v)));
    let mut child = command
        .args(turn_args(&request))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => Error::Login(format!(
                "Claude Code (`{binary}`) is not installed or not on PATH. Install it, then run `claude auth login`."
            )),
            _ => err.into(),
        })?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    // The turn's first message; Claude Code reads more until its input ends,
    // which is once the turn has its result and no subagent is still out.
    let mut stdin = child.stdin.take();
    if let Some(input) = &mut stdin {
        let _ = input.write_all(user_message(request.prompt).as_bytes()).await;
    }
    let mut collector = TurnCollector::new();
    let mut stray = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let steer = async {
            match (&mut steering, stdin.is_some()) {
                (Some(rx), true) => rx.recv().await,
                _ => std::future::pending().await,
            }
        };
        let line = tokio::select! {
            line = lines.next_line() => line?,
            text = steer => {
                match (text, &mut stdin) {
                    (Some(text), Some(input)) => {
                        let _ = input.write_all(user_message(&text).as_bytes()).await;
                    }
                    // Nobody will steer any more.
                    _ => steering = None,
                }
                continue;
            }
        };
        let Some(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(event) => {
                collector.on_event(&event).into_iter().for_each(&mut on_activity);
                if event.get("type").and_then(Value::as_str) == Some("result") && collector.subagents.is_empty() {
                    // Done: its input ends, and so does Claude Code.
                    stdin = None;
                }
            }
            // Keep the first few non-JSON lines for the error message.
            Err(_) if stray.len() < 5 => stray.push(line.to_owned()),
            Err(_) => {}
        }
    }

    let status = child.wait().await?;
    let stderr = stderr_task.await.unwrap_or_default();
    let mut diagnostics: Vec<String> = stray;
    if !stderr.trim().is_empty() {
        diagnostics.push(stderr.trim().chars().take(400).collect());
    }
    if !status.success() {
        diagnostics.push(format!("exit status {status}"));
    }
    collector.finish(&diagnostics.join(" · "))
}

/// One of the turn's messages, as Claude Code's `stream-json` input takes it.
fn user_message(text: &str) -> String {
    format!("{}\n", serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }))
}

/// How the person's next message is guessed: from the conversation's last
/// exchanges alone, by the small model, with no tools, no MCP servers and
/// nothing kept — a few hundred tokens, never the member's own conversation.
const SUGGEST_SYSTEM: &str = "You guess what a person will type next to a member of their software \
team, from the end of their conversation. Reply with only that message, as the person would type it: one short \
line, in the language the person writes in, without quotes. Prefer what moves the work forward — a yes to an \
offer, the obvious next step, a check of what was just done. If nothing is likely, reply with nothing.";

/// Arguments for guessing the person's next message.
pub fn suggest_args(conversation: &str) -> Vec<String> {
    let prompt = format!("{conversation}\n\nThe person's next message:");
    [
        "-p",
        &prompt,
        "--model",
        "haiku",
        "--output-format",
        "json",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--system-prompt",
        SUGGEST_SYSTEM,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// The guess in Claude Code's JSON result, cleaned to one line; `None` when
/// there is no guess worth showing.
pub fn parse_suggestion(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json.trim()).ok()?;
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let text = value.get("result")?.as_str()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '`')).trim();
    (!line.is_empty() && line.chars().count() <= 200).then(|| line.to_owned())
}

/// Guess the person's next message from `conversation` with their own Claude
/// Code, in `cwd`. Any failure is no guess.
pub async fn suggest(binary: &str, cwd: &Path, conversation: &str) -> Option<String> {
    let output = Command::new(binary)
        .args(suggest_args(conversation))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), output).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    parse_suggestion(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(events: &[Value]) -> (Vec<String>, Result<TurnResult>) {
        let mut collector = TurnCollector::new();
        let deltas = events
            .iter()
            .flat_map(|e| collector.on_event(e))
            .filter_map(|a| match a {
                Activity::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        (deltas, collector.finish("").map(|turn| turn.result))
    }

    fn done_of(activities: Vec<Activity>) -> Vec<ToolDone> {
        activities
            .into_iter()
            .filter_map(|a| match a {
                Activity::ToolDone(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_tool_answer_says_how_it_ended_and_a_command_its_exit_code() {
        let mut collector = TurnCollector::new();
        collector.on_event(&json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "cargo test" } },
            { "type": "tool_use", "id": "t2", "name": "Read", "input": { "file_path": "/r/a.rs" } },
        ] } }));
        let done = done_of(collector.on_event(&json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "t2", "content": "1→fn main() {}" },
            { "type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": "Exit code 101\nerror: test failed\nfailures: 1" },
        ] } })));
        assert_eq!(done.len(), 2);
        assert_eq!((done[0].name.as_str(), done[0].ok, done[0].output.as_str()), ("Read", true, ""), "a read's file isn't carried");
        assert_eq!((done[1].name.as_str(), done[1].target.as_deref()), ("Bash", Some("cargo test")));
        assert_eq!((done[1].ok, done[1].exit), (false, Some(101)));
        assert_eq!(done[1].output, "error: test failed\nfailures: 1");
        assert!(done_of(collector.on_event(&json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": "again" }] } }))).is_empty(), "once");
    }

    #[test]
    fn an_edit_comes_back_with_its_change() {
        let mut collector = TurnCollector::new();
        collector.on_event(&json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "e1", "name": "Edit", "input": { "file_path": "/r/a.rs", "old_string": "let a = 1;\nlet b = 2;", "new_string": "let a = 1;\nlet b = 3;\nlet c = 4;" } },
            { "type": "tool_use", "id": "e2", "name": "Edit", "input": { "file_path": "/r/b.rs", "old_string": "x", "new_string": "y" } },
        ] } }));
        let done = done_of(collector.on_event(&json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "e1", "content": "The file /r/a.rs has been updated." }] } })));
        let diff = done[0].diff.as_deref().unwrap();
        assert!(diff.starts_with("diff --git a//r/a.rs b//r/a.rs\n"), "{diff}");
        assert!(diff.contains("-let b = 2;\n+let b = 3;\n+let c = 4;\n") && diff.contains(" let a = 1;\n"), "{diff}");
        assert_eq!(done[0].output, "");
        // Claude Code's own patch wins: it knows the file's line numbers.
        let done = done_of(collector.on_event(&json!({ "type": "user",
            "tool_use_result": { "structuredPatch": [{ "oldStart": 40, "oldLines": 1, "newStart": 40, "newLines": 1, "lines": ["-x", "+y"] }] },
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "e2", "content": "ok" }] } })));
        assert_eq!(done[0].diff.as_deref(), Some("diff --git a//r/b.rs b//r/b.rs\n--- a//r/b.rs\n+++ b//r/b.rs\n@@ -40,1 +40,1 @@\n-x\n+y\n"));
    }

    #[test]
    fn reasoning_streams_apart_from_the_answer() {
        let mut collector = TurnCollector::new();
        let thought = json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hm" } } });
        assert_eq!(collector.on_event(&thought), vec![Activity::Thinking("hm".into())]);
        assert_eq!(collector.on_event(&text_delta("ok")), vec![Activity::Text("ok".into())]);
    }

    fn text_delta(text: &str) -> Value {
        json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": text } } })
    }

    fn success(result: &str) -> Value {
        json!({ "type": "result", "subtype": "success", "is_error": false, "result": result,
                "usage": { "input_tokens": 10, "output_tokens": 2 } })
    }

    // ── auth status ──

    #[test]
    fn parses_a_subscription_login_and_ignores_identifying_fields() {
        let status = parse_auth_status(
            r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty",
                "email":"someone@example.com","orgId":"o","subscriptionType":"max"}"#,
        )
        .unwrap();
        assert!(status.uses_subscription());
        assert_eq!(status.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn an_api_key_login_is_not_a_subscription() {
        let status = parse_auth_status(r#"{"loggedIn":true,"authMethod":"api_key"}"#).unwrap();
        assert!(!status.uses_subscription());
    }

    #[test]
    fn a_signed_out_status_is_not_a_subscription() {
        let status = parse_auth_status(r#"{"loggedIn":false}"#).unwrap();
        assert!(!status.logged_in);
        assert!(!status.uses_subscription());
    }

    #[test]
    fn unexpected_status_output_is_an_error() {
        assert!(parse_auth_status("Logged in as …").is_err());
    }

    // ── arguments ──

    #[test]
    fn runs_headless_with_streaming_json_and_no_edits() {
        let args = turn_args(&TurnRequest { prompt: "hello", ..Default::default() });
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(args.iter().any(|a| a == "-p"));
        assert!(!args.iter().any(|a| a == "hello"), "the prompt goes in on stdin, where steering follows it");
        assert!(has(["--input-format", "stream-json"]));
        assert!(args.iter().any(|a| a == "--replay-user-messages"), "each message echoed as it is read");
        assert!(has(["--output-format", "stream-json"]));
        assert!(has(["--permission-mode", "dontAsk"]));
        assert!(args.iter().any(|a| a == "--verbose"));
        assert!(args.iter().any(|a| a == "--include-partial-messages"));
        assert!(!args.iter().any(|a| a == "--model"));
    }

    #[test]
    fn never_uses_bare_mode_which_skips_the_subscription_login() {
        for access in [Access::ReadOnly, Access::EditFiles] {
            assert!(!turn_args(&TurnRequest { prompt: "x", model: Some("opus"), access, ..Default::default() }).iter().any(|a| a == "--bare"));
        }
    }

    #[test]
    fn edit_access_lets_claude_write_files_without_asking() {
        let args = turn_args(&TurnRequest { prompt: "x", access: Access::EditFiles, ..Default::default() });
        assert!(args.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "acceptEdits"));
        assert!(!args.iter().any(|a| a == "bypassPermissions" || a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn a_sandbox_fences_commands_in_and_questions_go_to_the_prompt_tool() {
        let domains = ["registry.npmjs.org".to_owned()];
        let ask = ["Bash(git push:*)".to_owned()];
        let blocked = ["Bash(sudo:*)".to_owned()];
        let args = turn_args(&TurnRequest {
            prompt: "x",
            access: Access::EditFiles,
            memory: Some(Path::new("/m")),
            sandbox: Some(Sandbox { allowed_domains: &domains, ask: &ask, deny_read: &["/h/browser".to_owned()], deny_write: &["/code/api".to_owned()], allow_write: &["/h/cache".to_owned()] }),
            prompt_tool: Some("mcp__equipe__approve"),
            disallowed_tools: &blocked,
            ..Default::default()
        });
        let at = args.iter().position(|a| a == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert_eq!(settings["autoMemoryDirectory"], "/m", "one settings object carries both");
        assert_eq!(settings["sandbox"]["enabled"], true);
        assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], true, "leaving the fence asks, never happens silently");
        assert_eq!(settings["sandbox"]["network"]["allowedDomains"], json!(["registry.npmjs.org"]));
        assert_eq!(settings["sandbox"]["network"]["allowLocalBinding"], true, "a local preview server stays in the sandbox");
        assert_eq!(settings["sandbox"]["filesystem"]["denyRead"], json!(["/h/browser"]), "the person's sign-ins stay unread");
        assert_eq!(settings["sandbox"]["filesystem"]["denyWrite"], json!(["/code/api"]), "what is only read stays unwritten");
        assert_eq!(settings["permissions"]["ask"], json!(["Bash(git push:*)"]));
        assert!(args.windows(2).any(|w| w[0] == "--permission-prompts" && w[1] == "host"));
        assert!(args.windows(2).any(|w| w[0] == "--permission-prompt-tool" && w[1] == "mcp__equipe__approve"));
        assert!(args.windows(2).any(|w| w[0] == "--disallowedTools" && w[1] == "Bash(sudo:*)"));
    }

    #[test]
    fn each_access_maps_to_a_permission_mode() {
        for (access, mode) in [(Access::ReadOnly, "dontAsk"), (Access::Ask, "manual"), (Access::Full, "bypassPermissions")] {
            let args = turn_args(&TurnRequest { prompt: "x", access, ..Default::default() });
            assert!(args.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == mode), "{mode}");
        }
    }

    #[test]
    fn board_tools_name_the_ticket() {
        let target = |name: &str, input: Value| board_target(name, Some(&input));
        assert_eq!(
            target("mcp__equipe__create_ticket", json!({ "title": "다국어 구조", "owner": "frontend", "body": "…" })).as_deref(),
            Some("다국어 구조 · → frontend")
        );
        assert_eq!(target("mcp__equipe__update_ticket", json!({ "id": 4, "status": "cancelled" })).as_deref(), Some("T-4 · cancelled"));
        assert_eq!(target("mcp__equipe__comment", json!({ "id": 2, "text": "done\nmore" })).as_deref(), Some("T-2 · done"));
        assert_eq!(
            target("mcp__equipe__block_on_person", json!({ "needs": ["GitHub token", "PostHog secrets"] })).as_deref(),
            Some("GitHub token, PostHog secrets")
        );
        assert_eq!(target("mcp__equipe__board", json!({})), None);
        assert_eq!(target("Read", json!({ "file_path": "/a" })), None);
    }

    #[test]
    fn reports_tool_use_with_its_target() {
        let mut collector = TurnCollector::new();
        let activity = collector.on_event(&json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "name": "Read", "input": { "file_path": "/repo/README.md" } },
            { "type": "tool_use", "name": "Bash", "input": { "command": "ls -la\necho hi" } },
            { "type": "tool_use", "name": "TodoWrite", "input": { "todos": [] } },
        ]}}));
        assert_eq!(activity, vec![
            Activity::Tool { name: "Read".into(), target: Some("/repo/README.md".into()) },
            Activity::Tool { name: "Bash".into(), target: Some("ls -la".into()) },
            Activity::Tool { name: "TodoWrite".into(), target: None },
        ]);
    }

    #[test]
    fn subagents_are_followed_from_start_to_end_apart_from_the_members_own_steps() {
        let mut collector = TurnCollector::new();
        let started = collector.on_event(&json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [
            { "type": "tool_use", "id": "toolu_1", "name": "Agent", "input": { "description": "Run the iOS tests", "prompt": "…", "subagent_type": "general-purpose" } },
            { "type": "tool_use", "id": "toolu_2", "name": "Task", "input": { "description": "Read the logs", "prompt": "…" } },
        ]}}));
        // The member's own step (shown in its conversation), and the subagent followed.
        assert_eq!(started, vec![
            Activity::Tool { name: "Agent".into(), target: Some("Run the iOS tests".into()) },
            Activity::Tool { name: "Task".into(), target: Some("Read the logs".into()) },
            Activity::SubagentStarted { id: "toolu_1".into(), what: "Run the iOS tests".into() },
            Activity::SubagentStarted { id: "toolu_2".into(), what: "Read the logs".into() },
        ]);
        // A subagent's own steps are its, not the member's.
        let step = collector.on_event(&json!({ "type": "assistant", "parent_tool_use_id": "toolu_1", "message": { "content": [
            { "type": "tool_use", "id": "toolu_9", "name": "Bash", "input": { "command": "xcodebuild test" } },
        ]}}));
        assert_eq!(step, vec![Activity::SubagentStep { id: "toolu_1".into(), step: "Bash xcodebuild test".into() }]);
        let text = collector.on_event(&json!({ "type": "stream_event", "parent_tool_use_id": "toolu_1",
            "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": "inside" } } }));
        assert!(text.is_empty(), "a subagent's words aren't the member's answer");
        let ended = collector.on_event(&json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "12 passed" },
        ]}}));
        let ended: Vec<Activity> = ended.into_iter().filter(|a| !matches!(a, Activity::ToolDone(_))).collect();
        assert_eq!(ended, vec![Activity::SubagentEnded { id: "toolu_1".into() }]);

        // One run in the background answers at once, and hands back later.
        let launched = collector.on_event(&json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_2", "content": [{ "type": "text", "text": "Async agent launched successfully. agentId: a1" }] },
        ]}}));
        let launched: Vec<Activity> = launched.into_iter().filter(|a| !matches!(a, Activity::ToolDone(_))).collect();
        assert!(launched.is_empty(), "still at work: {launched:?}");
        let progress = collector.on_event(&json!({ "type": "system", "subtype": "task_progress", "tool_use_id": "toolu_2", "description": "Reading build.log" }));
        assert_eq!(progress, vec![Activity::SubagentStep { id: "toolu_2".into(), step: "Reading build.log".into() }]);
        let done = collector.on_event(&json!({ "type": "system", "subtype": "task_notification", "tool_use_id": "toolu_2", "status": "completed" }));
        assert_eq!(done, vec![Activity::SubagentEnded { id: "toolu_2".into() }]);
        assert!(collector.on_event(&json!({ "type": "system", "subtype": "task_notification", "tool_use_id": "toolu_2" })).is_empty(), "once");
    }

    #[test]
    fn a_message_sent_while_the_turn_runs_is_seen_where_it_was_read() {
        let mut collector = TurnCollector::new();
        let replay = |text: &str| json!({ "type": "user", "isReplay": true, "message": { "role": "user", "content": text } });
        assert!(collector.on_event(&replay("fix the button")).is_empty(), "the prompt itself");
        assert_eq!(collector.on_event(&replay("skip the footer")), vec![Activity::Steered("skip the footer".into())]);
        let line = user_message("a \"b\"\nc");
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "one line: {line}");
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({ "type": "user", "message": { "role": "user", "content": "a \"b\"\nc" } }));
    }

    #[test]
    fn a_guess_at_the_next_message_is_small_and_touches_nothing() {
        let args = suggest_args("[person] fix the button\n[frontend] Done. Shall I tidy the other pages too?");
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has(["--model", "haiku"]));
        assert!(has(["--tools", ""]), "no tools at all");
        assert!(has(["--output-format", "json"]));
        assert!(args.iter().any(|a| a == "--strict-mcp-config"), "none of the person's MCP servers");
        assert!(args.iter().any(|a| a == "--no-session-persistence"), "nothing kept");
        assert!(!args.iter().any(|a| a == "--bare" || a == "--resume"), "{args:?}");
        assert!(args[1].contains("Shall I tidy"));
    }

    #[test]
    fn the_guess_is_one_clean_line_or_none() {
        let result = |text: &str| json!({"type": "result", "is_error": false, "result": text}).to_string();
        assert_eq!(parse_suggestion(&result("\"응, 다른 페이지도 정리해줘\"\n")).as_deref(), Some("응, 다른 페이지도 정리해줘"));
        assert_eq!(parse_suggestion(&result("\n  yes, go ahead\nand more")).as_deref(), Some("yes, go ahead"));
        assert_eq!(parse_suggestion(&result("   ")), None);
        assert_eq!(parse_suggestion(&result(&"x".repeat(300))), None, "a speech isn't a suggestion");
        assert_eq!(parse_suggestion(&json!({"is_error": true, "result": "Not logged in"}).to_string()), None);
        assert_eq!(parse_suggestion("not json"), None);
    }

    #[test]
    fn resumes_an_earlier_conversation() {
        let args = turn_args(&TurnRequest { prompt: "x", resume: Some("sess-1"), ..Default::default() });
        assert!(args.windows(2).any(|w| w[0] == "--resume" && w[1] == "sess-1"));
        assert!(!turn_args(&TurnRequest { prompt: "x", ..Default::default() }).iter().any(|a| a == "--resume"));
    }

    #[test]
    fn remembers_the_session_id_and_reports_the_model() {
        let mut collector = TurnCollector::new();
        let activity = collector.on_event(&json!({
            "type": "system", "subtype": "init", "model": "claude-haiku-4-5", "session_id": "sess-9"
        }));
        assert_eq!(activity, vec![Activity::Model("claude-haiku-4-5".into())]);
        let _ = collector.on_event(&success("ok"));
        assert_eq!(collector.finish("").unwrap().session_id.as_deref(), Some("sess-9"));
    }

    #[test]
    fn passes_the_model_through() {
        let args = turn_args(&TurnRequest { prompt: "x", model: Some("opus"), ..Default::default() });
        assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "opus"));
    }

    #[test]
    fn passes_the_effort_through_only_when_set() {
        let args = turn_args(&TurnRequest { prompt: "x", effort: Some("xhigh"), ..Default::default() });
        assert!(args.windows(2).any(|w| w[0] == "--effort" && w[1] == "xhigh"));
        assert!(!turn_args(&TurnRequest { prompt: "x", ..Default::default() }).iter().any(|a| a == "--effort"));
    }

    #[test]
    fn a_charter_is_appended_never_replacing_claude_codes_prompt() {
        let args = turn_args(&TurnRequest { prompt: "x", charter: Some("You are the PM."), ..Default::default() });
        assert!(args.windows(2).any(|w| w[0] == "--append-system-prompt" && w[1] == "You are the PM."));
        assert!(!args.iter().any(|a| a == "--system-prompt"));
    }

    #[test]
    fn a_member_keeps_its_own_memory() {
        let dir = Path::new("/home/.equipe/projects/app/members/frontend/memory");
        let args = turn_args(&TurnRequest { prompt: "x", memory: Some(dir), ..Default::default() });
        let at = args.iter().position(|a| a == "--settings").expect("settings passed");
        let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert_eq!(settings, json!({ "autoMemoryDirectory": dir.to_string_lossy() }));
        assert!(!turn_args(&TurnRequest { prompt: "x", ..Default::default() }).iter().any(|a| a == "--settings"));
    }

    #[test]
    fn a_member_reads_other_projects_and_uses_only_equipes_tools() {
        let dirs = [std::path::PathBuf::from("/code/api")];
        let allowed = ["mcp__equipe".to_owned()];
        let deny = ["Edit(//code/api/**)".to_owned()];
        let args = turn_args(&TurnRequest {
            prompt: "x",
            add_dirs: &dirs,
            mcp_config: Some(r#"{"mcpServers":{}}"#),
            allowed_tools: &allowed,
            deny: &deny,
            ..Default::default()
        });
        let at = args.iter().position(|a| a == "--settings").expect("settings passed");
        let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert_eq!(settings["permissions"]["deny"], json!(["Edit(//code/api/**)"]), "read, never written");
        assert!(args.windows(2).any(|w| w[0] == "--add-dir" && w[1] == "/code/api"));
        assert!(args.windows(2).any(|w| w[0] == "--mcp-config" && w[1] == r#"{"mcpServers":{}}"#));
        assert!(args.iter().any(|a| a == "--strict-mcp-config"), "the person's own servers stay out");
        assert!(args.windows(2).any(|w| w[0] == "--allowedTools" && w[1] == "mcp__equipe"));
        // The prompt isn't among them: it goes in on stdin.
        assert!(!args.iter().any(|a| a == "x"));
    }

    // ── stream ──

    #[test]
    fn streams_text_deltas_and_returns_the_result() {
        let (deltas, result) = collect(&[
            json!({ "type": "system", "subtype": "init", "model": "claude-haiku-4-5" }),
            json!({ "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hm" } } }),
            text_delta("po"),
            text_delta("ng"),
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "pong" }] } }),
            success("pong"),
        ]);
        assert_eq!(deltas, ["po", "ng"]);
        let result = result.unwrap();
        assert_eq!(result.text, "pong");
        assert_eq!(result.status, "completed");
        assert_eq!(result.usage, Some(TurnUsage { input_tokens: Some(10), output_tokens: Some(2), ..Default::default() }));
    }

    #[test]
    fn remembers_the_model_from_init() {
        let mut collector = TurnCollector::new();
        let _ = collector.on_event(&json!({ "type": "system", "subtype": "init", "model": "claude-opus-5-5" }));
        assert_eq!(collector.model(), Some("claude-opus-5-5"));
    }

    #[test]
    fn falls_back_to_the_assistant_message_when_the_result_is_empty() {
        let (_, result) = collect(&[
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "from message" }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false, "result": "" }),
        ]);
        assert_eq!(result.unwrap().text, "from message");
    }

    #[test]
    fn an_error_result_is_a_failure_with_its_reason() {
        let (_, result) = collect(&[json!({
            "type": "result", "subtype": "success", "is_error": true, "result": "Not logged in · Please run /login"
        })]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Not logged in"));
    }

    #[test]
    fn an_error_subtype_is_a_failure_even_without_text() {
        let (_, result) = collect(&[json!({ "type": "result", "subtype": "error_max_turns", "is_error": false })]);
        assert!(result.unwrap_err().to_string().contains("error_max_turns"));
    }

    #[test]
    fn keeps_streamed_text_when_no_result_arrives() {
        let (_, result) = collect(&[text_delta("partial")]);
        let result = result.unwrap();
        assert_eq!(result.text, "partial");
        assert_eq!(result.status, "incomplete");
    }

    #[test]
    fn nothing_at_all_is_an_error_carrying_diagnostics() {
        let err = TurnCollector::new().finish("exit status 1").unwrap_err();
        assert!(err.to_string().contains("exit status 1"));
    }

    // ── process plumbing, against a stand-in `claude` ──

    #[cfg(unix)]
    fn fake_claude(script_body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("equipe-fake-claude-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        // Written beside it and copied in by `cp`: this process never holds the
        // script open for writing, so a test thread forking meanwhile can't
        // leave it "busy" when it runs.
        let draft = dir.join("claude.draft");
        std::fs::write(&draft, format!("#!/bin/sh\n{script_body}\n")).unwrap();
        assert!(std::process::Command::new("cp").arg(&draft).arg(&bin).status().unwrap().success());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (bin, dir)
    }

    #[test]
    fn the_plans_windows_and_the_turns_output_are_reported() {
        let mut collector = TurnCollector::new();
        let plan = json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{
            "five_hour":{"utilization":0.06,"resetsAt":1791871800},"seven_day":{"utilization":0.03,"resetsAt":1792454400}}}});
        assert_eq!(
            collector.on_event(&plan),
            vec![Activity::Plan { five_hour: Some((0.06, 1791871800)), seven_day: Some((0.03, 1792454400)) }]
        );
        let delta = |n: u64| json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":n}}});
        assert_eq!(collector.on_event(&delta(120)), vec![Activity::Output(120)]);
        assert_eq!(collector.on_event(&delta(30)), vec![Activity::Output(150)], "summed over the turn's requests");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_turn_through_the_binary() {
        let (bin, dir) = fake_claude(
            r#"cat <<'EOF'
{"type":"system","subtype":"init","model":"claude-haiku-4-5"}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"po"}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ng"}}}
{"type":"result","subtype":"success","is_error":false,"result":"pong","usage":{"input_tokens":3,"output_tokens":1}}
EOF"#,
        );
        let mut seen = String::new();
        let request = TurnRequest { prompt: "ping", ..Default::default() };
        let turn = run_turn(bin.to_str().unwrap(), &dir, request, None, |a| {
            if let Activity::Text(t) = a {
                seen.push_str(&t)
            }
        })
        .await
        .unwrap();
        assert_eq!(seen, "pong");
        assert_eq!(turn.result.text, "pong");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_stderr_when_the_binary_fails_without_a_result() {
        let (bin, dir) = fake_claude("echo 'boom: config is broken' >&2\nexit 3");
        let request = TurnRequest { prompt: "ping", ..Default::default() };
        let err = run_turn(bin.to_str().unwrap(), &dir, request, None, |_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("boom: config is broken"), "{err}");
        assert!(err.contains("exit status"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn status_reads_the_binarys_own_report() {
        let (bin, _) = fake_claude(r#"echo '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"pro"}'"#);
        let status = auth_status(bin.to_str().unwrap()).await.unwrap().unwrap();
        assert!(status.uses_subscription());
    }

    #[tokio::test]
    async fn a_missing_binary_is_not_installed_rather_than_an_error() {
        assert_eq!(auth_status("equipe-no-such-claude-binary").await.unwrap(), None);
        let request = TurnRequest { prompt: "x", ..Default::default() };
        let err = run_turn("equipe-no-such-claude-binary", Path::new("."), request, None, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }
}
