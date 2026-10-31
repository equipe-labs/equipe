//! Signals read straight from a product's numbers, against stand-in APIs: a
//! PostHog that counts events and an App Store Connect that hands out daily
//! sales reports. The PM proposes each naming only the file the person saved
//! the keys to; on the person's yes the daemon reads the API itself and keeps
//! the number — the key never on the card, in a reading or on a screen. A
//! refused key fails the signal and wakes the PM; the person pauses one from
//! a screen.

#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use equipe_core::journal::Journal;
use equipe_core::protocol::rpc::{ToolCall, ToolResult};
use equipe_core::protocol::{Access, Command, Decision, Event, SessionSpec};
use equipe_core::runtime::{self, Config, Handle};
use equipe_core::squads;
use equipe_providers::CredentialStore;
use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, UnparsedPublicKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc::UnboundedReceiver;

const POSTHOG_KEY: &str = "phx_StandInPersonalKeyThatMustNeverShow";
const ASC_KEY_ID: &str = "STANDIN123";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("equipe-sigsrc-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A stand-in `claude`: signed in, and a turn takes a long while.
fn busy_claude() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = scratch("bin").join("claude");
    let script = "#!/bin/sh\nif [ \"$1\" = \"auth\" ]; then echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0; fi\nexec sleep 30\n";
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn config(claude: &Path, home: &Path) -> Config {
    Config {
        store: CredentialStore::new(home),
        home: home.to_owned(),
        client_version: "test".into(),
        claude_binary: claude.to_string_lossy().into_owned(),
        codex_binary: "/nonexistent/codex".into(),
        persist: true,
        retention_days: 90,
        equipe_binary: Some("/opt/equipe".into()),
        npx: None,
        mac: Box::new(equipe_core::desktop::ThisMac),
        suggest: false,
        browser_host: false,
        person_home: Some(home.join("person")),
        lang: equipe_core::i18n::Lang::Ko,
        nemotron: None,
    }
}

/// One request the stand-in API saw.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Answer = (u16, &'static str, Vec<u8>);

/// A stand-in HTTP API on a free port: each request kept, answered by `answer`.
async fn fake_api(answer: impl Fn(&Seen) -> Answer + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (kept, answer) = (seen.clone(), Arc::new(answer));
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let (kept, answer) = (kept.clone(), answer.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let Ok(n) = stream.read(&mut chunk).await else { return };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let (method, path) = (first.next().unwrap_or_default().to_owned(), first.next().unwrap_or_default().to_owned());
                let headers: HashMap<String, String> =
                    lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned())).collect();
                let length: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
                while buf.len() < head_end + length {
                    let Ok(n) = stream.read(&mut chunk).await else { return };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                let request = Seen { method, path, headers, body };
                let (status, kind, out) = answer(&request);
                kept.lock().unwrap().push(request);
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&out).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (url, seen)
}

