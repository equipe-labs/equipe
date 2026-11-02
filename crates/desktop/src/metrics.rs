//! 지표: a product's numbers, read from the daemon (`metrics.*`, docs/METRICS.md).
//! This is the screen's model — what the daemon said, turned into what is drawn
//! (cards, banners, sparklines) — and none of its drawing (`views/metrics.rs`).
//!
//! A key lives in a text box until the person connects, then goes to the
//! daemon in `metrics.connect` and out of the box. Nothing here holds one:
//! [`Draft`] keeps the field names and never a value, and what the daemon
//! answers has its keys cut out already (docs/METRICS.md, Rules).

use std::collections::BTreeMap;

use equipe_core::protocol::metrics::{Bucket, Connection, ErrorCode, FetchError, MetricValue, QueryResult, Sample, Shape, SourceKind, SourceSpec, Sources};

/// The periods the screen offers, in days.
pub const PERIODS: [u32; 2] = [7, 30];

/// What the person is looking at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Flow {
    /// The dashboard (or its empty state).
    #[default]
    Dash,
    /// Connecting, or managing what is connected.
    Connect(Draft),
}

/// A connection being set up. Holds no value the person types.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    /// The service picked (step 2).
    pub kind: Option<SourceKind>,
    /// The connection whose key or settings are being replaced.
    pub replacing: Option<String>,
    /// The request is out.
    pub sending: bool,
    /// The service connected (step 3).
    pub done: Option<SourceKind>,
    /// Why the last try failed.
    pub failed: Option<String>,
    /// The person pressed connect with a required box empty.
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metrics {
    pub days: u32,
    pub sources: Option<Sources>,
    pub query: Option<QueryResult>,
    pub flow: Flow,
    /// Numbers are being asked for.
    pub loading: bool,
    /// Why they couldn't be (the daemon's own words).
    pub failed: Option<String>,
    /// The screen's own sample (`--features snapshot`): no daemon is asked.
    pub demo: bool,
}

impl Metrics {
    pub fn new() -> Self {
        Self { days: PERIODS[0], ..Self::default() }
    }

    pub fn connections(&self) -> &[Connection] {
        self.sources.as_ref().map_or(&[], |s| &s.connections)
    }

    /// Nothing connected yet (once the daemon has said).
    pub fn is_empty(&self) -> bool {
        self.sources.as_ref().is_some_and(|s| s.connections.is_empty())
    }

    pub fn spec(&self, kind: SourceKind) -> Option<&SourceSpec> {
        self.sources.as_ref()?.kinds.iter().find(|k| k.kind == kind)
    }

    pub fn connection_of(&self, kind: SourceKind) -> Option<&Connection> {
        self.connections().iter().find(|c| c.kind == kind)
    }
}

// ---- numbers and days ----

/// `1,284`; a small fractional number keeps one decimal (`4.5`).
pub fn number(v: f64) -> String {
    let whole = v.abs() >= 100. || v.fract().abs() < 0.05;
    let text = if whole { format!("{:.0}", v.abs()) } else { format!("{:.1}", v.abs()) };
    let (int, frac) = text.split_once('.').map_or((text.as_str(), None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    for (n, ch) in int.chars().enumerate() {
        if n > 0 && (int.len() - n) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    if v < 0. && out != "0" { format!("−{out}") } else { out }
}

/// `(month, day)` of a `YYYY-MM-DD`.
pub fn month_day(day: &str) -> Option<(u32, u32)> {
    let mut parts = day.split('-');
    let _year = parts.next()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((month, day))
}

/// A day in words, in the person's language order (`11월 2일`).
pub fn day_words(day: &str) -> String {
    match month_day(day) {
        Some((month, day)) => equipe_core::t!("metrics.date", month = month, day = day),
        None => day.to_owned(),
    }
}

/// The local `YYYY-MM-DD` and `HH:MM` of unix seconds.
pub fn local_day_time(unix: u64) -> (String, String) {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let secs = unix as libc::time_t;
    unsafe { libc::localtime_r(&secs, &mut tm) };
    (format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday), format!("{:02}:{:02}", tm.tm_hour, tm.tm_min))
}

/// A moment: just the time when it is today, else the day and the time.
pub fn moment(unix: u64, now: u64) -> String {
    let (day, time) = local_day_time(unix);
    if day == local_day_time(now).0 { time } else { format!("{} {time}", day_words(&day)) }
}

// ---- cards ----

/// A rise or a fall against the period before.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub up: Option<bool>,
    /// `+8.2%`, `−3`.
    pub text: String,
}

fn change_of(m: &MetricValue) -> Option<Change> {
    let (sign, body) = match (m.change_pct, m.change) {
        (Some(pct), _) => (pct, format!("{}%", number(pct.abs()))),
        (None, Some(delta)) => (delta, number(delta.abs())),
        (None, None) => return None,
    };
    let (up, mark) = if sign > 0. { (Some(true), "+") } else if sign < 0. { (Some(false), "−") } else { (None, "") };
    Some(Change { up, text: format!("{mark}{body}") })
}

/// One bar of a breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: String,
    /// 0–1 of the biggest.
    pub fill: f32,
}

