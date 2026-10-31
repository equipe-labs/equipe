//! A relay on this machine's loopback between `codex app-server` and another
//! provider speaking OpenAI's Responses API, for one turn. What Codex sends
//! goes on unchanged but for two things:
//!
//! - tools grouped in a `namespace` (how Codex 0.154 sends an MCP server's
//!   tools, equipe's board among them) become plain function tools named
//!   `<namespace>__<tool>`, and the calls the model makes to them come back
//!   to Codex in its own shape (`namespace` and `name`) — a provider that
//!   serves only function tools then serves them all;
//! - Codex's own metadata for OpenAI (`client_metadata`, `x-codex-*` headers:
//!   the workspace's paths, an installation id) stays here.
//!
//! The other headers go on as they came, the key among them — passed through,
//! never read, kept or logged. The model's answer streams back as it streams in.
//!
//! To a provider that speaks Chat Completions ([`Wire::Chat`]) each
//! Responses request is said in chat, and its answer back in Responses
//! ([`crate::chat_wire`]): any model such a server has can be a member's.

use std::collections::HashMap;

use futures_util::StreamExt as _;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The API a provider is asked in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Wire {
    /// OpenAI's Responses API, as Codex asks: relayed as it is.
    Responses,
    /// Chat Completions, which nearly every model server speaks.
    #[default]
    Chat,
}

impl Wire {
    /// `responses` or `chat` (and `chat_completions`, `chat-completions`).
    pub fn parse(said: &str) -> Option<Self> {
        match said.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "responses" => Some(Self::Responses),
            "chat" | "chat_completions" | "completions" => Some(Self::Chat),
            _ => None,
        }
    }
}

/// Flattened tool names, back to their namespace and name.
pub type Names = HashMap<String, (String, String)>;

/// The flat name of `tool` in `namespace`: `mcp__equipe` and `take_ticket`
/// make `mcp__equipe__take_ticket`, as Claude Code names MCP tools.
pub fn flat_name(namespace: &str, tool: &str) -> String {
    format!("{}__{tool}", namespace.trim_end_matches('_'))
}

/// A request's body made plain: its namespaces' tools as function tools, and
/// earlier calls to them, in its input, by their flat names. What came from
/// where, to put the model's calls back.
pub fn flatten_request(body: &mut Value) -> Names {
    let mut names = Names::new();
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        let mut flat = Vec::with_capacity(tools.len());
        for tool in tools.drain(..) {
            if tool.get("type").and_then(Value::as_str) != Some("namespace") {
                flat.push(tool);
                continue;
            }
            let namespace = tool.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
            for mut inner in tool.get("tools").and_then(Value::as_array).cloned().unwrap_or_default() {
                let Some(name) = inner.get("name").and_then(Value::as_str).map(str::to_owned) else { continue };
                let joined = flat_name(&namespace, &name);
                inner["name"] = Value::String(joined.clone());
                names.insert(joined, (namespace.clone(), name));
                flat.push(inner);
            }
        }
        *tools = flat;
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input.iter_mut() {
            let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
            if !matches!(kind, "function_call" | "function_call_output") {
                continue;
            }
            let Some(map) = item.as_object_mut() else { continue };
            let Some(namespace) = map.remove("namespace").and_then(|n| n.as_str().map(str::to_owned)) else { continue };
            if let Some(name) = map.get("name").and_then(Value::as_str).map(str::to_owned) {
                let joined = flat_name(&namespace, &name);
                names.insert(joined.clone(), (namespace, name));
                map.insert("name".into(), Value::String(joined));
            }
        }
    }
    names
}

/// Codex's metadata for OpenAI taken out of a request's body. Whether there was any.
pub fn strip_metadata(body: &mut Value) -> bool {
    body.as_object_mut().is_some_and(|map| map.remove("client_metadata").is_some())
}

/// Whether a request header is Codex's own metadata, not for another provider.
fn codex_metadata(header: &str) -> bool {
    header.starts_with("x-codex-")
}

/// The model's calls to flattened tools, anywhere in a streamed event or a
/// whole response, put back as Codex named them. Whether anything changed.
pub fn restore(value: &mut Value, names: &Names) -> bool {
    if names.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut fix = |item: &mut Value| {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return;
        }
        let Some((namespace, name)) = item.get("name").and_then(Value::as_str).and_then(|n| names.get(n)).cloned() else { return };
        item["name"] = Value::String(name);
        item["namespace"] = Value::String(namespace);
        changed = true;
    };
    if let Some(item) = value.get_mut("item") {
        fix(item);
    }
    if let Some(Value::Array(items)) = value.get_mut("response").and_then(|r| r.get_mut("output")) {
        items.iter_mut().for_each(&mut fix);
    }
    if let Some(Value::Array(items)) = value.get_mut("output") {
        items.iter_mut().for_each(&mut fix);
    }
    changed
}

