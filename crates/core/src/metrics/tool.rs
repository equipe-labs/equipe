//! `metrics`: a member reads the squad's numbers — a period beside the one
//! before it — from the snapshots the daemon keeps. Read only: connecting,
//! keys and fetching are the person's, on their screens. Nothing it answers
//! comes from the Keychain.
//!
//! `metrics_query`: the PM, data and marketing members ask one read-only
//! HogQL `SELECT` through the squad's PostHog connection
//! ([`super::Metrics::hogql`]). The daemon sends it with the key it keeps;
//! the member sees rows, never the key.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use super::query;
use crate::routines::Clock as _;
use crate::protocol::metrics::{Bucket, HogqlParams, HogqlResult, QueryParams, Shape};

pub const METRICS: &str = "metrics";
pub const QUERY: &str = "metrics_query";

/// Rows a member's query answers when it doesn't say, and at most.
const QUERY_ROWS: u32 = 50;
const QUERY_MOST_ROWS: u32 = 500;

/// The roles that ask ad-hoc queries: the PM and the data-minded ones.
pub fn may_query(role: &str) -> bool {
    matches!(role, crate::roles::PM | "data" | crate::roles::MARKETING)
}

pub fn query_tool() -> Value {
    json!({
        "name": QUERY,
        "description": "Ask the squad's connected PostHog one read-only HogQL query — a single SELECT (or WITH … SELECT) \
over `events`, `persons` and the other HogQL tables — for numbers the `metrics` tool doesn't keep: a funnel, an event's \
count by day, where people drop off. equipe sends it with the key it keeps; you get rows, never the key. Anything but one \
SELECT is refused; no comments. Rows are capped (50 unless `limit` says, 500 at most) and each connection has an hourly \
budget, so aggregate in the query (count(), uniq(), GROUP BY) rather than pulling raw events, and keep personal data — \
names, emails — out of what you ask and what you write down.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "hogql": { "type": "string", "description": "The SELECT, in HogQL." },
                "connection": { "type": "string", "description": "The connection's id (m1…) when the squad has several PostHog ones." },
                "project": { "type": "string", "description": "Only a connection of this project." },
                "limit": { "type": "integer", "description": "Rows answered at most: 50 unless said, 500 at most." },
            },
            "required": ["hogql"],
        },
    })
}

/// A member's `metrics_query` call as the daemon's ask.
pub fn query_params(squad: &str, args: &Value) -> Result<HogqlParams, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let query = text("hogql").ok_or("Give the query as `hogql`: one SELECT.")?;
    let limit = args.get("limit").and_then(Value::as_u64).map_or(QUERY_ROWS, |l| l.clamp(1, u64::from(QUERY_MOST_ROWS)) as u32);
    Ok(HogqlParams { squad: squad.to_owned(), connection: text("connection"), project: text("project"), query, limit: Some(limit) })
}

/// A query's rows as a member reads them: a Markdown table, long cells clipped.
pub fn query_answer(r: &HogqlResult) -> String {
    let cell = |v: &Value| {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Null => "null".into(),
            other => other.to_string(),
        };
        let s = s.replace('|', "\\|").replace('\n', " ");
        if s.chars().count() > super::hogql::CELL { format!("{}…", s.chars().take(super::hogql::CELL).collect::<String>()) } else { s }
    };
    let mut out = format!("{} rows from {} in {} ms{}.\n\n", r.rows.len(), r.connection, r.took_ms, if r.truncated { format!(" (more rows were there: only the first {})", r.limit) } else { String::new() });
    if r.rows.is_empty() {
        out.push_str("No rows.");
        return out;
    }
    let width = r.columns.len().max(r.rows.iter().map(Vec::len).max().unwrap_or(0));
    let names: Vec<String> = (0..width).map(|i| r.columns.get(i).cloned().unwrap_or_else(|| format!("col{}", i + 1))).collect();
    let _ = writeln!(out, "| {} |", names.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(width));
    for row in &r.rows {
        let cells: Vec<String> = (0..width).map(|i| row.get(i).map_or_else(String::new, cell)).collect();
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out
}

/// Labels of a breakdown shown, at most.
const SHARES: usize = 6;

