//! Where a source's numbers come from: one adapter per service, one file each
//! (`adapters/<kind>.rs`, docs/METRICS.md). An adapter says what it is set up
//! with and what it reports ([`SourceSpec`]), and fetches days of points; the
//! rest — keys, snapshots, periods, comparisons — is `metrics`' own.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use crate::protocol::metrics::{ErrorCode, FetchError, Point, SourceKind, SourceSpec};

pub mod app_store;
pub mod fake;
pub mod github;
pub mod posthog;
#[cfg(test)]
pub mod stand_in;

/// What a fetch is given.
pub struct Fetch<'a> {
    /// The connection's fields that aren't secret.
    pub settings: &'a BTreeMap<String, String>,
    /// Its secret fields, from the Keychain. An adapter sends them to its
    /// service and nowhere else: not in a point, an error or a log.
    pub keys: &'a BTreeMap<String, String>,
    /// Today, the daemon machine's local day, `YYYY-MM-DD`.
    pub today: &'a str,
    /// How many days back to ask for (from today): more on the first fetch.
    pub days: u32,
    pub client: &'a reqwest::Client,
}

pub type Fetching<'a> = Pin<Box<dyn Future<Output = Result<Vec<Point>, FetchError>> + Send + 'a>>;

/// A service numbers come from.
pub trait Adapter: Send + Sync {
    fn spec(&self) -> SourceSpec;
    /// The points of the days asked for — a day the service doesn't know yet
    /// (App Store's today) left out, not zero.
    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a>;
}

/// The adapter for `kind`, when there is one yet.
pub fn adapter(kind: SourceKind) -> Option<Box<dyn Adapter>> {
    match kind {
        SourceKind::Fake => Some(Box::new(fake::Fake)),
        SourceKind::Posthog => Some(Box::new(posthog::Posthog)),
        SourceKind::Github => Some(Box::new(github::Github)),
        SourceKind::AppStore => Some(Box::new(app_store::AppStore)),
    }
}

/// The kinds a screen may offer: every one with an adapter. The fake one
/// only when `EQUIPE_METRICS_FAKE` is set, or in tests.
pub fn offered(with_fake: bool) -> Vec<SourceSpec> {
    [SourceKind::Posthog, SourceKind::Github, SourceKind::AppStore, SourceKind::Fake]
        .into_iter()
        .filter(|k| with_fake || *k != SourceKind::Fake)
        .filter_map(adapter)
        .map(|a| a.spec())
        .collect()
}

// ---- What the adapters share: a service's answer, read the same way ----

/// How long one request may take (the whole fetch has 90 s).
pub const REQUEST_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A service's answer: its status, `Retry-After` (seconds) and body.
pub struct Answer {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// Send `request`, and read its whole answer — or why it couldn't be had.
pub async fn send(service: &str, request: reqwest::RequestBuilder) -> Result<Answer, FetchError> {
    let unreachable = |e: reqwest::Error| {
        let message = if e.is_timeout() { format!("{service} didn't answer in {}s", REQUEST_WAIT.as_secs()) } else { format!("{service} couldn't be reached") };
        error(ErrorCode::Network, message)
    };
    let resp = request.timeout(REQUEST_WAIT).send().await.map_err(unreachable)?;
    let status = resp.status().as_u16();
    let retry_after = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    let body = resp.bytes().await.map_err(unreachable)?.to_vec();
    Ok(Answer { status, retry_after, body })
}

pub fn error(code: ErrorCode, message: impl Into<String>) -> FetchError {
    FetchError { code, message: message.into(), retry_at: None }
}

/// A refusal in the contract's codes: 401/403 `auth`, 429 `rate_limited`
/// (again at `now + Retry-After`, or in an hour), 5xx `network`, the rest
/// `other` — with the service's own words, clipped. `said` is what it said.
pub fn refused(service: &str, answer: &Answer, said: Option<String>, now: u64) -> FetchError {
    let said = said.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect::<String>()).filter(|s| !s.is_empty());
    let message = match said {
        Some(said) => format!("{service} said {}: {said}", answer.status),
        None => format!("{service} said {}", answer.status),
    };
    let code = match answer.status {
        401 | 403 => ErrorCode::Auth,
        429 => ErrorCode::RateLimited,
        500..=599 => ErrorCode::Network,
        _ => ErrorCode::Other,
    };
    let retry_at = (code == ErrorCode::RateLimited).then(|| now + answer.retry_after.unwrap_or(3600));
    FetchError { code, message, retry_at }
}

