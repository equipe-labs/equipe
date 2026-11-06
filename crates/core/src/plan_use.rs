//! Where the plans went: what each engine's plan was used on, from every
//! Claude Code and Codex transcript on this machine — equipe's members' and
//! the person's own — and the members' turns the squads' journals keep.
//!
//! - [`weights`]: a request's load, from its tokens and the model's prices.
//! - [`transcripts`]: reading the engines' transcripts on, incrementally.
//! - [`ledger`]: what they say, hour by hour (`<home>/plan-use.db`).
//! - [`report`]: who used what — outside equipe, a project, a ticket, a
//!   member — the report a screen shows, and the burns going on now.
//!
//! Everything here blocks (files, SQLite): the runtime calls it off its loop.

pub mod ledger;
pub mod report;
pub mod transcripts;
pub mod weights;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::protocol::plan_use::{Burn, Report, Span, Who};
use crate::protocol::{Engine, PlanWindow};

/// The ledger's file.
pub fn ledger_path(home: &Path) -> PathBuf {
    home.join("plan-use.db")
}

/// How long a look a screen waits for may read transcripts first; the rest
/// is read by the next pass.
pub const ASKED_BUDGET: Duration = Duration::from_secs(3);

/// Read on in the transcripts under `roots` (within `budget`, if given).
/// Whether a whole pass has been made yet.
pub fn refresh(home: &Path, roots: &transcripts::Roots, now: i64, budget: Option<Duration>) -> Result<bool, String> {
    let ledger = ledger::Ledger::open(&ledger_path(home)).map_err(|e| e.to_string())?;
    transcripts::scan(&ledger, roots, now, budget);
    Ok(transcripts::read_once(&ledger))
}

fn rows_of(home: &Path, engine: Engine, since: i64) -> (Vec<report::Row>, report::Squads) {
    let squads = report::Squads::load(home, since);
    let hours = ledger::Ledger::open(&ledger_path(home)).and_then(|l| l.hours(report::engine_word(engine), since - 3600)).unwrap_or_default();
    (report::rows(engine, hours, &squads, since), squads)
}

/// Where `engine`'s plan went over `span`, with its weekly window as last read.
pub fn look(home: &Path, engine: Engine, span: Span, week: Option<PlanWindow>, now: i64, reading: bool) -> Report {
    let since = now - 8 * 24 * 3600;
    let (rows, squads) = rows_of(home, engine, since);
    report::build(&report::Input { engine, span, now, week, rows: &rows, squads: &squads, reading })
}

/// The burns going on now on each engine's plan whose week is read.
pub fn burns(home: &Path, weeks: &[(Engine, PlanWindow)], now: i64) -> Vec<Burn> {
    let since = now - 8 * 24 * 3600;
    weeks
        .iter()
        .flat_map(|(engine, week)| {
            let (rows, squads) = rows_of(home, *engine, since);
            report::burns(*engine, &rows, &squads, Some(*week), now)
        })
        .collect()
}

// ── the PM's look ──

/// The PM's read-only tool: what burned the plans. Answered by the runtime,
/// but not in the PM's tools yet (`board::tools`): offering it changes what
/// the PM is told, which waits on the quality gate (docs/quality-gate.md)
/// and its pinned prompts (`crates/core/tests/members_without_phone.rs`).
pub const TOOL: &str = "plan_use";

pub fn tool() -> serde_json::Value {
    serde_json::json!({
        "name": TOOL,
        "description": "Read where the person's Claude and Codex plans went: for each engine, the share of its current \
weekly window (or the last day) taken by the person's own sessions outside equipe, by each project, ticket and member, \
the hours it burned fastest, and when the week runs out at this pace. Read-only; it changes nothing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "span": { "type": "string", "enum": ["week", "day"], "description": "The plan's week so far (the default) or the last 24 hours." },
            },
        },
    })
}

fn pct(bp: u32) -> String {
    format!("{:.1}%", f64::from(bp) / 100.0)
}

fn who_words(who: &Who, label: Option<&str>) -> String {
    let named = |base: String| match label {
        Some(l) => format!("{base} ({l})"),
        None => base,
    };
    match who {
        Who::Outside { folder: None } => "outside equipe (the person's own Claude Code / Codex sessions)".into(),
        Who::Outside { folder: Some(f) } => format!("outside equipe, in {f}"),
        Who::Ticket { ticket, .. } => named(format!("T-{ticket}")),
        Who::Member { member, squad } => named(format!("{member} in {squad}")),
        Who::Project { project, .. } => project.clone(),
        Who::Home { squad } => format!("{squad}'s own folder"),
        Who::Equipe => "equipe (other)".into(),
    }
}

