//! Gmail as the mailbox verification codes are read from, over the Gmail
//! API with the read-only scope (`gmail.readonly`: the narrowest one that
//! can read a message's text — `gmail.metadata` can't, and can't search).
//! The grant can't send, delete, label or change anything, whatever equipe
//! does with it.
//!
//! Connecting is Google's flow for desktop apps: the consent page opens in
//! the person's own browser (never a member's), and comes back to a
//! one-off listener on this machine's loopback address with a code, which
//! equipe trades — with its PKCE verifier — for a refresh token. That token
//! goes to the secret store alone, never to a member, a screen, an event or
//! the log.
//!
//! Reading asks Gmail for the messages from the site's sender domains since
//! the code was asked for (`from:(github.com) after:<time>`), reads each
//! one's sender and date, and the text only of those that pass — at most
//! [`READ_AT_MOST`]. No other mail is fetched.

use serde_json::Value;

use super::{Wanted, pct};
use crate::autopilot::mail::Message;

/// The one scope equipe asks for.
pub const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
/// Messages whose text is read for one look, the newest first.
pub const READ_AT_MOST: usize = 3;
/// How long the consent page may take.
pub const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// An OAuth client: this build's own (set when it was built, or in the
/// daemon's environment), or one the person brought.
#[derive(Clone, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub secret: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("id", &self.id).field("secret", &"•••").finish()
    }
}

/// The client this build carries: `EQUIPE_GOOGLE_CLIENT_ID` and
/// `EQUIPE_GOOGLE_CLIENT_SECRET`, from the daemon's environment or from
/// when equipe was built. A build from source has none until set.
pub fn builtin() -> Option<Client> {
    let read = |name: &str, built: Option<&'static str>| std::env::var(name).ok().or_else(|| built.map(str::to_owned)).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let id = read("EQUIPE_GOOGLE_CLIENT_ID", option_env!("EQUIPE_GOOGLE_CLIENT_ID"))?;
    let secret = read("EQUIPE_GOOGLE_CLIENT_SECRET", option_env!("EQUIPE_GOOGLE_CLIENT_SECRET")).unwrap_or_default();
    Some(Client { id, secret })
}

/// What is kept in the secret store once connected: the grant and the
/// client it belongs to. Its `Debug` shows neither secret.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Grant {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant").field("client_id", &self.client_id).field("client_secret", &"•••").field("refresh_token", &"•••").finish()
    }
}

/// Base64url without padding.
fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A random URL-safe token of `n` bytes.
fn random(n: usize) -> Result<String, String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| "no randomness on this machine".to_owned())?;
    Ok(b64url(&bytes))
}

/// One connect's secrets: the PKCE verifier and the state the redirect must carry back.
pub struct Flow {
    pub verifier: String,
    pub state: String,
    pub redirect: String,
}

impl Flow {
    pub fn new(port: u16) -> Result<Self, String> {
        Ok(Flow { verifier: random(48)?, state: random(24)?, redirect: format!("http://127.0.0.1:{port}") })
    }

    /// Google's consent page for this flow: read-only Gmail, offline (a
    /// refresh token), PKCE S256, consent shown so a token is always given.
    pub fn consent_url(&self, client_id: &str) -> String {
        let challenge = b64url(ring::digest::digest(&ring::digest::SHA256, self.verifier.as_bytes()).as_ref());
        format!(
            "{AUTH}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&code_challenge={challenge}&code_challenge_method=S256&state={}",
            pct(client_id),
            pct(&self.redirect),
            pct(SCOPE),
            pct(&self.state)
        )
    }

