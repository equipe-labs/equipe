//! Ad-hoc read-only queries through a connection (`metrics.hogql`,
//! docs/METRICS.md "Ad-hoc queries"): one `SELECT` the person (or a PM or data
//! member, through `metrics_query`) writes, sent by the daemon with the key it
//! keeps — the key never leaves the daemon, and every error has it cut out.
//!
//! What is sent is checked here, in code, before anything leaves the machine:
//! a single `SELECT` (or `WITH … SELECT`), no comments, no second statement,
//! no statement word that writes or changes anything, no table function that
//! reaches outside. Rows are capped, the whole ask is timed, and each
//! connection has an hourly budget and a gap between asks, on top of the
//! adapter's own patience with a busy service.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Rows answered when the ask doesn't say, and at most.
pub const DEFAULT_ROWS: u32 = 200;
pub const MAX_ROWS: u32 = 10_000;
/// The longest query accepted, in characters.
pub const MAX_QUERY: usize = 20_000;
/// How long one ask may take, waiting its turn included.
pub const WAIT: Duration = Duration::from_secs(if cfg!(test) { 5 } else { 60 });
/// Ad-hoc asks per connection in any hour: PostHog's query endpoint allows a
/// few more, and the daily fetch needs some of them.
pub const PER_HOUR: usize = 60;
/// A cell longer than this is clipped in a member's answer.
pub const CELL: usize = 200;

/// Words that write, change or reach past a read — refused anywhere outside a
/// string or a quoted name, whatever their case.
const FORBIDDEN: [&str; 21] = [
    "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "TRUNCATE", "RENAME", "ATTACH", "DETACH", "GRANT", "REVOKE", "KILL",
    "OPTIMIZE", "SET", "SETTINGS", "USE", "EXCHANGE", "INTO", "OUTFILE", "UPSERT",
];

/// Table functions that read from outside the project — refused when called.
const OUTSIDE: [&str; 14] =
    ["url", "file", "remote", "remotesecure", "s3", "s3cluster", "hdfs", "mysql", "postgresql", "jdbc", "odbc", "input", "executable", "cluster"];

/// One word of a query outside its strings and quoted names, and whether a
/// `(` follows it.
struct Word {
    text: String,
    called: bool,
}

