//! `equipe judge-relay`: a small server that lets people without a Nebius
//! key — the hackathon's judges — try `equipe review`. It holds the one
//! `NEBIUS_API_KEY`, in its own environment only; a judge gets an app token
//! of their own (`equipe judge-relay token <name>`) and points equipe at it:
//!
//! ```text
//! NEBIUS_API_KEY=<their app token> EQUIPE_NEBIUS_BASE_URL=https://<relay>/v1/ equipe review
//! ```
//!
//! It forwards only `GET /v1/models` (cut to NVIDIA Nemotron models) and
//! non-streamed `POST /v1/chat/completions` to an NVIDIA Nemotron model, with
//! its answers capped (`max_tokens`), its body bounded, and requests and tokens
//! counted per app token and per day (UTC; counts start over when it restarts).
//! Access ends at `EQUIPE_RELAY_UNTIL`. Its log has the token's name, the
//! model, the status, the tokens and the time taken — never a key, a token or
//! what was asked.
//!
//! It speaks plain HTTP/1.1, one request per connection: run it behind a
//! proxy that terminates TLS. Settings, from its environment:
//! - `NEBIUS_API_KEY`: the Token Factory key it calls with;
//! - `EQUIPE_RELAY_TOKENS`: `name:sha256hex,…` — the app tokens it lets in, hashed;
//! - `EQUIPE_RELAY_UNTIL`: when access ends, Unix seconds (none: no end);
//! - `EQUIPE_RELAY_REQUESTS_PER_DAY` (40), `EQUIPE_RELAY_TOKENS_PER_DAY`
//!   (150000) per app token; `EQUIPE_RELAY_GLOBAL_REQUESTS_PER_DAY` (400) for all;
//! - `EQUIPE_RELAY_MAX_TOKENS` (2048): the longest answer asked for;
//! - `EQUIPE_NEBIUS_BASE_URL`: where it forwards (Token Factory's by default).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use equipe_core::nemotron;
use equipe_core::t;
use serde_json::{Value, json};
use sha2::Digest as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// The largest request body let through: a review's 48,000-character diff, its
/// ticket and the instructions fit several times over.
const MAX_BODY: usize = 256 * 1024;
const MAX_HEAD: usize = 16 * 1024;
/// How long a client has to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// The relay's settings. Its `Debug` never shows the key.
#[derive(Clone)]
pub struct Settings {
    key: String,
    pub upstream: String,
    /// The app tokens let in: a name, and the token's SHA-256.
    pub tokens: Vec<(String, [u8; 32])>,
    pub until: Option<u64>,
    pub requests_per_day: u32,
    pub tokens_per_day: u64,
    pub global_requests_per_day: u32,
    pub max_tokens: u32,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.tokens.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("Settings").field("key", &"[redacted]").field("upstream", &self.upstream).field("tokens", &names).field("until", &self.until).finish_non_exhaustive()
    }
}

impl Settings {
    /// From variables as `var` reads them (the environment, or a test's).
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let set = |name: &str| var(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let number = |name: &str, default: u64| -> Result<u64, String> {
            match set(name) {
                None => Ok(default),
                Some(v) => v.parse().map_err(|_| t!("relay.bad_setting", name = name, value = v)),
            }
        };
        let key = set("NEBIUS_API_KEY").ok_or_else(|| t!("relay.no_key").to_owned())?;
        let mut tokens = Vec::new();
        for entry in set("EQUIPE_RELAY_TOKENS").unwrap_or_default().split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let parsed = entry.split_once(':').and_then(|(name, hex)| Some((name.trim().to_owned(), unhex(hex.trim())?))).filter(|(name, _)| !name.is_empty());
            tokens.push(parsed.ok_or_else(|| t!("relay.bad_setting", name = "EQUIPE_RELAY_TOKENS", value = entry.split(':').next().unwrap_or_default()))?);
        }
        if tokens.is_empty() {
            return Err(t!("relay.no_tokens").into());
        }
        Ok(Self {
            key,
            upstream: set("EQUIPE_NEBIUS_BASE_URL").unwrap_or_else(|| nemotron::DEFAULT_BASE_URL.to_owned()),
            tokens,
            until: set("EQUIPE_RELAY_UNTIL").map(|_| number("EQUIPE_RELAY_UNTIL", 0)).transpose()?,
            requests_per_day: number("EQUIPE_RELAY_REQUESTS_PER_DAY", 40)? as u32,
            tokens_per_day: number("EQUIPE_RELAY_TOKENS_PER_DAY", 150_000)?,
            global_requests_per_day: number("EQUIPE_RELAY_GLOBAL_REQUESTS_PER_DAY", 400)? as u32,
            max_tokens: number("EQUIPE_RELAY_MAX_TOKENS", 2048)?.clamp(64, 32_768) as u32,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.upstream.trim_end_matches('/'))
    }

