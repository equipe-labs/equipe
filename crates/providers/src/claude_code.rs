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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
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

/// Ask the installed Claude Code whether it is signed in.
/// `Ok(None)` means the binary isn't installed (or not on PATH).
pub async fn auth_status(binary: &str) -> Result<Option<AuthStatus>> {
    let output = match Command::new(binary)
        .args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .output()
        .await
    {
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
    /// (`acceptEdits`). Shell commands beyond Claude Code's read-only set and
    /// paths outside the working directory still need approval, which a headless
    /// run has no one to give — so they are denied.
    EditFiles,
}

impl Access {
    fn permission_mode(self) -> &'static str {
        match self {
            Access::ReadOnly => "dontAsk",
            Access::EditFiles => "acceptEdits",
        }
    }
}

/// Arguments for one headless turn.
///
/// `--bare` is deliberately absent: bare mode never reads the subscription login,
/// which is the whole point of bringing your own Claude Code.
pub fn turn_args(request: &TurnRequest<'_>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        request.prompt,
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
    // Continue the conversation Claude Code already holds, rather than resending it.
    if let Some(id) = request.resume {
        args.extend(["--resume".to_owned(), id.to_owned()]);
    }
    // Added to Claude Code's own system prompt, never replacing it.
    if let Some(charter) = request.charter {
        args.extend(["--append-system-prompt".to_owned(), charter.to_owned()]);
    }
    // Claude Code's auto memory, moved to a directory of the caller's choosing —
    // a setting of the unmodified binary, not a change to it.
    if let Some(dir) = request.memory {
        let settings = serde_json::json!({ "autoMemoryDirectory": dir.to_string_lossy() });
        args.extend(["--settings".to_owned(), settings.to_string()]);
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
}

/// One turn's outcome, with the id that continues the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub result: TurnResult,
    pub session_id: Option<String>,
}

/// Something worth showing while a turn runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    /// A fragment of the answer.
    Text(String),
    /// Claude Code is using a tool, e.g. `Edit` on a file.
    Tool { name: String, target: Option<String> },
    /// The model Claude Code is running, reported at startup.
    Model(String),
}

/// Folds Claude Code's `stream-json` lines into one turn.
#[derive(Default)]
pub struct TurnCollector {
    streamed: String,
    last_message: String,
    model: Option<String>,
    session_id: Option<String>,
    result: Option<Result<TurnResult>>,
}

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
            "stream_event" => text_delta(event)
                .map(|text| {
                    self.streamed.push_str(text);
                    vec![Activity::Text(text.to_owned())]
                })
                .unwrap_or_default(),
            "assistant" => {
                let text = message_text(event);
                if !text.is_empty() {
                    self.last_message = text;
                }
                tool_uses(event)
            }
            "result" => {
                self.result = Some(result_from(event, &self.streamed, &self.last_message));
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

/// The text of a `text_delta`; thinking and signature deltas are not output.
fn text_delta(event: &Value) -> Option<&str> {
    let delta = event.get("event")?.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta.get("text").and_then(Value::as_str).filter(|t| !t.is_empty())
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
            let input = block.get("input");
            let target = ["file_path", "path", "notebook_path", "pattern", "command", "url"]
                .iter()
                .find_map(|key| input?.get(*key).and_then(Value::as_str))
                .map(|t| t.lines().next().unwrap_or("").chars().take(120).collect());
            Some(Activity::Tool { name, target })
        })
        .collect()
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
        (input_tokens.is_some() || output_tokens.is_some()).then_some(TurnUsage {
            input_tokens,
            output_tokens,
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
    mut on_activity: impl FnMut(Activity),
) -> Result<Turn> {
    let mut child = Command::new(binary)
        .args(turn_args(&request))
        .current_dir(cwd)
        .stdin(Stdio::null())
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

    let mut collector = TurnCollector::new();
    let mut stray = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(event) => collector.on_event(&event).into_iter().for_each(&mut on_activity),
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
        assert!(has(["-p", "hello"]));
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
        let args = turn_args(&TurnRequest {
            prompt: "x",
            add_dirs: &dirs,
            mcp_config: Some(r#"{"mcpServers":{}}"#),
            allowed_tools: &allowed,
            ..Default::default()
        });
        assert!(args.windows(2).any(|w| w[0] == "--add-dir" && w[1] == "/code/api"));
        assert!(args.windows(2).any(|w| w[0] == "--mcp-config" && w[1] == r#"{"mcpServers":{}}"#));
        assert!(args.iter().any(|a| a == "--strict-mcp-config"), "the person's own servers stay out");
        assert!(args.windows(2).any(|w| w[0] == "--allowedTools" && w[1] == "mcp__equipe"));
        // The prompt still comes right after -p, whatever follows.
        assert!(args.windows(2).any(|w| w[0] == "-p" && w[1] == "x"));
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
        assert_eq!(result.usage, Some(TurnUsage { input_tokens: Some(10), output_tokens: Some(2) }));
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
        std::fs::write(&bin, format!("#!/bin/sh\n{script_body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (bin, dir)
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
        let turn = run_turn(bin.to_str().unwrap(), &dir, request, |a| {
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
        let err = run_turn(bin.to_str().unwrap(), &dir, request, |_| {})
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
        let err = run_turn("equipe-no-such-claude-binary", Path::new("."), request, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }
}
