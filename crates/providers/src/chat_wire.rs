//! OpenAI's Responses API said in Chat Completions, the API nearly every
//! model server speaks — Nebius Token Factory, OpenRouter, Together, Groq,
//! vLLM, Ollama, LM Studio… Codex 0.154 asks its providers in Responses only;
//! the relay ([`crate::relay`]) turns each of its requests into a chat
//! request ([`to_chat`]) and the chat stream that answers it back into the
//! Responses events Codex reads ([`Translator`]). So a member's model is any
//! model such a server has: the agent stays Codex, the model is swapped.
//!
//! What has no place in a chat request stays out: Codex's reasoning items,
//! its hosted tools (web search…), `store`, `include`. A message from the
//! developer goes with the system's, first.

use serde_json::{Map, Value, json};

/// Whether a request path is the Responses API's: what is said in chat.
pub fn is_responses(path: &str) -> bool {
    path.split('?').next().unwrap_or_default().trim_end_matches('/').ends_with("/responses")
}

/// The text of a message's content: a string, or its text parts joined.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A user's content: its text, or its parts with their images.
fn user_content(content: &Value) -> Value {
    let Value::Array(parts) = content else { return Value::String(text_of(content)) };
    let images = parts.iter().any(|p| p.get("type").and_then(Value::as_str) == Some("input_image"));
    if !images {
        return Value::String(text_of(content));
    }
    Value::Array(
        parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("input_image") => p.get("image_url").and_then(Value::as_str).map(|url| json!({ "type": "image_url", "image_url": { "url": url } })),
                _ => p.get("text").and_then(Value::as_str).map(|text| json!({ "type": "text", "text": text })),
            })
            .collect(),
    )
}

/// A tool's output as the chat API takes it: text.
fn output_text(output: &Value) -> String {
    match output {
        Value::String(s) => s.clone(),
        Value::Array(_) => text_of(output),
        Value::Object(map) => map.get("content").map(text_of).unwrap_or_else(|| output.to_string()),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A Responses request's body as a chat request's, streamed.
pub fn to_chat(body: &Value) -> Value {
    let mut system: Vec<String> = Vec::new();
    if let Some(instructions) = body.get("instructions").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        system.push(instructions.to_owned());
    }
    let mut messages: Vec<Value> = Vec::new();
    for item in body.get("input").and_then(Value::as_array).into_iter().flatten() {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("message");
        match kind {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                let content = item.get("content").cloned().unwrap_or(Value::Null);
                match role {
                    "system" | "developer" => system.push(text_of(&content)),
                    "assistant" => messages.push(json!({ "role": "assistant", "content": text_of(&content) })),
                    _ => messages.push(json!({ "role": "user", "content": user_content(&content) })),
                }
            }
            "function_call" => {
                let call = json!({
                    "id": item.get("call_id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": { "name": item.get("name").cloned().unwrap_or(Value::Null), "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or("{}") },
                });
                // Calls made together go in one assistant message, with what it said before them.
                match messages.last_mut() {
                    Some(last) if last["role"] == "assistant" => match last.get_mut("tool_calls").and_then(Value::as_array_mut) {
                        Some(calls) => calls.push(call),
                        None => last["tool_calls"] = json!([call]),
                    },
                    _ => messages.push(json!({ "role": "assistant", "content": Value::Null, "tool_calls": [call] })),
                }
            }
            "function_call_output" => messages.push(json!({
                "role": "tool",
                "tool_call_id": item.get("call_id").cloned().unwrap_or(Value::Null),
                "content": output_text(item.get("output").unwrap_or(&Value::Null)),
            })),
            _ => {}
        }
    }
    // An assistant message with neither text nor calls says nothing; some servers refuse it.
    messages.retain(|m| m["role"] != "assistant" || m.get("tool_calls").is_some() || m["content"].as_str().is_some_and(|s| !s.is_empty()));
    for m in &mut messages {
        if m["role"] == "assistant" && m.get("tool_calls").is_some() && m["content"].as_str().is_some_and(str::is_empty) {
            m["content"] = Value::Null;
        }
    }
    if !system.is_empty() {
        messages.insert(0, json!({ "role": "system", "content": system.join("\n\n") }));
    }
    let tools: Vec<Value> = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("function"))
        .map(|t| {
            let mut function = Map::new();
            function.insert("name".into(), t.get("name").cloned().unwrap_or(Value::Null));
            if let Some(description) = t.get("description") {
                function.insert("description".into(), description.clone());
            }
            function.insert("parameters".into(), t.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })));
            json!({ "type": "function", "function": function })
        })
        .collect();
    let mut chat = json!({
        "model": body.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        chat["tools"] = Value::Array(tools);
        chat["tool_choice"] = match body.get("tool_choice") {
            Some(Value::String(choice)) if matches!(choice.as_str(), "auto" | "none" | "required") => Value::String(choice.clone()),
            _ => Value::String("auto".into()),
        };
        if let Some(parallel) = body.get("parallel_tool_calls").and_then(Value::as_bool) {
            chat["parallel_tool_calls"] = Value::Bool(parallel);
        }
    }
    for (from, to) in [("max_output_tokens", "max_tokens"), ("temperature", "temperature"), ("top_p", "top_p")] {
        if let Some(value) = body.get(from).filter(|v| !v.is_null()) {
            chat[to] = value.clone();
        }
    }
    chat
}

