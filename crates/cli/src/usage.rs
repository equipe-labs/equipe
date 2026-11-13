//! `equipe usage`: where a squad's tokens went and what looks wasted
//! (`equipe_core::usage`, docs/USAGE.md). The numbers are computed in core
//! from the squad's journal, opened without writing; this only prints them
//! (or, with `--json`, hands them over whole). `equipe usage digest on|off`
//! switches the daily usage note.

use clap::{Args, Subcommand};
use equipe_core::t;
use equipe_core::usage::{self, Bucket, Report, Row, TicketRow, amount};
use serde_json::json;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct UsageArgs {
    #[command(subcommand)]
    pub action: Option<UsageAction>,
    /// The squad (default: the one this folder belongs to, else your first)
    #[arg(long)]
    pub squad: Option<String>,
    /// From when: 7d, 24h, 2w, or a day (2026-11-01)
    #[arg(long, default_value = "7d")]
    pub since: String,
    /// Compare with the window of this length before --since (`--since 7d --compare 7d`: this week against the one before)
    #[arg(long)]
    pub compare: Option<String>,
    /// Every measure and signal, as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum UsageAction {
    /// The daily usage note (sent only when a waste signal is over its threshold): `on`, `off`, or nothing to see it
    Digest {
        /// `on` or `off`
        state: Option<String>,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn command(args: UsageArgs) -> Result<(), String> {
    match args.action {
        Some(UsageAction::Digest { state, squad }) => digest(squad, state),
        None => report(args.squad, &args.since, args.compare.as_deref(), args.json),
    }
}

fn squad_of(named: Option<String>) -> Result<(std::path::PathBuf, String), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, named).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    Ok((home, squad))
}

fn report(squad: Option<String>, since_text: &str, compare: Option<&str>, json: bool) -> Result<(), String> {
    let (home, squad) = squad_of(squad)?;
    let until = now().max(0) as u64;
    let since = equipe_core::design::qa_stats::since(since_text, now()).ok_or_else(|| t!("usage.bad_since", since = since_text))?.max(0) as u64;
    let db = equipe_core::journal::squad_dir(&home, &squad).join("squad.db");
    if !db.is_file() {
        return Err(t!("design.qa.no_journal", squad = squad));
    }
    let window = |from: u64, to: u64| -> Result<Report, String> { Ok(usage::summarize(&usage::read(&db, from, to.saturating_add(1))?, from, to)) };
    let report = window(since, until)?;
    // The window before: as long as `--compare` says, ending where this one starts.
    let before = match compare {
        Some(text) => {
            let from = equipe_core::design::qa_stats::since(text, now()).ok_or_else(|| t!("usage.bad_compare", compare = text))?;
            let length = (now() - from).max(1) as u64;
            Some(window(since.saturating_sub(length), since.saturating_sub(1))?)
        }
        None => None,
    };
    if json {
        let changes = before.as_ref().map(|b| usage::compare(b, &report));
        println!("{}", serde_json::to_string_pretty(&json!({ "squad": squad, "report": report, "before": before, "changes": changes })).unwrap_or_default());
    } else {
        println!("{}", text(&squad, since_text, &report));
        if let Some(before) = &before {
            println!("\n{}", compare_text(before, &report));
        }
    }
    Ok(())
}

/// A number as `compare` names it: a time as a span, a share with its %, the rest to a decimal.
fn value(name: &str, v: Option<f64>) -> String {
    let Some(v) = v else { return "–".to_owned() };
    match name {
        "tokens" | "tokens_per_done" => amount(v as u64),
        "time_to_done_median_secs" | "time_to_done_p75_secs" => equipe_core::logins::span(v as u64),
        "turns" | "accepted" => format!("{v:.0}"),
        _ => format!("{v:.1}"),
    }
}

/// Two windows side by side, with the change of each number in percent.
pub(crate) fn compare_text(before: &Report, after: &Report) -> String {
    let span = |r: &Report| format!("{} – {}", equipe_core::squad_memory::date(r.since), equipe_core::squad_memory::date(r.until));
    let mut out = vec![t!("usage.compare", after = span(after), before = span(before))];
    for c in usage::compare(before, after) {
        let change = c.change_percent.map_or_else(|| "–".to_owned(), |p| format!("{p:+.0}%"));
        let label = crate::i18n::catalog().get(&format!("usage.change.{}", c.name)).to_owned();
        out.push(t!("usage.compare_row", name = label, before = value(c.name, c.before), after = value(c.name, c.after), change = change));
    }
    out.join("\n")
}

