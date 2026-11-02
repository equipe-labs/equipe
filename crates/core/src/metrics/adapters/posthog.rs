//! PostHog: a site's page loads, by language, referrer and UTM source, and
//! the clicks of the events the person names, by where they were clicked.
//! Asked by HogQL (`/api/projects/<id>/query/`) with a personal API key that
//! has `query:read`.
//!
//! PostHog gives each page load a new person ID on sites that don't keep one,
//! so `visits` is page loads, not people (docs/METRICS.md). `filter` keeps the
//! person's own visits out (`internal!=true`) — it is the person's, as they set
//! it, never guessed.
//!
//! Days are this Mac's days: events are cut by its time zone, not PostHog's.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Adapter, Answer, Fetch, Fetching, error, json, refused, send, unix_now};
use crate::metrics::{day_number, day_string};
use crate::protocol::metrics::{ErrorCode, FetchError, Field, MetricDef, Point, Shape, SourceKind, SourceSpec};

pub struct Posthog;

pub const US: &str = "https://us.posthog.com";
const SERVICE: &str = "PostHog";
/// Labels a breakdown keeps a day; the rest are added up as [`OTHER`].
const LABELS: usize = 30;
const OTHER: &str = "(other)";
/// Rows one query may bring back (HogQL's own limit is 100 unless asked).
const ROWS: u32 = 50_000;

impl Adapter for Posthog {
    fn spec(&self) -> SourceSpec {
        let field = |key: &str, label: &str, secret, required, hint: &str| Field { key: key.into(), label: label.into(), secret, required, hint: Some(hint.into()) };
        let metric = |name: &str, label: &str, unit: &str, shape| MetricDef { name: name.into(), label: label.into(), unit: unit.into(), shape };
        SourceSpec {
            kind: SourceKind::Posthog,
            label: "PostHog".into(),
            fields: vec![
                field("api_key", "Personal API key", true, true, "PostHog → Settings → Personal API keys, with Query: Read"),
                field("project_id", "Project ID", false, true, "The number in the project's address: us.posthog.com/project/<id>"),
                field("host", "Host", false, false, "https://us.posthog.com (US) or https://eu.posthog.com (EU)"),
                field("filter", "Count only", false, false, "Event properties, comma-separated: internal!=true, environment=production"),
                field("language_property", "Language property", false, false, "The event property with the page's language; $browser_language when empty"),
                field("click_events", "Click events", false, false, "Events counted as clicks, comma-separated: download_click, cta_github_click"),
                field("click_property", "Click place property", false, false, "The event property that says where it was clicked: location"),
            ],
            metrics: vec![
                metric("visits", "Visits", "page_loads", Shape::Count),
                metric("languages", "Languages", "page_loads", Shape::Breakdown),
                metric("referrers", "Referrers", "page_loads", Shape::Breakdown),
                metric("utm_sources", "UTM sources", "page_loads", Shape::Breakdown),
                metric("clicks", "Clicks", "clicks", Shape::Breakdown),
            ],
            keeps_days: None,
        }
    }

    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a> {
        Box::pin(async move {
            let ask = Ask::new(&fetch)?;
            let key = fetch.keys.get("api_key").map(|k| k.trim()).filter(|k| !k.is_empty()).ok_or_else(|| error(ErrorCode::Auth, "no API key"))?;
            let mut points = Vec::new();
            let language = format!("toString({})", ask.language);
            let pageviews = [
                ("languages", language.as_str(), "(unknown)"),
                ("referrers", "toString(properties.$referring_domain)", "$direct"),
                ("utm_sources", "toString(properties.utm_source)", "(none)"),
            ];
            for (metric, label, none) in pageviews {
                let rows = run(&fetch, key, &ask.query("event = '$pageview'", label, none)).await?;
                let days = by_day(&rows)?;
                if metric == "languages" {
                    // Every page load has a language row, so the days' totals are the visits.
                    points.extend(days.iter().map(|(day, split)| Point { metric: "visits".into(), day: day.clone(), value: split.values().sum(), breakdown: None }));
                }
                points.extend(breakdowns(metric, days));
            }
            if !ask.clicks.is_empty() {
                let events = ask.clicks.iter().map(|e| quote(e)).collect::<Vec<_>>().join(", ");
                let place = match &ask.click_place {
                    Some(p) => format!("toString({p})"),
                    None => "event".into(),
                };
                let rows = run(&fetch, key, &ask.query(&format!("event IN ({events})"), &place, "(unknown)")).await?;
                points.extend(breakdowns("clicks", by_day(&rows)?));
            }
            Ok(points)
        })
    }
}

