//! A GitHub repository's traffic and stars, from its REST API with a
//! fine-grained token. GitHub keeps traffic (views, clones, referrers) for 14
//! days only, so older days live in equipe's snapshots.
//!
//! GitHub's days are UTC days; we take each as the day it names.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{error, json, refused, send, unix_now, Adapter, Fetch, Fetching};
use crate::metrics::{day_number, day_string};
use crate::protocol::metrics::{ErrorCode, FetchError, Field, MetricDef, Point, Shape, SourceKind, SourceSpec};

pub struct Github;

const SERVICE: &str = "GitHub";
const API: &str = "https://api.github.com";
/// Days of traffic GitHub keeps, today included.
const KEEPS: u32 = 14;
/// Stargazers per page, and the most pages read for the stars' history.
const PER_PAGE: usize = 100;
const STAR_PAGES: u32 = 10;

impl Adapter for Github {
    fn spec(&self) -> SourceSpec {
        let field = |key: &str, label: &str, secret, hint: &str| Field { key: key.into(), label: label.into(), secret, required: true, hint: Some(hint.into()) };
        let metric = |name: &str, label: &str, unit: &str, shape| MetricDef { name: name.into(), label: label.into(), unit: unit.into(), shape };
        SourceSpec {
            kind: SourceKind::Github,
            label: "GitHub".into(),
            fields: vec![
                field(
                    "token",
                    "Token",
                    true,
                    "A fine-grained token with read access to the repository's Administration (traffic) and Metadata — github.com/settings/personal-access-tokens",
                ),
                field("repo", "Repository", false, "owner/name"),
            ],
            metrics: vec![
                metric("views", "Views", "views", Shape::Count),
                metric("visitors", "Unique visitors", "views", Shape::Count),
                metric("clones", "Clones", "clones", Shape::Count),
                metric("stars", "Stars", "stars", Shape::Total),
                metric("referrers", "Referrers (last 14 days)", "views", Shape::Total),
            ],
            keeps_days: Some(KEEPS),
        }
    }

    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a> {
        Box::pin(async move {
            let token = fetch.keys.get("token").map(|t| t.trim()).unwrap_or_default();
            if token.is_empty() {
                return Err(error(ErrorCode::Auth, "no token"));
            }
            let repo = fetch.settings.get("repo").map(|r| r.trim()).unwrap_or_default();
            if !is_repo(repo) {
                return Err(error(ErrorCode::Other, "the repository should be owner/name"));
            }
            let today = day_number(fetch.today).ok_or_else(|| error(ErrorCode::Other, "bad day"))?;
            let from = today - i64::from(fetch.days.max(1)) + 1;
            // Only tests set it: connecting refuses fields that aren't in the spec.
            let base = fetch.settings.get("api_url").map(|u| u.trim_end_matches('/')).unwrap_or(API);
            let api = Api { client: fetch.client, token, base: format!("{base}/repos/{repo}") };

            let about = api.get("", None).await?;
            let stars = about.get("stargazers_count").and_then(Value::as_u64).ok_or_else(bad)?;
            let mut points = Vec::new();

            let views = api.get("/traffic/views?per=day", None).await?;
            let listed = daily(&views, "views")?;
            for n in traffic_days(&listed, from, today) {
                let (count, uniques) = listed.get(&n).copied().unwrap_or((0, 0));
                points.push(point("views", n, count));
                points.push(point("visitors", n, uniques));
            }

            let clones = api.get("/traffic/clones?per=day", None).await?;
            let listed = daily(&clones, "clones")?;
            for n in traffic_days(&listed, from, today) {
                points.push(point("clones", n, listed.get(&n).map_or(0, |c| c.0)));
            }

            let referrers = api.get("/traffic/popular/referrers", None).await?;
            let mut split: BTreeMap<String, f64> = BTreeMap::new();
            for r in referrers.as_array().ok_or_else(bad)? {
                let (Some(name), Some(count)) = (r.get("referrer").and_then(Value::as_str), r.get("count").and_then(Value::as_u64)) else { return Err(bad()) };
                *split.entry(name.to_owned()).or_default() += count as f64;
            }
            let total: f64 = split.values().sum();
            points.push(Point { metric: "referrers".into(), day: day_string(today), value: total, breakdown: Some(split) });

            points.extend(star_points(&api, stars, from, today).await?);
            Ok(points)
        })
    }
}