    /// The code out of the redirect's request line (`GET /?state=…&code=… HTTP/1.1`):
    /// only with this flow's state; Google's error said plainly.
    pub fn code_from(&self, request_line: &str) -> Result<String, String> {
        let target = request_line.split_whitespace().nth(1).ok_or("not a redirect")?;
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
        let get = |key: &str| query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| unpct(v));
        if let Some(error) = get("error") {
            return Err(if error == "access_denied" { "the person didn't allow it".into() } else { format!("Google said: {error}") });
        }
        if get("state").as_deref() != Some(self.state.as_str()) {
            return Err("the answer wasn't for this connect".into());
        }
        get("code").filter(|c| !c.is_empty()).ok_or_else(|| "Google sent no code".into())
    }
}

/// `%41` → `A`, `+` → space.
pub(crate) fn unpct(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'+', _) => out.push(b' '),
            (b'%', Some(b)) => {
                out.push(b);
                i += 2;
            }
            (b, _) => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={}", pct(v))).collect::<Vec<_>>().join("&")
}

/// Google's answer as JSON, or why not — never echoing what was sent.
async fn json_of(response: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
    let response = response.map_err(|e| format!("Google didn't answer ({})", if e.is_timeout() { "timed out" } else { "network" }))?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| "Google's answer was cut off".to_owned())?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = body["error_description"].as_str().or(body["error"]["message"].as_str()).or(body["error"].as_str()).unwrap_or("refused");
        return Err(format!("Google said {}: {}", status.as_u16(), why.chars().take(120).collect::<String>()));
    }
    Ok(body)
}

/// Trade the redirect's code for the grant (a refresh token).
pub async fn exchange(http: &reqwest::Client, client: &Client, flow: &Flow, code: &str) -> Result<Grant, String> {
    let body = form(&[("client_id", &client.id), ("client_secret", &client.secret), ("code", code), ("code_verifier", &flow.verifier), ("redirect_uri", &flow.redirect), ("grant_type", "authorization_code")]);
    let answer = json_of(http.post(TOKEN).header("content-type", "application/x-www-form-urlencoded").body(body).send().await).await?;
    granted_only_readonly(answer["scope"].as_str().unwrap_or(SCOPE))?;
    let refresh_token = answer["refresh_token"].as_str().ok_or("Google gave no lasting grant; try Connect again")?.to_owned();
    Ok(Grant { client_id: client.id.clone(), client_secret: client.secret.clone(), refresh_token })
}

/// A grant for more than reading mail is refused, not kept.
pub fn granted_only_readonly(scope: &str) -> Result<(), String> {
    let mail: Vec<&str> = scope.split_whitespace().filter(|s| s.contains("mail")).collect();
    if mail.iter().all(|s| *s == SCOPE) && mail.contains(&SCOPE) { Ok(()) } else { Err("Google granted more than read-only mail; equipe won't keep it".into()) }
}

/// A short-lived access token from the grant.
pub async fn access(http: &reqwest::Client, grant: &Grant) -> Result<String, String> {
    let body = form(&[("client_id", &grant.client_id), ("client_secret", &grant.client_secret), ("refresh_token", &grant.refresh_token), ("grant_type", "refresh_token")]);
    let answer = json_of(http.post(TOKEN).header("content-type", "application/x-www-form-urlencoded").body(body).send().await).await.map_err(|why| {
        if why.contains("invalid_grant") { "Gmail's access was taken back; connect it again from Logins › Connections".to_owned() } else { why }
    })?;
    answer["access_token"].as_str().map(str::to_owned).ok_or_else(|| "Google gave no access token".into())
}

/// The connected address.
pub async fn address(http: &reqwest::Client, token: &str) -> Result<String, String> {
    let answer = json_of(http.get(format!("{API}/profile")).bearer_auth(token).send().await).await?;
    answer["emailAddress"].as_str().map(str::to_owned).ok_or_else(|| "Gmail didn't say which address".into())
}

/// The search that finds only the site's mail since `since`.
pub fn query(wanted: &Wanted, since: u64) -> String {
    let from = wanted.domains().join(" OR ");
    format!("from:({from}) after:{since}")
}

