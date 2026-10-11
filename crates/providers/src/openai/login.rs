//! Interactive login: a loopback callback server plus the user's browser.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use super::oauth::{self, CALLBACK_PATH, CALLBACK_PORT};
use crate::credentials::{CredentialStore, Credentials};
use crate::error::{Error, Result};

/// How long one connection may take to send its request line before we drop it,
/// so a stalled client can't hold up the real callback.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_BYTES: usize = 16 * 1024;

pub struct LoginOptions {
    /// How long to wait for the browser to come back.
    pub timeout: Duration,
    pub open_browser: bool,
}

/// What a request to the loopback server means for the login.
#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    /// Not the callback path (favicon, probes): answer 404 and keep waiting.
    NotFound,
    /// A callback that doesn't belong to this login attempt: reject, keep waiting.
    StateMismatch,
    /// The provider sent the user back with an error instead of a code.
    Denied(String),
    Code(String),
}

/// Classify a request target such as `/auth/callback?code=…&state=…`.
pub fn parse_callback(target: &str, expected_state: &str) -> Callback {
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return Callback::NotFound;
    };
    if url.path() != CALLBACK_PATH {
        return Callback::NotFound;
    }
    let get = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    if get("state").as_deref() != Some(expected_state) {
        return Callback::StateMismatch;
    }
    match get("code").filter(|code| !code.is_empty()) {
        Some(code) => Callback::Code(code),
        None => Callback::Denied(
            get("error_description")
                .or_else(|| get("error"))
                .unwrap_or_else(|| "Missing authorization code.".into()),
        ),
    }
}

/// Run the full login and persist the credentials.
pub async fn login(
    client: &reqwest::Client,
    store: &CredentialStore,
    options: LoginOptions,
    on_url: impl FnOnce(&str, std::result::Result<(), String>),
) -> Result<Credentials> {
    let req = oauth::authorization_request(CALLBACK_PORT);
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrInUse => Error::Login(format!(
                "port {CALLBACK_PORT} is in use. Close whatever holds it (another login?) and retry."
            )),
            _ => err.into(),
        })?;

    // Tell the caller whether a browser really opened, so it can keep the long
    // URL off the screen only when it isn't needed.
    let opened = if options.open_browser {
        open_in_browser(&req.url).await
    } else {
        Err("--no-browser".into())
    };
    on_url(&req.url, opened);

    let (code, mut stream) = tokio::time::timeout(options.timeout, wait_for_callback(&listener, &req.state))
        .await
        .map_err(|_| Error::LoginTimeout)??;

    // Answer the browser only once the exchange is done, so it never says
    // "signed in" for a login that then fails.
    match oauth::exchange_code(client, &code, &req.verifier, &req.redirect_uri).await {
        Ok(cred) => {
            store.save(&cred)?;
            respond(&mut stream, "200 OK", &page("You're signed in", "Return to your terminal — equipe has your session.")).await;
            Ok(cred)
        }
        Err(err) => {
            respond(&mut stream, "502 Bad Gateway", &page("Login failed", &escape_html(&err.to_string()))).await;
            Err(err)
        }
    }
}

/// Accept connections until the real callback arrives. Returns its code and the
/// still-open connection so the caller can report the outcome to the browser.
async fn wait_for_callback(listener: &TcpListener, state: &str) -> Result<(String, TcpStream)> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(Some(target)) = tokio::time::timeout(REQUEST_READ_TIMEOUT, read_request_target(&mut stream))
            .await
            .unwrap_or(Ok(None))
        else {
            continue;
        };
        match parse_callback(&target, state) {
            Callback::Code(code) => return Ok((code, stream)),
            Callback::NotFound => respond(&mut stream, "404 Not Found", "").await,
            Callback::StateMismatch => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    &page("Login failed", "This sign-in link is stale. Retry <code>equipe login</code>."),
                )
                .await
            }
            Callback::Denied(reason) => {
                respond(&mut stream, "400 Bad Request", &page("Login failed", &escape_html(&reason))).await;
                return Err(Error::Login(format!("sign-in was not completed: {reason}")));
            }
        }
    }
}