fn digest(squad: Option<String>, state: Option<String>) -> Result<(), String> {
    let (home, squad) = squad_of(squad)?;
    let journal = equipe_core::journal::Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    if let Some(state) = state {
        let on = match state.as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(t!("usage.bad_digest").to_owned()),
        };
        journal.set_usage_digest(on).map_err(|e| e.to_string())?;
    }
    println!("{}", if journal.usage_digest_on() { t!("usage.digest_on", squad = squad) } else { t!("usage.digest_off", squad = squad) });
    Ok(())
}

fn kind_word(name: &str) -> String {
    match name {
        usage::kind::PM_DESK => t!("usage.kind.pm_desk"),
        usage::kind::MEMBER_ON_TICKET => t!("usage.kind.member_on_ticket"),
        usage::kind::LANE_ON_TICKET => t!("usage.kind.lane_on_ticket"),
        usage::kind::OTHER => t!("usage.kind.other_no_ticket"),
        other => other,
    }
    .to_owned()
}

fn line(name: &str, r: &Row) -> String {
    let (tokens, share) = (amount(r.tokens), format!("{:.1}", r.share_percent));
    match (r.lanes, r.context_per_call) {
        (0, None) => t!("usage.row", name = name, tokens = tokens, share = share, turns = r.turns),
        (0, Some(context)) => t!("usage.row_context", name = name, tokens = tokens, share = share, turns = r.turns, context = amount(context)),
        (lanes, _) => t!("usage.row_lanes", name = name, tokens = tokens, share = share, turns = r.turns, lanes = lanes),
    }
}

fn bucket_line(b: &Bucket) -> String {
    let name = if b.label == "unknown" { t!("usage.bucket_unknown").to_owned() } else { b.label.clone() };
    t!("usage.row", name = name, tokens = amount(b.tokens), share = format!("{:.1}", b.share_percent), turns = b.turns)
}

fn ticket_line(t: &TicketRow) -> String {
    t!("usage.ticket_row", id = t.id, title = t.title, tokens = amount(t.tokens), turns = t.turns, status = t.status)
}

