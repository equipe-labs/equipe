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

/// Arguments for one headless turn.
///
/// `--bare` is deliberately absent: bare mode never reads the subscription login,
/// which is the whole point of bringing your own Claude Code. `dontAsk` denies
/// anything that would need approval, so a plain turn can read but never edit.
pub fn turn_args(prompt: &str, model: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "dontAsk",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    args
}

/// Folds Claude Code's `stream-json` lines into one turn.
#[derive(Default)]
pub struct TurnCollector {
    streamed: String,
    last_message: String,
    model: Option<String>,
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

    /// Apply one event. Returns the text fragment to display, if it carried one.
    pub fn on_event(&mut self, event: &Value) -> Option<String> {
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                self.model = event.get("model").and_then(Value::as_str).map(str::to_owned);
                None
            }
            "stream_event" => {
                let delta = event.get("event")?.get("delta")?;
                if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
                    return None; // thinking and signature deltas are not output
                }
                let text = delta.get("text").and_then(Value::as_str).filter(|t| !t.is_empty())?;
                self.streamed.push_str(text);
                Some(text.to_owned())
            }
            "assistant" => {
                let text = message_text(event);
                if !text.is_empty() {
                    self.last_message = text;
                }
                None
            }
            "result" => {
                self.result = Some(result_from(event, &self.streamed, &self.last_message));
                None
            }
            _ => None,
        }
    }

    /// The turn, once the stream has ended. `diagnostics` is what to show when
    /// Claude Code exited without ever reporting a result.
    pub fn finish(self, diagnostics: &str) -> Result<TurnResult> {
        match self.result {
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
        }
    }
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

/// Run one headless turn through the user's Claude Code, streaming text to `on_delta`.
pub async fn run_turn(
    binary: &str,
    cwd: &Path,
    prompt: &str,
    model: Option<&str>,
    mut on_delta: impl FnMut(&str),
) -> Result<TurnResult> {
    let mut child = Command::new(binary)
        .args(turn_args(prompt, model))
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
            Ok(event) => {
                if let Some(delta) = collector.on_event(&event) {
                    on_delta(&delta);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(events: &[Value]) -> (Vec<String>, Result<TurnResult>) {
        let mut collector = TurnCollector::new();
        let deltas = events.iter().filter_map(|e| collector.on_event(e)).collect();
        (deltas, collector.finish(""))
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
        let args = turn_args("hello", None);
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
        assert!(!turn_args("x", Some("opus")).iter().any(|a| a == "--bare"));
    }

    #[test]
    fn passes_the_model_through() {
        let args = turn_args("x", Some("opus"));
        assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "opus"));
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
        collector.on_event(&json!({ "type": "system", "subtype": "init", "model": "claude-opus-5-5" }));
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
        let result = run_turn(bin.to_str().unwrap(), &dir, "ping", None, |d| seen.push_str(d))
            .await
            .unwrap();
        assert_eq!(seen, "pong");
        assert_eq!(result.text, "pong");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_stderr_when_the_binary_fails_without_a_result() {
        let (bin, dir) = fake_claude("echo 'boom: config is broken' >&2\nexit 3");
        let err = run_turn(bin.to_str().unwrap(), &dir, "ping", None, |_| {})
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
        let err = run_turn("equipe-no-such-claude-binary", Path::new("."), "x", None, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }
}