/// How a metric is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// A number, with its line against the period before.
    Number { value: Option<String>, change: Option<Change>, series: Vec<Option<f64>>, previous: Vec<Option<f64>> },
    Bars(Vec<Bar>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub connection: String,
    pub kind: SourceKind,
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub body: Body,
    /// The service's latest fetch failed: what the card says, dimmed.
    pub error: Option<ErrorCode>,
    /// The latest day with a value.
    pub as_of: Option<String>,
    /// Days with a value, of those asked.
    pub days_known: u32,
    pub days: u32,
    /// The latest fetch that worked (unix seconds).
    pub last_ok: Option<u64>,
    /// The latest fetch (unix seconds).
    pub fetched: Option<u64>,
    /// The service itself keeps this many days only.
    pub keeps_days: Option<u32>,
}

fn values_of(samples: &[Sample]) -> Vec<Option<f64>> {
    samples.iter().map(|s| s.value).collect()
}

/// The dashboard's cards: one per metric, in the daemon's order.
pub fn cards(query: &QueryResult, sources: &Sources) -> Vec<Card> {
    query
        .values
        .iter()
        .map(|m| {
            let connection = sources.connections.iter().chain(&query.connections).find(|c| c.id == m.connection);
            let spec = sources.kinds.iter().find(|k| k.kind == m.kind);
            let body = match m.shape {
                Shape::Breakdown => {
                    let top = m.breakdown.iter().map(|s| s.value).fold(0., f64::max);
                    Body::Bars(m.breakdown.iter().map(|s| Bar { label: s.label.clone(), value: number(s.value), fill: if top > 0. { (s.value / top) as f32 } else { 0. } }).collect())
                }
                Shape::Count | Shape::Total => Body::Number { value: m.value.map(number), change: change_of(m), series: values_of(&m.series), previous: values_of(&m.previous_series) },
            };
            Card {
                connection: m.connection.clone(),
                kind: m.kind,
                metric: m.metric.clone(),
                label: m.label.clone(),
                unit: m.unit.clone(),
                body,
                error: connection.and_then(|c| c.last.as_ref()?.error.as_ref()).map(|e| e.code),
                as_of: m.as_of.clone(),
                days_known: m.days_known,
                days: query.days,
                last_ok: connection.and_then(|c| c.last_ok),
                fetched: connection.and_then(|c| c.last.as_ref()).map(|l| l.at),
                keeps_days: spec.and_then(|s| s.keeps_days),
            }
        })
        .collect()
}

/// A failed fetch the dashboard says above its cards.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub connection: String,
    pub kind: SourceKind,
    pub error: FetchError,
    pub last_ok: Option<u64>,
    pub at: u64,
}

pub fn banners(sources: &Sources) -> Vec<Banner> {
    sources
        .connections
        .iter()
        .filter_map(|c| {
            let last = c.last.as_ref()?;
            Some(Banner { connection: c.id.clone(), kind: c.kind, error: last.error.clone()?, last_ok: c.last_ok, at: last.at })
        })
        .collect()
}

/// What fixes it: a new key, or trying again.
pub fn needs_key(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::Auth | ErrorCode::Keychain)
}

/// The days of a period, as the screen words them: `10월 27일 ~ 11월 2일`.
pub fn range_words(query: &QueryResult) -> String {
    equipe_core::t!("metrics.range", from = day_words(&query.from), to = day_words(&query.to), previous_from = day_words(&query.previous_from), previous_to = day_words(&query.previous_to))
}

// ---- sparkline ----