    /// The name of the app token `token` is, if it is one.
    fn who(&self, token: &str) -> Option<&str> {
        let hash: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
        // Every entry looked at, equal or not: how long it takes says nothing.
        let mut found = None;
        for (name, known) in &self.tokens {
            if known.iter().zip(hash.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0 {
                found = Some(name.as_str());
            }
        }
        found
    }
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// A new app token for `name`: the token, to give them, and the line for
/// `EQUIPE_RELAY_TOKENS`, which keeps only its hash.
pub fn mint(name: &str) -> std::io::Result<(String, String)> {
    let mut bytes = [0u8; 24];
    std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom")?, &mut bytes)?;
    let token = format!("eqj_{}", crate::remote::b64(&bytes));
    let hash = sha2::Sha256::digest(token.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    Ok((token, format!("{name}:{hex}")))
}

/// What each app token, and everyone, used today.
#[derive(Default)]
struct Counts {
    day: u64,
    global: u32,
    by_token: HashMap<String, (u32, u64)>,
}

struct Relay {
    settings: Settings,
    client: reqwest::Client,
    counts: Mutex<Counts>,
    now: Box<dyn Fn() -> u64 + Send + Sync>,
}

/// Serve on `listener` until it fails.
pub async fn serve(settings: Settings, listener: TcpListener) {
    serve_with_clock(settings, listener, Box::new(unix_now)).await
}

async fn serve_with_clock(settings: Settings, listener: TcpListener, now: Box<dyn Fn() -> u64 + Send + Sync>) {
    let client = reqwest::Client::builder().timeout(UPSTREAM_TIMEOUT).connect_timeout(Duration::from_secs(10)).build().unwrap_or_default();
    let relay = Arc::new(Relay { settings, client, counts: Mutex::new(Counts::default()), now });
    while let Ok((socket, _)) = listener.accept().await {
        let relay = relay.clone();
        tokio::spawn(async move { relay.connection(socket).await });
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A request as it came: method, path, the headers asked about, the body.
struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

/// An answer: status, body, and the upstream request's id when there is one.
struct Answer {
    status: u16,
    body: Vec<u8>,
    request_id: Option<String>,
    retry_after: Option<u64>,
}

fn refusal(status: u16, message: &str) -> Answer {
    Answer { status, body: json!({ "error": { "message": message, "type": "relay" } }).to_string().into_bytes(), request_id: None, retry_after: None }
}

impl Relay {
    async fn connection(&self, mut socket: TcpStream) {
        let started = Instant::now();
        let (answer, who, model, tokens) = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut socket)).await {
            Ok(Ok(request)) => self.answer(request).await,
            Ok(Err(answer)) => (answer, None, None, None),
            Err(_) => (refusal(408, "request took too long"), None, None, None),
        };
        let line = log_line(who.as_deref(), model.as_deref(), &answer, tokens, started.elapsed());
        equipe_core::daemon_log!("{line}");
        let _ = write_answer(&mut socket, &answer).await;
    }

    /// The answer to `request`, who asked (the token's name), the model and the tokens it took.
    async fn answer(&self, request: Request) -> (Answer, Option<String>, Option<String>, Option<u64>) {
        let token = request.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer ").or_else(|| a.strip_prefix("bearer "))).map(str::trim);
        let Some(who) = token.and_then(|t| self.settings.who(t)).map(str::to_owned) else {
            return (refusal(401, "unknown app token"), None, None, None);
        };
        let now = (self.now)();
        if self.settings.until.is_some_and(|until| now >= until) {
            return (refusal(403, "access to this relay has ended"), Some(who), None, None);
        }
        match (request.method.as_str(), request.path.split('?').next().unwrap_or_default()) {
            ("GET", "/v1/models") => (self.models().await, Some(who), None, None),
            ("POST", "/v1/chat/completions") => {
                let (answer, model, tokens) = self.complete(&who, now, &request.body).await;
                (answer, Some(who), model, tokens)
            }
            _ => (refusal(404, "only GET /v1/models and POST /v1/chat/completions"), Some(who), None, None),
        }
    }

    /// The upstream's models, NVIDIA Nemotron ones only.
    async fn models(&self) -> Answer {
        let sent = self.client.get(self.settings.url("models")).bearer_auth(&self.settings.key).send().await;
        let Ok(response) = sent else { return refusal(502, "Token Factory couldn't be reached") };
        let status = response.status().as_u16();
        let request_id = response.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(|v| nemotron::clean_line(v, 120));
        let body: Value = response.bytes().await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            return Answer { request_id, ..refusal(502, &format!("Token Factory answered {status}")) };
        }
        let ids: Vec<String> = body.get("data").and_then(Value::as_array).into_iter().flatten().filter_map(|m| m.get("id").and_then(Value::as_str)).map(str::to_owned).collect();
        let data: Vec<Value> = nemotron::nemotron_models(&ids).into_iter().map(|id| json!({ "id": id, "object": "model" })).collect();
        Answer { status: 200, body: json!({ "object": "list", "data": data }).to_string().into_bytes(), request_id, retry_after: None }
    }

    /// A chat completion with an NVIDIA Nemotron model, within today's allowance.
    async fn complete(&self, who: &str, now: u64, body: &[u8]) -> (Answer, Option<String>, Option<u64>) {
        let Ok(Value::Object(mut ask)) = serde_json::from_slice::<Value>(body) else {
            return (refusal(400, "the body must be a JSON object"), None, None);
        };
        let model = ask.get("model").and_then(Value::as_str).map(str::to_owned).unwrap_or_default();
        if nemotron::nemotron_models(std::slice::from_ref(&model)).is_empty() {
            return (refusal(400, "only NVIDIA Nemotron chat models are relayed"), None, None);
        }
        let model = Some(nemotron::clean_line(&model, 120));
        if ask.get("stream").and_then(Value::as_bool) == Some(true) {
            return (refusal(400, "streaming isn't relayed"), model, None);
        }
        let asked = ask.get("max_tokens").and_then(Value::as_u64).unwrap_or(u64::from(self.settings.max_tokens));
        ask.insert("max_tokens".into(), json!(asked.min(u64::from(self.settings.max_tokens))));
        ask.remove("max_completion_tokens");
        // Today's allowance, taken before asking: a burst can't overshoot it.
        if let Some(wait) = self.take(who, now) {
            return (Answer { retry_after: Some(wait), ..refusal(429, "today's allowance for this app token is used up") }, model, None);
        }
        let sent = self.client.post(self.settings.url("chat/completions")).bearer_auth(&self.settings.key).header("content-type", "application/json").body(Value::Object(ask).to_string()).send().await;
        let response = match sent {
            Ok(response) => response,
            Err(err) if err.is_timeout() => return (refusal(504, "Token Factory didn't answer in time"), model, None),
            Err(_) => return (refusal(502, "Token Factory couldn't be reached"), model, None),
        };
        let status = response.status().as_u16();
        let request_id = response.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(|v| nemotron::clean_line(v, 120));
        let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let body = response.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        let used = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.pointer("/usage/total_tokens").and_then(Value::as_u64));
        if let Some(used) = used {
            self.spend(who, used);
        }
        // A refused upstream key is the relay's to fix, not the judge's: never a 401 back.
        let status = if matches!(status, 401 | 403) { 502 } else { status };
        let body = if (200..300).contains(&status) || status == 429 || status == 400 { body } else { refusal(status, &format!("Token Factory answered {status}")).body };
        (Answer { status, body, request_id, retry_after }, model, used)
    }

    /// Count a request for `who`, or say how long until it may ask again.
    fn take(&self, who: &str, now: u64) -> Option<u64> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let day = now / 86_400;
        if counts.day != day {
            *counts = Counts { day, ..Counts::default() };
        }
        let tomorrow = (day + 1) * 86_400 - now;
        let (requests, tokens) = counts.by_token.get(who).copied().unwrap_or_default();
        if requests >= self.settings.requests_per_day || tokens >= self.settings.tokens_per_day || counts.global >= self.settings.global_requests_per_day {
            return Some(tomorrow);
        }
        counts.global += 1;
        counts.by_token.insert(who.to_owned(), (requests + 1, tokens));
        None
    }