/// Read up to the end of the request head and return the request target.
async fn read_request_target(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && !buf.windows(2).any(|w| w == b"\n\n") {
        let n = stream.read(&mut chunk).await?;
        if n == 0 || buf.len() > MAX_REQUEST_BYTES {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    Ok(head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(str::to_owned))
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The browser tab may already be gone; the login result doesn't depend on it.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn page(title: &str, body_html: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         body{{font:16px system-ui;margin:0;display:grid;place-items:center;height:100vh;background:#0a0a0c;color:#efede7}}\
         .card{{max-width:420px;padding:32px;text-align:center}}h1{{font-weight:500;font-size:20px}}p{{color:#a0a0a0}}\
         </style></head><body><div class=\"card\"><h1>{title}</h1><p>{body_html}</p></div></body></html>"
    )
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Launch the system browser and wait for the opener's verdict.
///
/// Starting the opener is not enough: `open` and `xdg-open` start fine and then
/// fail when there is no desktop session to hand the URL to (SSH, some embedded
/// terminals). Their exit status is the only honest signal.
async fn open_in_browser(url: &str) -> std::result::Result<(), String> {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let run = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(10), run).await {
        Err(_) => Err("the browser opener didn't answer".into()),
        Ok(Err(err)) => Err(format!("couldn't start a browser opener: {err}")),
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => {
            let reason = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            Err(if reason.is_empty() {
                format!("the browser opener failed ({})", out.status)
            } else {
                reason
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_callback_with_a_code() {
        assert_eq!(
            parse_callback("/auth/callback?code=abc&state=s1", "s1"),
            Callback::Code("abc".into())
        );
    }

    #[test]
    fn other_paths_are_not_the_callback() {
        assert_eq!(parse_callback("/favicon.ico", "s1"), Callback::NotFound);
        assert_eq!(parse_callback("/", "s1"), Callback::NotFound);
    }

    #[test]
    fn rejects_a_callback_for_another_login_attempt() {
        assert_eq!(parse_callback("/auth/callback?code=abc&state=other", "s1"), Callback::StateMismatch);
        assert_eq!(parse_callback("/auth/callback?code=abc", "s1"), Callback::StateMismatch);
    }

    #[test]
    fn reports_why_the_provider_denied_the_login() {
        assert_eq!(
            parse_callback("/auth/callback?state=s1&error=access_denied&error_description=User%20cancelled", "s1"),
            Callback::Denied("User cancelled".into())
        );
        assert_eq!(
            parse_callback("/auth/callback?state=s1&error=access_denied", "s1"),
            Callback::Denied("access_denied".into())
        );
        assert_eq!(
            parse_callback("/auth/callback?state=s1&code=", "s1"),
            Callback::Denied("Missing authorization code.".into())
        );
    }

    #[test]
    fn escapes_text_shown_in_the_browser() {
        assert_eq!(
            escape_html(r#"<script>alert("x")</script> & 'y'"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;y&#39;"
        );
    }

    async fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn waits_past_stray_requests_for_the_real_callback() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            let favicon = get(port, "/favicon.ico").await;
            let stale = get(port, "/auth/callback?code=old&state=stale").await;
            let mut real = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            real.write_all(b"GET /auth/callback?code=good&state=s1 HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            (favicon, stale, real)
        });

        let (code, mut stream) = wait_for_callback(&listener, "s1").await.unwrap();
        assert_eq!(code, "good");
        respond(&mut stream, "200 OK", "ok").await;

        let (favicon, stale, mut real) = browser.await.unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(stale.starts_with("HTTP/1.1 400"));
        let mut answer = String::new();
        real.read_to_string(&mut answer).await.unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"));
    }

    #[tokio::test]
    async fn stops_waiting_when_the_provider_denies_the_login() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            get(port, "/auth/callback?state=s1&error=access_denied").await
        });
        let err = wait_for_callback(&listener, "s1").await.unwrap_err();
        assert!(err.to_string().contains("access_denied"));
        assert!(browser.await.unwrap().starts_with("HTTP/1.1 400"));
    }
}
