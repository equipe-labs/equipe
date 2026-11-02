use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::json;

use super::*;
use crate::protocol::metrics::{self as m};
use crate::routines::Utc;

const TOKEN: &str = "tok-SECRET-4815162342";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-metrics-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn metrics(home: &Path) -> Metrics {
    let mut m = Metrics::new(home, Box::new(InMemory::default()));
    m.with_fake = true;
    m
}

fn fake(extra: &[(&str, &str)]) -> ConnectParams {
    let mut settings = BTreeMap::from([("token".to_owned(), TOKEN.to_owned())]);
    settings.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
    ConnectParams { squad: "acme".into(), project: Some("site".into()), kind: Some(SourceKind::Fake), id: None, settings }
}

fn day(s: &str) -> i64 {
    day_number(s).unwrap()
}

/// Unix seconds of a day's noon, UTC.
fn noon(d: i64) -> u64 {
    (d * 86_400 + 12 * 3600) as u64
}

/// Every file under `dir`, read as text.
fn all_text(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
            }
        }
    }
    out
}

#[test]
fn days_go_both_ways() {
    for s in ["1970-01-01", "2026-11-02", "2024-02-29", "2000-12-31"] {
        assert_eq!(day_string(day(s)), s);
    }
    assert_eq!(day("1970-01-02"), 1);
    for bad in ["2026-02-30", "2026-13-01", "26-1-1", "", "2026-11-2"] {
        assert_eq!(day_number(bad), None, "{bad}");
    }
    // 2026-11-02 is a Monday.
    assert_eq!(monday(day("2026-11-08")), day("2026-11-02"));
    assert_eq!(monday(day("2026-11-02")), day("2026-11-02"));
}

