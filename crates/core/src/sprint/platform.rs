//! The equipe platform as a squad's product data: the analytics and
//! experiments service a cloud team's products are hosted on. The team
//! machine holds the team's platform key in `~/.equipe-platform.env`
//! (`PLATFORM_URL=`, `PLATFORM_KEY=`, optional `PLATFORM_APP=`); the key
//! stays in the daemon and is only ever sent as a bearer token.
//!
//! Reads go to `POST /v1/metrics/read` (a metric over whole UTC days, plain or
//! split by an experiment's variants); the review's glance comes from
//! `GET /v1/metrics?days=7`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use equipe_protocol::growth::{Better, Evidence};
use equipe_protocol::sprint::{Arm, Metric, Reading, Split};
use serde_json::{json, Value};

use super::data::{Fut, ProductData};
use crate::metrics::day_string;

pub const DEFAULT_URL: &str = "https://platform.equipe.im";
const SOURCE: &str = "equipe platform";

/// A team's connection to the platform. No `Debug`: it holds the key.
#[derive(Clone)]
pub struct Platform {
    url: String,
    key: String,
    app: Option<String>,
    client: reqwest::Client,
}

fn parse_env(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                out.insert(k.to_owned(), v.to_owned());
            }
        }
    }
    out
}

/// Where the team's platform settings live: `EQUIPE_PLATFORM_ENV`, else `~/.equipe-platform.env`.
pub fn env_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("EQUIPE_PLATFORM_ENV").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".equipe-platform.env"))
}

impl Platform {
    pub fn new(url: &str, key: &str, app: Option<&str>) -> Self {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(20)).build().unwrap_or_default();
        Platform { url: url.trim_end_matches('/').to_owned(), key: key.to_owned(), app: app.filter(|a| !a.is_empty()).map(str::to_owned), client }
    }

    /// From a settings file; `None` without a key in it.
    pub fn from_file(path: &Path) -> Option<Self> {
        let vars = parse_env(&std::fs::read_to_string(path).ok()?);
        let key = vars.get("PLATFORM_KEY").filter(|k| !k.is_empty())?;
        Some(Self::new(vars.get("PLATFORM_URL").map_or(DEFAULT_URL, String::as_str), key, vars.get("PLATFORM_APP").map(String::as_str)))
    }

    /// The machine's own: a cloud team's, if its settings file holds a key.
    pub fn discover() -> Option<Self> {
        Self::from_file(&env_path()?)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, String> {
        let res = req.header("authorization", format!("Bearer {}", self.key)).send().await.map_err(|e| format!("the platform couldn't be reached ({})", e.without_url()))?;
        let status = res.status();
        let text = res.text().await.map_err(|e| format!("the platform's answer broke off ({})", e.without_url()))?;
        let value: Value = serde_json::from_str(&text).map_err(|_| format!("the platform answered {status} with something unexpected"))?;
        if !status.is_success() {
            let message = value.pointer("/error/message").and_then(Value::as_str).unwrap_or("no reason given");
            return Err(format!("the platform said no ({status}): {message}"));
        }
        Ok(value)
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        self.send(self.client.get(format!("{}{path}", self.url))).await
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.send(self.client.post(format!("{}{path}", self.url)).header("content-type", "application/json").body(body.to_string())).await
    }

    fn body(&self, metric: &Metric, from: i64, to: i64, split: Option<&Split>) -> Value {
        let mut b = json!({ "metric": metric, "from": day_string(from), "to": day_string(to) });
        if let Some(app) = &self.app {
            b["app"] = json!(app);
        }
        if let Some(s) = split {
            b["split"] = json!({ "flag": s.flag });
        }
        b
    }

    /// Whether the team has anything live: an app deployed on the platform.
    pub async fn has_app(&self) -> Result<bool, String> {
        Ok(self.resources_state().await?.0)
    }

    /// From one `/v1/resources` read: whether a product is live for the
    /// sprint loop ([`live_from`]) and the commits its apps are live from
    /// ([`live_commits_from`]).
    pub async fn resources_state(&self) -> Result<(bool, Vec<String>), String> {
        let v = self.get("/v1/resources").await?;
        Ok((live_from(&v, now_unix()), live_commits_from(&v)))
    }

    /// The `equipe.metrics/v1` snapshot over `days`.
    pub async fn snapshot(&self, days: u32) -> Result<Value, String> {
        let app = self.app.as_deref().map(|a| format!("&app={a}")).unwrap_or_default();
        self.get(&format!("/v1/metrics?days={days}{app}")).await
    }
}

