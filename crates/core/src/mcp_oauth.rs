//! equipe as the OAuth client of an outside MCP server
//! ([`crate::squad_mcp`], [`McpAuth::Oauth`](crate::protocol::mcp_servers::McpAuth)):
//! the person signs in once, to equipe, and every member gets the server's
//! tools through equipe's own proxy whatever engine it runs on.
//!
//! The flow is the MCP authorization spec's:
//!
//! 1. The server's 401 names its protected-resource metadata (RFC 9728),
//!    which names its authorization server, whose metadata (RFC 8414, or
//!    OpenID discovery) gives the endpoints.
//! 2. equipe registers itself as a public client (RFC 7591 dynamic client
//!    registration) for this sign-in's loopback redirect.
//! 3. The authorization page is opened with PKCE S256 and the server as the
//!    `resource` (RFC 8707); its redirect comes back to a one-off listener on
//!    the daemon's loopback address — or the person pastes the address the
//!    page ended on.
//! 4. The code is traded for an access token and a refresh token.
//!
//! The tokens and the client registration are one JSON value in the squad's
//! secret store (the Keychain on a Mac) under [`secret_name`]; they never go
//! to a journal, an event, a log, a screen or a member. Each value is put on
//! the masking list as it is kept. Before a token runs out — or when the
//! server answers 401 — it is refreshed with the refresh token, one refresh
//! at a time per server.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::verify::pct;

/// How long a sign-in page waits for the person.
pub const SIGN_IN_WAIT: Duration = Duration::from_secs(15 * 60);
/// A token this close to running out is refreshed before it is used.
const EARLY: u64 = 60;
/// What equipe calls itself when it registers.
const CLIENT_NAME: &str = "equipe";

/// The name a server's sign-in is kept under in the squad's secret store.
pub fn secret_name(server: &str) -> String {
    format!("mcp-oauth-{server}")
}

/// What is kept for a signed-in server: the client equipe registered as, the
/// token endpoint, and the tokens.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub token_endpoint: String,
    /// The server the tokens are for (RFC 8707 `resource`).
    #[serde(default)]
    pub resource: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When the access token runs out, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens").field("client_id", &self.client_id).field("token_endpoint", &self.token_endpoint).field("access_token", &"•••").field("refresh_token", &self.refresh_token.as_ref().map(|_| "•••")).field("expires_at", &self.expires_at).finish()
    }
}

impl Tokens {
    /// Whether the access token should be refreshed before it is used, at `now`.
    pub fn stale(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|at| at <= now + EARLY)
    }
}

/// Where a server's sign-in happens, as discovered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Discovery {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    /// The scopes to ask for (the resource's own, else none named).
    pub scopes: Vec<String>,
    /// The server, as the tokens are asked for (`resource`).
    pub resource: String,
}

/// What a look at a server without a token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Challenge {
    /// It answered without one: no sign-in needed.
    Open,
    /// It wants a token; the metadata address its 401 named, if it did.
    Bearer(Option<String>),
}

fn http_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "it didn't answer in time".to_owned()
    } else if e.is_connect() {
        "it couldn't be reached".to_owned()
    } else {
        "the connection failed".to_owned()
    }
}

/// The `resource_metadata` a `WWW-Authenticate: Bearer …` header names.
pub fn resource_metadata(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &header[at..];
    let value = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split([',', ' ']).next()?,
    };
    (!value.is_empty()).then(|| value.to_owned())
}

/// An `initialize` request, to see how a server answers.
pub fn initialize_message() -> Value {
    json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": crate::mcp::PROTOCOL_VERSION, "capabilities": {}, "clientInfo": { "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION") } } })
}

