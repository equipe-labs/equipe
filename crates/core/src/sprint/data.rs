//! Where a squad's product numbers come from, for the sprint loop: one
//! interface ([`ProductData`]) over whatever the product's analytics and
//! feature flags live in. PostHog ([`PostHog`]) reads through the squad's
//! connection with read-only HogQL (`crate::metrics::hogql`), its key kept in
//! the daemon. Another platform — a Cloudflare-based analytics and flags
//! service, say — is another implementation: it reads a [`Metric`] over whole
//! days, and splits it by a flag's variants if it has flags. A host that
//! embeds the daemon (or a test) puts its own in with [`register`].
//!
//! [`judge`] reads a hypothesis' numbers through any of them and judges them
//! by its rule (`super::judge`, `super::judge_split`).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use equipe_protocol::growth::Measure;
use equipe_protocol::metrics::HogqlParams;
use equipe_protocol::sprint::{Arm, Experiment, Judged, Metric, Reading, Split, Verdict};

use crate::growth;
use crate::metrics::day_string;

/// A boxed future, so the interface can be a trait object.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A product's numbers, read over whole days (day numbers since 1970-01-01, UTC).
pub trait ProductData: Send + Sync {
    /// Where the numbers come from, as evidence names it: `posthog m1`.
    fn source(&self) -> String;
    /// Whether it splits a metric by a feature flag's variants (A/B).
    fn splits(&self) -> bool;
    /// What it would send to read `metric` over `from..=to`, for the evidence.
    fn query(&self, metric: &Metric, from: i64, to: i64) -> String;
    /// `metric` over `from..=to`: its value and how many it counted.
    fn read<'a>(&'a self, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Reading, String>>;
    /// `metric` over `from..=to` for each variant of `split`'s flag.
    fn read_split<'a>(&'a self, split: &'a Split, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Vec<Arm>, String>>;
}

// ---- Registered in-process ----

type Registry = Mutex<HashMap<String, Arc<dyn ProductData>>>;

fn registry() -> &'static Registry {
    static ALL: OnceLock<Registry> = OnceLock::new();
    ALL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read `squad`'s numbers through `data` from now on, before any connection.
pub fn register(squad: &str, data: Arc<dyn ProductData>) {
    registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(squad.to_owned(), data);
}

pub fn unregister(squad: &str) {
    registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(squad);
}

pub fn registered(squad: &str) -> Option<Arc<dyn ProductData>> {
    registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(squad).cloned()
}

// ---- PostHog ----

/// A squad's PostHog connection, its traffic left out as its projects
/// declare (`.equipe/growth.json`'s `exclude`, equipe's own browsers).
pub struct PostHog {
    pub home: PathBuf,
    pub squad: String,
    pub connection: String,
    /// The `AND NOT (…)` every query ends its `WHERE` with (`growth::exclusion`).
    pub exclude: String,
}

/// ` AND <condition>` for a metric's filter; empty without one.
fn and_filter(filter: &str) -> String {
    if filter.is_empty() { String::new() } else { growth::condition(filter).map(|c| format!(" AND {c}")).unwrap_or_default() }
}

/// The HogQL that reads `metric` over `from..=to`.
pub fn metric_query(metric: &Metric, from: i64, to: i64, exclude: &str) -> String {
    match metric {
        Metric::Event { event, filter } => growth::event_query(event, from, to, &format!("{exclude}{}", and_filter(filter))),
        Metric::Conversion { from: a, to: b, days, filter } => growth::funnel_query(a, b, *days, from, to, exclude, &and_filter(filter)),
    }
}