/// What a fetch asks, checked before anything is sent.
struct Ask {
    from: String,
    to: String,
    zone: String,
    filter: String,
    language: String,
    clicks: Vec<String>,
    click_place: Option<String>,
}

impl Ask {
    fn new(fetch: &Fetch) -> Result<Self, FetchError> {
        let setting = |k: &str| fetch.settings.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
        let bad = |m: String| error(ErrorCode::Other, m);
        let today = day_number(fetch.today).ok_or_else(|| bad("bad day".into()))?;
        let zone = setting("time_zone").map(str::to_owned).unwrap_or_else(local_zone);
        if !zone.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)) {
            return Err(bad(format!("{zone} isn't a time zone")));
        }
        let filter = filter(setting("filter").unwrap_or_default()).map_err(bad)?;
        let language = property(setting("language_property").unwrap_or("$browser_language")).map_err(bad)?;
        let clicks: Vec<String> = setting("click_events").unwrap_or_default().split(',').map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned).collect();
        let click_place = setting("click_property").map(property).transpose().map_err(bad)?;
        Ok(Self { from: day_string(today - i64::from(fetch.days.max(1)) + 1), to: day_string(today), zone, filter, language, clicks, click_place })
    }

    /// Per day and label, the count of events matching `which`.
    fn query(&self, which: &str, label: &str, none: &str) -> String {
        let day = format!("toDate(toTimeZone(timestamp, '{}'))", self.zone);
        format!(
            "SELECT toString({day}) AS day, coalesce(nullIf({label}, ''), {none}) AS label, count() AS n FROM events \
             WHERE {which}{filter} AND timestamp >= toDateTime('{from} 00:00:00') - INTERVAL 1 DAY \
             AND {day} >= toDate('{from}') AND {day} <= toDate('{to}') \
             GROUP BY day, label ORDER BY day, n DESC LIMIT {ROWS}",
            filter = self.filter,
            none = quote(none),
            from = self.from,
            to = self.to,
        )
    }
}

/// This Mac's time zone, by name: `Asia/Seoul`.
fn local_zone() -> String {
    if let Some(tz) = std::env::var("TZ").ok().map(|t| t.trim_start_matches(':').to_owned()).filter(|t| t.contains('/')) {
        return tz;
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().and_then(|p| p.split_once("zoneinfo/").map(|(_, z)| z.to_owned())))
        .unwrap_or_else(|| "UTC".into())
}

/// An event property as HogQL names it — `internal`, `$browser_language`.
fn property(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
        return Err(format!("{name} isn't an event property"));
    }
    Ok(format!("properties.{name}"))
}

/// A string as HogQL quotes it.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// `internal!=true, environment=production` as ` AND …` conditions on event
/// properties. A property that isn't there is the empty string, so
/// `internal!=true` keeps the events without it.
fn filter(text: &str) -> Result<String, String> {
    let mut out = String::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, op, value) = match (part.split_once("!="), part.split_once('=')) {
            (Some((n, v)), _) => (n, "!=", v),
            (None, Some((n, v))) => (n, "=", v),
            _ => return Err(format!("`{part}` isn't property=value or property!=value")),
        };
        out.push_str(&format!(" AND coalesce(toString({}), '') {op} {}", property(name)?, quote(value.trim())));
    }
    Ok(out)
}

/// Send one HogQL query; its rows.
async fn run(fetch: &Fetch<'_>, key: &str, query: &str) -> Result<Vec<Value>, FetchError> {
    let host = fetch.settings.get("host").map(|h| h.trim().trim_end_matches('/')).filter(|h| !h.is_empty()).unwrap_or(US);
    if !(host.starts_with("https://") || host.starts_with("http://127.0.0.1")) {
        return Err(error(ErrorCode::Other, format!("{host} isn't an https address")));
    }
    let project = fetch.settings.get("project_id").map(|p| p.trim()).unwrap_or_default();
    if project.is_empty() || !project.chars().all(|c| c.is_ascii_digit()) {
        return Err(error(ErrorCode::Other, "the project ID is a number"));
    }
    let body = json!({ "query": { "kind": "HogQLQuery", "query": query }, "name": "equipe metrics" });
    let request = fetch
        .client
        .post(format!("{host}/api/projects/{project}/query/"))
        .bearer_auth(key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string());
    let answer = send(SERVICE, request).await?;
    if !(200..300).contains(&answer.status) {
        return Err(refusal(&answer));
    }
    let v = json(SERVICE, &answer)?;
    v.get("results").and_then(Value::as_array).cloned().ok_or_else(|| error(ErrorCode::BadResponse, "PostHog's answer had no results"))
}

