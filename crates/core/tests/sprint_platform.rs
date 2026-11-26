//! The equipe platform as a squad's product data (docs/SPRINTS.md, "Cloud
//! teams"): reads over HTTP against a stand-in platform, a hypothesis judged
//! through it, errors told plainly, and the team's key never in anything
//! said or logged.

use std::sync::{Arc, Mutex};

use equipe_core::protocol::growth::Better;
use equipe_core::protocol::sprint::{Experiment, ExperimentStatus, Metric, Rule, Split, Verdict};
use equipe_core::sprint::data::{self, ProductData};
use equipe_core::sprint::platform::{glance, live_from, Platform};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const KEY: &str = "eqk_0123456789abcdef0123456789abcdef0123456789abcdef";

/// One request the stand-in platform saw.
#[derive(Clone, Debug)]
struct Seen {
    line: String,
    auth: String,
    body: Value,
}

/// A stand-in platform: `answer(path, body)` gives (status, JSON).
async fn serve(answer: impl Fn(&str, &Value) -> (u16, Value) + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (log, answer) = (seen.clone(), Arc::new(answer));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (log, answer) = (log.clone(), answer.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let line = text.lines().next().unwrap_or_default().to_owned();
                let auth = text.lines().find_map(|l| l.to_ascii_lowercase().starts_with("authorization:").then(|| l.split_once(':').unwrap().1.trim().to_owned())).unwrap_or_default();
                let body: Value = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
                let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
                let (status, out) = answer(&path, &body);
                log.lock().unwrap().push(Seen { line, auth, body });
                let out = out.to_string();
                let _ = sock.write_all(format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}", out.len()).as_bytes()).await;
            });
        }
    });
    (url, seen)
}

fn signup() -> Metric {
    Metric::Event { event: "signup".into(), filter: "plan=pro".into() }
}

#[tokio::test]
async fn reads_events_conversions_and_splits() {
    let (url, seen) = serve(|path, body| {
        assert_eq!(path, "/v1/metrics/read");
        if body.get("split").is_some() {
            return (200, json!({ "arms": [ { "variant": "control", "value": 5.0, "sample": 400.0 }, { "variant": "b", "value": 9.0, "sample": 410.0 } ] }));
        }
        match body["metric"]["kind"].as_str() {
            Some("event") => (200, json!({ "value": 42.0, "sample": 42.0 })),
            _ => (200, json!({ "value": 12.5, "sample": 800.0 })),
        }
    })
    .await;
    let p = Platform::new(&url, KEY, Some("shop"));
    assert_eq!(p.source(), "equipe platform");
    assert!(p.splits());
    let r = p.read(&signup(), 20_400, 20_406).await.unwrap();
    assert_eq!((r.value, r.sample), (42.0, 42.0));
    let conv = Metric::Conversion { from: "$pageview".into(), to: "signup".into(), days: 1, filter: String::new() };
    let r = p.read(&conv, 20_400, 20_406).await.unwrap();
    assert_eq!((r.value, r.sample), (12.5, 800.0));
    let arms = p.read_split(&Split { flag: "cta".into(), control: "control".into() }, &conv, 20_400, 20_406).await.unwrap();
    assert_eq!(arms.len(), 2);
    assert_eq!((arms[1].variant.as_str(), arms[1].value, arms[1].sample), ("b", 9.0, 410.0));

    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|s| s.auth == format!("Bearer {KEY}") && s.line.starts_with("POST /v1/metrics/read")));
    let first = &seen[0].body;
    assert_eq!(first["app"], "shop");
    assert_eq!(first["metric"], json!({ "kind": "event", "event": "signup", "filter": "plan=pro" }));
    assert_eq!((first["from"].as_str(), first["to"].as_str()), (Some("2025-11-08"), Some("2025-11-14")));
    assert_eq!(seen[2].body["split"], json!({ "flag": "cta" }));
    assert_eq!(seen[1].body["metric"]["days"], 1);
}

#[tokio::test]
async fn judges_a_split_by_its_rule_through_the_platform() {
    let (url, _) = serve(|_, _| (200, json!({ "arms": [ { "variant": "control", "value": 5.0, "sample": 400.0 }, { "variant": "b", "value": 9.0, "sample": 410.0 } ] }))).await;
    let p = Platform::new(&url, KEY, None);
    let x = Experiment {
        id: 1,
        sprint: 1,
        belief: "A shorter form gets more signups".into(),
        metric: Metric::Conversion { from: "$pageview".into(), to: "signup".into(), days: 1, filter: String::new() },
        rule: Rule { better: Better::Up, min_change_pct: 10.0, window_days: 7, min_sample: 200 },
        split: Some(Split { flag: "short-form".into(), control: "control".into() }),
        events: vec![],
        ticket: None,
        item: None,
        defined_by: "pm".into(),
        defined_at: 0,
        status: ExperimentStatus::Live,
        live_at: Some(1_700_000_000),
        due_at: None,
        checked: false,
        result: None,
    };
    let judged = data::judge(&p, &x, 1_700_000_000 + 8 * 86_400).await;
    assert_eq!(judged.verdict, Verdict::Worked, "{}", judged.why);
    assert_eq!(judged.source, "equipe platform");
    assert_eq!(judged.arms.len(), 2);
}

