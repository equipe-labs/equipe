//! The daemon's half of an outside MCP server's proxy: a member's engine
//! runs `equipe mcp proxy --squad … --server … --member …` as a stdio MCP
//! server; that process holds nothing, and hands each JSON-RPC message to
//! the daemon over its socket ([`crate::gateway`]), where this forwards it to
//! the server — streamable HTTP, or the older HTTP+SSE — with the server's
//! credential added (an OAuth access token equipe keeps, or a kept API key),
//! and hands back every message the server sends. The credential never
//! leaves the daemon: it isn't in the member's config, environment, files
//! or process.
//!
//! A 401 asks for a fresh token once (the daemon refreshes it); a second
//! one means the person signs in again, which the daemon puts on a card.
//! What a call shows of a server is reported back, so the squad's status is
//! equipe's own connection's.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::protocol::mcp_servers::McpTransport;

/// Where a server is reached for one member, and the header that carries its credential.
#[derive(Clone, PartialEq, Eq)]
pub struct Upstream {
    pub url: String,
    pub transport: McpTransport,
    pub header: Option<(String, String)>,
    /// Given only for its tools that read ([`ReadOnly`]): the PM's design
    /// library search, to show references with.
    pub read_only: bool,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upstream").field("url", &self.url).field("transport", &self.transport).field("header", &self.header.as_ref().map(|(n, _)| format!("{n}: •••"))).field("read_only", &self.read_only).finish()
    }
}

/// What a forwarded message showed of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// It answered `initialize`.
    Connected,
    /// It refused the credential, fresh or not: the person signs in (again).
    NeedsSignIn(String),
    /// It couldn't be reached, or answered with an error.
    Failing(String),
}

/// Ask the daemon for the server's address and credential; `Some(value)`:
/// the server just refused that credential's value (refresh it).
pub type Ask = Arc<dyn Fn(Option<String>) -> Pin<Box<dyn Future<Output = Result<Upstream, String>> + Send>> + Send + Sync>;
/// Tell the daemon what a message showed.
pub type Tell = Arc<dyn Fn(Seen) + Send + Sync>;

/// The HTTP client a proxy forwards with: no overall timeout (a tool call
/// may take minutes, an event stream lasts), a bounded connect.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).build().unwrap_or_default()
}

/// A JSON-RPC error answering `message`, when it was a request.
pub fn error_reply(message: &Value, why: &str) -> Option<Value> {
    let id = message.get("id").filter(|id| !id.is_null())?;
    message.get("method")?;
    Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": why } }))
}

/// What a read-only proxy lets through of a server's tools: those its server
/// marks as only reading (`annotations.readOnlyHint`), else those whose name
/// says they look something up — search, find, list, get, browse, fetch,
/// read, view, inspiration — and never one whose name says it makes,
/// changes or removes anything. A tool it keeps back is left out of
/// `tools/list` and refused if called.
#[derive(Debug, Default)]
pub struct ReadOnly {
    /// The ids of the `tools/list` requests on their way.
    listing: std::collections::HashSet<String>,
    /// The tools a listing said read.
    reads: std::collections::HashSet<String>,
}

/// Words in a tool's name that say it reads, and that it writes.
const READ_WORDS: [&str; 11] = ["search", "find", "list", "get", "browse", "fetch", "read", "view", "inspiration", "lookup", "query"];
const WRITE_WORDS: [&str; 16] = ["create", "update", "delete", "remove", "write", "edit", "build", "generate", "refine", "make", "save", "upload", "set", "add", "publish", "send"];

impl ReadOnly {
    /// Whether a tool, as `tools/list` describes it, only reads.
    pub fn reads(tool: &Value) -> bool {
        let name = tool["name"].as_str().unwrap_or_default().to_ascii_lowercase();
        if tool["annotations"]["destructiveHint"] == true {
            return false;
        }
        if tool["annotations"]["readOnlyHint"] == true {
            return true;
        }
        let words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
        !words.iter().any(|w| WRITE_WORDS.contains(w)) && words.iter().any(|w| READ_WORDS.iter().any(|r| w.starts_with(r)))
    }

    /// A message from the member's engine: passed on (`Ok`), or answered here (`Err`).
    pub fn outgoing(&mut self, message: Value) -> Result<Value, Value> {
        match message["method"].as_str() {
            Some("tools/list") => {
                if let Some(id) = message.get("id").filter(|id| !id.is_null()) {
                    self.listing.insert(id.to_string());
                }
                Ok(message)
            }
            Some("tools/call") => {
                let name = message["params"]["name"].as_str().unwrap_or_default();
                if self.reads.contains(name) {
                    return Ok(message);
                }
                let why = format!("{name} isn't open to you: you may only search this library, to show the person references with `show_references`.");
                Err(error_reply(&message, &why).unwrap_or(Value::Null))
            }
            _ => Ok(message),
        }
    }