/// The HogQL that reads a conversion over `from..=to` for each variant of
/// `flag`: each person counted in the variant their first `from` event
/// carried (`$feature/<flag>`, as PostHog's libraries set it).
pub fn split_query(flag: &str, metric: &Metric, from: i64, to: i64, exclude: &str) -> Option<String> {
    let Metric::Conversion { from: a, to: b, days, filter } = metric else { return None };
    let first = format!("event = {} AND timestamp < toDateTime('{} 00:00:00')", growth::quote(a), day_string(to + 1));
    Some(format!(
        "SELECT v, count() AS entered, countIf(arrayExists(x -> x >= t1 AND x <= t1 + toIntervalDay({days}), seen)) AS converted FROM (\
         SELECT person_id, argMinIf(toString(properties['$feature/{flag}']), timestamp, {first}) AS v, minIf(timestamp, {first}) AS t1, \
         groupArrayIf(timestamp, event = {qb}) AS seen FROM events WHERE event IN ({qa}, {qb}) AND {range}{exclude}{filter} \
         GROUP BY person_id HAVING countIf({first}) > 0) GROUP BY v ORDER BY v",
        qa = growth::quote(a),
        qb = growth::quote(b),
        range = growth::range(from, to + i64::from(*days)),
        filter = and_filter(filter),
    ))
}

/// A metric as the growth loop's measure reads it back.
fn measure(metric: &Metric, connection: &str, exclude: &str) -> Measure {
    match metric {
        Metric::Event { event, filter } => Measure::Event { connection: connection.into(), event: event.clone(), exclude: format!("{exclude}{}", and_filter(filter)) },
        Metric::Conversion { from, to, days, filter } => {
            Measure::Funnel { connection: connection.into(), from: from.clone(), to: to.clone(), days: *days, exclude: exclude.into(), audience: and_filter(filter) }
        }
    }
}

fn cell(row: &[serde_json::Value], i: usize) -> Option<f64> {
    row.get(i).and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
}

impl PostHog {
    async fn rows(&self, query: String) -> Result<Vec<Vec<serde_json::Value>>, String> {
        let p = HogqlParams { squad: self.squad.clone(), connection: Some(self.connection.clone()), project: None, query, limit: Some(20) };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        crate::metrics::shared(&self.home).hogql(&p, now).await.map(|r| r.rows).map_err(|e| e.message)
    }
}

impl ProductData for PostHog {
    fn source(&self) -> String {
        format!("posthog {}", self.connection)
    }

    fn splits(&self) -> bool {
        true
    }

    fn query(&self, metric: &Metric, from: i64, to: i64) -> String {
        metric_query(metric, from, to, &self.exclude)
    }

    fn read<'a>(&'a self, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Reading, String>> {
        Box::pin(async move {
            let m = measure(metric, &self.connection, &self.exclude);
            let rows = self.rows(metric_query(metric, from, to, &self.exclude)).await?;
            let (value, sample) = growth::read_value(&m, &rows).ok_or("PostHog's rows weren't what was asked")?;
            Ok(Reading { value, sample })
        })
    }

    fn read_split<'a>(&'a self, split: &'a Split, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Vec<Arm>, String>> {
        Box::pin(async move {
            let query = split_query(&split.flag, metric, from, to, &self.exclude).ok_or("a split reads a conversion only")?;
            let rows = self.rows(query).await?;
            Ok(arms(&rows))
        })
    }
}

/// A split query's rows as arms: each variant's conversion (%) of the people who entered.
pub fn arms(rows: &[Vec<serde_json::Value>]) -> Vec<Arm> {
    rows.iter()
        .filter_map(|row| {
            let first = row.first()?;
            let variant = first.as_str().map(str::to_owned).unwrap_or_else(|| first.to_string());
            let (entered, converted) = (cell(row, 1)?, cell(row, 2)?);
            Some(Arm { variant, value: if entered > 0.0 { growth::round1(converted / entered * 100.0) } else { 0.0 }, sample: entered })
        })
        .collect()
}

// ---- Made up, for tests ----

type ReadFn = dyn Fn(&Metric, i64, i64) -> Result<Reading, String> + Send + Sync;
type SplitFn = dyn Fn(&Split, &Metric, i64, i64) -> Result<Vec<Arm>, String> + Send + Sync;