/// Ask the server, with no token, whether it wants one: an `initialize`
/// posted, or — `sse`, the older transport — its event stream opened.
pub async fn challenge(http: &reqwest::Client, url: &str, sse: bool) -> Result<Challenge, String> {
    let request = if sse {
        http.get(url).header("accept", "text/event-stream")
    } else {
        http.post(url).header("content-type", "application/json").header("accept", "application/json, text/event-stream").body(initialize_message().to_string())
    };
    let response = request.send().await.map_err(|e| http_error(&e))?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        let header = response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
        return Ok(Challenge::Bearer(resource_metadata(&header)));
    }
    if response.status().is_success() {
        return Ok(Challenge::Open);
    }
    Err(format!("it answered {status}"))
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    let response = http.get(url).header("accept", "application/json").send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    serde_json::from_slice(&response.bytes().await.ok()?).ok()
}

/// The well-known addresses to try for `url`'s `suffix` metadata: with the
/// path inserted after the well-known part (RFC 8414 §3, RFC 9728 §3), then at the root.
fn well_known(url: &str, suffix: &str) -> Vec<String> {
    let Ok(parsed) = reqwest::Url::parse(url) else { return Vec::new() };
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{suffix}{path}"));
    }
    out.push(format!("{origin}/.well-known/{suffix}"));
    out
}

/// Find where `url`'s sign-in happens: its protected-resource metadata
/// (from its 401, else the well-known place), then its authorization server's.
pub async fn discover(http: &reqwest::Client, url: &str, metadata: Option<&str>) -> Result<Discovery, String> {
    let mut resource = None;
    for at in metadata.map(str::to_owned).into_iter().chain(well_known(url, "oauth-protected-resource")) {
        if let Some(found) = get_json(http, &at).await {
            resource = Some(found);
            break;
        }
    }
    let issuer = resource.as_ref().and_then(|r| r["authorization_servers"][0].as_str()).map(str::to_owned).unwrap_or_else(|| reqwest::Url::parse(url).map(|u| u.origin().ascii_serialization()).unwrap_or_default());
    let mut server = None;
    let mut tries = well_known(&issuer, "oauth-authorization-server");
    tries.extend(well_known(&issuer, "openid-configuration"));
    tries.push(format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/')));
    for at in tries {
        if let Some(found) = get_json(http, &at).await.filter(|m| m["authorization_endpoint"].is_string() && m["token_endpoint"].is_string()) {
            server = Some(found);
            break;
        }
    }
    let server = server.ok_or("its sign-in server's settings couldn't be found")?;
    let scopes = resource.as_ref().and_then(|r| r["scopes_supported"].as_array()).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    Ok(Discovery {
        authorization_endpoint: server["authorization_endpoint"].as_str().unwrap_or_default().to_owned(),
        token_endpoint: server["token_endpoint"].as_str().unwrap_or_default().to_owned(),
        registration_endpoint: server["registration_endpoint"].as_str().map(str::to_owned),
        scopes,
        resource: resource.as_ref().and_then(|r| r["resource"].as_str()).unwrap_or(url).to_owned(),
    })
}

/// The client equipe registered as for one sign-in.
#[derive(Clone, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub secret: Option<String>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("id", &self.id).field("secret", &self.secret.as_ref().map(|_| "•••")).finish()
    }
}

async fn answer_json(response: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
    let response = response.map_err(|e| http_error(&e))?;
    let status = response.status();
    let body: Value = serde_json::from_slice(&response.bytes().await.map_err(|_| "its answer was cut off".to_owned())?).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"].as_str()).unwrap_or("refused");
        return Err(format!("it answered {}: {}", status.as_u16(), why.chars().take(120).collect::<String>()));
    }
    Ok(body)
}