/// `owner/name`, each part of letters, digits, `.`, `_` and `-`.
fn is_repo(repo: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    matches!(repo.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

/// The repository's API, asked with the token.
struct Api<'a> {
    client: &'a reqwest::Client,
    token: &'a str,
    /// `<api>/repos/<owner>/<name>`.
    base: String,
}

impl Api<'_> {
    /// GET `base + path` as JSON, or why not.
    async fn get(&self, path: &str, accept: Option<&str>) -> Result<Value, FetchError> {
        let request = self
            .client
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", accept.unwrap_or("application/vnd.github+json"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "equipe");
        let answer = send(SERVICE, request).await?;
        if (200..300).contains(&answer.status) {
            return json(SERVICE, &answer);
        }
        let said = serde_json::from_slice::<Value>(&answer.body)
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(|m| m.replace(self.token, "[token]")));
        let now = unix_now();
        let mut refusal = refused(SERVICE, &answer, said.clone(), now);
        // GitHub says a rate limit with 403 (and `x-ratelimit-remaining: 0`).
        if answer.status == 403 && said.as_deref().is_some_and(|s| s.to_ascii_lowercase().contains("rate limit")) {
            refusal.code = ErrorCode::RateLimited;
            refusal.retry_at = Some(now + answer.retry_after.unwrap_or(3600));
        }
        if answer.status == 404 {
            refusal.message = format!("{SERVICE} said 404: no such repository, or the token can't see it");
        }
        Err(refusal)
    }
}

fn bad() -> FetchError {
    error(ErrorCode::BadResponse, format!("{SERVICE}'s answer wasn't what was expected"))
}

fn point(metric: &str, n: i64, value: u64) -> Point {
    Point { metric: metric.into(), day: day_string(n), value: value as f64, breakdown: None }
}

/// The day of a GitHub timestamp, `2026-11-01T00:00:00Z` → 2026-11-01's number.
fn day_of(stamp: Option<&str>) -> Result<i64, FetchError> {
    stamp.and_then(|s| s.get(..10)).and_then(day_number).ok_or_else(bad)
}

/// A traffic answer's days (`views` or `clones`): day → (count, uniques).
fn daily(answer: &Value, key: &str) -> Result<BTreeMap<i64, (u64, u64)>, FetchError> {
    let mut days = BTreeMap::new();
    for d in answer.get(key).and_then(Value::as_array).ok_or_else(bad)? {
        let n = day_of(d.get("timestamp").and_then(Value::as_str))?;
        let (Some(count), Some(uniques)) = (d.get("count").and_then(Value::as_u64), d.get("uniques").and_then(Value::as_u64)) else { return Err(bad()) };
        days.insert(n, (count, uniques));
    }
    Ok(days)
}

/// The traffic days to report: the ones GitHub listed from `from` to `today`.
/// It lists its days with none as 0, and leaves out the ones it hasn't
/// counted yet (its days are UTC's, a day behind) — those stay out.
fn traffic_days(listed: &BTreeMap<i64, (u64, u64)>, from: i64, today: i64) -> Vec<i64> {
    listed.keys().copied().filter(|n| (from..=today).contains(n)).collect()
}

