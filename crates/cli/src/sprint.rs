//! `equipe sprint show | end | set`: the sprint loop (docs/SPRINTS.md) from a
//! terminal on the daemon's machine — this sprint, what the last one taught,
//! the next one proposed; end the sprint under way now; change the loop's
//! cadence, budget or switch.

use clap::{Args, Subcommand};
use equipe_core::protocol::sprint::{self as wire, Ended, SetParams, SprintParams, SprintView};
use equipe_core::t;

#[derive(Subcommand)]
pub enum SprintAction {
    /// This sprint, what the last one taught and the next one proposed
    Show(ShowArgs),
    /// Review the sprint under way now (or plan a first one)
    End(SquadArgs),
    /// Change the loop: on, off or automatic; the review's day and hour; 7 or 14 days; the budget
    Set(SetArgs),
}

#[derive(Args)]
pub struct SquadArgs {
    /// The squad
    #[arg(long)]
    pub squad: String,
}

#[derive(Args)]
pub struct ShowArgs {
    /// The squad
    #[arg(long)]
    pub squad: String,
    /// Everything as JSON: sprints, items, hypotheses and their results
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct SetArgs {
    /// The squad
    #[arg(long)]
    pub squad: String,
    /// Run the loop now, even before anything is live
    #[arg(long, conflicts_with_all = ["off", "auto"])]
    pub on: bool,
    /// Stop the loop
    #[arg(long, conflicts_with_all = ["on", "auto"])]
    pub off: bool,
    /// Back to automatic: on once the product is live
    #[arg(long, conflicts_with_all = ["on", "off"])]
    pub auto: bool,
    /// The review's day: 1 Monday … 7 Sunday
    #[arg(long)]
    pub weekday: Option<u8>,
    /// The review's hour, 0–23, this machine's local time
    #[arg(long)]
    pub hour: Option<u8>,
    /// Days a sprint runs: 7 or 14
    #[arg(long)]
    pub length: Option<u32>,
    /// Tokens one sprint may use; 0 for equipe's default
    #[arg(long)]
    pub budget: Option<u64>,
}

pub async fn command(action: SprintAction) -> Result<(), String> {
    let socket = equipe_core::gateway::socket_path(&equipe_providers::credentials::equipe_home());
    match action {
        SprintAction::Show(a) => {
            let v: SprintView = ask(&socket, wire::GET, &SprintParams { squad: a.squad.clone() }).await?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
            } else {
                print!("{}", listing(&a.squad, &v));
            }
            Ok(())
        }
        SprintAction::End(a) => {
            let ended: Ended = ask(&socket, wire::END, &SprintParams { squad: a.squad.clone() }).await?;
            match ended.why {
                None => println!("{}", t!("sprint.ended", squad = a.squad)),
                Some(why) => return Err(t!("sprint.not_ended", why = why)),
            }
            Ok(())
        }
        SprintAction::Set(a) => {
            let p = SetParams {
                squad: a.squad.clone(),
                on: if a.on { Some(true) } else if a.off { Some(false) } else { None },
                auto: a.auto,
                weekday: a.weekday,
                hour: a.hour,
                length_days: a.length,
                budget_tokens: a.budget,
            };
            let v: SprintView = ask(&socket, wire::SET, &p).await?;
            println!("{}", t!("sprint.saved"));
            print!("{}", listing(&a.squad, &v));
            Ok(())
        }
    }
}

async fn ask<P: serde::Serialize, R: serde::de::DeserializeOwned>(socket: &std::path::Path, method: &str, p: &P) -> Result<R, String> {
    match equipe_core::gateway::sprint(socket, method, p).await {
        Ok(Some(answer)) => serde_json::from_value(answer).map_err(|e| e.to_string()),
        Ok(None) => Err(t!("metrics_query.no_daemon").to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

fn day(unix: u64) -> String {
    equipe_core::metrics::day_string((unix / 86_400) as i64)
}

/// The loop in a few lines: its switch and cadence, then each section.
fn listing(squad: &str, v: &SprintView) -> String {
    let tokens = equipe_core::sprint::tokens;
    let mut out = match &v.why_off {
        None => format!("{}\n", t!("sprint.loop_on", squad = squad, weekday = v.weekday, hour = v.hour, days = v.length_days, budget = tokens(v.budget_tokens))),
        Some(why) => format!("{}\n", t!("sprint.loop_off", squad = squad, why = why)),
    };
    if let Some(n) = &v.north_star {
        out.push_str(&format!("{}\n", t!("sprint.north_star", name = n.name)));
    }
    if let Some(c) = &v.current {
        out.push_str(&format!("\n{} S-{} · {}\n", t!("sprint.this"), c.id, c.goal));
        out.push_str(&format!("  {}\n", t!("sprint.status", done = v.current_done, total = c.tickets.len(), spent = tokens(c.spent_tokens), budget = tokens(c.budget_tokens), review = c.ends_at.map(day).unwrap_or_default())));
        for x in v.experiments.iter().filter(|x| c.experiments.contains(&x.id)) {
            out.push_str(&format!("  {}\n", equipe_core::sprint::experiment_line(x)));
        }
    }
    if let Some(p) = &v.proposed {
        out.push_str(&format!("\n{} S-{} · {}\n", t!("sprint.next"), p.id, p.goal));
        let mut items: Vec<_> = v.items.iter().filter(|i| p.items.contains(&i.id)).collect();
        items.sort_by_key(|i| i.rank);
        for i in items {
            let mark = if i.recommended { " ★" } else { "" };
            out.push_str(&format!("  P-{} {} — {} ({:?}, ~{}){mark}\n", i.id, i.title, i.expected, i.size, tokens(i.cost_tokens)));
        }
        out.push_str(&format!("  {}\n", t!("sprint.waits")));
    }
    if let Some(l) = &v.last {
        out.push_str(&format!("\n{} S-{} · {}\n", t!("sprint.learned"), l.id, l.goal));
        if let Some(r) = &l.retro {
            for line in r.shipped.iter().chain(&r.learned).chain(&r.wrong).chain(&r.next) {
                out.push_str(&format!("  · {line}\n"));
            }
        }
        for x in v.experiments.iter().filter(|x| l.experiments.contains(&x.id) && x.result.is_some()) {
            out.push_str(&format!("  {}\n", equipe_core::sprint::experiment_line(x)));
        }
    }
    if v.current.is_none() && v.proposed.is_none() && v.last.is_none() {
        out.push_str(&format!("{}\n", t!("sprint.none", squad = squad)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::sprint::{Phase, Sprint};

    #[test]
    fn the_listing_says_the_switch_then_each_section() {
        let sprint = Sprint { id: 3, goal: "More sign-ups".into(), done_when: "d".into(), phase: Phase::Active, proposed_at: 0, started_at: Some(0), ends_at: Some(86_400 * 20_420), closed_at: None, items: vec![], tickets: vec![1, 2], experiments: vec![], budget_tokens: 25_000_000, spent_tokens: 3_000_000, set: 0, retro: None, facts: None };
        let v = SprintView { on: true, weekday: 5, hour: 17, length_days: 7, budget_tokens: 25_000_000, current: Some(sprint), current_done: 1, ..SprintView::default() };
        let text = listing("acme", &v);
        assert!(text.contains("S-3 · More sign-ups"), "{text}");
        assert!(text.contains("3M") && text.contains("25M"), "{text}");
        let off = SprintView { why_off: Some("nothing is live yet".into()), ..SprintView::default() };
        assert!(listing("acme", &off).contains("nothing is live yet"));
    }
}
