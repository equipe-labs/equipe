//! One turn against the ChatGPT (Codex) Responses backend.
//!
//! This is the `llm` engine at its thinnest — no tools, no agent loop. Code-editing
//! turns go through an ACP engine instead; equipe does not grow its own loop here.
//!
//! Wire shape follows the Responses API as the Codex backend accepts it: SSE only,
//! `store` must be false, and the beta header is required.

use std::path::Path;

use futures_util::StreamExt as _;
use serde_json::{Value, json};

use super::CODEX_BACKEND_URL;
use super::oauth::with_identity;
use super::sse::SseDecoder;
use crate::credentials::Credentials;
use crate::error::{Error, Result};
pub use crate::turn::{TurnResult, TurnUsage};

const DEFAULT_INSTRUCTIONS: &str = "You are an agent running inside equipe, a multi-agent software squad. \
     Answer concisely and directly.";

pub struct TurnRequest<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    /// System-level framing for the turn.
    pub instructions: Option<&'a str>,
    /// Earlier turns of this conversation, oldest first. The backend keeps
    /// nothing (`store: false`), so the conversation travels with every turn.
    pub history: &'a [Message],
    /// The model's reasoning effort (`minimal` · `low` · `medium` · `high`); `None`, its default.
    pub effort: Option<&'a str>,
}

/// Who said a line of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// One line of the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

/// What one request carries at most: pictures (a video's frames count), and
/// the bytes of the files read into it.
const PICTURES: usize = 20;
const FILE_BYTES: u64 = 20 << 20;
/// A text file longer than this is cut, with a word saying so.
const TEXT_BYTES: usize = 256 << 10;

/// What one request may still take in, newest message first.
struct Budget {
    pictures: usize,
    bytes: u64,
}

/// What a message names that a Codex member can't open itself — it has no
/// Read — turned into parts it sees: `[image: p]` and a video's frames as
/// pictures, `[pdf: p …]` as a file, `[file: p …]` of text as its text. The
/// message's own text stays first, tags and all.
fn attachment_parts(text: &str, budget: &mut Budget) -> Vec<Value> {
    let mut parts = Vec::new();
    for (word, body) in tags(text) {
        let fields: Vec<&str> = body.split(" · ").collect();
        let path = Path::new(fields[0].trim());
        match word {
            "image" => parts.extend(picture(path, budget)),
            "video" => {
                // Its frames; when there are too many to fit, the one sheet of them.
                let frames: Vec<&str> = fields
                    .iter()
                    .find_map(|f| f.strip_prefix("frames: "))
                    .map(|list| list.split(", ").filter_map(|f| f.rsplit_once('@').map(|(p, _)| p)).collect())
                    .unwrap_or_default();
                let sheet = fields.iter().find_map(|f| f.strip_prefix("sheet: "));
                if !frames.is_empty() && frames.len() <= budget.pictures {
                    parts.extend(frames.iter().filter_map(|f| picture(Path::new(f), budget)));
                } else if let Some(sheet) = sheet {
                    parts.extend(picture(Path::new(sheet), budget));
                }
            }
            "pdf" => parts.extend(read(path, budget).map(|bytes| {
                let name = path.file_name().map_or_else(|| "file.pdf".into(), |n| n.to_string_lossy().into_owned());
                json!({ "type": "input_file", "filename": name, "file_data": data_url("application/pdf", &bytes) })
            })),
            "file" => parts.extend(text_file(path, budget)),
            _ => {}
        }
    }
    parts
}

/// The `[word: body]` tags in `text`, in order.
fn tags(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some((word, tail)) = after.split_once(": ") else { break };
        let known = matches!(word, "image" | "video" | "pdf" | "file" | "audio");
        match (known, tail.find(']')) {
            (true, Some(close)) => {
                out.push((word, &tail[..close]));
                rest = &tail[close + 1..];
            }
            _ => rest = after,
        }
    }
    out
}