impl ProductData for Platform {
    fn source(&self) -> String {
        SOURCE.into()
    }

    fn splits(&self) -> bool {
        true
    }

    fn query(&self, metric: &Metric, from: i64, to: i64) -> String {
        format!("{} from {} to {} (equipe platform analytics)", metric.label(), day_string(from), day_string(to))
    }

    fn read<'a>(&'a self, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Reading, String>> {
        Box::pin(async move {
            let v = self.post("/v1/metrics/read", &self.body(metric, from, to, None)).await?;
            let (value, sample) = (v.get("value").and_then(Value::as_f64), v.get("sample").and_then(Value::as_f64));
            match (value, sample) {
                (Some(value), Some(sample)) => Ok(Reading { value, sample }),
                _ => Err("the platform's numbers weren't what was asked".into()),
            }
        })
    }

    fn read_split<'a>(&'a self, split: &'a Split, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Vec<Arm>, String>> {
        Box::pin(async move {
            let v = self.post("/v1/metrics/read", &self.body(metric, from, to, Some(split))).await?;
            let arms = v.get("arms").and_then(Value::as_array).ok_or("the platform's numbers weren't what was asked")?;
            Ok(arms
                .iter()
                .filter_map(|a| Some(Arm { variant: a.get("variant")?.as_str()?.to_owned(), value: a.get("value")?.as_f64()?, sample: a.get("sample")?.as_f64()? }))
                .collect())
        })
    }
}

// ---- The review's glance ----

fn num(v: &Value, pointer: &str) -> Option<f64> {
    v.pointer(pointer).and_then(Value::as_f64)
}

/// A snapshot as evidence the PM can cite (visitors, page views, the
/// product's own key events, retention) and a few plain lines it can read
/// (top sources, running experiments).
pub fn glance(snap: &Value) -> (Vec<Evidence>, Vec<String>) {
    let from = snap.pointer("/period/from").and_then(Value::as_str).unwrap_or_default().to_owned();
    let to = snap.pointer("/period/to").and_then(Value::as_str).unwrap_or_default().to_owned();
    let days = snap.pointer("/period/days").and_then(Value::as_u64).unwrap_or(7);
    let query = format!("GET /v1/metrics?days={days}");
    let mk = |metric: &str, label: &str, value: f64, previous: Option<f64>, unit: &str| Evidence {
        id: String::new(),
        metric: metric.to_owned(),
        label: label.to_owned(),
        value,
        previous,
        unit: unit.to_owned(),
        from: from.clone(),
        to: to.clone(),
        query: query.clone(),
        source: SOURCE.to_owned(),
        better: Better::Up,
        measure: None,
    };
    let mut evidence = Vec::new();
    for (key, label, unit) in [("visitors", "Visitors", "people"), ("pageviews", "Page views", "page_loads")] {
        if let Some(v) = num(snap, &format!("/measures/{key}/value")) {
            let metric = if key == "visitors" { "visitors".to_owned() } else { "visits".to_owned() };
            evidence.push(mk(&metric, label, v, num(snap, &format!("/measures/{key}/previous")), unit));
        }
    }
    for (key, label) in [("retention_d1", "Came back the next day"), ("retention_d7", "Came back after 7 days")] {
        if let Some(v) = num(snap, &format!("/measures/{key}")) {
            evidence.push(mk(key, label, crate::growth::round1(v * 100.0), None, "%"));
        }
    }
    if let Some(events) = snap.get("top_events").and_then(Value::as_array) {
        for e in events.iter().filter(|e| e.get("name").and_then(Value::as_str).is_some_and(|n| !n.starts_with('$'))).take(4) {
            if let (Some(name), Some(count)) = (e.get("name").and_then(Value::as_str), e.get("count").and_then(Value::as_f64)) {
                evidence.push(mk(&format!("event:{name}"), &format!("{name} events"), count, None, "events"));
            }
        }
    }
    let mut lines = Vec::new();
    if let Some(r) = snap.get("top_referrers").and_then(Value::as_array) {
        let top: Vec<String> = r.iter().take(3).filter_map(|x| Some(format!("{} ({})", x.get("referrer")?.as_str()?, x.get("visitors")?.as_u64()?))).collect();
        if !top.is_empty() {
            lines.push(format!("Top sources (visitors): {}.", top.join(", ")));
        }
    }
    if let Some(x) = snap.get("experiments").and_then(Value::as_array) {
        for e in x.iter().take(4) {
            let key = e.get("key").and_then(Value::as_str).unwrap_or("?");
            let verdict = e.get("verdict").and_then(Value::as_str).unwrap_or("no result yet").replace('_', " ");
            lines.push(format!("Running experiment `{key}`: {verdict}."));
        }
    }
    (evidence, lines)
}