async fn until(rx: &mut UnboundedReceiver<Event>, seen: &mut Vec<Event>, done: impl Fn(&[Event]) -> bool) {
    let wait = async {
        while !done(seen) {
            match rx.recv().await {
                Some(event) => seen.push(event),
                None => return,
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
        panic!("timed out; events so far: {seen:#?}");
    }
}

async fn start(handle: &Handle, events: &mut UnboundedReceiver<Event>, id: u64, role: &str, project: &Path) {
    let spec = SessionSpec { engine: None, model: None, access: Access::ReadOnly, cwd: project.to_owned(), role: Some(role.into()), squad: Some("acme".into()) };
    handle.send(Command::Start { session: id, spec });
    let mut seen = Vec::new();
    until(events, &mut seen, |s| s.iter().any(|e| matches!(e, Event::SessionStarted { session, .. } if *session == id))).await;
}

async fn tool(handle: &Handle, member: &str, name: &str, arguments: Value) -> ToolResult {
    handle.tool(ToolCall { squad: "acme".into(), member: member.into(), name: name.into(), arguments }).await
}

/// The PM proposes a signal; the person says yes. The card's words, and every event till then.
async fn propose_yes(handle: &Handle, events: &mut UnboundedReceiver<Event>, seen: &mut Vec<Event>, args: Value) -> String {
    let h = handle.clone();
    let waiting = tokio::spawn(async move { tool(&h, "pm", "propose_signal", args).await });
    let from = seen.len();
    until(events, seen, |s| s[from..].iter().any(|e| matches!(e, Event::ApprovalRequested { .. }))).await;
    let approval = seen[from..].iter().find_map(|e| if let Event::ApprovalRequested { approval, .. } = e { Some(approval.clone()) } else { None }).unwrap();
    handle.send(Command::Decide { squad: "acme".into(), approval: approval.id, decision: Decision::Allow, words: None });
    let said = waiting.await.unwrap();
    assert!(!said.is_error, "{said:?}");
    approval.action
}

/// The latest reading of `key` once there is one.
async fn reading(journal: &Journal, key: &str) -> equipe_core::signals::Reading {
    for _ in 0..200 {
        if let Some(r) = journal.signal_states().get(key).and_then(|s| s.readings.last().cloned()) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{key} was never read: {:?}", journal.signal_states());
}

fn gzip(text: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(text.as_bytes()).unwrap();
    enc.finish().unwrap()
}

#[tokio::test]
async fn posthog_and_app_store_signals_keep_their_numbers_and_never_show_a_key() {
    let project = scratch("p").join("site");
    std::fs::create_dir_all(&project).unwrap();
    let home = scratch("home");
    squads::create(&home, "acme", &project).unwrap();
    let person = home.join("person");

    // PostHog: 1234 pageviews for the right key, a refusal for any other.
    let (posthog, posthog_seen) = fake_api(|r| {
        if r.headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {POSTHOG_KEY}")) {
            return (401, "application/json", br#"{"detail":"Invalid personal API key."}"#.to_vec());
        }
        if r.method == "POST" && r.path == "/api/projects/700951/query/" {
            return (200, "application/json", br#"{"results":[[1234]],"columns":["count()"]}"#.to_vec());
        }
        (404, "application/json", b"{}".to_vec())
    })
    .await;

    // App Store Connect: a signed token checked against the key's public half;
    // each day's report gzipped, one day with no sales (404).
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let public = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap().public_key().as_ref().to_vec();
    let p8 = scratch("asc").join("AuthKey_STANDIN123.p8");
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref()));
    std::fs::write(&p8, pem).unwrap();
    let day = std::sync::atomic::AtomicU32::new(0);
    let (asc, asc_seen) = fake_api(move |r| {
        let token = r.headers.get("authorization").and_then(|a| a.strip_prefix("Bearer ")).unwrap_or_default();
        let parts: Vec<&str> = token.split('.').collect();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verified = parts.len() == 3
            && UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
                .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &b64.decode(parts[2]).unwrap_or_default())
                .is_ok();
        let header: Value = b64.decode(parts.first().unwrap_or(&"")).ok().and_then(|h| serde_json::from_slice(&h).ok()).unwrap_or_default();
        if !verified || header["kid"] != ASC_KEY_ID {
            return (401, "application/json", br#"{"errors":[{"status":"401","detail":"Provide a properly configured and signed bearer token."}]}"#.to_vec());
        }
        if !r.path.starts_with("/v1/salesReports?") || !r.path.contains("filter%5BvendorNumber%5D=89000000") {
            return (400, "application/json", b"{}".to_vec());
        }
        if day.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            return (404, "application/json", br#"{"errors":[{"status":"404","detail":"There were no sales for the date specified."}]}"#.to_vec());
        }
        let tsv = "Provider\tSKU\tUnits\tProduct Type Identifier\tApple Identifier\n\
APPLE\tequipe\t4\t1\t6740000000\nAPPLE\tequipe\t9\t7\t6740000000\nAPPLE\tequipe\t1\t1T\t6740000000\nAPPLE\tother\t50\t1\t1111111111\n";
        (200, "application/a-gzip", gzip(tsv))
    })
    .await;

    // The keys, as ask_secret cards would have saved them.
    let site_keys = person.join(".config/equipe-site");
    std::fs::create_dir_all(&site_keys).unwrap();
    std::fs::write(site_keys.join("analytics.env"), format!("POSTHOG_PROJECT_KEY=\"phc_x\"\nPOSTHOG_PERSONAL_API_KEY=\"{POSTHOG_KEY}\"\nPOSTHOG_HOST=\"{posthog}\"\n")).unwrap();
    std::fs::write(site_keys.join("wrong.env"), format!("POSTHOG_PERSONAL_API_KEY=\"phx_NotTheRightKeyAtAll000000\"\nPOSTHOG_HOST=\"{posthog}\"\n")).unwrap();
    let app_keys = person.join(".config/equipe-app");
    std::fs::create_dir_all(&app_keys).unwrap();
    std::fs::write(
        app_keys.join("asc.env"),
        format!("ASC_KEY_ID=\"{ASC_KEY_ID}\"\nASC_ISSUER_ID=\"69a6de7e-0000-47e3-e053-5b8c7c11a4d1\"\nASC_VENDOR_NUMBER=\"89000000\"\nASC_KEY_FILE=\"{}\"\nASC_API_URL=\"{asc}\"\n", p8.display()),
    )
    .unwrap();

    let (handle, mut events) = runtime::start(config(&busy_claude(), &home));
    start(&handle, &mut events, 1, "pm", &project).await;
    let journal = Journal::for_squad(&home, "acme").unwrap();
    let mut seen = Vec::new();

    // PostHog: the card says what it counts and from which file — not the key.
    let pageviews = json!({ "name": "pageviews", "source": "posthog", "posthog_project": 700951, "event": "$pageview", "days": 1, "keys": "~/.config/equipe-site/analytics.env", "change_pct": 30 });
    let card = propose_yes(&handle, &mut events, &mut seen, pageviews).await;
    assert!(card.contains("PostHog 700951 · $pageview · 1d") && card.contains("~/.config/equipe-site/analytics.env"), "{card}");
    let r = reading(&journal, "site/pageviews").await;
    assert!(r.ok && r.value.as_deref() == Some("1234"), "{r:?}");
    let query = posthog_seen.lock().unwrap()[0].clone();
    let sent: Value = serde_json::from_str(&query.body).unwrap();
    let hogql = sent["query"]["query"].as_str().unwrap();
    assert!(sent["query"]["kind"] == "HogQLQuery" && hogql.contains("event = '$pageview'") && hogql.contains("INTERVAL 1 DAY"), "{hogql}");

    // App Store: three days of first downloads of this app alone — 5, none, 5.
    let downloads = json!({ "name": "downloads", "source": "app_store", "app_id": "6740000000", "days": 3, "keys": "~/.config/equipe-app/asc.env" });
    let card = propose_yes(&handle, &mut events, &mut seen, downloads).await;
    assert!(card.contains("App Store 6740000000 · downloads · 3d"), "{card}");
    let r = reading(&journal, "site/downloads").await;
    assert!(r.ok && r.value.as_deref() == Some("10"), "{r:?}");
    let asked: Vec<String> = asc_seen.lock().unwrap().iter().map(|s| s.path.clone()).collect();
    assert_eq!(asked.len(), 3, "a report a day: {asked:?}");
    assert!(asked.iter().all(|p| p.contains("filter%5Bfrequency%5D=DAILY") && p.contains("filter%5BreportType%5D=SALES")), "{asked:?}");

    // A key PostHog refuses: failing with PostHog's words, and the PM woken.
    let refused = json!({ "name": "pageviews-wrong", "source": "posthog", "posthog_project": 700951, "keys": "~/.config/equipe-site/wrong.env" });
    propose_yes(&handle, &mut events, &mut seen, refused).await;
    let r = reading(&journal, "site/pageviews-wrong").await;
    assert_eq!((r.ok, r.note.as_deref()), (false, Some("PostHog said 401: Invalid personal API key.")));
    until(&mut events, &mut seen, |s| {
        s.iter().any(|e| matches!(e, Event::UserMessage { session: 1, text } | Event::MessageQueued { session: 1, text, .. } | Event::SteerQueued { session: 1, text, .. } if text.starts_with("[Signal · pageviews-wrong · failing]")))
    })
    .await;

    // Every screen saw the numbers.
    until(&mut events, &mut seen, |s| {
        s.iter().any(|e| matches!(e, Event::SignalsChanged { signals, .. } if signals.iter().any(|v| v.name == "downloads" && v.last().is_some_and(|r| r.value.as_deref() == Some("10")))))
    })
    .await;

    // The person pauses one from a screen: kept, shown paused, not run.
    handle.send(Command::SetSignalOn { squad: "acme".into(), project: "site".into(), name: "pageviews".into(), on: false });
    until(&mut events, &mut seen, |s| {
        s.iter().any(|e| matches!(e, Event::SignalsChanged { signals, .. } if signals.iter().any(|v| v.name == "pageviews" && !v.on)))
    })
    .await;
    assert!(journal.signals().iter().any(|s| s.name == "pageviews" && !s.on));
    let report = tool(&handle, "pm", "signals", json!({})).await;
    assert!(report.text.contains("site · pageviews — paused by the person") && report.text.contains("reads: PostHog 700951"), "{}", report.text);

    // No key anywhere a member or a screen could read it.
    let shown = format!("{seen:?}{:?}{:?}{}", journal.signals(), journal.signal_states(), report.text);
    assert!(!shown.contains(POSTHOG_KEY) && !shown.contains("phx_NotTheRightKey") && !shown.contains("PRIVATE KEY"), "a key showed");
    handle.shutdown().await;
}