#[tokio::test]
async fn errors_are_plain_and_never_carry_the_key() {
    let (url, _) = serve(|_, _| (403, json!({ "error": { "code": "forbidden", "message": "This key cannot read that app." } }))).await;
    let p = Platform::new(&url, KEY, None);
    let why = p.read(&signup(), 1, 2).await.unwrap_err();
    assert!(why.contains("403") && why.contains("This key cannot read that app."), "{why}");
    assert!(!why.contains(KEY));

    let (url, _) = serve(|_, _| (200, json!({ "nope": true }))).await;
    let why = Platform::new(&url, KEY, None).read(&signup(), 1, 2).await.unwrap_err();
    assert!(why.contains("weren't what was asked") && !why.contains(KEY), "{why}");

    // Nothing listens there.
    let gone = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let why = Platform::new(&gone, KEY, None).read(&signup(), 1, 2).await.unwrap_err();
    assert!(why.contains("couldn't be reached") && !why.contains(KEY), "{why}");
}

#[tokio::test]
async fn the_machine_settings_file_is_read_and_a_missing_key_means_no_platform() {
    let dir = std::env::temp_dir().join(format!("equipe-platform-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (url, seen) = serve(|_, _| (200, json!({ "resources": [ { "kind": "database", "name": "main" }, { "kind": "app", "name": "shop" } ] }))).await;
    let file = dir.join("env");
    std::fs::write(&file, format!("# team\nexport PLATFORM_URL='{url}/'\nPLATFORM_KEY={KEY}\nPLATFORM_APP=shop\n")).unwrap();
    let p = Platform::from_file(&file).expect("a key is there");
    assert!(p.has_app().await.unwrap());
    assert_eq!(seen.lock().unwrap()[0].line, "GET /v1/resources HTTP/1.1");
    std::fs::write(&file, "PLATFORM_URL=https://example.test\nPLATFORM_KEY=\n").unwrap();
    assert!(Platform::from_file(&file).is_none());
    assert!(Platform::from_file(&dir.join("absent")).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_glance_is_short_and_in_plain_words() {
    let snap = json!({
        "period": { "from": "2026-11-14", "to": "2026-11-20", "days": 7 },
        "measures": { "visitors": { "value": 120, "previous": 100 }, "pageviews": { "value": 300, "previous": 250 }, "retention_d1": 0.25, "retention_d7": null },
        "top_events": [ { "name": "$pageview", "count": 300 }, { "name": "signup", "count": 9 }, { "name": "booking", "count": 3 } ],
        "top_referrers": [ { "referrer": "news.example", "visitors": 40 }, { "referrer": "x.example", "visitors": 7 } ],
        "experiments": [ { "key": "cta", "verdict": "insufficient_data" } ]
    });
    let (evidence, lines) = glance(&snap);
    let metrics: Vec<&str> = evidence.iter().map(|e| e.metric.as_str()).collect();
    assert_eq!(metrics, ["visitors", "visits", "retention_d1", "event:signup", "event:booking"]);
    assert_eq!(evidence[0].previous, Some(100.0));
    assert_eq!(evidence[2].value, 25.0);
    assert!(evidence.iter().all(|e| e.source == "equipe platform" && e.from == "2026-11-14"));
    assert_eq!(lines, ["Top sources (visitors): news.example (40), x.example (7).", "Running experiment `cta`: insufficient data."]);
    assert!(!format!("{evidence:?}{lines:?}").contains("eqk_"));
}

/// A cloud team's product counts as live for the sprint loop only with an app on
/// the platform and its first free version's quiet day over.
#[test]
fn live_needs_an_app_and_the_quiet_day_over() {
    let app = json!({ "resources": [{ "kind": "database", "name": "db" }, { "kind": "app", "name": "shop" }] });
    assert!(live_from(&app, 1_000));
    assert!(!live_from(&json!({ "resources": [{ "kind": "database", "name": "db" }] }), 1_000), "no app: nothing live");
    let quiet = json!({ "resources": [{ "kind": "app", "name": "shop" }], "quiet_until": 2_000 });
    assert!(!live_from(&quiet, 1_000), "the first free version's quiet day");
    assert!(live_from(&quiet, 2_001), "over");
    assert!(live_from(&json!({ "resources": [{ "kind": "app", "name": "shop" }], "quiet_until": 0 }), 1_000));
}