pub fn tool() -> Value {
    json!({
        "name": METRICS,
        "description": "Read the product's numbers the person connected (PostHog, GitHub, App Store Connect): each metric \
over a period beside the period just before it, with the change, the latest day known and when it was last fetched. \
Read only — the person connects services and their keys on their own screen; ask them for a connection that is missing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Only this project's connections." },
                "days": { "type": "integer", "description": "The period's length in days, 1 to 366; default 7." },
                "to": { "type": "string", "description": "The period's last day, YYYY-MM-DD; default today." },
                "metric": { "type": "string", "description": "Only this metric, by name (visits, stars…)." },
                "by_week": { "type": "boolean", "description": "Show the series by week rather than by day." },
            },
        },
    })
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// The answer to a call, from the snapshots under `home`.
pub fn answer(home: &Path, squad: &str, args: &Value, today: i64) -> Result<String, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let p = QueryParams {
        squad: squad.to_owned(),
        project: text("project"),
        days: args.get("days").and_then(Value::as_u64).map(|d| d.min(10_000) as u32),
        to: text("to"),
        bucket: if args.get("by_week").and_then(Value::as_bool) == Some(true) { Bucket::Week } else { Bucket::Day },
        connections: Vec::new(),
        metrics: text("metric").into_iter().collect(),
    };
    let r = query(home, &p, today).map_err(|e| e.message)?;
    if r.connections.is_empty() {
        return Ok("No service is connected for these numbers yet. The person connects PostHog, GitHub or App Store Connect \
in equipe's Metrics screen; ask them if you need one."
            .into());
    }
    let mut out = format!("{} to {} ({} days), beside {} to {}.\n", r.from, r.to, r.days, r.previous_from, r.previous_to);
    for c in &r.connections {
        let project = c.project.as_deref().map(|p| format!(" · {p}")).unwrap_or_default();
        let _ = write!(out, "\n## {} ({}){project}", super::kind_label(c.kind), c.id);
        match (&c.last, c.last_ok) {
            (Some(l), _) if l.error.is_some() => {
                let e = l.error.as_ref().expect("checked");
                let _ = write!(out, " — the last fetch failed ({:?}: {}); the numbers are the last kept", e.code, e.message);
            }
            (_, Some(ok)) => {
                let t = crate::routines::Local.parts(ok);
                let _ = write!(out, " — fetched {:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute);
            }
            _ => out.push_str(" — not fetched yet"),
        }
        out.push('\n');
        for v in r.values.iter().filter(|v| v.connection == c.id) {
            let value = v.value.map_or_else(|| "no data".to_owned(), number);
            let previous = v.previous.map_or_else(|| "no data".to_owned(), number);
            let change = match (v.change, v.change_pct) {
                (Some(d), Some(pct)) => format!(", {}{} ({}{}%)", if d >= 0.0 { "+" } else { "" }, number(d), if pct >= 0.0 { "+" } else { "" }, number(pct)),
                (Some(d), None) => format!(", {}{}", if d >= 0.0 { "+" } else { "" }, number(d)),
                _ => String::new(),
            };
            let total = match v.shape {
                Shape::Total => ", running total",
                Shape::Rolling => ", 7 days before; each value counts the 7 days up to its day",
                _ => "",
            };
            let as_of = v.as_of.as_deref().map(|d| format!(", latest day {d}")).unwrap_or_default();
            let _ = writeln!(out, "- {} `{}`: {value} {} (before: {previous}{change}{total}; {}/{} days known{as_of})", v.label, v.metric, v.unit, v.days_known, r.days);
            for s in v.breakdown.iter().take(SHARES) {
                let before = s.previous.map(|p| format!(" (before {})", number(p))).unwrap_or_default();
                let _ = writeln!(out, "  - {}: {}{before}", s.label, number(s.value));
            }
            if !v.series.is_empty() && v.shape != Shape::Breakdown {
                let series: Vec<String> = v.series.iter().map(|s| format!("{} {}", &s.day[5..], s.value.map_or_else(|| "–".to_owned(), number))).collect();
                let by = match (v.shape, r.bucket) {
                    (Shape::Rolling, _) => "7 days",
                    (_, Bucket::Week) => "week",
                    _ => "day",
                };
                let _ = writeln!(out, "  - by {by}: {}", series.join(", "));
            }
        }
    }
    Ok(out)
}