/// A call the model is making, as its fragments come in.
#[derive(Debug, Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

/// A chat stream turned into the Responses events of one response, chunk by chunk.
#[derive(Debug)]
pub struct Translator {
    id: String,
    started: bool,
    text: String,
    text_open: bool,
    calls: Vec<Call>,
    usage: Option<Value>,
    finished: bool,
}

impl Translator {
    /// For a response named `id` (anything unique: Codex only reads it back).
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned(), started: false, text: String::new(), text_open: false, calls: Vec::new(), usage: None, finished: false }
    }

    fn message_id(&self) -> String {
        format!("msg_{}", self.id)
    }

    fn start(&mut self, out: &mut Vec<Value>) {
        if !self.started {
            self.started = true;
            out.push(json!({ "type": "response.created", "response": { "id": self.id, "status": "in_progress", "output": [] } }));
        }
    }

    /// The events one chat chunk (a `data:` line's JSON) makes.
    pub fn feed(&mut self, chunk: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        self.start(&mut out);
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(usage.clone());
        }
        for choice in chunk.get("choices").and_then(Value::as_array).into_iter().flatten() {
            let Some(delta) = choice.get("delta") else { continue };
            if let Some(text) = delta.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                if !self.text_open {
                    self.text_open = true;
                    out.push(json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "message", "id": self.message_id(), "role": "assistant", "status": "in_progress", "content": [] } }));
                }
                self.text.push_str(text);
                out.push(json!({ "type": "response.output_text.delta", "item_id": self.message_id(), "output_index": 0, "content_index": 0, "delta": text }));
            }
            for fragment in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let index = fragment.get("index").and_then(Value::as_u64).map_or(self.calls.len().saturating_sub(1), |i| i as usize);
                while self.calls.len() <= index {
                    self.calls.push(Call::default());
                }
                let call = &mut self.calls[index];
                if let Some(id) = fragment.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    call.id = id.to_owned();
                }
                if let Some(function) = fragment.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        call.name.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        call.arguments.push_str(arguments);
                    }
                }
            }
        }
        out
    }

    /// The events that end the response, once the stream has: what was said
    /// whole, each call, and `response.completed` with the tokens it took.
    pub fn finish(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        self.finished = true;
        self.start(&mut out);
        let mut output = Vec::new();
        if !self.text.is_empty() {
            let item = json!({ "type": "message", "id": self.message_id(), "role": "assistant", "status": "completed", "content": [{ "type": "output_text", "text": self.text, "annotations": [] }] });
            out.push(json!({ "type": "response.output_item.done", "output_index": 0, "item": item }));
            output.push(item);
        }
        for (n, call) in self.calls.iter().enumerate().filter(|(_, c)| !c.name.is_empty()) {
            let call_id = if call.id.is_empty() { format!("call_{}_{n}", self.id) } else { call.id.clone() };
            let arguments = if call.arguments.trim().is_empty() { "{}".to_owned() } else { call.arguments.clone() };
            let item = json!({ "type": "function_call", "id": format!("fc_{}_{n}", self.id), "call_id": call_id, "name": call.name, "arguments": arguments, "status": "completed" });
            out.push(json!({ "type": "response.output_item.added", "output_index": output.len(), "item": item }));
            out.push(json!({ "type": "response.output_item.done", "output_index": output.len(), "item": item }));
            output.push(item);
        }
        let usage = self.usage.as_ref().map(responses_usage).unwrap_or_else(|| responses_usage(&json!({})));
        out.push(json!({ "type": "response.completed", "response": { "id": self.id, "status": "completed", "output": output, "usage": usage } }));
        out
    }
}