/// Numbers made up by closures: for tests, and for building screens before
/// a real service is connected.
pub struct Fake {
    pub read: Box<ReadFn>,
    pub split: Option<Box<SplitFn>>,
}

impl Fake {
    pub fn new(read: impl Fn(&Metric, i64, i64) -> Result<Reading, String> + Send + Sync + 'static) -> Self {
        Fake { read: Box::new(read), split: None }
    }

    pub fn with_split(mut self, split: impl Fn(&Split, &Metric, i64, i64) -> Result<Vec<Arm>, String> + Send + Sync + 'static) -> Self {
        self.split = Some(Box::new(split));
        self
    }
}

impl ProductData for Fake {
    fn source(&self) -> String {
        "fake".into()
    }

    fn splits(&self) -> bool {
        self.split.is_some()
    }

    fn query(&self, metric: &Metric, from: i64, to: i64) -> String {
        format!("{} from {} to {}", metric.label(), day_string(from), day_string(to))
    }

    fn read<'a>(&'a self, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Reading, String>> {
        Box::pin(async move { (self.read)(metric, from, to) })
    }

    fn read_split<'a>(&'a self, split: &'a Split, metric: &'a Metric, from: i64, to: i64) -> Fut<'a, Result<Vec<Arm>, String>> {
        Box::pin(async move {
            match &self.split {
                Some(f) => f(split, metric, from, to),
                None => Err("this source has no feature flags".into()),
            }
        })
    }
}

// ---- Judging through it ----

/// Read `x`'s numbers through `data` and judge them by its rule: a split
/// over the window after it went live, each variant beside control; else the
/// window after beside as many days before. Numbers that can't be read are
/// inconclusive, saying why.
pub async fn judge(data: &dyn ProductData, x: &Experiment, now: u64) -> Judged {
    let live = x.live_at.unwrap_or(now);
    let ((bf, bt), (af, at)) = growth::windows(live, x.rule.window_days);
    let mut out = Judged {
        verdict: Verdict::Inconclusive,
        why: String::new(),
        worse: false,
        before: None,
        after: None,
        arms: Vec::new(),
        change_pct: None,
        from: day_string(af),
        to: day_string(at),
        before_from: String::new(),
        before_to: String::new(),
        query: data.query(&x.metric, af, at),
        source: data.source(),
        at: now,
    };
    match &x.split {
        Some(split) if data.splits() => match data.read_split(split, &x.metric, af, at).await {
            Ok(arms) => {
                let (verdict, why, worse, pct) = super::judge_split(&x.rule, &arms, &split.control);
                out = Judged { verdict, why, worse, change_pct: pct, arms, ..out };
            }
            Err(why) => out.why = format!("the numbers couldn't be read: {why}"),
        },
        _ => {
            out.before_from = day_string(bf);
            out.before_to = day_string(bt);
            match (data.read(&x.metric, bf, bt).await, data.read(&x.metric, af, at).await) {
                (Ok(before), Ok(after)) => {
                    let (verdict, mut why, worse, pct) = super::judge(&x.rule, x.metric.is_share(), before, after);
                    if x.split.is_some() {
                        why = format!("{why} (no flags where the numbers are read: before beside after, not A/B)");
                    }
                    out = Judged { verdict, why, worse, change_pct: pct, before: Some(before), after: Some(after), ..out };
                }
                (Err(why), _) | (_, Err(why)) => out.why = format!("the numbers couldn't be read: {why}"),
            }
        }
    }
    out
}

/// Whether `x`'s events arrive: its metric (a conversion's first step)
/// counted anything from the day it went live to the next.
pub async fn arriving(data: &dyn ProductData, x: &Experiment) -> Result<bool, String> {
    let day = (x.live_at.unwrap_or(0) / super::DAY) as i64;
    let metric = match &x.metric {
        Metric::Conversion { from, filter, .. } => Metric::Event { event: from.clone(), filter: filter.clone() },
        m => m.clone(),
    };
    data.read(&metric, day, day + 1).await.map(|r| r.sample > 0.0)
}