/// Register equipe for a sign-in that comes back to `redirect` (RFC 7591).
pub async fn register(http: &reqwest::Client, found: &Discovery, redirect: &str) -> Result<Client, String> {
    let at = found.registration_endpoint.as_deref().ok_or("it doesn't let apps register themselves (no dynamic client registration)")?;
    let body = json!({
        "client_name": CLIENT_NAME,
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let answer = answer_json(http.post(at).header("content-type", "application/json").header("accept", "application/json").body(body.to_string()).send().await).await?;
    let id = answer["client_id"].as_str().filter(|s| !s.is_empty()).ok_or("it registered no client")?;
    Ok(Client { id: id.to_owned(), secret: answer["client_secret"].as_str().filter(|s| !s.is_empty()).map(str::to_owned) })
}

/// Base64url without padding.
fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> Result<String, String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| "no randomness on this machine".to_owned())?;
    Ok(b64url(&bytes))
}

/// One sign-in's secrets: the PKCE verifier and the state its redirect must carry back.
pub struct Pkce {
    pub verifier: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Result<Self, String> {
        Ok(Pkce { verifier: random(48)?, state: random(24)? })
    }

    pub fn challenge(&self) -> String {
        b64url(ring::digest::digest(&ring::digest::SHA256, self.verifier.as_bytes()).as_ref())
    }
}

/// The authorization page for this sign-in.
pub fn authorize_url(found: &Discovery, client: &Client, redirect: &str, pkce: &Pkce) -> String {
    let mut url = format!(
        "{}{}response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&resource={}",
        found.authorization_endpoint,
        if found.authorization_endpoint.contains('?') { "&" } else { "?" },
        pct(&client.id),
        pct(redirect),
        pct(&pkce.challenge()),
        pct(&pkce.state),
        pct(&found.resource)
    );
    if !found.scopes.is_empty() {
        url.push_str(&format!("&scope={}", pct(&found.scopes.join(" "))));
    }
    url
}