    /// A message from the server: a listing of tools has those that don't read taken out.
    pub fn incoming(&mut self, mut message: Value) -> Value {
        let listed = message.get("id").is_some_and(|id| self.listing.remove(&id.to_string()));
        if listed && let Some(tools) = message["result"]["tools"].as_array_mut() {
            tools.retain(Self::reads);
            self.reads.extend(tools.iter().filter_map(|t| t["name"].as_str().map(str::to_owned)));
        }
        message
    }
}

/// Server-sent events, read as they arrive: each event's name and data.
#[derive(Default)]
pub struct Sse {
    buf: String,
    event: Option<String>,
    data: Vec<String>,
}

impl Sse {
    /// More of the stream; the events it completed.
    pub fn feed(&mut self, chunk: &str) -> Vec<(Option<String>, String)> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(at) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=at).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push((self.event.take(), self.data.join("\n")));
                }
                self.data.clear();
                self.event = None;
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data.push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            } else if let Some(event) = line.strip_prefix("event:") {
                self.event = Some(event.trim().to_owned());
            }
        }
        out
    }
}

/// The JSON-RPC messages in a body: one, or a batch.
fn messages_of(text: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(Value::Array(all)) => all,
        Ok(one @ Value::Object(_)) => vec![one],
        _ => Vec::new(),
    }
}

fn reach_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the server didn't answer in time".to_owned()
    } else if e.is_connect() {
        "the server couldn't be reached".to_owned()
    } else {
        "the connection to the server failed".to_owned()
    }
}

/// The state one proxy's streamable-HTTP conversation keeps.
#[derive(Default)]
struct Session {
    id: Option<String>,
    version: Option<String>,
}

/// Forward one proxy's messages until it goes: what comes `from_client` to
/// the server, what the server says `to_client`. `on_call` hears the name of
/// each tool called (never its arguments).
pub async fn serve(http: reqwest::Client, ask: Ask, tell: Tell, mut from_client: UnboundedReceiver<Value>, to_client: UnboundedSender<Value>, on_call: Arc<dyn Fn(&str) + Send + Sync>) {
    let first = match ask(None).await {
        Ok(up) => up,
        Err(why) => {
            // Every request is answered with why, so the engine says it.
            while let Some(message) = from_client.recv().await {
                if let Some(reply) = error_reply(&message, &why) {
                    let _ = to_client.send(reply);
                }
            }
            return;
        }
    };
    match first.transport {
        McpTransport::Http => streamable(http, ask, tell, from_client, to_client, on_call).await,
        McpTransport::Sse => legacy_sse(http, ask, tell, first, from_client, to_client, on_call).await,
    }
}

