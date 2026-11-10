//! Made-up numbers that look like a small site's, the same for the same day
//! and `seed`: for tests, and for building screens before a real service is
//! connected. Like GitHub it remembers only 14 days, so snapshots matter.
//!
//! `fail` makes every fetch fail with that code (`auth`, `rate_limited`,
//! `network`) — its message quoting the token, as a careless service might,
//! for `metrics` to keep out of everything it writes and answers.

use std::collections::BTreeMap;

use super::{Adapter, Fetch, Fetching};
use crate::metrics::{day_number, day_string};
use crate::protocol::metrics::{ErrorCode, FetchError, Field, MetricDef, Point, Shape, SourceKind, SourceSpec};

pub struct Fake;

/// Days it remembers back from today.
const KEEPS: u32 = 14;
const LANGUAGES: [(&str, u64); 4] = [("English", 58), ("Korean", 22), ("Japanese", 12), ("Spanish", 8)];

impl Adapter for Fake {
    fn spec(&self) -> SourceSpec {
        let field = |key: &str, label: &str, secret, required| Field { key: key.into(), label: label.into(), secret, required, hint: None };
        let metric = |name: &str, label: &str, unit: &str, shape| MetricDef { name: name.into(), label: label.into(), unit: unit.into(), shape };
        SourceSpec {
            kind: SourceKind::Fake,
            label: "Fake".into(),
            fields: vec![
                field("token", "Token", true, true),
                field("seed", "Seed", false, false),
                field("fail", "Fail with", false, false),
            ],
            metrics: vec![
                metric("visits", "Visits", "page_loads", Shape::Count),
                metric("stars", "Stars", "stars", Shape::Total),
                metric("languages", "Languages", "page_loads", Shape::Breakdown),
            ],
            keeps_days: Some(KEEPS),
        }
    }

    fn fetch<'a>(&'a self, fetch: Fetch<'a>) -> Fetching<'a> {
        Box::pin(async move {
            let token = fetch.keys.get("token").map(String::as_str).unwrap_or_default();
            if token.is_empty() {
                return Err(FetchError { code: ErrorCode::Auth, message: "no token".into(), retry_at: None });
            }
            if let Some(fail) = fetch.settings.get("fail").filter(|f| !f.is_empty()) {
                let code = match fail.as_str() {
                    "rate_limited" => ErrorCode::RateLimited,
                    "network" => ErrorCode::Network,
                    _ => ErrorCode::Auth,
                };
                return Err(FetchError { code, message: format!("token {token} was refused"), retry_at: None });
            }
            let seed: u64 = fetch.settings.get("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
            let today = day_number(fetch.today).ok_or_else(|| FetchError { code: ErrorCode::Other, message: "bad day".into(), retry_at: None })?;
            let mut points = Vec::new();
            for back in 0..fetch.days.min(KEEPS) {
                let n = today - i64::from(back);
                let day = day_string(n);
                let visits = visits(seed, n);
                points.push(Point { metric: "visits".into(), day: day.clone(), value: visits as f64, breakdown: None });
                points.push(Point { metric: "stars".into(), day: day.clone(), value: stars(seed, n) as f64, breakdown: None });
                let mut split: BTreeMap<String, f64> = BTreeMap::new();
                let mut left = visits;
                for (i, (label, share)) in LANGUAGES.iter().enumerate() {
                    let v = if i + 1 == LANGUAGES.len() { left } else { (visits * share / 100).min(left) };
                    left -= v;
                    split.insert((*label).into(), v as f64);
                }
                points.push(Point { metric: "languages".into(), day, value: visits as f64, breakdown: Some(split) });
            }
            Ok(points)
        })
    }
}

/// A day's visits: a slow rise, a weekly swing, a little noise.
pub fn visits(seed: u64, day: i64) -> u64 {
    let d = day.max(0) as u64;
    let noise = (d.wrapping_mul(2_654_435_761).wrapping_add(seed.wrapping_mul(40_503))) % 23;
    100 + seed % 50 + (d % 365) / 2 + [0, 12, 18, 15, 9, 30, 25][(d % 7) as usize] + noise
}

/// The stars by that day: one more every third day.
pub fn stars(seed: u64, day: i64) -> u64 {
    seed % 10 + (day.max(0) as u64 % 3650) / 3
}