/// A chat answer's usage as a response's.
fn responses_usage(usage: &Value) -> Value {
    let n = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0);
    let input = n(usage.get("prompt_tokens"));
    let output = n(usage.get("completion_tokens"));
    json!({
        "input_tokens": input,
        "input_tokens_details": { "cached_tokens": n(usage.pointer("/prompt_tokens_details/cached_tokens")) },
        "output_tokens": output,
        "output_tokens_details": { "reasoning_tokens": n(usage.pointer("/completion_tokens_details/reasoning_tokens")) },
        "total_tokens": usage.get("total_tokens").and_then(Value::as_u64).unwrap_or(input + output),
    })
}

/// One Responses event as a server-sent event.
pub fn sse_event(event: &Value) -> String {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("message");
    format!("event: {kind}\ndata: {event}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_responses_path_is_said_in_chat() {
        assert!(is_responses("/responses") && is_responses("/v1/responses") && is_responses("/v1/responses?x=1"));
        assert!(!is_responses("/models") && !is_responses("/v1/chat/completions") && !is_responses("/responses/compact"));
    }

    #[test]
    fn a_codex_request_becomes_a_chat_request() {
        let body = json!({
            "model": "nvidia/x",
            "instructions": "You are a coding agent.",
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "reasoning": { "effort": "medium" },
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "tools": [
                { "type": "function", "name": "exec_command", "description": "Run a command.", "strict": false, "parameters": { "type": "object", "properties": { "cmd": { "type": "string" } } } },
                { "type": "web_search" }
            ],
            "input": [
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": "Sandbox: workspace-write." }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Write made.txt." }] },
                { "type": "reasoning", "summary": [], "encrypted_content": "zzz" },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "On it." }] },
                { "type": "function_call", "call_id": "c1", "name": "exec_command", "arguments": "{\"cmd\":\"ls\"}" },
                { "type": "function_call", "call_id": "c2", "name": "exec_command", "arguments": "{\"cmd\":\"pwd\"}" },
                { "type": "function_call_output", "call_id": "c1", "output": "a.txt" },
                { "type": "function_call_output", "call_id": "c2", "output": [{ "type": "input_text", "text": "/w" }] }
            ]
        });
        let chat = to_chat(&body);
        assert_eq!(chat["model"], "nvidia/x");
        assert_eq!((chat["stream"].as_bool(), chat["stream_options"]["include_usage"].as_bool()), (Some(true), Some(true)));
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages[0], json!({ "role": "system", "content": "You are a coding agent.\n\nSandbox: workspace-write." }));
        assert_eq!(messages[1], json!({ "role": "user", "content": "Write made.txt." }));
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "On it.");
        let calls = messages[2]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2, "calls made together go together");
        assert_eq!(calls[0], json!({ "id": "c1", "type": "function", "function": { "name": "exec_command", "arguments": "{\"cmd\":\"ls\"}" } }));
        assert_eq!(messages[3], json!({ "role": "tool", "tool_call_id": "c1", "content": "a.txt" }));
        assert_eq!(messages[4], json!({ "role": "tool", "tool_call_id": "c2", "content": "/w" }));
        assert_eq!(messages.len(), 5, "reasoning stays out");
        assert_eq!(chat["tools"].as_array().unwrap().len(), 1, "hosted tools stay out");
        assert_eq!(chat["tools"][0], json!({ "type": "function", "function": { "name": "exec_command", "description": "Run a command.", "parameters": { "type": "object", "properties": { "cmd": { "type": "string" } } } } }));
        assert_eq!((chat["tool_choice"].as_str(), chat["parallel_tool_calls"].as_bool()), (Some("auto"), Some(false)));
        for gone in ["store", "include", "reasoning", "instructions", "input"] {
            assert!(chat.get(gone).is_none(), "{gone}");
        }
    }

    #[test]
    fn a_call_with_nothing_said_before_it_has_no_text() {
        let chat = to_chat(&json!({ "model": "m", "input": [
            { "type": "message", "role": "user", "content": "go" },
            { "type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}" },
            { "type": "function_call_output", "call_id": "c1", "output": "ok" },
            { "type": "message", "role": "assistant", "content": [] }
        ] }));
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3, "the empty assistant message is left out: {messages:?}");
        assert_eq!(messages[1]["content"], Value::Null);
        assert!(chat.get("tools").is_none() && chat.get("tool_choice").is_none());
    }

    #[test]
    fn a_chat_stream_becomes_a_response() {
        let mut t = Translator::new("r1");
        let mut events = t.feed(&json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Hel" } }] }));
        events.extend(t.feed(&json!({ "choices": [{ "index": 0, "delta": { "content": "lo" } }] })));
        events.extend(t.feed(&json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": "call_a", "type": "function", "function": { "name": "exec_command", "arguments": "{\"cmd\":" } }] } }] })));
        events.extend(t.feed(&json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "\"ls\"}" } }] } }] })));
        events.extend(t.feed(&json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] })));
        events.extend(t.feed(&json!({ "choices": [], "usage": { "prompt_tokens": 100, "completion_tokens": 7, "total_tokens": 107, "prompt_tokens_details": { "cached_tokens": 40 } } })));
        events.extend(t.finish());
        assert!(t.finish().is_empty(), "once");
        let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, [
            "response.created",
            "response.output_item.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_item.done",
            "response.output_item.added",
            "response.output_item.done",
            "response.completed",
        ]);
        assert_eq!(events[4]["item"]["content"][0]["text"], "Hello");
        assert_eq!(events[6]["item"], json!({ "type": "function_call", "id": "fc_r1_0", "call_id": "call_a", "name": "exec_command", "arguments": "{\"cmd\":\"ls\"}", "status": "completed" }));
        let done = &events[7]["response"];
        assert_eq!(done["id"], "r1");
        assert_eq!(done["output"].as_array().unwrap().len(), 2);
        assert_eq!(done["usage"], json!({ "input_tokens": 100, "input_tokens_details": { "cached_tokens": 40 }, "output_tokens": 7, "output_tokens_details": { "reasoning_tokens": 0 }, "total_tokens": 107 }));
    }

    #[test]
    fn an_empty_stream_still_completes_and_a_call_without_an_id_gets_one() {
        let mut t = Translator::new("r2");
        let none = t.finish();
        assert_eq!(none.iter().map(|e| e["type"].as_str().unwrap()).collect::<Vec<_>>(), ["response.created", "response.completed"]);
        let mut t = Translator::new("r3");
        t.feed(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "name": "hand_in", "arguments": "" } }] } }] }));
        let done = t.finish();
        let call = &done.iter().find(|e| e["type"] == "response.output_item.done").unwrap()["item"];
        assert_eq!((call["call_id"].as_str(), call["arguments"].as_str()), (Some("call_r3_0"), Some("{}")));
        assert!(sse_event(&done[0]).starts_with("event: response.output_item.added\ndata: {"));
    }
}