async fn streamable(http: reqwest::Client, ask: Ask, tell: Tell, mut from_client: UnboundedReceiver<Value>, to_client: UnboundedSender<Value>, on_call: Arc<dyn Fn(&str) + Send + Sync>) {
    let session = Arc::new(Mutex::new(Session::default()));
    while let Some(message) = from_client.recv().await {
        if message["method"] == "tools/call"
            && let Some(name) = message["params"]["name"].as_str()
        {
            on_call(name);
        }
        let (http, ask, tell, to_client, session) = (http.clone(), ask.clone(), tell.clone(), to_client.clone(), session.clone());
        // Each on its own: a long tool call holds up nothing else.
        tokio::spawn(async move {
            if let Err(why) = post(&http, &ask, &tell, &message, &to_client, &session).await
                && let Some(reply) = error_reply(&message, &why)
            {
                let _ = to_client.send(reply);
            }
        });
    }
    // Gone: the server's session ends with it, politely.
    let (id, up) = (session.lock().ok().and_then(|s| s.id.clone()), ask(None).await);
    if let (Some(id), Ok(up)) = (id, up) {
        let mut request = http.delete(&up.url).header("mcp-session-id", id);
        if let Some((name, value)) = &up.header {
            request = request.header(name.as_str(), value.as_str());
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await;
    }
}

async fn post(http: &reqwest::Client, ask: &Ask, tell: &Tell, message: &Value, to_client: &UnboundedSender<Value>, session: &Mutex<Session>) -> Result<(), String> {
    let mut up = ask(None).await?;
    let mut retried = false;
    let response = loop {
        let mut request = http.post(&up.url).header("content-type", "application/json").header("accept", "application/json, text/event-stream").body(message.to_string());
        if let Ok(s) = session.lock() {
            if let Some(id) = &s.id {
                request = request.header("mcp-session-id", id.as_str());
            }
            if let Some(version) = &s.version {
                request = request.header("mcp-protocol-version", version.as_str());
            }
        }
        if let Some((name, value)) = &up.header {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                let why = reach_error(&e);
                tell(Seen::Failing(why.clone()));
                return Err(why);
            }
        };
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            if !retried {
                retried = true;
                // The credential was refused: a fresh one, once.
                let rejected = up.header.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
                match ask(Some(rejected)).await {
                    Ok(fresh) => {
                        up = fresh;
                        continue;
                    }
                    Err(why) => {
                        tell(Seen::NeedsSignIn(why.clone()));
                        return Err(why);
                    }
                }
            }
            let why = "the server refused equipe's sign-in: the person was asked to sign in again".to_owned();
            tell(Seen::NeedsSignIn(why.clone()));
            return Err(why);
        }
        break response;
    };
    let status = response.status();
    if let Some(id) = response.headers().get("mcp-session-id").and_then(|v| v.to_str().ok())
        && let Ok(mut s) = session.lock()
    {
        s.id = Some(id.to_owned());
    }
    if status.as_u16() == 404 && session.lock().ok().is_some_and(|s| s.id.is_some()) {
        if let Ok(mut s) = session.lock() {
            s.id = None;
        }
        return Err("the server ended its session: start again".to_owned());
    }
    if status.as_u16() == 202 || status.as_u16() == 204 {
        return Ok(());
    }
    if !status.is_success() {
        let why = format!("the server answered {}", status.as_u16());
        tell(Seen::Failing(why.clone()));
        return Err(why);
    }
    let initialize = message["method"] == "initialize";
    let deliver = |reply: Value| {
        if initialize && reply.get("result").is_some() {
            if let (Some(version), Ok(mut s)) = (reply["result"]["protocolVersion"].as_str(), session.lock()) {
                s.version = Some(version.to_owned());
            }
            tell(Seen::Connected);
        }
        let _ = to_client.send(reply);
    };
    let streamed = response.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/event-stream"));
    let mut response = response;
    if streamed {
        let mut sse = Sse::default();
        while let Ok(Some(chunk)) = response.chunk().await {
            for (_, data) in sse.feed(&String::from_utf8_lossy(&chunk)) {
                for reply in messages_of(&data) {
                    deliver(reply);
                }
            }
        }
    } else {
        let body = response.bytes().await.map_err(|e| reach_error(&e))?;
        for reply in messages_of(&String::from_utf8_lossy(&body)) {
            deliver(reply);
        }
    }
    Ok(())
}

