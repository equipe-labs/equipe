//! A stand-in outside MCP server on this machine's loopback address, for the
//! tests of equipe's own connection to one: an OAuth-protected streamable-HTTP
//! server (`/mcp`), its protected-resource and authorization-server metadata,
//! dynamic client registration, an authorization page that approves at once
//! (redirecting with a code), and a token endpoint that checks PKCE and
//! renews with a refresh token; a keyed server (`/keyed`, `x-api-key`); and
//! the older HTTP+SSE transport (`/sse`, posting to `/messages`).
//!
//! Each request is answered on a connection of its own (`connection: close`).

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

pub const KEY: &str = "k-fake-0123456789";

#[derive(Default)]
pub struct State {
    pub port: u16,
    /// Access tokens it takes now.
    pub valid: HashSet<String>,
    pub refresh: HashSet<String>,
    /// Registered clients and their redirect addresses.
    pub clients: HashMap<String, Vec<String>>,
    /// Codes given, with their PKCE challenge and redirect.
    pub codes: HashMap<String, (String, String)>,
    pub registrations: usize,
    pub refreshes: usize,
    pub issued: usize,
    /// The tools called, by name, and the token or key each came with.
    pub calls: Vec<(String, String)>,
    /// What every request's headers carried for credentials, to show none came from a member.
    pub seen_auth: Vec<String>,
}

#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<State>>,
    pub port: u16,
    events: broadcast::Sender<String>,
}