/// `%41` → `A`, `+` → space.
fn unpct(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() => {
                let hex = |b: u8| (b as char).to_digit(16).unwrap_or_default() as u8;
                out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
                i += 3;
                continue;
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The code out of where the page came back to — a request line
/// (`GET /callback?code=…&state=… HTTP/1.1`) or the address pasted — only
/// with this sign-in's state.
pub fn code_from(answer: &str, state: &str) -> Result<String, String> {
    let answer = answer.trim();
    let target = if answer.starts_with("GET ") { answer.split_whitespace().nth(1).unwrap_or_default() } else { answer };
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default().split('#').next().unwrap_or_default();
    let get = |key: &str| query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| unpct(v));
    if let Some(error) = get("error") {
        return Err(if error == "access_denied" { "the sign-in wasn't allowed".to_owned() } else { format!("the sign-in page said: {}", error.chars().take(80).collect::<String>()) });
    }
    if get("state").as_deref() != Some(state) {
        return Err("that address isn't this sign-in's (paste the whole address the page ended on)".to_owned());
    }
    get("code").filter(|c| !c.is_empty()).ok_or_else(|| "the page sent no code back".to_owned())
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", pct(v))).collect::<Vec<_>>().join("&")
}

fn tokens_from(answer: &Value, client: &Client, token_endpoint: &str, resource: &str, now: u64, keep_refresh: Option<&str>) -> Result<Tokens, String> {
    let access = answer["access_token"].as_str().filter(|s| !s.is_empty()).ok_or("it gave no access token")?;
    Ok(Tokens {
        client_id: client.id.clone(),
        client_secret: client.secret.clone(),
        token_endpoint: token_endpoint.to_owned(),
        resource: resource.to_owned(),
        access_token: access.to_owned(),
        refresh_token: answer["refresh_token"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).or_else(|| keep_refresh.map(str::to_owned)),
        expires_at: answer["expires_in"].as_u64().or_else(|| answer["expires_in"].as_str().and_then(|s| s.parse().ok())).map(|s| now + s),
    })
}

fn token_request(http: &reqwest::Client, endpoint: &str, client: &Client, mut pairs: Vec<(&str, String)>) -> reqwest::RequestBuilder {
    pairs.push(("client_id", client.id.clone()));
    if let Some(secret) = &client.secret {
        pairs.push(("client_secret", secret.clone()));
    }
    let body = form(&pairs.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
    http.post(endpoint).header("content-type", "application/x-www-form-urlencoded").header("accept", "application/json").body(body)
}

/// Trade the redirect's code for the tokens.
pub async fn exchange(http: &reqwest::Client, found: &Discovery, client: &Client, code: &str, pkce: &Pkce, redirect: &str, now: u64) -> Result<Tokens, String> {
    let pairs = vec![("grant_type", "authorization_code".to_owned()), ("code", code.to_owned()), ("redirect_uri", redirect.to_owned()), ("code_verifier", pkce.verifier.clone()), ("resource", found.resource.clone())];
    let answer = answer_json(token_request(http, &found.token_endpoint, client, pairs).send().await).await?;
    tokens_from(&answer, client, &found.token_endpoint, &found.resource, now, None)
}

/// A fresh access token for `tokens`, with its refresh token.
pub async fn refresh(http: &reqwest::Client, tokens: &Tokens, now: u64) -> Result<Tokens, String> {
    let refresh = tokens.refresh_token.as_deref().ok_or("the sign-in ran out and gave no way to renew it")?;
    let client = Client { id: tokens.client_id.clone(), secret: tokens.client_secret.clone() };
    let mut pairs = vec![("grant_type", "refresh_token".to_owned()), ("refresh_token", refresh.to_owned())];
    if !tokens.resource.is_empty() {
        pairs.push(("resource", tokens.resource.clone()));
    }
    let answer = answer_json(token_request(http, &tokens.token_endpoint, &client, pairs).send().await).await?;
    tokens_from(&answer, &client, &tokens.token_endpoint, &tokens.resource, now, Some(refresh))
}

// ── keeping them ──

/// Where a server's tokens are read and kept: the squad's secret store.
pub trait Store: Send + Sync {
    fn load(&self, squad: &str, name: &str) -> Option<String>;
    fn keep(&self, squad: &str, name: &str, value: &str) -> Result<(), String>;
    fn forget(&self, squad: &str, name: &str);
}

/// The daemon's secret store ([`crate::vault::Vault`]).
pub struct VaultStore(pub Arc<Mutex<crate::vault::Vault>>);

impl Store for VaultStore {
    fn load(&self, squad: &str, name: &str) -> Option<String> {
        self.0.lock().ok()?.squad_secret(squad, name, false).ok()
    }
    fn keep(&self, squad: &str, name: &str, value: &str) -> Result<(), String> {
        self.0.lock().map_err(|_| "the secret store is unreadable".to_owned())?.set_squad_secret(squad, name, value)
    }
    fn forget(&self, squad: &str, name: &str) {
        if let Ok(mut vault) = self.0.lock() {
            let _ = vault.delete_squad_secret(squad, name);
        }
    }
}

/// What was read last for each squad's server, so a call doesn't go to the Keychain each time.
static KNOWN: LazyLock<Mutex<HashMap<(String, String), Tokens>>> = LazyLock::new(Default::default);
/// One refresh at a time per squad's server.
static RENEWING: LazyLock<Mutex<HashMap<(String, String), Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

fn watch(squad: &str, tokens: &Tokens) {
    let name = secret_name("");
    crate::squad_secrets::watch(&crate::squad_secrets::watch_key(squad, &format!("{name}access:{}", tokens.client_id)), &tokens.access_token);
    if let Some(refresh) = &tokens.refresh_token {
        crate::squad_secrets::watch(&crate::squad_secrets::watch_key(squad, &format!("{name}refresh:{}", tokens.client_id)), refresh);
    }
}

/// The tokens kept for `server`, if it was signed in.
pub fn load(store: &dyn Store, squad: &str, server: &str) -> Option<Tokens> {
    let key = (squad.to_owned(), server.to_owned());
    if let Some(known) = KNOWN.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return Some(known);
    }
    let tokens: Tokens = serde_json::from_str(&store.load(squad, &secret_name(server))?).ok()?;
    watch(squad, &tokens);
    if let Ok(mut known) = KNOWN.lock() {
        known.insert(key, tokens.clone());
    }
    Some(tokens)
}

/// Keep `tokens` for `server`.
pub fn keep(store: &dyn Store, squad: &str, server: &str, tokens: &Tokens) -> Result<(), String> {
    let text = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    store.keep(squad, &secret_name(server), &text)?;
    watch(squad, tokens);
    if let Ok(mut known) = KNOWN.lock() {
        known.insert((squad.to_owned(), server.to_owned()), tokens.clone());
    }
    Ok(())
}

/// Sign `server` out: its tokens go from the store.
pub fn forget(store: &dyn Store, squad: &str, server: &str) {
    store.forget(squad, &secret_name(server));
    if let Ok(mut known) = KNOWN.lock() {
        known.remove(&(squad.to_owned(), server.to_owned()));
    }
}

/// Why a token isn't at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoToken {
    /// Never signed in.
    Never,
    /// The sign-in can't be renewed: the person signs in again.
    SignIn(String),
    /// The renewal didn't go through for now (the network): tried again later.
    Failing(String),
}