/// The older HTTP+SSE transport: one event stream the server answers on,
/// whose first `endpoint` event says where to post.
async fn legacy_sse(http: reqwest::Client, ask: Ask, tell: Tell, first: Upstream, mut from_client: UnboundedReceiver<Value>, to_client: UnboundedSender<Value>, on_call: Arc<dyn Fn(&str) + Send + Sync>) {
    let mut up = first;
    let mut retried = false;
    let response = loop {
        let mut request = http.get(&up.url).header("accept", "text/event-stream");
        if let Some((name, value)) = &up.header {
            request = request.header(name.as_str(), value.as_str());
        }
        match request.send().await {
            Ok(r) if (r.status().as_u16() == 401 || r.status().as_u16() == 403) && !retried => {
                retried = true;
                match ask(Some(up.header.as_ref().map(|(_, v)| v.clone()).unwrap_or_default())).await {
                    Ok(fresh) => up = fresh,
                    Err(why) => break Err(why),
                }
            }
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => break Err("the server refused equipe's sign-in: the person was asked to sign in again".to_owned()),
            Ok(r) if r.status().is_success() => break Ok(r),
            Ok(r) => break Err(format!("the server answered {}", r.status().as_u16())),
            Err(e) => break Err(reach_error(&e)),
        }
    };
    let mut response = match response {
        Ok(r) => r,
        Err(why) => {
            tell(if why.contains("sign") { Seen::NeedsSignIn(why.clone()) } else { Seen::Failing(why.clone()) });
            while let Some(message) = from_client.recv().await {
                if let Some(reply) = error_reply(&message, &why) {
                    let _ = to_client.send(reply);
                }
            }
            return;
        }
    };
    let base = up.url.clone();
    let (endpoint_tx, mut endpoint_rx) = tokio::sync::watch::channel::<Option<String>>(None);
    let reader = {
        let (to_client, tell) = (to_client.clone(), tell.clone());
        tokio::spawn(async move {
            let mut sse = Sse::default();
            while let Ok(Some(chunk)) = response.chunk().await {
                for (event, data) in sse.feed(&String::from_utf8_lossy(&chunk)) {
                    match event.as_deref() {
                        Some("endpoint") => {
                            let at = reqwest::Url::parse(&base).and_then(|b| b.join(data.trim())).map(|u| u.to_string()).unwrap_or_default();
                            let _ = endpoint_tx.send(Some(at));
                        }
                        _ => {
                            for reply in messages_of(&data) {
                                if reply["result"]["protocolVersion"].is_string() {
                                    tell(Seen::Connected);
                                }
                                let _ = to_client.send(reply);
                            }
                        }
                    }
                }
            }
        })
    };
    while let Some(message) = from_client.recv().await {
        if message["method"] == "tools/call"
            && let Some(name) = message["params"]["name"].as_str()
        {
            on_call(name);
        }
        let endpoint = loop {
            if let Some(at) = endpoint_rx.borrow().clone() {
                break Some(at);
            }
            if endpoint_rx.changed().await.is_err() {
                break None;
            }
        };
        let Some(endpoint) = endpoint.filter(|e| !e.is_empty()) else {
            if let Some(reply) = error_reply(&message, "the server's event stream ended") {
                let _ = to_client.send(reply);
            }
            continue;
        };
        let mut request = http.post(&endpoint).header("content-type", "application/json").body(message.to_string());
        if let Some((name, value)) = &up.header {
            request = request.header(name.as_str(), value.as_str());
        }
        match request.send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => {
                if let Some(reply) = error_reply(&message, &format!("the server answered {}", r.status().as_u16())) {
                    let _ = to_client.send(reply);
                }
            }
            Err(e) => {
                tell(Seen::Failing(reach_error(&e)));
                if let Some(reply) = error_reply(&message, &reach_error(&e)) {
                    let _ = to_client.send(reply);
                }
            }
        }
    }
    reader.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_read_across_chunks_with_their_names() {
        let mut sse = Sse::default();
        assert!(sse.feed("event: endpoint\ndata: /messages?s").is_empty());
        assert_eq!(sse.feed("=1\n\n"), [(Some("endpoint".to_owned()), "/messages?s=1".to_owned())]);
        let got = sse.feed("data: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":1}\r\n\r\n: keep-alive\n\n");
        assert_eq!(got, [(None, "{\"jsonrpc\":\"2.0\",\n\"id\":1}".to_owned())]);
    }

    #[test]
    fn a_request_gets_an_error_and_a_notification_nothing() {
        let reply = error_reply(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }), "needs sign-in").unwrap();
        assert_eq!((reply["id"].as_i64(), reply["error"]["message"].as_str()), (Some(4), Some("needs sign-in")));
        assert!(error_reply(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }), "x").is_none());
        assert!(error_reply(&json!({ "jsonrpc": "2.0", "id": 4, "result": {} }), "x").is_none(), "an answer to the server's own request isn't answered");
    }

    #[test]
    fn an_upstream_never_shows_its_credential() {
        let up = Upstream { url: "https://x.example.com/mcp".into(), transport: McpTransport::Http, header: Some(("authorization".into(), "Bearer at-secret".into())), read_only: false };
        assert!(!format!("{up:?}").contains("at-secret"));
    }

    #[test]
    fn a_read_only_proxy_lists_and_lets_through_only_the_tools_that_read() {
        let mut filter = ReadOnly::default();
        let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        assert_eq!(filter.outgoing(list.clone()), Ok(list));
        let listing = json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [
            { "name": "search_screens" },
            { "name": "get_flow" },
            { "name": "21st_magic_component_inspiration" },
            { "name": "21st_magic_component_builder" },
            { "name": "logo_search", "annotations": { "destructiveHint": true } },
            { "name": "save_to_collection" },
            { "name": "summarize", "annotations": { "readOnlyHint": true } },
            { "name": "create_or_get" },
        ] } });
        let shown = filter.incoming(listing);
        let names: Vec<&str> = shown["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["search_screens", "get_flow", "21st_magic_component_inspiration", "summarize"]);
        let call = |name: &str| json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": name, "arguments": {} } });
        assert!(filter.outgoing(call("search_screens")).is_ok());
        let refused = filter.outgoing(call("21st_magic_component_builder")).unwrap_err();
        assert_eq!(refused["id"], 9);
        assert!(refused["error"]["message"].as_str().unwrap().contains("show_references"));
        assert!(filter.outgoing(call("unlisted")).is_err(), "a tool no listing said reads is refused");
        // Another message of the server's, with an id no listing had, passes as it is.
        let other = json!({ "jsonrpc": "2.0", "id": 3, "result": { "tools": [{ "name": "create_x" }] } });
        assert_eq!(filter.incoming(other.clone()), other);
    }
}