#[tokio::test]
async fn connect_refresh_query_with_the_fake_source_keeps_a_snapshot_a_day() {
    let home = scratch("flow");
    let metrics = metrics(&home);
    let d = day("2026-11-02");
    let c = metrics.connect(fake(&[("seed", "3")]), noon(d)).unwrap();
    assert_eq!((c.id.as_str(), c.kind, c.keys.clone()), ("m1", SourceKind::Fake, vec!["token".to_owned()]));
    assert_eq!(c.settings, BTreeMap::from([("seed".to_owned(), "3".to_owned())]), "the token isn't a setting");

    metrics.refresh(&RefreshParams { squad: "acme".into(), ..Default::default() }, noon(d), d).await.unwrap();
    metrics.refresh(&RefreshParams { squad: "acme".into(), ..Default::default() }, noon(d + 1), d + 1).await.unwrap();
    let files: Vec<String> = std::fs::read_dir(home.join("metrics/acme/m1")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let mut files = files;
    files.sort();
    assert_eq!(files, ["2026-11-02.json", "2026-11-03.json"], "one snapshot per day fetched");
    let snap = &snapshots(&home, "acme", "m1")[0];
    assert_eq!((snap.schema, snap.kind), (SCHEMA, SourceKind::Fake));

    let r = query(&home, &QueryParams { squad: "acme".into(), days: Some(7), ..Default::default() }, d + 1).unwrap();
    assert_eq!((r.from.as_str(), r.to.as_str(), r.previous_from.as_str(), r.previous_to.as_str()), ("2026-10-28", "2026-11-03", "2026-10-21", "2026-10-27"));
    let visits = r.values.iter().find(|v| v.metric == "visits").unwrap();
    let want: u64 = (d + 1 - 6..=d + 1).map(|n| adapters::fake::visits(3, n)).sum();
    assert_eq!(visits.value, Some(want as f64));
    assert_eq!(visits.days_known, 7);
    assert_eq!(visits.series.len(), 7);
    let before: u64 = (d + 1 - 13..=d + 1 - 7).map(|n| adapters::fake::visits(3, n)).sum();
    assert_eq!(visits.previous, Some(before as f64));
    assert_eq!(visits.change_pct, Some(round1((want as f64 - before as f64) / before as f64 * 100.0)));

    let stars = r.values.iter().find(|v| v.metric == "stars").unwrap();
    assert_eq!(stars.value, Some(adapters::fake::stars(3, d + 1) as f64), "a running total is its last day's");
    let langs = r.values.iter().find(|v| v.metric == "languages").unwrap();
    assert_eq!(langs.breakdown[0].label, "English", "biggest first");
    assert_eq!(langs.breakdown.iter().map(|s| s.value).sum::<f64>(), want as f64, "the labels add up");

    let week = query(&home, &QueryParams { squad: "acme".into(), days: Some(14), bucket: Bucket::Week, ..Default::default() }, d + 1).unwrap();
    let series: Vec<&str> = week.values[0].series.iter().map(|s| s.day.as_str()).collect();
    assert_eq!(series, ["2026-10-21", "2026-10-26", "2026-11-02"], "cut at Mondays, the first bucket from the period's start");
}

#[tokio::test]
async fn days_the_service_forgot_stay_in_the_snapshots() {
    let home = scratch("history");
    let metrics = metrics(&home);
    let d = day("2026-10-01");
    metrics.connect(fake(&[]), noon(d)).unwrap();
    metrics.refresh_one("m1", noon(d), d).await;
    // Three weeks later the fake source (like GitHub) remembers 14 days only.
    metrics.refresh_one("m1", noon(d + 21), d + 21).await;
    let r = query(&home, &QueryParams { squad: "acme".into(), days: Some(35), metrics: vec!["visits".into()], ..Default::default() }, d + 21).unwrap();
    let v = &r.values[0];
    assert_eq!(v.days_known, 28, "14 days from each fetch");
    assert!(v.series.iter().any(|s| s.day == "2026-09-20" && s.value.is_some()), "a day only the first fetch knew");
    assert!(v.series.iter().any(|s| s.day == "2026-10-05" && s.value.is_none()), "a day neither knew is a gap, not 0");
}

#[tokio::test]
async fn a_key_is_never_written_logged_or_answered() {
    let home = scratch("keys");
    let metrics = metrics(&home);
    let now = 1_793_000_000;
    // One that works, one whose service quotes the token in its error.
    let ok = metrics.call(m::CONNECT, serde_json::to_value(fake(&[])).unwrap(), now).await.unwrap();
    let mut failing = fake(&[("fail", "auth")]);
    failing.project = Some("app".into());
    let bad = metrics.call(m::CONNECT, serde_json::to_value(failing).unwrap(), now).await.unwrap();
    let refreshed = metrics.call(m::REFRESH, json!({ "squad": "acme" }), now).await.unwrap();
    let mut answers = vec![ok, bad, refreshed];
    answers.push(metrics.call(m::SOURCES, json!({ "squad": "acme" }), now).await.unwrap());
    answers.push(metrics.call(m::QUERY, json!({ "squad": "acme", "days": 30 }), now).await.unwrap());
    answers.push(metrics.call(m::SUMMARY, json!({ "squad": "acme" }), now).await.unwrap());
    let today = day_of(&crate::routines::Local, now);
    let said = tool::answer(&home, "acme", &json!({ "days": 14 }), today).unwrap();

    for (i, a) in answers.iter().enumerate() {
        assert!(!a.to_string().contains(TOKEN), "RPC answer {i} holds the key: {a}");
    }
    assert!(!said.contains(TOKEN), "the tool's answer holds the key: {said}");
    assert!(said.contains("Visits") && said.contains("the last fetch failed"), "{said}");
    assert!(!all_text(&home).contains(TOKEN), "a file under equipe's home holds the key");

    // The failure is kept, its message without the key.
    let sources: Sources = serde_json::from_value(answers[3].clone()).unwrap();
    let e = sources.connections.iter().find(|c| c.project.as_deref() == Some("app")).unwrap().last.clone().unwrap().error.unwrap();
    assert_eq!(e.code, ErrorCode::Auth);
    assert_eq!(e.message, "token ••• was refused");
    // The keys are where they belong.
    assert_eq!(metrics.lock_keys().get("m1").unwrap().get("token").map(String::as_str), Some(TOKEN));
}

#[test]
fn the_files_are_the_persons_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let home = scratch("modes");
    metrics(&home).connect(fake(&[]), 1).unwrap();
    let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(home.join("metrics/connections.json")), 0o600);
    assert_eq!(mode(home.join("metrics")), 0o700);
}

