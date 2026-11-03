//! A product's numbers in equipe (docs/METRICS.md): the person connects the
//! services their product reports to — PostHog, GitHub, App Store Connect —
//! and the daemon on their Mac asks each one itself, once a day and on a
//! `metrics.refresh`, and keeps what it hears as daily snapshots. Screens and
//! members read the numbers; the keys stay in that Mac's Keychain and are in
//! none of these messages, except the one `metrics.connect` that hands them in.
//!
//! Days are the daemon machine's local calendar days, `YYYY-MM-DD`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The methods (JSON-RPC requests over the daemon's socket).
pub const SOURCES: &str = "metrics.sources";
pub const CONNECT: &str = "metrics.connect";
pub const DISCONNECT: &str = "metrics.disconnect";
pub const QUERY: &str = "metrics.query";
pub const REFRESH: &str = "metrics.refresh";
pub const SUMMARY: &str = "metrics.summary";

/// What snapshots on disk are written as; read back from this one down.
pub const SCHEMA: u32 = 1;

/// The services numbers can come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Made-up numbers, for tests and for building screens.
    Fake,
    Posthog,
    Github,
    AppStore,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::Posthog => "posthog",
            Self::Github => "github",
            Self::AppStore => "app_store",
        }
    }
}

/// One thing a connection is set up with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Its key in `settings`: `project_id`, `api_key`.
    pub key: String,
    /// How a screen names it, in English (screens translate by `key`).
    pub label: String,
    /// A key or a token: typed hidden, kept in the Keychain only, never sent back.
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required: bool,
    /// Where the person finds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// How a metric adds up over a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A count per day; a period is the sum of its days (visits, downloads).
    Count,
    /// A running total; a period is its last day's value (stars).
    Total,
    /// A count per day split by a label; a period sums each label (languages).
    Breakdown,
    /// A distinct count over the 7 days up to each day (the North Star's
    /// squads), which days can't be added into: a period is its last day's
    /// value, beside the value 7 days before it, and its series is 12 such
    /// values 7 days apart.
    Rolling,
}

/// A metric a source reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricDef {
    /// Its name, unique within the source: `visits`, `stars`.
    pub name: String,
    pub label: String,
    /// What one is: `page_loads`, `stars`, `downloads`, `clicks`, `views`, `clones`.
    pub unit: String,
    pub shape: Shape,
}

/// A kind of source, as a screen offers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpec {
    pub kind: SourceKind,
    pub label: String,
    pub fields: Vec<Field>,
    pub metrics: Vec<MetricDef>,
    /// How many days back the service itself keeps (GitHub traffic: 14);
    /// `None`: as far back as asked. Older days are only in equipe's snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeps_days: Option<u32>,
}

/// Why numbers couldn't be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The key was refused or has expired: the person puts in a new one.
    Auth,
    /// The service said not now; tried again at `retry_at`.
    RateLimited,
    /// The service couldn't be reached.
    Network,
    /// The service answered something equipe couldn't read.
    BadResponse,
    /// The key couldn't be read from the Keychain.
    Keychain,
    Other,
}

/// A fetch that failed. Its message never holds a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchError {
    pub code: ErrorCode,
    pub message: String,
    /// Unix seconds of the next try, when the service said when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
}

/// The latest fetch of a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    /// Unix seconds.
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<FetchError>,
}

/// A service connected for a squad's project: its settings without its keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// `m1`, `m2`… — unique on the daemon's machine.
    pub id: String,
    pub squad: String,
    /// The project it reports on; `None`: the squad as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub kind: SourceKind,
    /// The fields that aren't secret, as set.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// The secret fields kept in the Keychain — their names, never their values.
    #[serde(default)]
    pub keys: Vec<String>,
    /// Unix seconds.
    pub connected_at: u64,
    /// The latest fetch, and the latest that worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Fetched>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ok: Option<u64>,
}