fn read(path: &Path, budget: &mut Budget) -> Option<Vec<u8>> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > budget.bytes {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    budget.bytes -= bytes.len() as u64;
    Some(bytes)
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn picture(path: &Path, budget: &mut Budget) -> Option<Value> {
    if budget.pictures == 0 {
        return None;
    }
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = read(path, budget)?;
    budget.pictures -= 1;
    Some(json!({ "type": "input_image", "image_url": data_url(mime, &bytes) }))
}

/// A text file's text, headed by its name; nothing for one that isn't text.
fn text_file(path: &Path, budget: &mut Budget) -> Option<Value> {
    let bytes = read(path, budget)?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut cut = text.len().min(TEXT_BYTES);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let more = if cut < text.len() { format!("\n… ({} more bytes)", text.len() - cut) } else { String::new() };
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Some(json!({ "type": "input_text", "text": format!("--- {name} ---\n{}{more}", &text[..cut]) }))
}

fn input_item(role: Role, text: &str, budget: &mut Budget) -> Value {
    match role {
        Role::User => {
            let mut content = vec![json!({ "type": "input_text", "text": text })];
            content.extend(attachment_parts(text, budget));
            json!({ "type": "message", "role": "user", "content": content })
        }
        Role::Assistant => json!({
            "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": text }],
        }),
    }
}

/// Body for a single stateless turn.
pub fn build_turn_request(req: &TurnRequest<'_>) -> Value {
    // The newest message gets its attachments first; older ones as there is room.
    let mut budget = Budget { pictures: PICTURES, bytes: FILE_BYTES };
    let mut input = vec![input_item(Role::User, req.prompt, &mut budget)];
    for m in req.history.iter().rev() {
        input.push(input_item(m.role, &m.text, &mut budget));
    }
    input.reverse();
    let mut body = json!({
        "model": req.model,
        "instructions": req.instructions.unwrap_or(DEFAULT_INSTRUCTIONS),
        "input": input,
        // The Codex backend rejects `store: true` outright, and equipe keeps turn
        // history on its own board rather than on OpenAI's servers.
        "store": false,
        "stream": true,
    });
    if let Some(effort) = req.effort {
        body["reasoning"] = json!({ "effort": effort });
    }
    body
}

/// Folds a Responses event stream into one turn.
///
/// The terminal event's `output` is authoritative when present — deltas are for
/// live display and can be missing entirely if the backend chooses not to stream
/// them — so they only serve as the fallback.
#[derive(Default)]
pub struct TurnCollector {
    streamed: String,
    result: Option<TurnResult>,
}

impl TurnCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one event. Returns the text fragment to display, if it carried one.
    pub fn on_event(&mut self, event: &Value) -> Result<Option<String>> {
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_text.delta" => {
                let delta = event.get("delta").and_then(Value::as_str).unwrap_or("");
                if delta.is_empty() {
                    return Ok(None);
                }
                self.streamed.push_str(delta);
                Ok(Some(delta.to_owned()))
            }
            "error" | "response.failed" => Err(error_from(event).unwrap_or_else(|| Error::Backend {
                message: "the turn failed without a reason".into(),
                code: None,
            })),
            "response.completed" | "response.done" | "response.incomplete" => {
                if let Some(failure) = error_from(event) {
                    return Err(failure);
                }
                let response = event.get("response").unwrap_or(&Value::Null);
                let text = text_from_output(response);
                self.result = Some(TurnResult {
                    text: if text.is_empty() { self.streamed.clone() } else { text },
                    status: response
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("completed")
                        .to_owned(),
                    usage: usage_from(response),
                });
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    pub fn finish(self) -> Result<TurnResult> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        // A stream that ends without a terminal event still carries usable text;
        // only a stream with nothing at all is an outright failure.
        if self.streamed.is_empty() {
            return Err(Error::Protocol(
                "the backend closed the stream without a response".into(),
            ));
        }
        Ok(TurnResult {
            text: self.streamed,
            status: "incomplete".into(),
            usage: None,
        })
    }
}