/// Stars by day: every day from `from` when all the stargazers could be
/// listed (a big repository's can't), else today's total only.
async fn star_points(api: &Api<'_>, stars: u64, from: i64, today: i64) -> Result<Vec<Point>, FetchError> {
    let mut starred: Vec<i64> = Vec::new();
    let mut all = stars == 0;
    if stars > 0 && stars <= u64::from(STAR_PAGES) * PER_PAGE as u64 {
        for page in 1..=STAR_PAGES {
            let path = format!("/stargazers?per_page={PER_PAGE}&page={page}");
            let listed = match api.get(&path, Some("application/vnd.github.star+json")).await {
                Ok(listed) => listed,
                // A fine-grained token for traffic may not list stargazers: today's total still counts.
                Err(e) if e.code == ErrorCode::Auth => break,
                Err(e) => return Err(e),
            };
            let listed = listed.as_array().ok_or_else(bad)?;
            for s in listed {
                starred.push(day_of(s.get("starred_at").and_then(Value::as_str))?);
            }
            if listed.len() < PER_PAGE || starred.len() as u64 >= stars {
                all = true;
                break;
            }
        }
    }
    if !all {
        return Ok(vec![point("stars", today, stars)]);
    }
    starred.sort_unstable();
    Ok((from..=today)
        .map(|n| {
            let by_then = if n == today { stars } else { starred.partition_point(|&d| d <= n) as u64 };
            point("stars", n, by_then)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::stand_in::{serve, Reply, Seen};
    use super::*;

    const TOKEN: &str = "github_pat_11ABCDEFG0secretsecret";
    const TODAY: &str = "2026-11-03";

    fn fixtures(seen: &Seen) -> Reply {
        let path = seen.path.as_str();
        let body = if path.contains("/traffic/views") {
            include_str!("fixtures/github/views.json")
        } else if path.contains("/traffic/clones") {
            include_str!("fixtures/github/clones.json")
        } else if path.contains("/traffic/popular/referrers") {
            include_str!("fixtures/github/referrers.json")
        } else if path.contains("/stargazers") {
            include_str!("fixtures/github/stargazers.json")
        } else if path == "/repos/acme/site" {
            include_str!("fixtures/github/repo.json")
        } else {
            return Reply::status(404, include_str!("fixtures/github/not_found.json"));
        };
        Reply::ok(body)
    }

    async fn run(url: &str, repo: &str, days: u32) -> Result<Vec<Point>, FetchError> {
        let settings = BTreeMap::from([("repo".to_owned(), repo.to_owned()), ("api_url".to_owned(), url.to_owned())]);
        let keys = BTreeMap::from([("token".to_owned(), TOKEN.to_owned())]);
        let client = reqwest::Client::new();
        Github.fetch(Fetch { settings: &settings, keys: &keys, today: TODAY, days, client: &client }).await
    }

    fn values(points: &[Point], metric: &str) -> BTreeMap<String, f64> {
        points.iter().filter(|p| p.metric == metric).map(|p| (p.day.clone(), p.value)).collect()
    }

    #[tokio::test]
    async fn a_fetch_maps_traffic_stars_and_referrers() {
        let (url, seen) = serve(fixtures).await;
        let points = run(&url, "acme/site", 30).await.unwrap();

        // Views and clones: the 14 days GitHub listed (recorded from a real
        // repository: to 11-01, its UTC days a day behind), none made up after.
        let views = values(&points, "views");
        assert_eq!(views.len(), 14);
        assert_eq!(views.keys().next().unwrap(), "2026-10-19");
        assert_eq!(views.keys().last().unwrap(), "2026-11-01");
        assert_eq!((views["2026-10-21"], views["2026-10-30"]), (0.0, 336.0), "a day with none is GitHub's 0");
        assert!(!views.contains_key(TODAY), "a day not counted yet is left out, not 0");
        assert_eq!(views.values().sum::<f64>(), 524.0);
        let visitors = values(&points, "visitors");
        assert_eq!((visitors.len(), visitors["2026-10-30"]), (14, 267.0));
        let clones = values(&points, "clones");
        assert_eq!((clones.len(), clones["2026-10-24"], clones["2026-11-01"]), (14, 4.0, 120.0));

        // Stars: cumulative over the 30 days asked, today the repository's count.
        let stars = values(&points, "stars");
        assert_eq!(stars.len(), 30);
        assert_eq!(stars["2026-10-05"], 2.0);
        assert_eq!(stars["2026-10-24"], 2.0);
        assert_eq!(stars["2026-10-25"], 3.0);
        assert_eq!(stars["2026-10-31"], 3.0);
        assert_eq!(stars["2026-11-01"], 5.0);
        assert_eq!(stars[TODAY], 5.0);

        // Referrers: one point today, its breakdown by referrer.
        let referrers: Vec<_> = points.iter().filter(|p| p.metric == "referrers").collect();
        assert_eq!(referrers.len(), 1);
        assert_eq!((referrers[0].day.as_str(), referrers[0].value), (TODAY, 414.0));
        let split = referrers[0].breakdown.as_ref().unwrap();
        assert_eq!((split["linkedin.com"], split["equipe.im"], split.len()), (274.0, 36.0, 9));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        for s in seen.iter() {
            assert_eq!(s.headers["authorization"], format!("Bearer {TOKEN}"));
            assert_eq!(s.headers["x-github-api-version"], "2022-11-28");
            assert_eq!(s.headers["user-agent"], "equipe");
        }
        assert!(seen.iter().any(|s| s.path == "/repos/acme/site/traffic/views?per=day"));
        let stargazers = seen.iter().find(|s| s.path.contains("/stargazers")).unwrap();
        assert_eq!(stargazers.path, "/repos/acme/site/stargazers?per_page=100&page=1");
        assert_eq!(stargazers.headers["accept"], "application/vnd.github.star+json");
        assert!(!serde_json::to_string(&points).unwrap().contains(TOKEN));
    }

    #[tokio::test]
    async fn a_short_fetch_keeps_to_its_days() {
        let (url, _) = serve(fixtures).await;
        let points = run(&url, "acme/site", 7).await.unwrap();
        let views = values(&points, "views");
        assert_eq!(views.keys().next().unwrap(), "2026-10-28");
        assert_eq!(views.len(), 5, "10-28 to 11-01: GitHub hasn't counted 11-02 and 11-03 yet");
        assert_eq!(values(&points, "stars").len(), 7);
    }

    #[tokio::test]
    async fn a_big_repository_has_only_todays_stars() {
        let (url, seen) = serve(|seen| {
            if seen.path == "/repos/acme/site" {
                Reply::ok(include_str!("fixtures/github/repo.json").replace("\"stargazers_count\": 5", "\"stargazers_count\": 15200"))
            } else {
                fixtures(seen)
            }
        })
        .await;
        let points = run(&url, "acme/site", 30).await.unwrap();
        assert_eq!(values(&points, "stars"), BTreeMap::from([(TODAY.to_owned(), 15200.0)]));
        assert!(!seen.lock().unwrap().iter().any(|s| s.path.contains("/stargazers")));
    }

    #[tokio::test]
    async fn a_token_that_cant_list_stargazers_still_has_todays_stars() {
        let (url, _) = serve(|s: &Seen| {
            if s.path.contains("/stargazers") {
                Reply::status(403, r#"{"message":"Resource not accessible by personal access token","status":"403"}"#)
            } else {
                fixtures(s)
            }
        })
        .await;
        let points = run(&url, "acme/site", 14).await.unwrap();
        let stars: Vec<&Point> = points.iter().filter(|p| p.metric == "stars").collect();
        assert_eq!(stars.len(), 1);
        assert_eq!((stars[0].day.as_str(), stars[0].value), (TODAY, 5.0));
        assert!(points.iter().any(|p| p.metric == "views"));
    }

    #[tokio::test]
    async fn a_refused_token_is_auth_and_not_quoted() {
        let (url, _) = serve(|_| Reply::status(401, include_str!("fixtures/github/bad_credentials.json"))).await;
        let e = run(&url, "acme/site", 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Auth);
        assert!(e.message.contains("Bad credentials"), "{}", e.message);
        assert!(!e.message.contains(TOKEN));
    }

    #[tokio::test]
    async fn a_403_rate_limit_is_rate_limited() {
        let (url, _) = serve(|_| Reply::status(403, include_str!("fixtures/github/rate_limited.json")).header("x-ratelimit-remaining", "0")).await;
        let e = run(&url, "acme/site", 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        let at = e.retry_at.unwrap();
        assert!(at.abs_diff(unix_now() + 3600) < 30, "{at}");
    }

    #[tokio::test]
    async fn a_429_waits_as_long_as_it_says() {
        let (url, _) = serve(|_| Reply::status(429, "{\"message\":\"slow down\"}").header("Retry-After", "120")).await;
        let e = run(&url, "acme/site", 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        assert!(e.retry_at.unwrap().abs_diff(unix_now() + 120) < 30);
    }

    #[tokio::test]
    async fn an_unseen_repository_says_so() {
        let (url, _) = serve(fixtures).await;
        let e = run(&url, "acme/other", 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Other);
        assert!(e.message.contains("no such repository"), "{}", e.message);
    }

    #[tokio::test]
    async fn an_answer_that_isnt_json_is_a_bad_response() {
        let (url, _) = serve(|seen| if seen.path.contains("/traffic/views") { Reply::ok("<html>oops</html>") } else { fixtures(seen) }).await;
        let e = run(&url, "acme/site", 7).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::BadResponse);
    }

    #[tokio::test]
    async fn a_bad_repository_asks_nothing() {
        let (url, seen) = serve(fixtures).await;
        for repo in ["acme", "acme/site/extra", "/site", "acme/", "ac me/site", "acme/si?te", "../etc"] {
            let e = run(&url, repo, 7).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::Other, "{repo}");
        }
        assert!(seen.lock().unwrap().is_empty());
        assert!(is_repo("my-org/site.io_2"));
    }
}
