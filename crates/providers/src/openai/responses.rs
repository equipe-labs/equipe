//! One turn against the ChatGPT (Codex) Responses backend.
//!
//! This is the `llm` engine at its thinnest — no tools, no agent loop. Code-editing
//! turns go through an ACP engine instead; equipe does not grow its own loop here.
//!
//! Wire shape follows the Responses API as the Codex backend accepts it: SSE only,
//! `store` must be false, and the beta header is required.

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

fn input_item(role: Role, text: &str) -> Value {
    match role {
        Role::User => json!({
            "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": text }],
        }),
        Role::Assistant => json!({
            "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": text }],
        }),
    }
}

/// Body for a single stateless turn.
pub fn build_turn_request(req: &TurnRequest<'_>) -> Value {
    let input: Vec<Value> = req
        .history
        .iter()
        .map(|m| input_item(m.role, &m.text))
        .chain(std::iter::once(input_item(Role::User, req.prompt)))
        .collect();
    json!({
        "model": req.model,
        "instructions": req.instructions.unwrap_or(DEFAULT_INSTRUCTIONS),
        "input": input,
        // The Codex backend rejects `store: true` outright, and equipe keeps turn
        // history on its own board rather than on OpenAI's servers.
        "store": false,
        "stream": true,
    })
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
        let body = build_turn_request(&TurnRequest { prompt: "hello", model: "gpt-5.6", instructions: None, history: &[] });
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
        let body = build_turn_request(&TurnRequest { prompt: "x", model: "m", instructions: Some("Be terse."), history: &[] });
        assert_eq!(body["instructions"], "Be terse.");
    }

    #[test]
    fn request_carries_the_conversation_so_far() {
        let history = [
            Message { role: Role::User, text: "my name is abel".into() },
            Message { role: Role::Assistant, text: "hi abel".into() },
        ];
        let body = build_turn_request(&TurnRequest { prompt: "what's my name?", model: "m", instructions: None, history: &history });
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["content"][0]["text"], "what's my name?");
    }
}