/// Join the `output_text` parts of a completed response's message items.
fn text_from_output(response: &Value) -> String {
    let Some(items) = response.get("output").and_then(Value::as_array) else {
        return String::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

fn usage_from(response: &Value) -> Option<TurnUsage> {
    let usage = response.get("usage")?.as_object()?;
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    Some(TurnUsage {
        input_tokens,
        output_tokens,
        ..Default::default()
    })
}

fn error_from(event: &Value) -> Option<Error> {
    let err = event
        .get("error")
        .and_then(Value::as_object)
        .or_else(|| event.get("response")?.get("error")?.as_object())?;
    Some(Error::Backend {
        message: err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the backend reported an error")
            .to_owned(),
        code: err.get("code").and_then(Value::as_str).map(str::to_owned),
    })
}

/// Run one turn. Streams text fragments to `on_delta` as they arrive.
pub async fn run_turn(
    client: &reqwest::Client,
    cred: &Credentials,
    client_version: &str,
    req: TurnRequest<'_>,
    mut on_delta: impl FnMut(&str),
) -> Result<TurnResult> {
    let builder = client
        .post(format!("{CODEX_BACKEND_URL}/responses"))
        .header("Accept", "text/event-stream")
        .header("OpenAI-Beta", "responses=experimental")
        .header("session_id", uuid::Uuid::new_v4().to_string())
        .json(&build_turn_request(&req));
    let res = with_identity(builder, cred, client_version).send().await?;

    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(Error::from_status("turn", status.as_u16(), &body));
    }

    let mut decoder = SseDecoder::new();
    let mut collector = TurnCollector::new();
    let mut apply = |events: Vec<Value>, collector: &mut TurnCollector| -> Result<()> {
        for event in &events {
            if let Some(delta) = collector.on_event(event)? {
                on_delta(&delta);
            }
        }
        Ok(())
    };

    let mut body = res.bytes_stream();
    while let Some(chunk) = body.next().await {
        let events = decoder.push(&chunk?)?;
        apply(events, &mut collector)?;
    }
    apply(decoder.finish()?, &mut collector)?;
    collector.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(events: &[Value]) -> (Vec<String>, Result<TurnResult>) {
        let mut collector = TurnCollector::new();
        let mut seen = Vec::new();
        for event in events {
            match collector.on_event(event) {
                Ok(Some(delta)) => seen.push(delta),
                Ok(None) => {}
                Err(err) => return (seen, Err(err)),
            }
        }
        (seen, collector.finish())
    }

    fn delta(text: &str) -> Value {
        json!({ "type": "response.output_text.delta", "delta": text })
    }

    fn completed(text: &str) -> Value {
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "output": [{ "type": "message", "content": [{ "type": "output_text", "text": text }] }],
            }
        })
    }

    #[test]
    fn streams_deltas_and_returns_the_completed_text() {
        let (seen, result) = collect(&[delta("Hel"), delta("lo"), completed("Hello")]);
        let result = result.unwrap();
        assert_eq!(seen, ["Hel", "lo"]);
        assert_eq!(result.text, "Hello");
        assert_eq!(result.status, "completed");
    }

    #[test]
    fn prefers_the_terminal_output_over_accumulated_deltas() {
        let (_, result) = collect(&[delta("partial"), completed("final answer")]);
        assert_eq!(result.unwrap().text, "final answer");
    }

    #[test]
    fn falls_back_to_deltas_when_the_terminal_event_carries_no_output() {
        let (_, result) = collect(&[
            delta("streamed only"),
            json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ]);
        assert_eq!(result.unwrap().text, "streamed only");
    }

    #[test]
    fn joins_multiple_output_text_parts_and_skips_other_items() {
        let (_, result) = collect(&[json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "output": [
                    { "type": "reasoning", "content": [{ "type": "output_text", "text": "IGNORED" }] },
                    { "type": "message", "content": [
                        { "type": "output_text", "text": "one " },
                        { "type": "refusal", "text": "IGNORED" },
                        { "type": "output_text", "text": "two" },
                    ]},
                ],
            }
        })]);
        assert_eq!(result.unwrap().text, "one two");
    }

    #[test]
    fn reports_usage_when_present() {
        let mut event = completed("hi");
        event["response"]["usage"] = json!({ "input_tokens": 12, "output_tokens": 3 });
        let (_, result) = collect(&[event]);
        assert_eq!(
            result.unwrap().usage,
            Some(TurnUsage { input_tokens: Some(12), output_tokens: Some(3), ..Default::default() })
        );
    }

    #[test]
    fn omits_usage_when_not_reported() {
        let (_, result) = collect(&[completed("hi")]);
        assert_eq!(result.unwrap().usage, None);
    }

    #[test]
    fn surfaces_the_backend_error_code() {
        let (_, result) = collect(&[json!({
            "type": "error",
            "error": { "message": "usage limit reached", "code": "usage_limit_reached" }
        })]);
        match result.unwrap_err() {
            Error::Backend { message, code } => {
                assert_eq!(message, "usage limit reached");
                assert_eq!(code.as_deref(), Some("usage_limit_reached"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn treats_response_failed_as_a_failure() {
        let (_, result) = collect(&[json!({
            "type": "response.failed",
            "response": { "error": { "message": "boom" } }
        })]);
        assert!(result.unwrap_err().to_string().contains("boom"));
    }

    #[test]
    fn rejects_a_stream_that_yields_nothing() {
        let (_, result) = collect(&[]);
        assert!(result.unwrap_err().to_string().contains("closed the stream"));
    }

    #[test]
    fn keeps_text_from_a_stream_that_ends_without_a_terminal_event() {
        let (_, result) = collect(&[delta("truncated")]);
        let result = result.unwrap();
        assert_eq!(result.text, "truncated");
        assert_eq!(result.status, "incomplete");
    }

    #[test]
    fn request_matches_what_the_codex_backend_accepts() {
        let body = build_turn_request(&TurnRequest { prompt: "hello", model: "gpt-5.6", instructions: None, history: &[], effort: None });
        assert_eq!(body["model"], "gpt-5.6");
        // The backend rejects `store: true` and serves this endpoint over SSE only.
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(
            body["input"],
            json!([{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hello" }] }])
        );
        assert!(body["instructions"].as_str().unwrap().contains("equipe"));
    }

    #[test]
    fn request_honours_custom_instructions() {
        let body = build_turn_request(&TurnRequest { prompt: "x", model: "m", instructions: Some("Be terse."), history: &[], effort: None });
        assert_eq!(body["instructions"], "Be terse.");
    }

    #[test]
    fn request_carries_the_conversation_so_far() {
        let history = [
            Message { role: Role::User, text: "my name is abel".into() },
            Message { role: Role::Assistant, text: "hi abel".into() },
        ];
        let body = build_turn_request(&TurnRequest { prompt: "what's my name?", model: "m", instructions: None, history: &history, effort: None });
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["content"][0]["text"], "what's my name?");
    }

    fn uploads(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-codex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_codex_member_sees_what_the_message_attaches() {
        let dir = uploads("parts");
        let (shot, spec, notes, f1, f2) = (dir.join("shot.png"), dir.join("spec.pdf"), dir.join("notes.md"), dir.join("f1.jpg"), dir.join("f2.jpg"));
        for (path, bytes) in [(&shot, &b"png"[..]), (&spec, b"%PDF"), (&notes, "# 메모\n".as_bytes()), (&f1, b"j1"), (&f2, b"j2")] {
            std::fs::write(path, bytes).unwrap();
        }
        let prompt = format!(
            "봐 줘 [image: {}] [pdf: {} · 2 pages] [file: {} · 1 line] [video: /x/a.mov · 0:20 · 1920×1080 · frames: {}@0:05, {}@0:15] [file: /gone.bin]",
            shot.display(),
            spec.display(),
            notes.display(),
            f1.display(),
            f2.display()
        );
        let body = build_turn_request(&TurnRequest { prompt: &prompt, model: "m", instructions: None, history: &[], effort: Some("high") });
        assert_eq!(body["reasoning"]["effort"], "high");
        let content = body["input"][0]["content"].as_array().unwrap();
        let kinds: Vec<&str> = content.iter().map(|p| p["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["input_text", "input_image", "input_file", "input_text", "input_image", "input_image"]);
        assert_eq!(content[0]["text"], prompt, "the words and the tags stay");
        assert!(content[1]["image_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(content[2]["filename"], "spec.pdf");
        assert!(content[2]["file_data"].as_str().unwrap().starts_with("data:application/pdf;base64,"));
        assert!(content[3]["text"].as_str().unwrap().starts_with("--- notes.md ---\n# 메모"));
        assert!(content[4]["image_url"].as_str().unwrap().starts_with("data:image/jpeg;base64,"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pictures_go_to_the_newest_message_first_and_never_past_the_limit() {
        let dir = uploads("budget");
        let shot = dir.join("a.png");
        std::fs::write(&shot, b"png").unwrap();
        let many = (0..PICTURES).map(|_| format!("[image: {}]", shot.display())).collect::<Vec<_>>().join(" ");
        let history = [Message { role: Role::User, text: format!("old [image: {}]", shot.display()) }, Message { role: Role::Assistant, text: "ok".into() }];
        let body = build_turn_request(&TurnRequest { prompt: &many, model: "m", instructions: None, history: &history, effort: None });
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[2]["content"].as_array().unwrap().len(), 1 + PICTURES);
        assert_eq!(input[0]["content"].as_array().unwrap().len(), 1, "no room left for the older one's picture");
        assert!(body.get("reasoning").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn brackets_that_are_not_tags_are_left_alone() {
        assert_eq!(tags("a [b] [link: x] [image: /a.png] [file: /b.md · 3 lines]"), vec![("image", "/a.png"), ("file", "/b.md · 3 lines")]);
    }
}