#[tokio::test]
async fn connecting_again_keeps_the_key_left_out_and_checks_the_fields() {
    let home = scratch("again");
    let metrics = metrics(&home);
    metrics.connect(fake(&[("seed", "1")]), 1).unwrap();
    let again = ConnectParams { squad: "acme".into(), id: Some("m1".into()), settings: BTreeMap::from([("seed".into(), "9".into())]), ..Default::default() };
    let c = metrics.connect(again, 2).unwrap();
    assert_eq!((c.id.as_str(), c.connected_at, c.settings["seed"].as_str()), ("m1", 1, "9"));
    assert_eq!(metrics.lock_keys().get("m1").unwrap()["token"], TOKEN, "the token left out is kept");
    assert_eq!(connections(&home).len(), 1);

    let refused = |p: ConnectParams| metrics.connect(p, 3).unwrap_err().code;
    assert_eq!(refused(ConnectParams { settings: BTreeMap::new(), ..fake(&[]) }), BAD_SETTINGS, "no token");
    assert_eq!(refused(fake(&[("colour", "red")])), BAD_SETTINGS, "no such field");
    assert_eq!(refused(ConnectParams { kind: Some(SourceKind::Posthog), ..fake(&[]) }), UNKNOWN_KIND, "no adapter yet");
    assert_eq!(refused(ConnectParams { kind: None, ..fake(&[]) }), BAD_PARAMS);
    assert_eq!(refused(ConnectParams { squad: "../x".into(), ..fake(&[]) }), BAD_PARAMS);
    assert_eq!(refused(ConnectParams { id: Some("m9".into()), ..fake(&[]) }), NOT_FOUND);
    assert_eq!(metrics.call("metrics.nope", json!({}), 1).await.unwrap_err().code, -32601);
    assert_eq!(metrics.call(m::QUERY, json!({ "squad": "acme", "days": 0 }), 1).await.unwrap_err().code, BAD_PARAMS);
}

#[tokio::test]
async fn disconnecting_takes_the_key_away_and_forgets_on_request() {
    let home = scratch("disconnect");
    let metrics = metrics(&home);
    let d = day("2026-11-02");
    metrics.connect(fake(&[]), 1).unwrap();
    metrics.connect(fake(&[]), 1).unwrap();
    metrics.refresh(&RefreshParams { squad: "acme".into(), ..Default::default() }, noon(d), d).await.unwrap();
    metrics.disconnect(&DisconnectParams { squad: "acme".into(), id: "m1".into(), forget: false }).unwrap();
    assert!(metrics.lock_keys().get("m1").is_err());
    assert!(home.join("metrics/acme/m1").exists(), "its numbers stay");
    metrics.disconnect(&DisconnectParams { squad: "acme".into(), id: "m2".into(), forget: true }).unwrap();
    assert!(!home.join("metrics/acme/m2").exists());
    assert!(connections(&home).is_empty());
    let other_squad = DisconnectParams { squad: "other".into(), id: "m1".into(), forget: false };
    assert_eq!(metrics.disconnect(&other_squad).unwrap_err().code, NOT_FOUND);
}

#[test]
fn a_connection_is_fetched_once_a_day_from_seven_and_retried_after_a_failure() {
    let d = day("2026-11-02");
    let at = |h: u64| (d * 86_400) as u64 + h * 3600;
    let mut c = Connection {
        id: "m1".into(),
        squad: "acme".into(),
        project: None,
        kind: SourceKind::Fake,
        settings: BTreeMap::new(),
        keys: vec![],
        connected_at: 0,
        last: None,
        last_ok: None,
    };
    assert!(due(&c, at(3), &Utc), "never fetched: now");
    c.last = Some(Fetched { at: at(7) - 86_400, error: None });
    c.last_ok = Some(at(7) - 86_400);
    assert!(!due(&c, at(6), &Utc), "yesterday's, before seven");
    assert!(due(&c, at(7), &Utc));
    c.last_ok = Some(at(7));
    c.last = Some(Fetched { at: at(7), error: None });
    assert!(!due(&c, at(20), &Utc), "today's is kept");
    c.last_ok = Some(at(7) - 86_400);
    c.last = Some(Fetched { at: at(7), error: Some(FetchError { code: ErrorCode::Network, message: String::new(), retry_at: None }) });
    assert!(!due(&c, at(7) + 1800, &Utc), "failed half an hour ago");
    assert!(due(&c, at(8), &Utc), "an hour later");
    c.last = Some(Fetched { at: at(7), error: Some(FetchError { code: ErrorCode::RateLimited, message: String::new(), retry_at: Some(at(10)) }) });
    assert!(!due(&c, at(9), &Utc), "the service said when");
    assert!(due(&c, at(10), &Utc));
}

#[test]
fn redact_cuts_every_key() {
    let keys = BTreeMap::from([("a".to_owned(), "abcd1234".to_owned()), ("b".to_owned(), "xy".to_owned())]);
    assert_eq!(redact("got abcd1234 and abcd1234", &keys), "got ••• and •••");
    assert_eq!(redact("xy stays: too short to be a key", &keys), "xy stays: too short to be a key");
}

#[test]
fn the_tool_says_when_nothing_is_connected() {
    let home = scratch("empty");
    let said = tool::answer(&home, "acme", &json!({}), day("2026-11-02")).unwrap();
    assert!(said.starts_with("No service is connected"), "{said}");
}