/// The report, as the PM reads it: plain lines, the heaviest first.
pub fn answer(reports: &[Report]) -> String {
    let mut out = Vec::new();
    for r in reports {
        let engine = report::engine_word(r.engine);
        let share = |s: &crate::protocol::plan_use::Slice| s.week_bp.map_or_else(|| format!("{} of what was seen", pct(s.share_bp)), |w| format!("{} of the week", pct(w)));
        let mut lines = vec![match r.week {
            Some(w) => format!("{engine}: {}% of the week used; it starts over at {} (Unix).", w.used_percent, w.resets_at),
            None => format!("{engine}: the plan's use hasn't been read; shares are of what was seen."),
        }];
        if let Some(p) = r.pace {
            lines.push(if p.before_reset {
                format!("At this pace ({} of the week a day) it runs out at {} (Unix), before it starts over.", pct(p.per_day_bp), p.runs_out_at)
            } else {
                format!("At this pace ({} of the week a day) it lasts until it starts over.", pct(p.per_day_bp))
            });
        }
        if r.load == 0 {
            lines.push("Nothing used in this stretch.".into());
            out.push(lines.join("\n"));
            continue;
        }
        if let Some(o) = &r.outside {
            lines.push(format!("- {}: {}, {} requests", who_words(&o.who, None), share(o), o.calls));
            for c in o.children.iter().take(3) {
                lines.push(format!("  - {}: {}", who_words(&c.who, None), share(c)));
            }
        }
        for (title, list) in [("Projects", &r.projects), ("Tickets", &r.tickets), ("Members", &r.members)] {
            if list.is_empty() {
                continue;
            }
            lines.push(format!("{title}:"));
            for s in list.iter().take(8) {
                lines.push(format!("- {}: {}, {} requests, mostly {}", who_words(&s.who, s.label.as_deref()), share(s), s.calls, mostly_words(s.mostly)));
            }
        }
        if !r.spikes.is_empty() {
            lines.push("Fastest burns:".into());
            for sp in &r.spikes {
                lines.push(format!("- {}: {} in {} hours from {} (Unix), {} requests", who_words(&sp.slice.who, sp.slice.label.as_deref()), share(&sp.slice), (sp.to - sp.from) / 3600, sp.from, sp.slice.calls));
            }
        }
        if r.reading {
            lines.push("(The machine's transcripts are still being read for the first time: this is short of it.)".into());
        }
        out.push(lines.join("\n"));
    }
    if out.is_empty() { "No plan's use is known yet.".into() } else { out.join("\n\n") }
}

fn mostly_words(mostly: crate::protocol::plan_use::Mostly) -> &'static str {
    use crate::protocol::plan_use::Mostly;
    match mostly {
        Mostly::Rereading => "re-reading a long context",
        Mostly::Loading => "loading new context",
        Mostly::Writing => "writing",
        Mostly::Input => "fresh input",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::plan_use::{Mostly as Kind, Slice, Tokens};

    #[test]
    fn the_pm_reads_the_heaviest_first_in_plain_words() {
        let slice = |who: Who, label: Option<&str>, week_bp: u32| Slice { who, label: label.map(str::to_owned), load: 1, share_bp: 0, week_bp: Some(week_bp), tokens: Tokens::default(), calls: 1204, mostly: Kind::Rereading, children: vec![] };
        let report = Report {
            engine: Engine::Claude,
            span: Span::Week,
            from: 0,
            to: 0,
            week: Some(PlanWindow { used_percent: 61, resets_at: 99 }),
            pace: Some(crate::protocol::plan_use::Pace { runs_out_at: 50, before_reset: true, per_day_bp: 1500 }),
            load: 10,
            week_load: 10,
            tokens: Tokens::default(),
            calls: 0,
            outside: Some(slice(Who::Outside { folder: None }, None, 3300)),
            projects: vec![],
            tickets: vec![slice(Who::Ticket { squad: Some("acme".into()), ticket: 427 }, Some("Checkout"), 700)],
            members: vec![],
            beats: vec![],
            spikes: vec![],
            reading: false,
        };
        let said = answer(&[report]);
        assert!(said.contains("61% of the week used"), "{said}");
        assert!(said.contains("runs out at 50"), "{said}");
        assert!(said.contains("outside equipe (the person's own Claude Code / Codex sessions): 33.0% of the week"), "{said}");
        assert!(said.contains("T-427 (Checkout): 7.0% of the week, 1204 requests, mostly re-reading a long context"), "{said}");
    }
}