/// The points of two lines in a box `w` × `h`, scaled together (so the period
/// and the one before compare) and with gaps kept: `None` is a day not known.
pub fn spark(series: &[Option<f64>], previous: &[Option<f64>], w: f32, h: f32) -> (Vec<Option<(f32, f32)>>, Vec<Option<(f32, f32)>>) {
    let all = || series.iter().chain(previous).flatten().copied();
    let (lo, hi) = all().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let place = |line: &[Option<f64>]| -> Vec<Option<(f32, f32)>> {
        let last = line.len().saturating_sub(1).max(1) as f32;
        line.iter()
            .enumerate()
            .map(|(n, v)| {
                let v = (*v)?;
                let t = if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
                Some((w * n as f32 / last, h - t * h))
            })
            .collect()
    };
    (place(series), place(previous))
}

// ---- the connect form ----

/// The required boxes of a form that are empty: `values` by field key. A key
/// the daemon already keeps (`kept`, when replacing settings) may stay empty.
pub fn missing(spec: &SourceSpec, values: &BTreeMap<String, String>, kept: &[String]) -> Vec<String> {
    spec.fields.iter().filter(|f| f.required && !kept.contains(&f.key) && values.get(&f.key).is_none_or(|v| v.trim().is_empty())).map(|f| f.key.clone()).collect()
}

/// The settings to send: every box with something in it, trimmed.
pub fn settings(values: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.trim().to_owned())).collect()
}

/// The query's bucket for a period: days up to a month, then weeks.
pub fn bucket(days: u32) -> Bucket {
    if days > 31 { Bucket::Week } else { Bucket::Day }
}

#[cfg(any(test, feature = "snapshot"))]
pub mod demo {
    //! A sample of the daemon's answers, for the screen's pictures and tests.
    use super::*;
    use equipe_core::protocol::metrics::{Field, Fetched, MetricDef, Share};

    fn field(key: &str, label: &str, secret: bool, hint: Option<&str>) -> Field {
        Field { key: key.into(), label: label.into(), secret, required: true, hint: hint.map(Into::into) }
    }

    fn def(name: &str, label: &str, unit: &str, shape: Shape) -> MetricDef {
        MetricDef { name: name.into(), label: label.into(), unit: unit.into(), shape }
    }

    pub fn kinds() -> Vec<SourceSpec> {
        vec![
            SourceSpec { kind: SourceKind::Posthog, label: "PostHog".into(), fields: vec![field("api_key", "Personal API key", true, None), field("project_id", "Project ID", false, None)], metrics: vec![def("visits", "Visits", "page_loads", Shape::Count), def("languages", "Languages", "page_loads", Shape::Breakdown)], keeps_days: None },
            SourceSpec { kind: SourceKind::Github, label: "GitHub".into(), fields: vec![field("token", "Token", true, None), field("repo", "Repository", false, None)], metrics: vec![def("views", "Views", "views", Shape::Count), def("stars", "Stars", "stars", Shape::Total)], keeps_days: Some(14) },
            SourceSpec { kind: SourceKind::AppStore, label: "App Store Connect".into(), fields: vec![field("private_key", "Private key", true, None), field("app_id", "App ID", false, None)], metrics: vec![def("downloads", "Downloads", "downloads", Shape::Count)], keeps_days: None },
        ]
    }

    fn connection(id: &str, kind: SourceKind, error: Option<FetchError>) -> Connection {
        // 2026-11-02 07:00 UTC, near enough to any zone's morning.
        let at = 1_793_602_800;
        Connection { id: id.into(), squad: "demo".into(), project: Some("equipe-site".into()), kind, settings: BTreeMap::new(), keys: vec!["token".into()], connected_at: at - 86_400 * 9, last: Some(Fetched { at, error: error.clone() }), last_ok: Some(if error.is_some() { at - 86_400 } else { at }) }
    }

    fn line(values: &[f64]) -> Vec<Sample> {
        values.iter().enumerate().map(|(n, v)| Sample { day: format!("2026-10-{:02}", 27 + n), value: Some(*v) }).collect()
    }

    fn share(label: &str, value: f64, previous: f64) -> Share {
        Share { label: label.into(), value, previous: Some(previous) }
    }

    fn visits(connection: &str) -> MetricValue {
        MetricValue { connection: connection.into(), kind: SourceKind::Posthog, metric: "visits".into(), label: "Visits".into(), unit: "page_loads".into(), shape: Shape::Count, value: Some(1284.), previous: Some(1187.), change: Some(97.), change_pct: Some(8.2), as_of: Some("2026-11-02".into()), days_known: 7, series: line(&[160., 172., 181., 176., 190., 188., 217.]), previous_series: line(&[150., 158., 162., 170., 168., 175., 180.]), breakdown: vec![] }
    }