/// The access token to use for `server` now: the kept one, refreshed first
/// when it runs out soon — or, `rejected` (the server just refused it), whatever its time.
pub async fn access(http: &reqwest::Client, store: Arc<dyn Store>, squad: &str, server: &str, rejected: Option<&str>, now: u64) -> Result<String, NoToken> {
    let lock = {
        let mut all = RENEWING.lock().map_err(|_| NoToken::Failing("busy".into()))?;
        all.entry((squad.to_owned(), server.to_owned())).or_default().clone()
    };
    let _held = lock.lock().await;
    let loaded = {
        let (store, squad, server) = (store.clone(), squad.to_owned(), server.to_owned());
        tokio::task::spawn_blocking(move || load(store.as_ref(), &squad, &server)).await.ok().flatten()
    };
    let tokens = loaded.ok_or(NoToken::Never)?;
    // Another call renewed it while this one waited: the new one is used.
    let refused = rejected.is_some_and(|r| r == tokens.access_token);
    if !refused && !tokens.stale(now) {
        return Ok(tokens.access_token);
    }
    match refresh(http, &tokens, now).await {
        Ok(fresh) => {
            let (store, squad_s, server_s, kept) = (store.clone(), squad.to_owned(), server.to_owned(), fresh.clone());
            let _ = tokio::task::spawn_blocking(move || keep(store.as_ref(), &squad_s, &server_s, &kept)).await;
            Ok(fresh.access_token)
        }
        Err(why) if why.starts_with("it answered 4") || tokens.refresh_token.is_none() => Err(NoToken::SignIn(why)),
        Err(why) => Err(NoToken::Failing(why)),
    }
}

// ── the sign-in itself ──

/// How a sign-in ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Done,
    Expired,
    Failed(String),
}

/// The one-off page the browser lands on.
fn landing(ok: bool, why: &str) -> String {
    let (title, body) = if ok { ("Signed in", "equipe is connected. You can close this tab.") } else { ("Not signed in", why) };
    let escape = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let html = format!("<!doctype html><meta charset=utf-8><title>equipe</title><body style=\"font:16px -apple-system,system-ui,sans-serif;max-width:32em;margin:15vh auto;padding:0 1em\"><h1 style=\"font-size:20px\">{}</h1><p>{}</p></body>", escape(title), escape(body));
    format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n\r\n{html}", html.len())
}

/// Wait on `listener` for the redirect: the request line of the first
/// request that carries a `state`, and the stream to answer on.
async fn redirect(listener: &tokio::net::TcpListener) -> Result<(String, tokio::net::TcpStream), String> {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    loop {
        let (stream, peer) = listener.accept().await.map_err(|e| e.to_string())?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut reader = tokio::io::BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            continue;
        }
        let mut stream = reader.into_inner();
        if !line.contains("state=") && !line.contains("error=") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
            continue;
        }
        return Ok((line.trim_end().to_owned(), stream));
    }
}