// ---- "Product live", cached ----

struct Live {
    at: std::time::Instant,
    live: bool,
    busy: bool,
    /// The commits the team's apps are live from; `None` until read once.
    commits: Option<Vec<String>>,
}

fn live_cache() -> &'static Mutex<Option<Live>> {
    static C: OnceLock<Mutex<Option<Live>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

const LIVE_EVERY: Duration = Duration::from_secs(5 * 60);

/// Whether `/v1/resources` says the team has a product live for the sprint
/// loop: an app, and the first free version's quiet day over (`quiet_until`,
/// unix seconds: the day after a free team's first version went live, nothing
/// starts by itself on the person's free credit).
pub fn live_from(v: &Value, now: u64) -> bool {
    let app = v.get("resources").and_then(Value::as_array).is_some_and(|r| r.iter().any(|x| x.get("kind").and_then(Value::as_str) == Some("app")));
    let quiet = v.get("quiet_until").and_then(Value::as_u64).is_some_and(|until| until > now);
    app && !quiet
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether the platform reports an app live for this machine's team. Answers
/// from a cache and refreshes it in the background at most every few minutes,
/// so the loop's tick never waits on the network.
pub fn live_cached(platform: &Platform) -> bool {
    refresh_cached(platform, LIVE_EVERY);
    live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().is_some_and(|l| l.live)
}

/// The commits this machine's team's apps are live from (`live_commit` of
/// each app in `/v1/resources`), from the same cache, refreshed in the
/// background at most every [`COMMITS_EVERY`]. `None` until the platform
/// answered once: nothing is known, so nothing is held back.
pub fn live_commits_cached(platform: &Platform) -> Option<Vec<String>> {
    refresh_cached(platform, COMMITS_EVERY);
    live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|l| l.commits.clone())
}

/// A deploy is seen this soon: the person is told "live" from it.
const COMMITS_EVERY: Duration = Duration::from_secs(60);

fn refresh_cached(platform: &Platform, every: Duration) {
    let mut guard = live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stale = guard.as_ref().is_none_or(|l| l.at.elapsed() > every);
    let busy = guard.as_ref().is_some_and(|l| l.busy);
    if stale && !busy && tokio::runtime::Handle::try_current().is_ok() {
        match guard.as_mut() {
            Some(l) => l.busy = true,
            None => *guard = Some(Live { at: std::time::Instant::now(), live: false, busy: true, commits: None }),
        }
        let p = platform.clone();
        tokio::spawn(async move {
            let got = p.resources_state().await;
            let mut guard = live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let kept = guard.as_ref().and_then(|l| l.commits.clone());
            *guard = Some(match got {
                Ok((live, commits)) => Live { at: std::time::Instant::now(), live, busy: false, commits: Some(commits) },
                // Unreachable: keep what was known; try again next time.
                Err(_) => Live { at: std::time::Instant::now(), live: false, busy: false, commits: kept },
            });
        });
    }
}

// ── what the person sees on the platform, for the PM's reply guard ──

/// What the platform knows that the PM may speak of: what waits on the person
/// on the platform's pages (a goal to confirm, this week's plan, a bigger idea
/// to say yes to) and whether the team's credits pause its work now. The PM's
/// replies are checked against it before the person reads them
/// (`runtime::reply_guard`): at o1path the PM said "nothing is waiting on
/// you" right after it proposed a weekly plan that waited on them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudFacts {
    /// Each thing waiting on the person, in a few plain English words.
    pub waiting: Vec<String>,
    /// Whether the team's credits pause its work (`/v1/credits`); `None`: not known.
    pub paused: Option<bool>,
}

/// The apps in a `/v1/resources` answer, by name (archived ones left out).
pub fn apps_from(v: &Value) -> Vec<String> {
    v.get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|x| x.get("kind").and_then(Value::as_str) == Some("app") && x.get("app_archived").and_then(Value::as_bool) != Some(true))
        .filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// What of one app's operating loop (`/v1/oploop?app=`) waits on the person.