impl Fake {
    pub async fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State { port, ..State::default() }));
        let (events, _) = broadcast::channel(64);
        let fake = Fake { state, port, events };
        let serving = fake.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let fake = serving.clone();
                tokio::spawn(async move { fake.answer(stream).await });
            }
        });
        fake
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// The server stops taking every access token it gave (as when one runs out early).
    pub fn revoke_all(&self) {
        self.state.lock().unwrap().valid.clear();
    }

    async fn answer(&self, stream: TcpStream) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or_default().to_owned(), parts.next().unwrap_or_default().to_owned());
        let mut headers = HashMap::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).await.is_err() || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }
        let length: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
        let mut body = vec![0u8; length];
        if length > 0 && reader.read_exact(&mut body).await.is_err() {
            return;
        }
        let body = String::from_utf8_lossy(&body).into_owned();
        let mut stream = reader.into_inner();
        let (path, query) = target.split_once('?').map_or((target.as_str(), ""), |(p, q)| (p, q));
        let reply = self.route(&method, path, query, &headers, &body, &mut stream).await;
        if let Some(reply) = reply {
            let _ = stream.write_all(reply.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    async fn route(&self, method: &str, path: &str, query: &str, headers: &HashMap<String, String>, body: &str, stream: &mut TcpStream) -> Option<String> {
        let base = self.base();
        let auth = headers.get("authorization").cloned().or_else(|| headers.get("x-api-key").map(|k| format!("key {k}"))).unwrap_or_default();
        self.state.lock().unwrap().seen_auth.push(auth.clone());
        match (method, path) {
            ("GET", "/.well-known/oauth-protected-resource/mcp") => Some(json_reply(200, &json!({ "resource": format!("{base}/mcp"), "authorization_servers": [format!("{base}/auth")], "scopes_supported": ["mcp"] }), "")),
            ("GET", "/.well-known/oauth-protected-resource/sse") => Some(json_reply(200, &json!({ "resource": format!("{base}/sse"), "authorization_servers": [format!("{base}/auth")] }), "")),
            ("GET", "/.well-known/oauth-authorization-server/auth") => Some(json_reply(
                200,
                &json!({ "issuer": format!("{base}/auth"), "authorization_endpoint": format!("{base}/auth/authorize"), "token_endpoint": format!("{base}/auth/token"), "registration_endpoint": format!("{base}/auth/register"), "code_challenge_methods_supported": ["S256"] }),
                "",
            )),
            ("POST", "/auth/register") => {
                let asked: Value = serde_json::from_str(body).unwrap_or_default();
                let mut s = self.state.lock().unwrap();
                s.registrations += 1;
                let id = format!("client-{}", s.registrations);
                let redirects = asked["redirect_uris"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                s.clients.insert(id.clone(), redirects);
                Some(json_reply(201, &json!({ "client_id": id, "token_endpoint_auth_method": "none" }), ""))
            }
            ("GET", "/auth/authorize") => {
                let q = form(query);
                let mut s = self.state.lock().unwrap();
                let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
                if !s.clients.get(q.get("client_id").map(String::as_str).unwrap_or_default()).is_some_and(|r| r.contains(&redirect)) || q.get("code_challenge_method").map(String::as_str) != Some("S256") {
                    return Some(json_reply(400, &json!({ "error": "invalid_request" }), ""));
                }
                s.issued += 1;
                let code = format!("code-{}", s.issued);
                s.codes.insert(code.clone(), (q.get("code_challenge").cloned().unwrap_or_default(), redirect.clone()));
                let state = q.get("state").cloned().unwrap_or_default();
                Some(format!("HTTP/1.1 302 Found\r\nlocation: {redirect}?code={code}&state={state}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"))
            }
            ("POST", "/auth/token") => {
                let f = form(body);
                let mut s = self.state.lock().unwrap();
                match f.get("grant_type").map(String::as_str) {
                    Some("authorization_code") => {
                        let Some((challenge, redirect)) = f.get("code").and_then(|c| s.codes.remove(c)) else {
                            return Some(json_reply(400, &json!({ "error": "invalid_grant" }), ""));
                        };
                        let verifier = f.get("code_verifier").cloned().unwrap_or_default();
                        if b64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref()) != challenge || f.get("redirect_uri") != Some(&redirect) {
                            return Some(json_reply(400, &json!({ "error": "invalid_grant", "error_description": "PKCE" }), ""));
                        }
                        s.issued += 1;
                        let (access, refresh) = (format!("at-{}", s.issued), format!("rt-{}", s.issued));
                        s.valid.insert(access.clone());
                        s.refresh.insert(refresh.clone());
                        Some(json_reply(200, &json!({ "access_token": access, "refresh_token": refresh, "token_type": "Bearer", "expires_in": 3600 }), ""))
                    }
                    Some("refresh_token") => {
                        if !f.get("refresh_token").is_some_and(|r| s.refresh.contains(r)) {
                            return Some(json_reply(400, &json!({ "error": "invalid_grant" }), ""));
                        }
                        s.refreshes += 1;
                        s.issued += 1;
                        let access = format!("at-{}", s.issued);
                        s.valid.insert(access.clone());
                        Some(json_reply(200, &json!({ "access_token": access, "token_type": "Bearer", "expires_in": 3600 }), ""))
                    }
                    _ => Some(json_reply(400, &json!({ "error": "unsupported_grant_type" }), "")),
                }
            }
            ("POST", "/mcp") => {
                let token = auth.strip_prefix("Bearer ").unwrap_or_default().to_owned();
                if !self.state.lock().unwrap().valid.contains(&token) {
                    return Some(unauthorized(&format!("{base}/.well-known/oauth-protected-resource/mcp")));
                }
                Some(self.mcp_reply(body, &token, true))
            }
            ("DELETE", "/mcp") | ("DELETE", "/keyed") => Some(json_reply(200, &json!({}), "")),
            ("POST", "/keyed") => {
                if headers.get("x-api-key").map(String::as_str) != Some(KEY) {
                    return Some("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned());
                }
                Some(self.mcp_reply(body, &format!("key {KEY}"), false))
            }
            ("GET", "/sse") => {
                let token = auth.strip_prefix("Bearer ").unwrap_or_default().to_owned();
                if !self.state.lock().unwrap().valid.contains(&token) {
                    return Some(unauthorized(&format!("{base}/.well-known/oauth-protected-resource/sse")));
                }
                let mut rx = self.events.subscribe();
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\r\nevent: endpoint\ndata: /messages?session=s1\n\n").await;
                while let Ok(message) = rx.recv().await {
                    if stream.write_all(format!("event: message\ndata: {message}\n\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
                None
            }
            ("POST", "/messages") => {
                let token = auth.strip_prefix("Bearer ").unwrap_or_default().to_owned();
                if !self.state.lock().unwrap().valid.contains(&token) {
                    return Some(unauthorized(&format!("{base}/.well-known/oauth-protected-resource/sse")));
                }
                if let Some(reply) = self.answer_message(body, &token) {
                    let _ = self.events.send(reply.to_string());
                }
                Some("HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned())
            }
            _ => Some("HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()),
        }
    }

    /// A message's answer, `None` for a notification.
    fn answer_message(&self, body: &str, credential: &str) -> Option<Value> {
        let message: Value = serde_json::from_str(body).ok()?;
        let id = message.get("id").filter(|i| !i.is_null())?.clone();
        Some(match message["method"].as_str().unwrap_or_default() {
            "initialize" => json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake", "version": "1" } } }),
            "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [{ "name": "search_screens", "description": "Find product screens", "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } } } }] } }),
            "tools/call" => {
                let name = message["params"]["name"].as_str().unwrap_or_default().to_owned();
                let query = message["params"]["arguments"]["query"].as_str().unwrap_or_default().to_owned();
                self.state.lock().unwrap().calls.push((name.clone(), credential.to_owned()));
                json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": format!("3 screens for {query}") }] } })
            }
            other => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("no {other}") } }),
        })
    }

    /// Streamable HTTP: `initialize` as JSON with a session, `tools/list` as an event stream, the rest as JSON.
    fn mcp_reply(&self, body: &str, credential: &str, stream_lists: bool) -> String {
        let message: Value = serde_json::from_str(body).unwrap_or_default();
        let Some(reply) = self.answer_message(body, credential) else {
            return "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned();
        };
        if stream_lists && message["method"] == "tools/list" {
            let text = format!("event: message\ndata: {reply}\n\n");
            return format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
        }
        let session = if message["method"] == "initialize" { "mcp-session-id: s-1\r\n" } else { "" };
        json_reply(200, &reply, session)
    }
}

fn json_reply(status: u16, body: &Value, extra: &str) -> String {
    let text = body.to_string();
    format!("HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{text}", text.len())
}

fn unauthorized(metadata: &str) -> String {
    format!("HTTP/1.1 401 Unauthorized\r\nwww-authenticate: Bearer resource_metadata=\"{metadata}\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unpct(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(text: &str) -> HashMap<String, String> {
    text.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (unpct(k), unpct(v))).collect()
}

/// What a browser does with a sign-in page here: open it, and follow its
/// redirect back to where equipe waits. The page it lands on, as text.
pub async fn browse(url: &str) -> String {
    let http = reqwest::Client::builder().build().unwrap();
    let page = http.get(url).send().await.expect("the page opens");
    page.text().await.unwrap_or_default()
}