/// One line of a server-sent stream, its calls put back.
fn restore_line(line: &str, names: &Names) -> Option<String> {
    let data = line.strip_prefix("data:")?;
    let mut value: Value = serde_json::from_str(data.trim()).ok()?;
    restore(&mut value, names).then(|| format!("data: {value}"))
}

/// One request relayed: what the provider said, by its own request id, and
/// how long it took to the last byte. Nothing of what was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub path: String,
    /// The provider's status; 502 when it couldn't be reached.
    pub status: u16,
    /// The provider's id for it (`x-request-id`), when it names one.
    pub request_id: Option<String>,
    pub ms: u64,
}

type Exchanges = std::sync::Arc<std::sync::Mutex<Vec<Exchange>>>;

/// The relay, running until dropped.
pub struct Relay {
    /// Where Codex sends what it would send the provider: its base URL.
    pub url: String,
    exchanges: Exchanges,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    /// The requests relayed so far, in order.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges.lock().map(|e| e.clone()).unwrap_or_default()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Start a relay to the provider at `upstream` (its base URL, e.g.
/// `https://api.tokenfactory.nebius.com/v1`), asked in `wire`.
pub async fn start(upstream: &str, wire: Wire) -> std::io::Result<Relay> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let upstream = upstream.trim_end_matches('/').to_owned();
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).build().map_err(std::io::Error::other)?;
    let exchanges = Exchanges::default();
    let kept = exchanges.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (client, upstream, kept) = (client.clone(), upstream.clone(), kept.clone());
            tokio::spawn(async move {
                let _ = relay_one(stream, &client, &upstream, wire, &kept).await;
            });
        }
    });
    Ok(Relay { url, exchanges, task })
}

