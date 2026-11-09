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

#[cfg(windows)]
#[test]
fn credential_manager_keys_survive_a_new_store() {
    let home = scratch("credential-manager-restart");
    let keys = BTreeMap::from([("token".to_owned(), "restart-secret".to_owned())]);
    let mut first = Keychain::new(&home);
    first.set("m42", &keys).unwrap();
    let second = Keychain::new(&home);
    assert_eq!(second.get("m42").unwrap(), keys);
    first.delete("m42").unwrap();
    assert!(second.get("m42").is_err());
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
    let metrics = Arc::new(metrics(&home));
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

#[cfg(unix)]
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
    let metrics = Arc::new(metrics(&home));
    metrics.connect(fake(&[("seed", "1")]), 1).unwrap();
    let again = ConnectParams { squad: "acme".into(), id: Some("m1".into()), settings: BTreeMap::from([("seed".into(), "9".into())]), ..Default::default() };
    let c = metrics.connect(again, 2).unwrap();
    assert_eq!((c.id.as_str(), c.connected_at, c.settings["seed"].as_str()), ("m1", 1, "9"));
    assert_eq!(metrics.lock_keys().get("m1").unwrap()["token"], TOKEN, "the token left out is kept");
    assert_eq!(connections(&home).len(), 1);
    let blank = ConnectParams { squad: "acme".into(), id: Some("m1".into()), settings: BTreeMap::from([("seed".into(), "".into())]), ..Default::default() };
    let c = metrics.connect(blank, 2).unwrap();
    assert!(!c.settings.contains_key("seed"), "an empty optional setting clears it");
    assert_eq!(metrics.lock_keys().get("m1").unwrap()["token"], TOKEN);
    metrics.connect(ConnectParams { settings: BTreeMap::from([("seed".into(), "9".into())]), ..ConnectParams { squad: "acme".into(), id: Some("m1".into()), ..Default::default() } }, 2).unwrap();

    let refused = |p: ConnectParams| metrics.connect(p, 3).unwrap_err().code;
    assert_eq!(refused(ConnectParams { settings: BTreeMap::new(), ..fake(&[]) }), BAD_SETTINGS, "no token");
    assert_eq!(refused(fake(&[("colour", "red")])), BAD_SETTINGS, "no such field");
    assert_eq!(refused(ConnectParams { kind: Some(SourceKind::Posthog), ..fake(&[]) }), BAD_SETTINGS, "a token isn't PostHog's");
    let github = |extra: (&str, &str)| ConnectParams {
        kind: Some(SourceKind::Github),
        settings: BTreeMap::from([("token".into(), TOKEN.into()), ("repo".into(), "a/b".into()), (extra.0.into(), extra.1.into())]),
        ..fake(&[])
    };
    assert_eq!(refused(github(("api_url", "http://127.0.0.1:9"))), BAD_SETTINGS, "where the key goes isn't a setting");
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

#[test]
fn a_total_split_by_label_shows_its_last_days_split() {
    let def = MetricDef { name: "referrers".into(), label: "Referrers".into(), unit: "views".into(), shape: Shape::Total };
    let point = |d: &str, split: &[(&str, f64)]| Point {
        metric: "referrers".into(),
        day: d.into(),
        value: split.iter().map(|(_, v)| v).sum(),
        breakdown: Some(split.iter().map(|(l, v)| ((*l).into(), *v)).collect()),
    };
    let points: BTreeMap<(String, i64), Point> = [point("2026-10-30", &[("github.com", 10.0)]), point("2026-11-01", &[("github.com", 12.0), ("t.co", 3.0)])]
        .into_iter()
        .map(|p| ((p.metric.clone(), day(&p.day)), p))
        .collect();
    let (value, _, split, as_of, _) = over(&def, &points, day("2026-10-27"), day("2026-11-02"), Bucket::Day);
    assert_eq!(value, Some(15.0), "the last day's, not a sum of overlapping windows");
    assert_eq!(split, BTreeMap::from([("github.com".into(), 12.0), ("t.co".into(), 3.0)]));
    assert_eq!(as_of, Some(day("2026-11-01")));
}

#[test]
fn a_rolling_metric_is_its_last_day_beside_seven_days_before_in_twelve_weeks() {
    let home = scratch("rolling");
    let today = day("2026-11-02");
    let c = Connection {
        id: "m1".into(),
        squad: "acme".into(),
        project: None,
        kind: SourceKind::Posthog,
        settings: BTreeMap::new(),
        keys: vec!["api_key".into()],
        connected_at: noon(today - 100),
        last: None,
        last_ok: Some(noon(today)),
    };
    keep_connections(&home, &[c]).unwrap();
    // Squads in the 7 days up to each day: known from 30 days back, today's not fetched yet.
    let points: Vec<Point> = (today - 30..today).map(|n| Point { metric: NORTH_STAR.into(), day: day_string(n), value: (n - today + 40) as f64, breakdown: None }).collect();
    let snapshot = Snapshot { schema: SCHEMA, connection: "m1".into(), kind: SourceKind::Posthog, fetched_at: noon(today), points };
    private_write(&snapshots_dir(&home, "acme", "m1").join("2026-11-02.json"), &serde_json::to_string(&snapshot).unwrap()).unwrap();

    for (days, bucket) in [(7, Bucket::Day), (30, Bucket::Week)] {
        let q = query(&home, &QueryParams { squad: "acme".into(), days: Some(days), bucket, metrics: vec![NORTH_STAR.into()], ..Default::default() }, today).unwrap();
        let v = &q.values[0];
        assert_eq!((v.shape, v.unit.as_str()), (Shape::Rolling, "squads"));
        assert_eq!(v.as_of.as_deref(), Some("2026-11-01"));
        assert_eq!(v.value, Some(39.0), "the last day's, not a sum");
        assert_eq!(v.previous, Some(32.0), "7 days before it, whatever the period");
        assert_eq!((v.change, v.change_pct), (Some(7.0), Some(21.9)));
        assert_eq!(v.series.len(), 12, "{days} days");
        assert_eq!(v.series.last().unwrap().day, "2026-11-01");
        assert_eq!(v.series[0].day, day_string(today - 1 - 77));
        let values: Vec<Option<f64>> = v.series.iter().map(|s| s.value).collect();
        assert_eq!(values[..7], [None; 7], "before it was known");
        assert_eq!(values[7..], [Some(11.0), Some(18.0), Some(25.0), Some(32.0), Some(39.0)]);
    }

    let s = summary(&home, &SummaryParams { squad: "acme".into(), project: None }, today).unwrap();
    assert_eq!(s.tiles[0].metric, NORTH_STAR, "the phone's first tile");
    assert_eq!((s.tiles[0].value, s.tiles[0].previous), (Some(39.0), Some(32.0)));

    // A source that never reported it (no work_accepted there): not answered at all.
    let none = Snapshot { points: vec![], ..serde_json::from_str(&std::fs::read_to_string(snapshots_dir(&home, "acme", "m1").join("2026-11-02.json")).unwrap()).unwrap() };
    private_write(&snapshots_dir(&home, "acme", "m1").join("2026-11-02.json"), &serde_json::to_string(&none).unwrap()).unwrap();
    let q = query(&home, &QueryParams { squad: "acme".into(), ..Default::default() }, today).unwrap();
    assert!(q.values.iter().all(|v| v.metric != NORTH_STAR));
    assert!(q.values.iter().any(|v| v.metric == "visits"));
}

/// Keys whose reads never come back until the test lets them go: the
/// Keychain asking whether a new build of equipe may read them, with no one
/// there to answer (the daemon of 2026-11-04 12:12).
struct Unanswered {
    kept: InMemory,
    held: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Keys for Unanswered {
    fn set(&mut self, id: &str, keys: &BTreeMap<String, String>) -> Result<(), String> {
        self.kept.set(id, keys)
    }

    fn get(&self, _: &str) -> Result<BTreeMap<String, String>, String> {
        let _ = self.held.lock().unwrap().recv();
        Err("let go".into())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.kept.delete(id)
    }
}

/// The daily fetch woke on the one worker parked on the runtime's timers and
/// sockets, and waited there on the Keychain for good: no hello, no status,
/// no timer was answered again. One worker here makes it the only one.
#[test]
fn a_keychain_that_never_answers_fails_the_fetch_and_holds_up_nothing() {
    let home = scratch("unanswered");
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (told, heard) = std::sync::mpsc::channel::<String>();
    let d = day("2026-11-04");
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        rt.block_on(async move {
            let mut metrics = Metrics::new(&home, Box::new(Unanswered { kept: InMemory::default(), held: std::sync::Mutex::new(held) }));
            metrics.keychain_wait = Duration::from_millis(300);
            let metrics = Arc::new(metrics);
            metrics.connect(fake(&[]), noon(d)).unwrap_or_else(|e| panic!("{}", e.message));
            let fetch = tokio::spawn({
                let metrics = metrics.clone();
                async move { metrics.refresh_one("m1", noon(d), d).await }
            });
            // Meanwhile the runtime keeps its time: a screen's hello is answered.
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = told.send("answered meanwhile".into());
            fetch.await.unwrap();
            let last = connections(&home).into_iter().find(|c| c.id == "m1").and_then(|c| c.last).and_then(|l| l.error);
            let _ = told.send(format!("{:?}", last.map(|e| e.code)));
            // Asked again while the first ask still waits: no other piles up behind it.
            let again = ConnectParams { id: Some("m1".into()), ..fake(&[]) };
            let again = metrics.call(m::CONNECT, serde_json::to_value(again).unwrap(), noon(d)).await;
            let _ = told.send(format!("{:?}", again.err().map(|e| e.code)));
        });
        rt.shutdown_background();
    });
    let wait = Duration::from_secs(10);
    let froze = "the runtime froze while the Keychain didn't answer";
    assert_eq!(heard.recv_timeout(wait).expect(froze), "answered meanwhile");
    assert_eq!(heard.recv_timeout(wait).expect(froze), "Some(Keychain)", "the fetch fails, and says why");
    assert_eq!(heard.recv_timeout(wait).expect(froze), format!("{:?}", Some(KEYCHAIN)), "connecting again waits its turn, then says why");
    drop(release);
}