/// One day's value of a metric, as a source reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub metric: String,
    /// `YYYY-MM-DD`.
    pub day: String,
    pub value: f64,
    /// For a breakdown: each label's value (they add up to `value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<BTreeMap<String, f64>>,
}

/// What one fetch of one connection brought, as kept on disk:
/// `metrics/<squad>/<connection>/<YYYY-MM-DD>.json`, the day it was fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32,
    pub connection: String,
    pub kind: SourceKind,
    /// Unix seconds.
    pub fetched_at: u64,
    pub points: Vec<Point>,
}

// ---- Requests and answers ----

/// `metrics.sources`: the kinds there are, and the squad's connections.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourcesParams {
    pub squad: String,
    /// Only this project's connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    pub kinds: Vec<SourceSpec>,
    pub connections: Vec<Connection>,
}

/// `metrics.connect` → [`Connection`]: a new connection, or (with `id`) new
/// settings or keys for one — a secret field left out keeps its key.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConnectParams {
    pub squad: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub kind: Option<SourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Every field, secret ones included.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
}

/// `metrics.disconnect` → [`Disconnected`]: its keys leave the Keychain; its
/// snapshots stay unless `forget`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DisconnectParams {
    pub squad: String,
    pub id: String,
    #[serde(default)]
    pub forget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disconnected {
    pub id: String,
}

/// `metrics.refresh` → [`Sources`]: fetch now, and answer once done.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RefreshParams {
    pub squad: String,
    /// Only this connection; `None`: all of the squad's (of `project`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// How a series is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    #[default]
    Day,
    /// Monday to Sunday.
    Week,
}

/// `metrics.query` → [`QueryResult`]: a period of `days` ending `to`, beside
/// the same length just before it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QueryParams {
    pub squad: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// 1 to 366; default 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// The period's last day; default today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default)]
    pub bucket: Bucket,
    /// Only these connections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<String>,
    /// Only these metrics, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<String>,
}

/// One bucket of a series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The bucket's first day.
    pub day: String,
    /// `None`: no snapshot holds that day.
    pub value: Option<f64>,
}

/// One label of a breakdown, over the period and the one before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Share {
    pub label: String,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<f64>,
}

/// A metric over the period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricValue {
    pub connection: String,
    pub kind: SourceKind,
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub shape: Shape,
    /// `None`: no day of the period has a value.
    pub value: Option<f64>,
    pub previous: Option<f64>,
    /// `value - previous`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<f64>,
    /// Percent; `None` when there is no previous value or it is 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// The latest day with a value (App Store reports a day late).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    /// Days of the period with a value, out of `QueryResult::days`.
    pub days_known: u32,
    pub series: Vec<Sample>,
    pub previous_series: Vec<Sample>,
    /// Breakdowns: each label, biggest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakdown: Vec<Share>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub from: String,
    pub to: String,
    pub previous_from: String,
    pub previous_to: String,
    pub days: u32,
    pub bucket: Bucket,
    pub values: Vec<MetricValue>,
    /// The connections asked about, with their latest fetch.
    pub connections: Vec<Connection>,
}

/// `metrics.summary` → [`Summary`]: the phone's card.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SummaryParams {
    pub squad: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// The phone's card: this week beside the last, four numbers at most.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub squad: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub from: String,
    pub to: String,
    pub tiles: Vec<Tile>,
    /// Unix seconds of the latest fetch that worked, of any connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub connection: String,
    pub kind: SourceKind,
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub value: Option<f64>,
    pub previous: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    /// The connection's latest fetch failed: the value is the last one kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<FetchError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_plain_words_on_the_wire() {
        assert_eq!(serde_json::to_string(&SourceKind::AppStore).unwrap(), "\"app_store\"");
        for kind in [SourceKind::Fake, SourceKind::Posthog, SourceKind::Github, SourceKind::AppStore] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{}\"", kind.as_str()));
        }
        assert_eq!(serde_json::to_string(&ErrorCode::RateLimited).unwrap(), "\"rate_limited\"");
    }
}