pub fn waiting_from(app: &str, v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let live = v.get("live").and_then(Value::as_bool).unwrap_or(false);
    if live && v.pointer("/goal/status").and_then(Value::as_str).is_some_and(|s| s != "confirmed") {
        out.push(format!("confirming the goal for {app} (Goal page)"));
    }
    if v.pointer("/plan/status").and_then(Value::as_str) == Some("proposed") {
        let open = v.pointer("/plan/bets").and_then(Value::as_array).map_or(0, |b| b.iter().filter(|x| x.get("state").and_then(Value::as_str) == Some("proposed")).count());
        if open > 0 {
            out.push(format!("this week's plan for {app}: {open} idea(s) to say go to (Goal page)"));
        }
    }
    let asked = v.get("signals").and_then(Value::as_array).map_or(0, |s| s.iter().filter(|x| x.get("state").and_then(Value::as_str) == Some("asked") && x.pointer("/proposal/worth").and_then(Value::as_bool) == Some(true)).count());
    if asked > 0 {
        out.push(format!("{asked} bigger idea(s) for {app} to say yes or no to"));
    }
    out
}

impl Platform {
    /// [`CloudFacts`], read now: `/v1/resources`, each app's `/v1/oploop`, `/v1/credits`.
    pub async fn cloud_facts(&self) -> Result<CloudFacts, String> {
        let apps = apps_from(&self.get("/v1/resources").await?);
        let mut waiting = Vec::new();
        for app in apps.iter().take(8) {
            if let Ok(v) = self.get(&format!("/v1/oploop?app={app}")).await {
                waiting.extend(waiting_from(app, &v));
            }
        }
        let paused = self.get("/v1/credits").await.ok().and_then(|v| v.get("paused").and_then(Value::as_bool));
        Ok(CloudFacts { waiting, paused })
    }
}

struct FactsCache {
    at: std::time::Instant,
    busy: bool,
    facts: Option<CloudFacts>,
}

fn facts_cache() -> &'static Mutex<Option<FactsCache>> {
    static C: OnceLock<Mutex<Option<FactsCache>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// How fresh the platform's facts are kept for the reply guard.
const FACTS_EVERY: Duration = Duration::from_secs(90);

/// The platform's [`CloudFacts`] as last read, refreshed in the background at
/// most every [`FACTS_EVERY`] so a reply is never held on the network. `None`
/// until the platform answered once: nothing is known, so nothing is checked.
pub fn cloud_facts_cached(platform: &Platform) -> Option<CloudFacts> {
    let mut guard = facts_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stale = guard.as_ref().is_none_or(|c| c.at.elapsed() > FACTS_EVERY);
    let busy = guard.as_ref().is_some_and(|c| c.busy);
    if stale && !busy && tokio::runtime::Handle::try_current().is_ok() {
        match guard.as_mut() {
            Some(c) => c.busy = true,
            None => *guard = Some(FactsCache { at: std::time::Instant::now(), busy: true, facts: None }),
        }
        let p = platform.clone();
        tokio::spawn(async move {
            let got = p.cloud_facts().await;
            let mut guard = facts_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let kept = guard.as_ref().and_then(|c| c.facts.clone());
            *guard = Some(FactsCache { at: std::time::Instant::now(), busy: false, facts: got.ok().or(kept) });
        });
    }
    guard.as_ref().and_then(|c| c.facts.clone())
}

/// The commits a team's apps are live from, as `/v1/resources` says
/// (each app's `live_commit.commit`, set by the platform on every live
/// deploy that named its commit).
pub fn live_commits_from(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|x| x.get("kind").and_then(Value::as_str) == Some("app"))
        .filter_map(|x| x.get("live_commit")?.get("commit")?.as_str())
        .filter(|c| c.len() >= 7 && c.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod facts_tests {
    use super::*;

    #[test]
    fn what_waits_on_the_person_on_the_platforms_pages() {
        let resources = json!({ "resources": [{ "kind": "app", "name": "o1path" }, { "kind": "app", "name": "old", "app_archived": true }, { "kind": "database", "name": "main" }] });
        assert_eq!(apps_from(&resources), ["o1path"]);
        let loop_ = json!({ "live": true, "goal": { "status": "proposed" }, "plan": { "status": "proposed", "bets": [{ "state": "proposed" }, { "state": "proposed" }, { "state": "skipped" }] }, "signals": [{ "state": "asked", "proposal": { "worth": true } }] });
        let waiting = waiting_from("o1path", &loop_);
        assert_eq!(waiting.len(), 3, "{waiting:?}");
        assert!(waiting[1].contains("2 idea"), "{waiting:?}");
        let settled = json!({ "live": true, "goal": { "status": "confirmed" }, "plan": { "status": "approved", "bets": [] }, "signals": [] });
        assert!(waiting_from("o1path", &settled).is_empty());
        let not_live = json!({ "live": false, "goal": { "status": "proposed" } });
        assert!(waiting_from("o1path", &not_live).is_empty(), "no Goal page before the product is live");
    }
}
