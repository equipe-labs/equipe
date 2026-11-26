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
        let v = self.get("/v1/resources").await?;
        Ok(live_from(&v, now_unix()))
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
    let mut guard = live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stale = guard.as_ref().is_none_or(|l| l.at.elapsed() > LIVE_EVERY);
    let busy = guard.as_ref().is_some_and(|l| l.busy);
    let known = guard.as_ref().is_some_and(|l| l.live);
    if stale && !busy && tokio::runtime::Handle::try_current().is_ok() {
        match guard.as_mut() {
            Some(l) => l.busy = true,
            None => *guard = Some(Live { at: std::time::Instant::now(), live: false, busy: true }),
        }
        let p = platform.clone();
        tokio::spawn(async move {
            let live = p.has_app().await.unwrap_or(false);
            *live_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Live { at: std::time::Instant::now(), live, busy: false });
        });
    }
    known
}