    fn spend(&self, who: &str, used: u64) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, tokens)) = counts.by_token.get_mut(who) {
            *tokens += used;
        }
    }
}

/// The log's line for one request: no key, no token, nothing that was asked.
fn log_line(who: Option<&str>, model: Option<&str>, answer: &Answer, tokens: Option<u64>, took: Duration) -> String {
    format!(
        "equipe judge-relay: {} {} model {} tokens {} request {} {} ms",
        who.unwrap_or("-"),
        answer.status,
        model.unwrap_or("-"),
        tokens.map_or_else(|| "-".to_owned(), |t| t.to_string()),
        answer.request_id.as_deref().unwrap_or("-"),
        took.as_millis()
    )
}

/// A request off `socket`: its head, then a body of `content-length` — or an
/// answer saying why not.
async fn read_request(socket: &mut TcpStream) -> Result<Request, Answer> {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let end = loop {
        let n = socket.read(&mut buf).await.map_err(|_| refusal(400, "couldn't read the request"))?;
        if n == 0 {
            return Err(refusal(400, "the request ended early"));
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        if data.len() > MAX_HEAD {
            return Err(refusal(431, "the request's head is too large"));
        }
    };
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (method, path) = (first.next().unwrap_or_default().to_owned(), first.next().unwrap_or_default().to_owned());
    let (mut length, mut authorization) = (0usize, None);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().map_err(|_| refusal(400, "a bad content-length"))?,
            "authorization" => authorization = Some(value.trim().to_owned()),
            "transfer-encoding" => return Err(refusal(411, "send a content-length, not chunks")),
            _ => {}
        }
    }
    if length > MAX_BODY {
        return Err(refusal(413, "the request is too large"));
    }
    let mut body = data[end + 4..].to_vec();
    while body.len() < length {
        let n = socket.read(&mut buf).await.map_err(|_| refusal(400, "couldn't read the request"))?;
        if n == 0 {
            return Err(refusal(400, "the request ended early"));
        }
        body.extend_from_slice(&buf[..n]);
    }
    body.truncate(length);
    Ok(Request { method, path, authorization, body })
}