    fn languages(connection: &str) -> MetricValue {
        MetricValue { metric: "languages".into(), label: "Languages".into(), shape: Shape::Breakdown, value: Some(1284.), previous: Some(1187.), change: None, change_pct: None, series: vec![], previous_series: vec![], breakdown: vec![share("English", 742., 700.), share("한국어", 283., 240.), share("日本語", 112., 100.), share("Español", 64., 60.), share("기타", 83., 87.)], ..visits(connection) }
    }

    fn github(connection: &str, metric: &str, label: &str, unit: &str, shape: Shape, value: f64, pct: f64) -> MetricValue {
        MetricValue { connection: connection.into(), kind: SourceKind::Github, metric: metric.into(), label: label.into(), unit: unit.into(), shape, value: Some(value), previous: Some(value / (1. + pct / 100.)), change: Some(value - value / (1. + pct / 100.)), change_pct: Some(pct), as_of: Some("2026-11-02".into()), days_known: 7, series: line(&[310., 322., 318., 340., 335., 351., 334.]), previous_series: line(&[300., 305., 312., 309., 318., 321., 325.]), breakdown: vec![] }
    }

    fn query(values: Vec<MetricValue>, connections: Vec<Connection>) -> QueryResult {
        QueryResult { from: "2026-10-27".into(), to: "2026-11-02".into(), previous_from: "2026-10-20".into(), previous_to: "2026-10-26".into(), days: 7, bucket: Bucket::Day, values, connections }
    }

    /// Everything connected and well.
    pub fn healthy() -> (Sources, QueryResult) {
        let connections = vec![connection("m1", SourceKind::Posthog, None), connection("m2", SourceKind::Github, None)];
        let values = vec![visits("m1"), languages("m1"), github("m2", "views", "Views", "views", Shape::Count, 2310., 5.1), github("m2", "stars", "Stars", "stars", Shape::Total, 25., 19.0)];
        (Sources { kinds: kinds(), connections: connections.clone() }, query(values, connections))
    }

    /// The key of one service refused, the other asked too often.
    pub fn failing() -> (Sources, QueryResult) {
        let auth = FetchError { code: ErrorCode::Auth, message: "key ••• was refused".into(), retry_at: None };
        let limit = FetchError { code: ErrorCode::RateLimited, message: "slow down".into(), retry_at: Some(1_793_602_800 + 7 * 3600) };
        let connections = vec![connection("m1", SourceKind::Posthog, Some(auth)), connection("m2", SourceKind::Github, Some(limit))];
        let values = vec![visits("m1"), languages("m1"), github("m2", "stars", "Stars", "stars", Shape::Total, 25., 19.0)];
        (Sources { kinds: kinds(), connections: connections.clone() }, query(values, connections))
    }