/// The words of `query` outside strings and quoted names — or why it isn't
/// one plain statement: a comment, a `;` before the end, an open quote.
fn words(query: &str) -> Result<Vec<Word>, String> {
    let chars: Vec<char> = query.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' | '`' => {
                let close = c;
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("a quote is left open".into()),
                        Some('\\') => i += 2,
                        Some(&q) if q == close && chars.get(i + 1) == Some(&close) => i += 2,
                        Some(&q) if q == close => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => return Err("comments aren't accepted: send the query without them".into()),
            '/' if chars.get(i + 1) == Some(&'*') => return Err("comments aren't accepted: send the query without them".into()),
            '#' => return Err("comments aren't accepted: send the query without them".into()),
            ';' => return Err("one statement only: no `;` inside the query".into()),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '$' => {
                let start = i;
                while chars.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let mut j = i;
                while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                    j += 1;
                }
                out.push(Word { text, called: chars.get(j) == Some(&'(') });
            }
            _ => i += 1,
        }
    }
    Ok(out)
}

/// `query` as it may be sent — trimmed, one trailing `;` dropped — when it is
/// a single read-only `SELECT`; else why not.
pub fn select_only(query: &str) -> Result<String, String> {
    let query = query.trim();
    let query = query.strip_suffix(';').unwrap_or(query).trim_end();
    if query.is_empty() {
        return Err("the query is empty: send one SELECT".into());
    }
    if query.chars().count() > MAX_QUERY {
        return Err(format!("the query is longer than {MAX_QUERY} characters"));
    }
    if query.contains('\0') {
        return Err("the query has a NUL character".into());
    }
    let words = words(query)?;
    let first = words.first().map(|w| w.text.to_ascii_uppercase()).unwrap_or_default();
    if first != "SELECT" && first != "WITH" {
        return Err(format!("only a SELECT is sent (read-only); this starts with `{}`", words.first().map_or("", |w| w.text.as_str())));
    }
    if !words.iter().any(|w| w.text.eq_ignore_ascii_case("SELECT")) {
        return Err("only a SELECT is sent (read-only)".into());
    }
    if let Some(w) = words.iter().find(|w| FORBIDDEN.iter().any(|f| w.text.eq_ignore_ascii_case(f))) {
        return Err(format!("only a SELECT is sent (read-only): `{}` isn't accepted", w.text));
    }
    if let Some(w) = words.iter().find(|w| w.called && OUTSIDE.iter().any(|f| w.text.eq_ignore_ascii_case(f))) {
        return Err(format!("`{}(…)` reads from outside the project and isn't accepted", w.text));
    }
    Ok(query.to_owned())
}

/// The rows asked for: the default when unsaid, never more than [`MAX_ROWS`].
pub fn rows_asked(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS)
}

/// A checked query with a `LIMIT` of one more row than `rows` when it has
/// none of its own — the extra row says there were more.
pub fn capped(query: &str, rows: u32) -> String {
    let has_limit = words(query).map(|w| w.iter().any(|w| w.text.eq_ignore_ascii_case("LIMIT"))).unwrap_or(true);
    if has_limit { query.to_owned() } else { format!("{query}\nLIMIT {}", u64::from(rows) + 1) }
}

/// Every string in `rows` with the keys cut out.
pub fn redact_rows(rows: &mut [Vec<Value>], keys: &BTreeMap<String, String>) {
    fn walk(v: &mut Value, keys: &BTreeMap<String, String>) {
        match v {
            Value::String(s) => *s = super::redact(s, keys),
            Value::Array(a) => a.iter_mut().for_each(|v| walk(v, keys)),
            Value::Object(o) => o.values_mut().for_each(|v| walk(v, keys)),
            _ => {}
        }
    }
    rows.iter_mut().flatten().for_each(|v| walk(v, keys));
}

/// Each connection's ad-hoc asks: one at a time over the daemon, a gap
/// between two asks of a connection, and at most [`PER_HOUR`] an hour.
#[derive(Default)]
pub struct Pacing {
    turn: tokio::sync::Mutex<()>,
    sent: Mutex<HashMap<String, VecDeque<u64>>>,
    last: Mutex<HashMap<String, Instant>>,
}

impl Pacing {
    /// Count an ask of `connection` at `now` (Unix seconds) against its hour,
    /// or the seconds until one is free.
    pub fn take(&self, connection: &str, now: u64) -> Result<(), u64> {
        let mut sent = self.sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let times = sent.entry(connection.to_owned()).or_default();
        while times.front().is_some_and(|t| *t + 3600 <= now) {
            times.pop_front();
        }
        if times.len() >= PER_HOUR {
            return Err(times.front().map_or(60, |t| (t + 3600).saturating_sub(now).max(1)));
        }
        times.push_back(now);
        Ok(())
    }

    /// Wait for the daemon's turn and `connection`'s gap; the turn is held
    /// until the guard is dropped.
    pub async fn wait_turn(&self, connection: &str, gap: Duration) -> tokio::sync::MutexGuard<'_, ()> {
        let turn = self.turn.lock().await;
        let since = self.last.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(connection).map(Instant::elapsed);
        if let Some(since) = since
            && since < gap
        {
            tokio::time::sleep(gap - since).await;
        }
        self.last.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(connection.to_owned(), Instant::now());
        turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_select_is_accepted_as_it_is() {
        for q in [
            "SELECT count() FROM events",
            "select event, count() AS n from events where timestamp > now() - interval 7 day group by event order by n desc",
            "  SELECT 1;  ",
            "WITH s AS (SELECT distinct_id FROM events WHERE event = 'question_sent') SELECT count() FROM s",
            "SELECT properties.$current_url, replaceAll(event, 'a', 'b') FROM events LIMIT 10",
            "SELECT 'delete me; drop table x -- not a comment' AS s, \"insert\" FROM events",
            "SELECT count() FROM events WHERE event = 'it\\'s' AND properties.note = 'a''b'",
            "SELECT url AS page FROM events",
        ] {
            assert!(select_only(q).is_ok(), "{q}: {:?}", select_only(q));
        }
        assert_eq!(select_only("  SELECT 1;  ").unwrap(), "SELECT 1");
    }

    #[test]
    fn anything_but_one_select_is_refused() {
        for q in [
            "",
            ";",
            "DELETE FROM events",
            "INSERT INTO events VALUES (1)",
            "DROP TABLE events",
            "ALTER TABLE events DELETE WHERE 1",
            "TRUNCATE events",
            "CREATE TABLE x AS SELECT 1",
            "SHOW TABLES",
            "SYSTEM FLUSH LOGS",
            "SELECT 1; DROP TABLE events",
            "SELECT 1; SELECT 2",
            "SELECT 1 -- hidden\n",
            "SELECT 1 /* hidden */",
            "SELECT 1 # hidden",
            "SELECT * FROM events SETTINGS max_execution_time = 9999",
            "SELECT * INTO OUTFILE 'x' FROM events",
            "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
            "SELECT * FROM url('https://example.com/x.csv', CSV)",
            "SELECT * FROM s3 ('https://bucket/x', 'CSV')",
            "SELECT * FROM remote('host', db.t)",
            "SELECT 'open",
            "select 1;;",
            "sElEcT 1;dElEtE FROM events",
            "EXPLAIN SELECT 1",
            "WITH 1 AS x",
            "SELECT 1 UPDATE",
            "set x = 1",
        ] {
            assert!(select_only(q).is_err(), "{q:?} should be refused");
        }
        assert!(select_only(&format!("SELECT '{}'", "a".repeat(MAX_QUERY))).is_err(), "too long");
    }

    #[test]
    fn a_limit_is_added_only_when_missing() {
        assert_eq!(capped("SELECT 1", 200), "SELECT 1\nLIMIT 201");
        assert_eq!(capped("SELECT 1 FROM events limit 5", 200), "SELECT 1 FROM events limit 5");
        assert_eq!(capped("SELECT 'limit' FROM events", 10), "SELECT 'limit' FROM events\nLIMIT 11", "a string isn't a LIMIT");
        assert_eq!(rows_asked(None), DEFAULT_ROWS);
        assert_eq!(rows_asked(Some(0)), 1);
        assert_eq!(rows_asked(Some(1_000_000)), MAX_ROWS);
    }

    #[test]
    fn each_connection_has_an_hourly_budget() {
        let pacing = Pacing::default();
        for k in 0..PER_HOUR as u64 {
            assert!(pacing.take("m1", 1_000 + k).is_ok());
        }
        let wait = pacing.take("m1", 1_100).unwrap_err();
        assert_eq!(wait, 3600 - 100, "free again an hour after the first");
        assert!(pacing.take("m2", 1_100).is_ok(), "another connection's budget is its own");
        assert!(pacing.take("m1", 1_000 + 3600).is_ok(), "an hour later, room again");
    }

    #[test]
    fn keys_are_cut_out_of_rows() {
        let keys = BTreeMap::from([("api_key".to_owned(), "phx_secret_key_1234".to_owned())]);
        let mut rows = vec![vec![Value::from("a phx_secret_key_1234 b"), serde_json::json!({ "k": ["phx_secret_key_1234"] }), Value::from(3)]];
        redact_rows(&mut rows, &keys);
        assert!(!serde_json::to_string(&rows).unwrap().contains("phx_secret_key_1234"));
        assert_eq!(rows[0][2], Value::from(3));
    }
}