/// A message's sender and arrival out of a `format=metadata` answer.
fn head_of(json: &Value) -> (String, u64, String) {
    let headers = json["payload"]["headers"].as_array().cloned().unwrap_or_default();
    let header = |name: &str| headers.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|h| h["value"].as_str()).unwrap_or_default().to_owned();
    let at = json["internalDate"].as_str().and_then(|d| d.parse::<u64>().ok()).map_or(0, |ms| ms / 1000);
    (header("from"), at, header("subject"))
}

/// A `format=full` message's readable text: its text parts, else its HTML read as text.
pub fn text_of(json: &Value) -> String {
    fn walk(part: &Value, plain: &mut Vec<String>, html: &mut Vec<String>) {
        use base64::Engine as _;
        let mime = part["mimeType"].as_str().unwrap_or_default().to_ascii_lowercase();
        if let Some(data) = part["body"]["data"].as_str() {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')).or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(data)).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if mime == "text/plain" {
                plain.push(text);
            } else if mime == "text/html" {
                html.push(crate::autopilot::mail::html_as_text(&text));
            }
        }
        for child in part["parts"].as_array().into_iter().flatten() {
            walk(child, plain, html);
        }
    }
    let (mut plain, mut html) = (Vec::new(), Vec::new());
    walk(&json["payload"], &mut plain, &mut html);
    if plain.is_empty() { html.join("\n") } else { plain.join("\n") }
}

/// The site's messages since `since`: searched for on Gmail's side, each
/// one's sender and date read, and the text of the newest few whose sender
/// is the site's own.
pub async fn look(http: &reqwest::Client, grant: &Grant, wanted: &Wanted, since: u64) -> Result<Vec<Message>, String> {
    let token = access(http, grant).await?;
    let list = json_of(http.get(format!("{API}/messages?maxResults=10&q={}", pct(&query(wanted, since)))).bearer_auth(&token).send().await).await?;
    let ids: Vec<String> = list["messages"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect();
    let mut heads = Vec::new();
    for id in ids {
        let meta = json_of(http.get(format!("{API}/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject", pct(&id))).bearer_auth(&token).send().await).await?;
        let (from, at, subject) = head_of(&meta);
        let sender_ok = crate::autopilot::codes::sender_domain(&from).is_some_and(|d| crate::autopilot::codes::sender_matches(&d, &wanted.site, wanted.mail_from.as_deref()));
        if sender_ok && at >= since {
            heads.push((id, from, at, subject));
        }
    }
    heads.sort_by_key(|h| std::cmp::Reverse(h.2));
    let mut out = Vec::new();
    for (id, from, at, subject) in heads.into_iter().take(READ_AT_MOST) {
        let full = json_of(http.get(format!("{API}/messages/{}?format=full", pct(&id))).bearer_auth(&token).send().await).await?;
        out.push(Message { from, at, subject, text: text_of(&full) });
    }
    Ok(out)
}

/// The one-off listener's answer to the browser: a plain page that says
/// what happened, in the person's language.
pub fn redirect_page(title: &str, body: &str) -> String {
    let escape = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>equipe</title><body style=\"font:16px -apple-system,system-ui,sans-serif;max-width:32em;margin:15vh auto;padding:0 1em;color:#222\"><h1 style=\"font-size:20px\">{}</h1><p>{}</p></body>",
        escape(title),
        escape(body)
    );
    format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n\r\n{html}", html.len())
}