    pub fn nothing() -> Sources {
        Sources { kinds: kinds(), connections: vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_get_separators_and_small_ones_keep_a_decimal() {
        assert_eq!(number(1284.), "1,284");
        assert_eq!(number(1_234_567.), "1,234,567");
        assert_eq!(number(14.), "14");
        assert_eq!(number(8.24), "8.2");
        assert_eq!(number(0.), "0");
        assert_eq!(number(-3.), "−3");
    }

    #[test]
    fn a_change_is_signed_and_a_rise_is_up() {
        let (_, q) = demo::healthy();
        let c = change_of(&q.values[0]).unwrap();
        assert_eq!((c.up, c.text.as_str()), (Some(true), "+8.2%"));
        let mut fall = q.values[0].clone();
        fall.change_pct = Some(-2.3);
        assert_eq!(change_of(&fall).unwrap(), Change { up: Some(false), text: "−2.3%".into() });
        fall.change_pct = None;
        fall.change = Some(-4.);
        assert_eq!(change_of(&fall).unwrap().text, "−4");
        fall.change = None;
        assert_eq!(change_of(&fall), None);
    }

    #[test]
    fn a_breakdown_is_bars_scaled_to_the_biggest() {
        let (s, q) = demo::healthy();
        let cards = cards(&q, &s);
        let Body::Bars(bars) = &cards[1].body else { panic!("languages are bars") };
        assert_eq!((bars[0].label.as_str(), bars[0].value.as_str(), bars[0].fill), ("English", "742", 1.0));
        assert!((bars[1].fill - 283. / 742.).abs() < 1e-6);
        assert!(matches!(cards[0].body, Body::Number { .. }));
    }

    #[test]
    fn a_card_of_a_failed_connection_says_so_and_keeps_the_last_value() {
        let (s, q) = demo::failing();
        let cards = cards(&q, &s);
        assert_eq!(cards[0].error, Some(ErrorCode::Auth));
        assert_eq!(cards[2].error, Some(ErrorCode::RateLimited));
        assert!(cards[0].last_ok.is_some());
        let Body::Number { value, .. } = &cards[0].body else { panic!() };
        assert_eq!(value.as_deref(), Some("1,284"));
        let (healthy_s, healthy_q) = demo::healthy();
        assert!(super::cards(&healthy_q, &healthy_s).iter().all(|c| c.error.is_none()));
    }

    #[test]
    fn banners_come_from_failed_fetches_and_a_key_error_asks_for_a_new_key() {
        let (s, _) = demo::failing();
        let b = banners(&s);
        assert_eq!(b.iter().map(|b| b.error.code).collect::<Vec<_>>(), [ErrorCode::Auth, ErrorCode::RateLimited]);
        assert!(needs_key(ErrorCode::Auth) && needs_key(ErrorCode::Keychain));
        assert!(!needs_key(ErrorCode::RateLimited) && !needs_key(ErrorCode::Network));
        assert!(banners(&demo::healthy().0).is_empty());
    }

    #[test]
    fn nothing_connected_is_the_empty_state_only_once_the_daemon_said() {
        let mut m = Metrics::new();
        assert!(!m.is_empty());
        m.sources = Some(demo::nothing());
        assert!(m.is_empty());
        m.sources = Some(demo::healthy().0);
        assert!(!m.is_empty());
        assert_eq!(m.connection_of(SourceKind::Github).map(|c| c.id.as_str()), Some("m2"));
        assert!(m.connection_of(SourceKind::AppStore).is_none());
    }

    #[test]
    fn the_form_asks_for_every_required_box_and_sends_only_what_is_filled() {
        let spec = demo::kinds().remove(0);
        let mut values = BTreeMap::new();
        assert_eq!(missing(&spec, &values, &[]), ["api_key", "project_id"]);
        values.insert("api_key".into(), "  phx_x  ".into());
        values.insert("project_id".into(), " ".into());
        assert_eq!(missing(&spec, &values, &[]), ["project_id"]);
        values.insert("project_id".into(), "700951".into());
        assert!(missing(&spec, &values, &[]).is_empty());
        values.insert("api_key".into(), String::new());
        assert!(missing(&spec, &values, &["api_key".to_owned()]).is_empty(), "a kept key may stay empty");
        values.insert("api_key".into(), "  phx_x  ".into());
        assert_eq!(settings(&values).get("api_key").map(String::as_str), Some("phx_x"));
    }

    #[test]
    fn a_draft_is_debug_printed_without_any_value_to_leak() {
        // It has no field to hold one: what is typed stays in the boxes.
        let shown = format!("{:?}", Draft { kind: Some(SourceKind::Posthog), ..Draft::default() });
        assert!(shown.contains("Posthog") && !shown.contains("phx"));
    }

    #[test]
    fn the_sparkline_scales_both_lines_together_and_keeps_gaps() {
        let (a, b) = spark(&[Some(0.), None, Some(10.)], &[Some(5.), Some(5.), Some(5.)], 100., 20.);
        assert_eq!(a[0], Some((0., 20.)));
        assert_eq!(a[1], None);
        assert_eq!(a[2], Some((100., 0.)));
        assert_eq!(b[1], Some((50., 10.)));
        // One value, all the same: a line across the middle, no division by zero.
        let (flat, _) = spark(&[Some(3.), Some(3.)], &[], 10., 10.);
        assert_eq!(flat[0], Some((0., 5.)));
    }

    #[test]
    fn months_and_days_read_off_a_day() {
        assert_eq!(month_day("2026-11-02"), Some((11, 2)));
        assert_eq!(month_day("nope"), None);
        assert_eq!(day_words("2026-11-02"), "11월 2일");
        assert_eq!(bucket(7), Bucket::Day);
        assert_eq!(bucket(90), Bucket::Week);
    }
}