fn refusal(answer: &Answer) -> FetchError {
    let said = serde_json::from_slice::<Value>(&answer.body).ok().and_then(|v| v.get("detail").and_then(Value::as_str).map(str::to_owned));
    // A key without `query:read` is a 403 too: `auth`, with PostHog's words.
    refused(SERVICE, answer, said, unix_now())
}

/// `[day, label, n]` rows as each day's split.
fn by_day(rows: &[Value]) -> Result<BTreeMap<String, BTreeMap<String, f64>>, FetchError> {
    let bad = || error(ErrorCode::BadResponse, "PostHog's rows weren't day, label, count");
    let mut days: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for row in rows {
        let (Some(day), Some(n)) = (row.get(0).and_then(Value::as_str), row.get(2).and_then(Value::as_f64)) else { return Err(bad()) };
        let label = match row.get(1) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => "(unknown)".into(),
            Some(other) => other.to_string(),
        };
        if day_number(day).is_none() {
            return Err(bad());
        }
        *days.entry(day.to_owned()).or_default().entry(label).or_insert(0.0) += n;
    }
    Ok(days)
}

/// Each day's split as a breakdown point: its biggest [`LABELS`], the rest as one.
fn breakdowns(metric: &str, days: BTreeMap<String, BTreeMap<String, f64>>) -> impl Iterator<Item = Point> + '_ {
    days.into_iter().map(move |(day, split)| {
        let value = split.values().sum();
        let mut labels: Vec<(String, f64)> = split.into_iter().collect();
        labels.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut kept: BTreeMap<String, f64> = labels.iter().take(LABELS).cloned().collect();
        let rest: f64 = labels.iter().skip(LABELS).map(|(_, v)| v).sum();
        if rest > 0.0 {
            *kept.entry(OTHER.into()).or_insert(0.0) += rest;
        }
        Point { metric: metric.into(), day, value, breakdown: Some(kept) }
    })
}

#[cfg(test)]
mod tests {
    use super::super::stand_in::{Reply, serve};
    use super::*;

    const KEY: &str = "phx_secret_key_1234";

    fn settings(host: &str, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut s: BTreeMap<String, String> =
            [("project_id", "700951"), ("host", host), ("time_zone", "Asia/Seoul")].iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        s.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
        s
    }

    async fn fetch_with(settings: &BTreeMap<String, String>, days: u32) -> Result<Vec<Point>, FetchError> {
        let keys: BTreeMap<String, String> = [("api_key".to_owned(), KEY.to_owned())].into();
        let client = reqwest::Client::new();
        Posthog.fetch(Fetch { settings, keys: &keys, today: "2026-11-02", days, client: &client }).await
    }

    /// The recorded answer for the query `body` asks.
    fn recorded(body: &str) -> Reply {
        let fixture: &[u8] = if body.contains("referring_domain") {
            include_bytes!("fixtures/posthog/referrers.json")
        } else if body.contains("utm_source") {
            include_bytes!("fixtures/posthog/utm_sources.json")
        } else if body.contains("event IN") {
            include_bytes!("fixtures/posthog/clicks.json")
        } else {
            include_bytes!("fixtures/posthog/languages.json")
        };
        Reply::ok(fixture)
    }