/// Sign in to the server at `url` as equipe and keep the tokens for
/// `server`: `on_page` hears the page to open; `pasted` is what the person
/// pastes (the address the page ended on), for a page opened elsewhere.
#[allow(clippy::too_many_arguments)]
pub async fn sign_in(
    http: &reqwest::Client,
    store: Arc<dyn Store>,
    squad: &str,
    server: &str,
    url: &str,
    sse: bool,
    wait: Duration,
    mut pasted: tokio::sync::mpsc::UnboundedReceiver<crate::protocol::cli_signin::PastedCode>,
    mut on_page: impl FnMut(&str),
) -> Ended {
    use tokio::io::AsyncWriteExt as _;
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // The loopback listener first: its port is in the redirect equipe registers.
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => return Ended::Failed(format!("no port for its redirect: {e}")),
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else { return Ended::Failed("no port for its redirect".into()) };
    // The browser may try `localhost` on IPv6 first: the same port there too, when it is free.
    let six = tokio::net::TcpListener::bind(("::1", port)).await.ok();
    let redirect_uri = format!("http://localhost:{port}/callback");
    let started: Result<(Discovery, Client, Pkce), String> = async {
        let metadata = match challenge(http, url, sse).await? {
            Challenge::Open => return Err("open".to_owned()),
            Challenge::Bearer(metadata) => metadata,
        };
        let found = discover(http, url, metadata.as_deref()).await?;
        let client = register(http, &found, &redirect_uri).await?;
        Ok((found, client, Pkce::new()?))
    }
    .await;
    let (found, client, pkce) = match started {
        Ok(s) => s,
        // It needs no sign-in at all: nothing to keep.
        Err(why) if why == "open" => return Ended::Done,
        Err(why) => return Ended::Failed(why),
    };
    on_page(&authorize_url(&found, &client, &redirect_uri, &pkce));
    let deadline = tokio::time::Instant::now() + wait;
    let accept_six = async {
        match &six {
            Some(l) => redirect(l).await,
            None => std::future::pending().await,
        }
    };
    let (code, stream) = tokio::select! {
        got = redirect(&listener) => match got {
            Ok((line, stream)) => (code_from(&line, &pkce.state), Some(stream)),
            Err(why) => (Err(why), None),
        },
        got = accept_six => match got {
            Ok((line, stream)) => (code_from(&line, &pkce.state), Some(stream)),
            Err(why) => (Err(why), None),
        },
        code = async {
            loop {
                match pasted.recv().await {
                    Some(text) => match code_from(&text.0, &pkce.state) {
                        Ok(code) => return Ok(code),
                        // A paste that isn't this sign-in's: waited on, the card says nothing new.
                        Err(_) => continue,
                    },
                    None => std::future::pending::<()>().await,
                }
            }
        } => (code, None),
        _ = tokio::time::sleep_until(deadline) => return Ended::Expired,
    };
    let done = match code {
        Ok(code) => exchange(http, &found, &client, &code, &pkce, &redirect_uri, now()).await,
        Err(why) => Err(why),
    };
    let done = match done {
        Ok(tokens) => {
            let (store, squad, server) = (store.clone(), squad.to_owned(), server.to_owned());
            tokio::task::spawn_blocking(move || keep(store.as_ref(), &squad, &server, &tokens)).await.unwrap_or_else(|_| Err("the secret store failed".into()))
        }
        Err(why) => Err(why),
    };
    if let Some(mut stream) = stream {
        let _ = stream.write_all(landing(done.is_ok(), done.as_ref().err().map_or("", String::as_str)).as_bytes()).await;
        let _ = stream.shutdown().await;
    }
    match done {
        Ok(()) => Ended::Done,
        Err(why) => Ended::Failed(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_401s_challenge_names_its_metadata() {
        assert_eq!(resource_metadata(r#"Bearer error="invalid_token", resource_metadata="https://api.mobbin.com/.well-known/oauth-protected-resource/mcp""#).as_deref(), Some("https://api.mobbin.com/.well-known/oauth-protected-resource/mcp"));
        assert_eq!(resource_metadata("Bearer resource_metadata=https://x.example.com/m, scope=a").as_deref(), Some("https://x.example.com/m"));
        assert_eq!(resource_metadata("Bearer realm=x"), None);
    }

    #[test]
    fn well_known_places_put_the_path_after_the_well_known_part() {
        assert_eq!(well_known("https://a.example.com/auth/v1", "oauth-authorization-server"), ["https://a.example.com/.well-known/oauth-authorization-server/auth/v1", "https://a.example.com/.well-known/oauth-authorization-server"]);
        assert_eq!(well_known("https://a.example.com", "oauth-protected-resource"), ["https://a.example.com/.well-known/oauth-protected-resource"]);
    }

    #[test]
    fn the_page_asks_with_pkce_s256_the_state_and_the_resource() {
        let found = Discovery { authorization_endpoint: "https://auth.example.com/authorize".into(), token_endpoint: "https://auth.example.com/token".into(), registration_endpoint: None, scopes: vec!["mcp".into(), "offline_access".into()], resource: "https://api.example.com/mcp".into() };
        let pkce = Pkce { verifier: "v".repeat(64), state: "st8".into() };
        let url = authorize_url(&found, &Client { id: "c1".into(), secret: None }, "http://localhost:5000/callback", &pkce);
        assert!(url.starts_with("https://auth.example.com/authorize?response_type=code&client_id=c1&redirect_uri=http%3A%2F%2Flocalhost%3A5000%2Fcallback"), "{url}");
        assert!(url.contains("code_challenge_method=S256") && url.contains(&format!("code_challenge={}", pkce.challenge())) && url.contains("state=st8"));
        assert!(url.contains("resource=https%3A%2F%2Fapi.example.com%2Fmcp") && url.contains("scope=mcp%20offline_access"));
        assert_ne!(pkce.challenge(), pkce.verifier, "the challenge is the verifier's hash");
        assert_eq!(crate::cli_signin::loopback_port(&url), Some(5000), "a screen can tunnel its redirect");
    }

    #[test]
    fn the_code_comes_back_only_with_this_sign_ins_state() {
        assert_eq!(code_from("GET /callback?code=abc&state=st8 HTTP/1.1", "st8").as_deref(), Ok("abc"));
        assert_eq!(code_from(" http://localhost:5000/callback?state=st8&code=a%2Fb ", "st8").as_deref(), Ok("a/b"));
        assert!(code_from("http://localhost:5000/callback?code=abc&state=other", "st8").is_err());
        assert!(code_from("http://localhost:5000/callback?error=access_denied&state=st8", "st8").unwrap_err().contains("wasn't allowed"));
    }

    #[test]
    fn tokens_never_show_in_a_log_line_and_run_out_a_minute_early() {
        let t = Tokens { client_id: "c".into(), client_secret: Some("cs".into()), token_endpoint: "https://a/t".into(), resource: String::new(), access_token: "at-secret".into(), refresh_token: Some("rt-secret".into()), expires_at: Some(1000) };
        let shown = format!("{t:?}");
        assert!(!shown.contains("at-secret") && !shown.contains("rt-secret"), "{shown}");
        assert!(!t.stale(900) && t.stale(941) && t.stale(2000));
        let answer = json!({ "access_token": "new", "expires_in": 3600 });
        let fresh = tokens_from(&answer, &Client { id: "c".into(), secret: None }, "https://a/t", "r", 10, Some("rt-secret")).unwrap();
        assert_eq!((fresh.refresh_token.as_deref(), fresh.expires_at), (Some("rt-secret"), Some(3610)), "a refresh that gives no new refresh token keeps the old one");
    }
}