/// The answer as JSON, or `bad_response`.
pub fn json(service: &str, answer: &Answer) -> Result<serde_json::Value, FetchError> {
    serde_json::from_slice(&answer.body).map_err(|_| error(ErrorCode::BadResponse, format!("{service}'s answer wasn't JSON")))
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// One real fetch against the person's own accounts, run by hand:
/// `EQUIPE_METRICS_LIVE=github cargo test -p equipe-core --lib live_fetch -- --ignored --nocapture`,
/// with the connection's fields as `EQUIPE_METRICS_<FIELD>` in the environment
/// (sourced from the keys' file, never typed). It prints each metric's days
/// and totals — numbers only, never a key.
#[cfg(test)]
#[tokio::test]
#[ignore = "calls a real service with real keys"]
async fn live_fetch() {
    let kind = match std::env::var("EQUIPE_METRICS_LIVE").as_deref() {
        Ok("posthog") => SourceKind::Posthog,
        Ok("github") => SourceKind::Github,
        Ok("app_store") => SourceKind::AppStore,
        other => panic!("EQUIPE_METRICS_LIVE is posthog, github or app_store, not {other:?}"),
    };
    let a = adapter(kind).unwrap();
    let spec = a.spec();
    let (mut settings, mut keys) = (BTreeMap::new(), BTreeMap::new());
    for f in &spec.fields {
        if let Ok(v) = std::env::var(format!("EQUIPE_METRICS_{}", f.key.to_uppercase())) {
            if f.secret { keys.insert(f.key.clone(), v) } else { settings.insert(f.key.clone(), v) };
        }
    }
    let today = std::env::var("EQUIPE_METRICS_TODAY").expect("EQUIPE_METRICS_TODAY, YYYY-MM-DD");
    let days = std::env::var("EQUIPE_METRICS_DAYS").ok().and_then(|d| d.parse().ok()).unwrap_or(14);
    let client = reqwest::Client::new();
    let started = std::time::Instant::now();
    let got = a.fetch(Fetch { settings: &settings, keys: &keys, today: &today, days, client: &client }).await;
    let got = got.map_err(|e| FetchError { message: crate::metrics::redact(&e.message, &keys), ..e });
    println!("{} · {days} days to {today} · {:.1}s", spec.label, started.elapsed().as_secs_f64());
    match got {
        Err(e) => println!("failed: {:?}: {}", e.code, e.message),
        Ok(points) => {
            for m in &spec.metrics {
                let mut of: Vec<&Point> = points.iter().filter(|p| p.metric == m.name).collect();
                of.sort_by(|x, y| x.day.cmp(&y.day));
                let days: Vec<String> = of.iter().map(|p| format!("{} {}", &p.day[5..], p.value)).collect();
                println!("{} ({:?}): {}", m.name, m.shape, days.join(", "));
                let mut split: BTreeMap<String, f64> = BTreeMap::new();
                for p in &of {
                    for (l, v) in p.breakdown.iter().flatten() {
                        *split.entry(l.clone()).or_insert(0.0) += v;
                    }
                }
                let mut split: Vec<_> = split.into_iter().collect();
                split.sort_by(|x, y| y.1.total_cmp(&x.1));
                if !split.is_empty() {
                    println!("  by label: {}", split.iter().take(8).map(|(l, v)| format!("{l} {v}")).collect::<Vec<_>>().join(", "));
                }
            }
            let text = format!("{points:?}");
            assert!(keys.values().all(|k| k.len() < 4 || !text.contains(k.as_str())), "a key is in the points");
        }
    }
}