    fn point<'a>(points: &'a [Point], metric: &str, day: &str) -> &'a Point {
        points.iter().find(|p| p.metric == metric && p.day == day).unwrap_or_else(|| panic!("no {metric} on {day}"))
    }

    #[tokio::test]
    async fn a_fetch_reads_visits_languages_referrers_utm_and_clicks_by_day() {
        let (url, seen) = serve(|s| recorded(&s.body)).await;
        let s = settings(&url, &[("filter", "internal!=true, environment=production"), ("language_property", "locale"), ("click_events", "cta_github_click, desktop_download_click"), ("click_property", "location")]);
        let points = fetch_with(&s, 7).await.unwrap();

        let visits = point(&points, "visits", "2026-11-01");
        let languages = point(&points, "languages", "2026-11-01");
        assert_eq!(visits.value, languages.value, "visits are the page loads of every language");
        assert!(visits.value > 0.0);
        assert!(points.iter().filter(|p| p.metric == "visits").all(|p| p.day.as_str() >= "2026-10-27" && p.day.as_str() <= "2026-11-02"));
        let referrers = point(&points, "referrers", "2026-11-01");
        assert_eq!(referrers.breakdown.as_ref().unwrap().values().sum::<f64>(), referrers.value);
        assert!(referrers.breakdown.as_ref().unwrap().contains_key("$direct"), "no referrer is $direct");
        assert!(points.iter().any(|p| p.metric == "utm_sources"));
        assert!(points.iter().any(|p| p.metric == "clicks" && p.breakdown.as_ref().is_some_and(|b| !b.is_empty())));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "three page-load queries and one of clicks");
        for r in seen.iter() {
            assert_eq!(r.path, "/api/projects/700951/query/");
            assert_eq!(r.headers.get("authorization").map(String::as_str), Some(format!("Bearer {KEY}").as_str()));
            assert!(r.body.contains("coalesce(toString(properties.internal), '') != 'true'"), "{}", r.body);
            assert!(r.body.contains("coalesce(toString(properties.environment), '') = 'production'"));
            assert!(r.body.contains("toTimeZone(timestamp, 'Asia/Seoul')"));
            assert!(r.body.contains(">= toDate('2026-10-27')") && r.body.contains("<= toDate('2026-11-02')"));
            assert!(r.body.contains("LIMIT 50000"));
        }
        assert!(seen.iter().any(|r| r.body.contains("event IN ('cta_github_click', 'desktop_download_click')") && r.body.contains("properties.location")));
        assert!(seen.iter().any(|r| r.body.contains("properties.locale")));
        assert!(!points.iter().any(|p| format!("{p:?}").contains(KEY)));
    }

    #[tokio::test]
    async fn without_click_events_no_clicks_are_asked() {
        let (url, seen) = serve(|s| recorded(&s.body)).await;
        let points = fetch_with(&settings(&url, &[]), 3).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert!(!points.iter().any(|p| p.metric == "clicks"));
        assert!(seen.lock().unwrap().iter().any(|r| r.body.contains("properties.$browser_language")), "the browser's language when none is named");
    }

    #[tokio::test]
    async fn refusals_are_the_contracts_codes() {
        let cases: [(u16, Option<&str>, &[u8], ErrorCode); 5] = [
            (401, None, include_bytes!("fixtures/posthog/401.json"), ErrorCode::Auth),
            (403, None, include_bytes!("fixtures/posthog/403_scope.json"), ErrorCode::Auth),
            (429, Some("120"), include_bytes!("fixtures/posthog/429.json"), ErrorCode::RateLimited),
            (400, None, include_bytes!("fixtures/posthog/400_query.json"), ErrorCode::Other),
            (200, None, b"<html>oops</html>", ErrorCode::BadResponse),
        ];
        for (status, retry, body, code) in cases {
            let body = body.to_vec();
            let (url, _) = serve(move |_| {
                let r = Reply::status(status, body.clone());
                match retry {
                    Some(s) => r.header("Retry-After", s),
                    None => r,
                }
            })
            .await;
            let e = fetch_with(&settings(&url, &[]), 7).await.unwrap_err();
            assert_eq!(e.code, code, "{status}: {}", e.message);
            assert!(!e.message.contains(KEY));
            if code == ErrorCode::RateLimited {
                let at = e.retry_at.unwrap();
                assert!(at >= unix_now() + 110 && at <= unix_now() + 130);
            }
        }
    }

    #[tokio::test]
    async fn bad_settings_send_nothing() {
        let (url, seen) = serve(|s| recorded(&s.body)).await;
        for extra in [("filter", "internal"), ("filter", "a'b=1"), ("language_property", "x) OR (1"), ("click_property", "lo cation"), ("project_id", "70x")] {
            let e = fetch_with(&settings(&url, &[extra]), 7).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::Other, "{extra:?}");
        }
        let e = fetch_with(&settings("http://example.com", &[]), 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Other, "a plain http host isn't sent the key");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn values_are_quoted_and_filters_read() {
        assert_eq!(quote("it's"), "'it\\'s'");
        assert_eq!(quote("a\\"), "'a\\\\'");
        assert_eq!(filter("").unwrap(), "");
        assert_eq!(filter(" internal != true ").unwrap(), " AND coalesce(toString(properties.internal), '') != 'true'");
        assert!(filter("x>1").is_err());
    }

    #[test]
    fn a_crowded_day_keeps_its_biggest_labels() {
        let split: BTreeMap<String, f64> = (0..40).map(|i| (format!("l{i:02}"), f64::from(i + 1))).collect();
        let total: f64 = split.values().sum();
        let p = breakdowns("referrers", [("2026-11-01".to_owned(), split)].into()).next().unwrap();
        let b = p.breakdown.unwrap();
        assert_eq!(b.len(), LABELS + 1);
        assert_eq!(b.values().sum::<f64>(), total);
        assert_eq!(b[OTHER], (1..=10).sum::<i32>() as f64);
    }
}