/// A request read off the loopback.
struct Incoming {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Headers that are the connection's own, not the request's.
const HOP: [&str; 6] = ["host", "content-length", "connection", "transfer-encoding", "accept-encoding", "keep-alive"];

async fn read_incoming(stream: &mut TcpStream) -> std::io::Result<Option<Incoming>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 1 << 20 {
            return Ok(None);
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (Some(method), Some(path)) = (first.next(), first.next()) else { return Ok(None) };
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned())).collect();
    let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    let mut body = buf[end + 4..].to_vec();
    if header("transfer-encoding").is_some_and(|t| t.eq_ignore_ascii_case("chunked")) {
        loop {
            if let Some(decoded) = dechunk(&body) {
                body = decoded;
                break;
            }
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Ok(None);
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else {
        let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        while body.len() < length {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(length);
    }
    Ok(Some(Incoming { method: method.to_owned(), path: path.to_owned(), headers, body }))
}

/// A chunked body, whole once its last chunk is in; `None` until then.
fn dechunk(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let line_end = raw[at..].windows(2).position(|w| w == b"\r\n")? + at;
        let size = usize::from_str_radix(String::from_utf8_lossy(&raw[at..line_end]).split(';').next()?.trim(), 16).ok()?;
        at = line_end + 2;
        if size == 0 {
            return Some(out);
        }
        if raw.len() < at + size + 2 {
            return None;
        }
        out.extend_from_slice(&raw[at..at + size]);
        at += size + 2;
    }
}

async fn relay_one(mut stream: TcpStream, client: &reqwest::Client, upstream: &str, wire: Wire, kept: &Exchanges) -> std::io::Result<()> {
    let Some(request) = read_incoming(&mut stream).await? else { return Ok(()) };
    let since = std::time::Instant::now();
    let path = request.path.clone();
    let keep = |status: u16, request_id: Option<String>| {
        let ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Ok(mut all) = kept.lock() {
            all.push(Exchange { path: path.clone(), status, request_id, ms });
        }
    };
    // Codex is given the relay's root as the provider's base URL: its paths are the provider's own.
    let mut url = format!("{upstream}/{}", request.path.trim_start_matches('/'));
    let mut body = request.body;
    let mut names = Names::new();
    let mut in_chat = false;
    let is_json = request.headers.iter().any(|(n, v)| n == "content-type" && v.contains("json"));
    let encoded = request.headers.iter().any(|(n, v)| n == "content-encoding" && !v.eq_ignore_ascii_case("identity"));
    if request.method == "POST" && is_json && !encoded
        && let Ok(mut value) = serde_json::from_slice::<Value>(&body)
    {
        names = flatten_request(&mut value);
        let stripped = strip_metadata(&mut value);
        if wire == Wire::Chat && crate::chat_wire::is_responses(&request.path) {
            in_chat = true;
            url = format!("{upstream}/chat/completions");
            body = serde_json::to_vec(&crate::chat_wire::to_chat(&value)).unwrap_or(body);
        } else if !names.is_empty() || stripped {
            body = serde_json::to_vec(&value).unwrap_or(body);
        }
    }
    let method = reqwest::Method::from_bytes(request.method.as_bytes()).unwrap_or(reqwest::Method::POST);
    let mut outgoing = client.request(method, &url).body(body);
    for (name, value) in &request.headers {
        if !HOP.contains(&name.as_str()) && !codex_metadata(name) && !(in_chat && name == "accept") {
            outgoing = outgoing.header(name.as_str(), value.as_str());
        }
    }
    if in_chat {
        outgoing = outgoing.header("accept", "text/event-stream");
    }
    let response = match outgoing.send().await {
        Ok(response) => response,
        Err(err) => {
            // The provider unreachable: said as a gateway's error, the way Codex expects one.
            let said = format!("{{\"error\":{{\"message\":\"the provider couldn't be reached: {}\"}}}}", err.without_url().to_string().replace('"', "'"));
            let head = format!("HTTP/1.1 502 Bad Gateway\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", said.len());
            keep(502, None);
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(said.as_bytes()).await?;
            return stream.shutdown().await;
        }
    };
    let status = response.status();
    let request_id = response.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(|v| v.chars().take(120).collect::<String>());
    if in_chat && status.is_success() {
        let id = request_id.clone().filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        answer_in_responses(&mut stream, response, &id, &names).await?;
        keep(status.as_u16(), request_id);
        stream.flush().await?;
        return stream.shutdown().await;
    }
    let mut head = format!("HTTP/1.1 {} {}\r\n", status.as_u16(), status.canonical_reason().unwrap_or(""));
    for (name, value) in response.headers() {
        let name = name.as_str();
        if HOP.contains(&name) || name == "content-encoding" {
            continue;
        }
        if let Ok(value) = value.to_str() {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("connection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    let streamed = response.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));
    let mut bytes = response.bytes_stream();
    if names.is_empty() {
        // Nothing to put back: as it comes.
        while let Some(Ok(chunk)) = bytes.next().await {
            stream.write_all(&chunk).await?;
        }
    } else if streamed {
        let mut pending: Vec<u8> = Vec::new();
        while let Some(Ok(chunk)) = bytes.next().await {
            pending.extend_from_slice(&chunk);
            while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=at).collect();
                let text = String::from_utf8_lossy(&line);
                match restore_line(text.trim_end_matches(['\r', '\n']), &names) {
                    Some(fixed) => stream.write_all(format!("{fixed}\n").as_bytes()).await?,
                    None => stream.write_all(&line).await?,
                }
            }
            stream.flush().await?;
        }
        stream.write_all(&pending).await?;
    } else {
        let mut all = Vec::new();
        while let Some(Ok(chunk)) = bytes.next().await {
            all.extend_from_slice(&chunk);
        }
        if let Ok(mut value) = serde_json::from_slice::<Value>(&all)
            && restore(&mut value, &names)
        {
            all = serde_json::to_vec(&value).unwrap_or(all);
        }
        stream.write_all(&all).await?;
    }
    keep(status.as_u16(), request_id);
    stream.flush().await?;
    stream.shutdown().await
}

/// A chat answer, streamed or whole, written back as the Responses events of one response.
async fn answer_in_responses(stream: &mut TcpStream, response: reqwest::Response, id: &str, names: &Names) -> std::io::Result<()> {
    use crate::chat_wire::{Translator, sse_event};
    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n").await?;
    let mut translator = Translator::new(id);
    let send = |events: Vec<Value>| {
        events
            .into_iter()
            .map(|mut event| {
                restore(&mut event, names);
                sse_event(&event)
            })
            .collect::<String>()
    };
    let streamed = response.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("event-stream"));
    let mut bytes = response.bytes_stream();
    if streamed {
        let mut pending: Vec<u8> = Vec::new();
        'read: while let Some(Ok(chunk)) = bytes.next().await {
            pending.extend_from_slice(&chunk);
            let mut out = String::new();
            while let Some(at) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=at).collect();
                let line = String::from_utf8_lossy(&line);
                let Some(data) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:").map(str::trim) else { continue };
                if data == "[DONE]" {
                    stream.write_all(out.as_bytes()).await?;
                    break 'read;
                }
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    out.push_str(&send(translator.feed(&value)));
                }
            }
            stream.write_all(out.as_bytes()).await?;
            stream.flush().await?;
        }
    } else {
        // A server that answered whole, though asked to stream: its message as one chunk.
        let mut all = Vec::new();
        while let Some(Ok(chunk)) = bytes.next().await {
            all.extend_from_slice(&chunk);
        }
        if let Ok(whole) = serde_json::from_slice::<Value>(&all) {
            let mut delta = whole.pointer("/choices/0/message").cloned().unwrap_or(Value::Null);
            if let Some(calls) = delta.get_mut("tool_calls").and_then(Value::as_array_mut) {
                for (n, call) in calls.iter_mut().enumerate() {
                    call["index"] = Value::from(n);
                }
            }
            let chunk = serde_json::json!({ "choices": [{ "delta": delta }], "usage": whole.get("usage").cloned().unwrap_or(Value::Null) });
            stream.write_all(send(translator.feed(&chunk)).as_bytes()).await?;
        }
    }
    stream.write_all(send(translator.finish()).as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value {
        json!({
            "model": "nvidia/x",
            "tools": [
                { "type": "function", "name": "exec_command", "parameters": {} },
                { "type": "namespace", "name": "mcp__equipe", "description": "Tools in the mcp__equipe namespace.", "tools": [
                    { "type": "function", "name": "take_ticket", "description": "Take a ticket.", "strict": false, "parameters": { "type": "object" } },
                    { "type": "function", "name": "hand_in", "parameters": { "type": "object" } }
                ] }
            ],
            "input": [
                { "type": "message", "role": "user", "content": [] },
                { "type": "function_call", "call_id": "c1", "name": "take_ticket", "namespace": "mcp__equipe", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "c1", "output": "ok" }
            ]
        })
    }

    #[test]
    fn namespaced_tools_go_as_function_tools_and_earlier_calls_by_their_flat_names() {
        let mut body = request();
        let names = flatten_request(&mut body);
        let tools: Vec<(&str, &str)> = body["tools"].as_array().unwrap().iter().map(|t| (t["type"].as_str().unwrap(), t["name"].as_str().unwrap())).collect();
        assert_eq!(tools, [("function", "exec_command"), ("function", "mcp__equipe__take_ticket"), ("function", "mcp__equipe__hand_in")]);
        assert_eq!(body["tools"][1]["description"], "Take a ticket.");
        assert_eq!(body["input"][1], json!({ "type": "function_call", "call_id": "c1", "name": "mcp__equipe__take_ticket", "arguments": "{}" }));
        assert_eq!(names.get("mcp__equipe__hand_in"), Some(&("mcp__equipe".to_owned(), "hand_in".to_owned())));
        assert_eq!(body["model"], "nvidia/x");
        // Nothing namespaced: nothing changes.
        let mut plain = json!({ "tools": [{ "type": "function", "name": "exec_command" }], "input": [] });
        let before = plain.clone();
        assert!(flatten_request(&mut plain).is_empty());
        assert_eq!(plain, before);
    }

    #[test]
    fn the_models_calls_come_back_as_codex_named_them() {
        let names = flatten_request(&mut request());
        let call = json!({ "type": "function_call", "id": "fc", "call_id": "c2", "name": "mcp__equipe__hand_in", "arguments": "{\"ticket\":3}" });
        let mut done = json!({ "type": "response.output_item.done", "output_index": 0, "item": call.clone() });
        assert!(restore(&mut done, &names));
        assert_eq!((done["item"]["namespace"].as_str(), done["item"]["name"].as_str()), (Some("mcp__equipe"), Some("hand_in")));
        assert_eq!(done["item"]["arguments"], "{\"ticket\":3}");
        let mut completed = json!({ "type": "response.completed", "response": { "id": "r", "output": [call.clone(), { "type": "function_call", "name": "exec_command" }] } });
        assert!(restore(&mut completed, &names));
        assert_eq!(completed["response"]["output"][0]["name"], "hand_in");
        assert_eq!(completed["response"]["output"][1], json!({ "type": "function_call", "name": "exec_command" }));
        let line = format!("data: {}", json!({ "type": "response.output_item.added", "item": call }));
        assert!(restore_line(&line, &names).is_some_and(|l| l.contains("\"namespace\":\"mcp__equipe\"")));
        assert_eq!(restore_line("event: response.completed", &names), None);
        assert_eq!(restore_line("data: [DONE]", &names), None);
    }

    #[test]
    fn codexs_metadata_for_openai_stays_here() {
        let mut body = json!({ "model": "nvidia/x", "client_metadata": { "x-codex-turn-metadata": "{\"workspaces\":{\"/Users/me/secret-project\":{}}}" }, "input": [] });
        assert!(strip_metadata(&mut body));
        assert_eq!(body, json!({ "model": "nvidia/x", "input": [] }));
        assert!(!strip_metadata(&mut body));
        assert!(codex_metadata("x-codex-turn-metadata") && codex_metadata("x-codex-window-id"));
        assert!(!codex_metadata("authorization") && !codex_metadata("x-request-id") && !codex_metadata("content-type"));
    }

    #[test]
    fn a_chunked_body_is_read_whole() {
        assert_eq!(dechunk(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n").as_deref(), Some(&b"hello world"[..]));
        assert_eq!(dechunk(b"5\r\nhel"), None);
    }
}
