//! `equipe metrics query`: one read-only HogQL `SELECT` through a squad's
//! PostHog connection, asked of the daemon on this machine (`metrics.hogql`,
//! docs/METRICS.md "Ad-hoc queries"). The daemon checks it is a single
//! `SELECT`, sends it with the key it keeps in the Keychain, caps the rows and
//! the time and cuts the key out of any error; this command never sees the
//! key. Run on the daemon's machine, over SSH too.

use clap::{Args, Subcommand};
use equipe_core::protocol::metrics::{HogqlParams, HogqlResult};
use equipe_core::t;
use serde_json::Value;

#[derive(Subcommand)]
pub enum MetricsAction {
    /// Ask a squad's PostHog connection one read-only HogQL SELECT, through the daemon (the key stays in it)
    Query(QueryArgs),
}

#[derive(Args)]
pub struct QueryArgs {
    /// The squad whose connection is asked
    #[arg(long)]
    pub squad: String,
    /// The connection's id (m1…), when the squad has several PostHog connections
    #[arg(long)]
    pub connection: Option<String>,
    /// Only a connection of this project
    #[arg(long)]
    pub project: Option<String>,
    /// The query: one SELECT (or WITH … SELECT), no comments
    #[arg(long)]
    pub hogql: String,
    /// Rows answered at most (default 200, at most 10000)
    #[arg(long)]
    pub limit: Option<u32>,
    /// The answer as JSON: columns, rows, whether it was cut, the query as sent
    #[arg(long)]
    pub json: bool,
}

pub async fn command(action: MetricsAction) -> Result<(), String> {
    match action {
        MetricsAction::Query(args) => query(args).await,
    }
}

async fn query(a: QueryArgs) -> Result<(), String> {
    // Checked here for a quick answer; the daemon checks again before sending.
    let query = equipe_core::metrics::hogql::select_only(&a.hogql)?;
    let socket = equipe_core::gateway::socket_path(&equipe_providers::credentials::equipe_home());
    let p = HogqlParams { squad: a.squad, connection: a.connection, project: a.project, query, limit: a.limit };
    let r = match equipe_core::gateway::metrics_hogql(&socket, &p).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(t!("metrics_query.no_daemon").to_owned()),
        Err(e) => return Err(e.to_string()),
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?);
        return Ok(());
    }
    print!("{}", table(&r));
    eprintln!("{}", t!("metrics_query.rows", rows = r.rows.len(), connection = r.connection, ms = r.took_ms));
    if r.truncated {
        eprintln!("{}", t!("metrics_query.truncated", limit = r.limit, most = equipe_core::metrics::hogql::MAX_ROWS));
    }
    Ok(())
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.replace(['\n', '\t'], " "),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// The rows as aligned columns, a header first.
fn table(r: &HogqlResult) -> String {
    let width = r.columns.len().max(r.rows.iter().map(Vec::len).max().unwrap_or(0));
    if width == 0 {
        return String::new();
    }
    let names: Vec<String> = (0..width).map(|i| r.columns.get(i).cloned().unwrap_or_else(|| format!("col{}", i + 1))).collect();
    let rows: Vec<Vec<String>> = r.rows.iter().map(|row| (0..width).map(|i| row.get(i).map_or_else(String::new, cell)).collect()).collect();
    let sizes: Vec<usize> = (0..width).map(|i| rows.iter().map(|r| r[i].chars().count()).chain([names[i].chars().count()]).max().unwrap_or(0).min(60)).collect();
    let line = |cells: &[String]| {
        let parts: Vec<String> = cells
            .iter()
            .zip(&sizes)
            .map(|(c, w)| {
                let c: String = if c.chars().count() > *w { format!("{}…", c.chars().take(w.saturating_sub(1)).collect::<String>()) } else { c.clone() };
                format!("{c:<w$}")
            })
            .collect();
        format!("{}\n", parts.join("  ").trim_end())
    };
    let mut out = line(&names);
    out.push_str(&line(&sizes.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>()));
    for row in &rows {
        out.push_str(&line(row));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::metrics::SourceKind;

    #[test]
    fn rows_print_as_aligned_columns() {
        let r = HogqlResult {
            connection: "m1".into(),
            kind: SourceKind::Posthog,
            query: "SELECT 1".into(),
            columns: vec!["event".into(), "n".into()],
            rows: vec![vec![Value::from("question_sent"), Value::from(42)], vec![Value::from("x"), Value::Null]],
            truncated: false,
            limit: 200,
            took_ms: 5,
        };
        assert_eq!(table(&r), "event          n\n-------------  --\nquestion_sent  42\nx              null\n");
    }
}