/// Wait on `listener` for Google's redirect: the request line of the first
/// request that carries a `state` (a browser's favicon request is answered
/// and passed over), and the stream to answer on.
pub async fn redirect(listener: &tokio::net::TcpListener) -> Result<(String, tokio::net::TcpStream), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn flow() -> Flow {
        Flow { verifier: "v".repeat(64), state: "st8".into(), redirect: "http://127.0.0.1:53682".into() }
    }

    #[test]
    fn the_consent_page_asks_for_read_only_mail_with_pkce_and_nothing_else() {
        let url = flow().consent_url("123.apps.googleusercontent.com");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"), "{url}");
        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly&"), "the one read-only scope: {url}");
        assert!(!url.contains("gmail.modify") && !url.contains("mail.google.com%2F&") && !url.contains("gmail.send"));
        assert!(url.contains("code_challenge_method=S256") && url.contains("access_type=offline") && url.contains("state=st8"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53682"));
        assert!(!url.contains(&"v".repeat(64)), "the verifier stays here; only its hash goes");
    }

    #[test]
    fn the_redirect_gives_its_code_only_with_this_flows_state() {
        let f = flow();
        assert_eq!(f.code_from("GET /?state=st8&code=4%2F0Ab-x&scope=x HTTP/1.1").as_deref(), Ok("4/0Ab-x"));
        assert!(f.code_from("GET /?state=other&code=4%2F0Ab HTTP/1.1").is_err(), "another flow's answer");
        assert_eq!(f.code_from("GET /?error=access_denied&state=st8 HTTP/1.1"), Err("the person didn't allow it".into()));
        assert!(f.code_from("GET /?state=st8 HTTP/1.1").is_err());
    }

    #[test]
    fn a_grant_for_more_than_reading_is_refused() {
        assert!(granted_only_readonly(SCOPE).is_ok());
        assert!(granted_only_readonly(&format!("openid {SCOPE}")).is_ok());
        assert!(granted_only_readonly("https://www.googleapis.com/auth/gmail.modify").is_err());
        assert!(granted_only_readonly(&format!("{SCOPE} https://mail.google.com/")).is_err());
        let grant = Grant { client_id: "c".into(), client_secret: "GOCSPX-x".into(), refresh_token: "1//refresh".into() };
        let shown = format!("{grant:?}");
        assert!(!shown.contains("GOCSPX-x") && !shown.contains("1//refresh"), "{shown}");
    }

    #[test]
    fn the_search_names_only_the_sites_sender_and_the_time() {
        let wanted = Wanted { site: "github.com".into(), mail_from: None, requested_at: 1_794_528_000 };
        assert_eq!(query(&wanted, 1_794_527_910), "from:(github.com) after:1794527910");
        let aws = Wanted { site: "console.aws.amazon.com".into(), mail_from: Some("signin.aws".into()), requested_at: 0 };
        assert_eq!(query(&aws, 5), "from:(amazon.com OR signin.aws) after:5");
    }

    #[test]
    fn a_messages_text_and_head_are_read_from_gmails_answer() {
        use base64::Engine as _;
        let enc = |t: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(t);
        let full = serde_json::json!({
            "internalDate": "1794528030000",
            "payload": { "mimeType": "multipart/alternative", "headers": [{ "name": "From", "value": "GitHub <noreply@github.com>" }, { "name": "Subject", "value": "[GitHub] Sudo email verification code" }],
                "parts": [
                    { "mimeType": "text/plain", "body": { "data": enc("Verification code: 48213907\n") } },
                    { "mimeType": "text/html", "body": { "data": enc("<p>Verification code: <b>48213907</b></p>") } }
                ] }
        });
        assert_eq!(head_of(&full), ("GitHub <noreply@github.com>".to_owned(), 1_794_528_030, "[GitHub] Sudo email verification code".to_owned()));
        assert_eq!(text_of(&full).trim(), "Verification code: 48213907");
        let html_only = serde_json::json!({ "payload": { "mimeType": "text/html", "body": { "data": enc("<style>.x{}</style><p>Code <b>123456</b></p>") } } });
        assert!(text_of(&html_only).contains("123456") && !text_of(&html_only).contains("<b>"));
    }

    #[test]
    fn the_listeners_page_is_a_whole_http_answer_with_escaped_words() {
        let page = redirect_page("Connected <ok>", "You can close this tab.");
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n") && page.contains("Connected &lt;ok&gt;"));
        let (head, body) = page.split_once("\r\n\r\n").unwrap();
        assert!(head.contains(&format!("content-length: {}", body.len())));
    }
}