async fn write_answer(socket: &mut TcpStream, answer: &Answer) -> std::io::Result<()> {
    let reason = match answer.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n", answer.status, answer.body.len());
    if let Some(id) = &answer.request_id {
        head.push_str(&format!("x-request-id: {id}\r\n"));
    }
    if let Some(wait) = answer.retry_after {
        head.push_str(&format!("retry-after: {wait}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&answer.body).await?;
    socket.shutdown().await
}

/// `equipe judge-relay`: serve on `listen`, or with `token <name>`, make an app token.
pub async fn command(words: &[String], listen: &str) -> Result<(), String> {
    match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["token", name] => {
            let (token, line) = mint(name).map_err(|e| e.to_string())?;
            println!("{}\n  {token}\n", t!("relay.token_made", name = name));
            println!("{}\n  {line}", t!("relay.token_env"));
            Ok(())
        }
        [] => {
            let settings = Settings::from_vars(|name| std::env::var(name).ok())?;
            let listener = TcpListener::bind(listen).await.map_err(|err| t!("relay.bad_listen", addr = listen, err = err))?;
            let names: Vec<&str> = settings.tokens.iter().map(|(n, _)| n.as_str()).collect();
            eprintln!("{}", t!("relay.listening", addr = listen, upstream = settings.upstream, names = names.join(", ")));
            serve(settings, listener).await;
            Ok(())
        }
        _ => Err(t!("relay.usage").into()),
    }
}

#[cfg(test)]
mod tests {
    //! The relay between equipe's own client and a mock Token Factory on
    //! localhost (canned answers): nothing here reaches Nebius.

    use super::*;
    use equipe_core::nemotron::{Config, ReviewError, ReviewInput};
    use std::sync::atomic::{AtomicU64, Ordering};

    const UPSTREAM_KEY: &str = "sk-upstream-mock-key";

    /// A mock Token Factory: `/v1/models` lists models of several vendors;
    /// a completion answers a review. Every request's text is kept.
    async fn mock_upstream() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let kept = kept.clone();
                tokio::spawn(async move {
                    let Ok(request) = read_request(&mut socket).await else { return };
                    kept.lock().unwrap().push(format!("{} {} {} {}", request.method, request.path, request.authorization.unwrap_or_default(), String::from_utf8_lossy(&request.body)));
                    let body = if request.path.ends_with("/models") {
                        json!({ "data": [{ "id": "meta-llama/Llama-3.3-70B-Instruct" }, { "id": "nvidia/Llama-3_3-Nemotron-Super-49B-v1_5" }, { "id": "nvidia/nv-embed-v2" }] })
                    } else {
                        let review = r#"{"verdict":"risky","summary":"Deletes any folder it is given.","risks":[],"missing_tests":[]}"#;
                        json!({ "model": "nvidia/Llama-3_3-Nemotron-Super-49B-v1_5", "choices": [{ "message": { "content": review } }], "usage": { "prompt_tokens": 700, "completion_tokens": 60, "total_tokens": 760 } })
                    };
                    let answer = Answer { status: 200, body: body.to_string().into_bytes(), request_id: Some("req-upstream".into()), retry_after: None };
                    let _ = write_answer(&mut socket, &answer).await;
                });
            }
        });
        (base, seen)
    }

    fn settings(upstream: &str, extra: &[(&str, &str)]) -> (Settings, String) {
        let (token, line) = mint("judge-1").unwrap();
        let mut vars: HashMap<String, String> = [("NEBIUS_API_KEY", UPSTREAM_KEY), ("EQUIPE_NEBIUS_BASE_URL", upstream)].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        vars.insert("EQUIPE_RELAY_TOKENS".into(), format!("other:{},{line}", "0".repeat(64)));
        for (k, v) in extra {
            vars.insert(k.to_string(), v.to_string());
        }
        (Settings::from_vars(|name| vars.get(name).cloned()).unwrap(), token)
    }

    async fn relay(settings: Settings, now: Arc<AtomicU64>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1/", listener.local_addr().unwrap());
        tokio::spawn(serve_with_clock(settings, listener, Box::new(move || now.load(Ordering::SeqCst))));
        base
    }

    /// equipe's own client, pointed at the relay with an app token.
    fn client(relay: &str, token: &str) -> Config {
        let mut cfg = Config::from_vars(|name| match name {
            "NEBIUS_API_KEY" => Some(token.to_owned()),
            "EQUIPE_NEBIUS_BASE_URL" => Some(relay.to_owned()),
            _ => None,
        })
        .unwrap();
        cfg.backoff = Duration::from_millis(5);
        cfg
    }

    fn input() -> ReviewInput {
        ReviewInput { ticket: 0, title: "Clean the cache".into(), body: String::new(), diff: "+ std::fs::remove_dir_all(path)".into(), language: "English".into() }
    }

    #[tokio::test]
    async fn an_app_token_reviews_through_the_relay_with_the_relays_key() {
        let (upstream, seen) = mock_upstream().await;
        let (settings, token) = settings(&upstream, &[("EQUIPE_RELAY_MAX_TOKENS", "512")]);
        let base = relay(settings, Arc::new(AtomicU64::new(1_800_000_000))).await;
        let cfg = client(&base, &token);
        // The models it offers are NVIDIA Nemotron chat models only.
        assert_eq!(nemotron::models(&cfg).await.unwrap(), vec!["nvidia/Llama-3_3-Nemotron-Super-49B-v1_5".to_owned()]);
        let reviewed = nemotron::review_detailed(&cfg, input()).await.unwrap();
        assert_eq!(reviewed.review.summary, "Deletes any folder it is given.");
        assert_eq!(reviewed.request_id.as_deref(), Some("req-upstream"));
        // Upstream saw the relay's key, never the app token; the answer was capped.
        let seen = seen.lock().unwrap().clone();
        assert!(seen.iter().all(|r| r.contains(&format!("Bearer {UPSTREAM_KEY}")) && !r.contains(&token)), "{seen:#?}");
        assert!(seen.last().unwrap().contains("\"max_tokens\":512"), "{seen:#?}");
    }

    #[tokio::test]
    async fn no_token_a_wrong_one_another_model_or_a_late_ask_is_refused() {
        let (upstream, seen) = mock_upstream().await;
        let (settings, token) = settings(&upstream, &[("EQUIPE_RELAY_UNTIL", "1800000100")]);
        let now = Arc::new(AtomicU64::new(1_800_000_000));
        let base = relay(settings, now.clone()).await;
        assert_eq!(nemotron::review(&client(&base, "eqj_not-a-token"), input()).await.unwrap_err(), ReviewError::Unauthorized);
        // Another vendor's model isn't relayed.
        let mut cfg = client(&base, &token);
        cfg.model = Some("meta-llama/Llama-3.3-70B-Instruct".into());
        assert!(matches!(nemotron::review(&cfg, input()).await.unwrap_err(), ReviewError::Http(400, _)));
        assert!(seen.lock().unwrap().is_empty(), "nothing reached Token Factory");
        // Once access has ended, a known token is refused too.
        now.store(1_800_000_100, Ordering::SeqCst);
        assert_eq!(nemotron::review(&client(&base, &token), input()).await.unwrap_err(), ReviewError::Unauthorized);
    }

    #[tokio::test]
    async fn a_token_s_allowance_runs_out_for_the_day_and_comes_back_the_next() {
        let (upstream, seen) = mock_upstream().await;
        let (settings, token) = settings(&upstream, &[("EQUIPE_RELAY_REQUESTS_PER_DAY", "2")]);
        let now = Arc::new(AtomicU64::new(1_800_000_000));
        let base = relay(settings, now.clone()).await;
        let mut cfg = client(&base, &token);
        cfg.model = Some("nvidia/Llama-3_3-Nemotron-Super-49B-v1_5".into());
        assert!(nemotron::review(&cfg, input()).await.is_ok());
        assert!(nemotron::review(&cfg, input()).await.is_ok());
        assert_eq!(nemotron::review(&cfg, input()).await.unwrap_err(), ReviewError::RateLimited);
        assert_eq!(seen.lock().unwrap().len(), 2, "the third never reached Token Factory");
        now.store(1_800_000_000 + 86_400, Ordering::SeqCst);
        assert!(nemotron::review(&cfg, input()).await.is_ok());
    }

    #[test]
    fn settings_need_a_key_and_tokens_and_never_show_them() {
        assert!(Settings::from_vars(|_| None).is_err());
        assert!(Settings::from_vars(|n| (n == "NEBIUS_API_KEY").then(|| "sk".into())).is_err(), "no tokens");
        let bad = |n: &str| match n {
            "NEBIUS_API_KEY" => Some("sk".into()),
            "EQUIPE_RELAY_TOKENS" => Some("judge:nothex".into()),
            _ => None,
        };
        assert!(Settings::from_vars(bad).is_err());
        let (settings, token) = settings("http://127.0.0.1:9/v1/", &[]);
        assert_eq!(settings.who(&token), Some("judge-1"));
        assert_eq!(settings.who("eqj_other"), None);
        let shown = format!("{settings:?}");
        assert!(!shown.contains(UPSTREAM_KEY) && shown.contains("judge-1"), "{shown}");
        let line = log_line(Some("judge-1"), Some("nvidia/x"), &Answer { status: 200, body: Vec::new(), request_id: Some("r1".into()), retry_after: None }, Some(760), Duration::from_millis(1200));
        assert_eq!(line, "equipe judge-relay: judge-1 200 model nvidia/x tokens 760 request r1 1200 ms");
    }
}