fn counts(rows: &[(String, u64)]) -> String {
    if rows.is_empty() { "–".to_owned() } else { rows.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ") }
}

fn quality_lines(q: &usage::Quality) -> Vec<String> {
    let mut out = vec![t!("usage.q_accepted", count = q.accepted)];
    out.push(t!("usage.q_first", value = value("first_handin_accept_percent", q.first_handin_accept_percent) + if q.first_handin_accept_percent.is_some() { "%" } else { "" }, ok = q.first_handins_accepted, decided = q.first_handins_decided));
    out.push(t!("usage.q_send_backs", value = value("send_backs_per_accepted", q.send_backs_per_accepted), count = q.send_backs_on_accepted));
    if !q.send_backs_by_reason.is_empty() {
        out.push(t!("usage.q_reasons", reasons = counts(&q.send_backs_by_reason)));
    }
    if !q.design_qa_verdicts.is_empty() {
        out.push(t!("usage.q_design", verdicts = counts(&q.design_qa_verdicts), decisions = counts(&q.design_qa_decisions)));
    }
    out.push(t!("usage.q_time", median = value("time_to_done_median_secs", q.time_to_done_median_secs.map(|s| s as f64)), p75 = value("time_to_done_p75_secs", q.time_to_done_p75_secs.map(|s| s as f64))));
    out.push(t!("usage.q_cost", turns = value("turns_per_done", q.turns_per_done), tokens = value("tokens_per_done", q.tokens_per_done.map(|t| t as f64))));
    out
}

/// The report, as printed.
pub(crate) fn text(squad: &str, since: &str, r: &Report) -> String {
    if r.turns == 0 {
        return t!("usage.none", squad = squad, since = since);
    }
    let mut out = vec![t!("usage.heading", squad = squad, from = equipe_core::squad_memory::date(r.since), to = equipe_core::squad_memory::date(r.until), tokens = amount(r.tokens.total), turns = r.turns)];
    let k = &r.tokens;
    out.push(t!("usage.split", input = amount(k.input), output = amount(k.output), read = amount(k.cache_read), write = amount(k.cache_write)));
    if k.unsplit > 0 {
        out.push(t!("usage.unsplit", tokens = amount(k.unsplit)));
    }
    let section = |out: &mut Vec<String>, head: String, lines: Vec<String>| {
        if !lines.is_empty() {
            out.push(String::new());
            out.push(head);
            out.extend(lines);
        }
    };
    section(&mut out, t!("usage.by_day").to_owned(), r.by_day.iter().map(|d| line(&d.name, d)).collect());
    section(&mut out, t!("usage.by_model").to_owned(), r.by_model.iter().map(|m| line(&m.name, m)).collect());
    section(&mut out, t!("usage.by_member").to_owned(), r.by_member.iter().map(|m| line(&m.name, m)).collect());
    section(&mut out, t!("usage.by_kind").to_owned(), r.by_kind.iter().map(|m| line(&kind_word(&m.name), m)).collect());
    section(&mut out, t!("usage.context").to_owned(), r.context.iter().map(bucket_line).collect());
    section(&mut out, t!("usage.top_tickets").to_owned(), r.top_tickets.iter().map(ticket_line).collect());
    if r.cancelled_tokens > 0 {
        out.push(String::new());
        out.push(t!("usage.cancelled", tokens = amount(r.cancelled_tokens), share = format!("{:.1}", r.cancelled_share_percent)));
        out.extend(r.cancelled_tickets.iter().map(ticket_line));
    }
    section(&mut out, t!("usage.rework", turns = usage::REWORK_TURNS), r.rework.iter().map(ticket_line).collect());
    let words = equipe_core::i18n::core(crate::i18n::lang());
    let ending = |kind: &str| match kind {
        "limited" | "failed_fast" | "handed_back" => words.get(&format!("usage.ending.{kind}")).to_owned(),
        other => other.replace('_', " "),
    };
    section(&mut out, t!("usage.endings").to_owned(), r.endings.iter().map(|(kind, n)| format!("  {}: {n}", ending(kind))).collect());
    out.push(String::new());
    out.push(t!("usage.quality").to_owned());
    out.extend(quality_lines(&r.quality));
    out.push(String::new());
    out.push(t!("usage.signals").to_owned());
    if r.signals.is_empty() {
        out.push(format!("  {}", t!("usage.signals_none")));
    }
    for s in &r.signals {
        let (what, todo) = usage::say_signal(words, s);
        out.push(format!("- {what}"));
        out.push(format!("  {todo}"));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage::{Input, TicketInfo, TurnRow};

    fn turn(member: &str, role: &str, ticket: Option<u64>, read: u64, calls: u64) -> TurnRow {
        TurnRow { member: member.into(), role: role.into(), ticket, ended: 1_794_000_000, tokens: read + 100, output: 100, cache_read: read, calls, engine: Some("claude".into()), model: Some("claude-sonnet-5".into()), measured: true, ..TurnRow::default() }
    }

    #[test]
    fn the_report_names_the_split_the_kinds_and_each_signal_with_what_to_do() {
        assert!(text("acme", "7d", &Report::default()).contains("No turns in acme since 7d"));
        let tickets = [(1373, TicketInfo { title: "Dropped idea".into(), status: "cancelled".into() })].into();
        let turns = vec![turn("pm", "pm", None, 290_000_000, 290), turn("frontend-12", "frontend", Some(1373), 40_000_000, 400), turn("backend", "backend", Some(2), 670_000_000, 6_700)];
        let endings = vec![("limited".to_owned(), 4), ("handed_back".to_owned(), 2)];
        let r = usage::summarize(&Input { turns, tickets, endings, ..Input::default() }, 1_793_900_000, 1_794_100_000);
        let out = text("acme", "7d", &r);
        for want in ["Token usage of acme", "cache read", "PM desk (no ticket)", "lanes: 1", "T-1373 Dropped idea", "Waste signals", "1.0M tokens per call", "tickets that were cancelled afterwards: T-1373 Dropped idea", "hit a plan limit 4", ">400k", "Quality (same window):", "Tickets accepted: 0"] {
            assert!(out.contains(want), "{want}\n{out}");
        }
    }

    #[test]
    fn two_windows_print_side_by_side_with_the_change_in_percent() {
        let mut before = Report { quality: usage::Quality { accepted: 4, tokens_per_done: Some(1_000_000), time_to_done_median_secs: Some(7_200), ..Default::default() }, ..Report::default() };
        before.tokens.total = 2_000_000_000;
        let mut after = Report { quality: usage::Quality { accepted: 5, tokens_per_done: Some(750_000), time_to_done_median_secs: Some(3_600), ..Default::default() }, ..Report::default() };
        after.tokens.total = 1_000_000_000;
        let out = compare_text(&before, &after);
        for want in ["tokens: 2.0B → 1.0B (-50%)", "tokens per done ticket: 1.0M → 750k (-25%)", "tickets accepted: 4 → 5 (+25%)", "time to done, median: 2h → 1h (-50%)"] {
            assert!(out.contains(want), "{want}\n{out}");
        }
        assert!(out.contains("send-backs per accepted ticket: – → – (–)"), "{out}");
    }
}
