//! `equipe flow`: how long the person waits for what they ask — request →
//! first work → first merge → production, tickets and hand-offs per request,
//! meta-ticket share and queue wait (`equipe_core::lead_time`, docs/FLOW.md).
//! The numbers are computed in core from the squad's journal, opened without
//! writing; this only prints them (or, with `--json`, hands them over whole).

use clap::Args;
use equipe_core::lead_time::{self, Report, Stat, percent, span};
use equipe_core::t;
use serde_json::json;

#[derive(Args)]
pub struct FlowArgs {
    /// The squad (default: the one this folder belongs to, else your first)
    #[arg(long)]
    pub squad: Option<String>,
    /// From when: 48h, 7d, 2w, or a day (2026-11-01)
    #[arg(long, default_value = "7d")]
    pub since: String,
    /// Read this journal file instead of the squad's (a copy, say)
    #[arg(long)]
    pub db: Option<std::path::PathBuf>,
    /// Every number, as JSON
    #[arg(long)]
    pub json: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn command(args: FlowArgs) -> Result<(), String> {
    let (db, squad) = match args.db {
        Some(db) => {
            let full = std::fs::canonicalize(&db).unwrap_or_else(|_| db.clone());
            let name = full.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (db, name)
        }
        None => {
            let home = equipe_providers::credentials::equipe_home();
            let cwd = std::env::current_dir().unwrap_or_default();
            let squad = crate::home::squad_for(&home, &cwd, args.squad).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
            (equipe_core::journal::squad_dir(&home, &squad).join("squad.db"), squad)
        }
    };
    if !db.is_file() {
        return Err(t!("design.qa.no_journal", squad = squad));
    }
    let until = now().max(0) as u64;
    let since = equipe_core::design::qa_stats::since(&args.since, now()).ok_or_else(|| t!("usage.bad_since", since = args.since))?.max(0) as u64;
    let report = lead_time::summarize(&lead_time::read(&db, since, until)?, since, until);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "squad": squad, "report": report })).unwrap_or_default());
    } else {
        println!("{}", text(&squad, &report));
    }
    Ok(())
}

/// A median and p90 of durations, as people read them.
fn times(s: &Stat) -> String {
    match (s.median, s.p90) {
        (Some(median), Some(p90)) => t!("flow.times", median = span(median), p90 = span(p90), count = s.count),
        _ => "–".to_owned(),
    }
}

/// A median and p90 of counts.
fn counts(s: &Stat) -> String {
    match (s.median, s.p90) {
        (Some(median), Some(p90)) => t!("flow.counts", median = median, p90 = p90),
        _ => "–".to_owned(),
    }
}

/// The report, as printed.
pub(crate) fn text(squad: &str, r: &Report) -> String {
    let date = equipe_core::squad_memory::date;
    let mut out = vec![t!("flow.heading", squad = squad, from = date(r.since), to = date(r.until))];
    let source = if r.source == "requests" { t!("flow.source_requests") } else { t!("flow.source_reconstructed") };
    out.push(t!("flow.requests", count = r.requests, source = source));
    out.push(t!("flow.to_start", value = times(&r.to_start)));
    out.push(t!("flow.to_first_merge", value = times(&r.to_first_merge)));
    out.push(t!("flow.to_production", value = times(&r.to_production), deploys = r.production_by_deploy));
    out.push(t!("flow.tickets_per_request", value = counts(&r.tickets_per_request)));
    out.push(t!("flow.handoffs_per_request", value = counts(&r.handoffs_per_request)));
    out.push(t!("flow.queue_wait", value = times(&r.queue_wait)));
    out.push(t!("flow.ticket_to_merge", value = times(&r.ticket_to_merge)));
    out.push(t!("flow.meta", meta = r.meta_tickets, all = r.tickets_created, share = percent(r.meta_share_permille), tokens = percent(r.meta_token_share_permille)));
    let creators = r.by_creator.iter().map(|(who, n)| format!("{who} {n}")).collect::<Vec<_>>().join(", ");
    out.push(t!("flow.creators", creators = if creators.is_empty() { "–".to_owned() } else { creators }));
    out.push(t!("flow.batons", batons = r.baton_passes, landed = r.self_landed));
    if !r.slowest.is_empty() {
        out.push(String::new());
        out.push(t!("flow.slowest").to_owned());
        for l in &r.slowest {
            let live = l.to_production.map_or_else(|| t!("flow.not_live").to_owned(), span);
            out.push(t!("flow.slow_row", title = l.title, live = live, tickets = l.tickets, handoffs = l.handoffs));
        }
    }
    out.join("\n")
}
