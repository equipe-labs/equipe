//! `equipe growth run | list`: the growth loop (docs/GROWTH.md) from a
//! terminal on the daemon's machine — start a run now, or list the squad's
//! cards with their evidence and measured outcomes.

use clap::{Args, Subcommand};
use equipe_core::protocol::growth::{Growth, GrowthParams, RunStarted};
use equipe_core::t;

#[derive(Subcommand)]
pub enum GrowthAction {
    /// Read the squad's product data now and hand it to the PM for recommendation cards
    Run(SquadArgs),
    /// The squad's growth cards: what each stood on, the answer, its ticket, what it did
    List(ListArgs),
}

#[derive(Args)]
pub struct SquadArgs {
    /// The squad
    #[arg(long)]
    pub squad: String,
}

#[derive(Args)]
pub struct ListArgs {
    /// The squad
    #[arg(long)]
    pub squad: String,
    /// Everything as JSON: each card's evidence, answer, ticket and follow-ups
    #[arg(long)]
    pub json: bool,
}

pub async fn command(action: GrowthAction) -> Result<(), String> {
    let socket = equipe_core::gateway::socket_path(&equipe_providers::credentials::equipe_home());
    match action {
        GrowthAction::Run(a) => {
            let started: RunStarted = ask(&socket, equipe_core::protocol::growth::RUN, &a.squad).await?;
            match started.why {
                None => println!("{}", t!("growth.started", squad = a.squad)),
                Some(why) => return Err(t!("growth.not_started", why = why)),
            }
            Ok(())
        }
        GrowthAction::List(a) => {
            let growth: Growth = ask(&socket, equipe_core::protocol::growth::LIST, &a.squad).await?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&growth).map_err(|e| e.to_string())?);
                return Ok(());
            }
            print!("{}", listing(&a.squad, &growth));
            Ok(())
        }
    }
}

async fn ask<R: serde::de::DeserializeOwned>(socket: &std::path::Path, method: &str, squad: &str) -> Result<R, String> {
    match equipe_core::gateway::growth(socket, method, &GrowthParams { squad: squad.to_owned() }).await {
        Ok(Some(answer)) => serde_json::from_value(answer).map_err(|e| e.to_string()),
        Ok(None) => Err(t!("metrics_query.no_daemon").to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

/// The cards, newest first: one line each, then its numbers and outcomes.
fn listing(squad: &str, g: &Growth) -> String {
    let when = g.last_run.map_or_else(|| t!("growth.never").to_owned(), |at| equipe_core::metrics::day_string((at / 86_400) as i64));
    let mut out = format!("{}\n", t!("growth.summary", squad = squad, when = when, spent = g.spent, cap = g.cap));
    if g.cards.is_empty() {
        out.push_str(&format!("{}\n", t!("growth.none", squad = squad)));
        return out;
    }
    for card in &g.cards {
        out.push_str(&format!("\n{}\n", equipe_core::growth::card_line(card)));
        for e in &card.evidence {
            out.push_str(&format!("  {} · {}\n", e.id, equipe_core::growth::evidence_line(e)));
        }
        for f in card.follow_ups.iter().filter(|f| f.outcome.is_some()) {
            if let Some(o) = &f.outcome {
                out.push_str(&format!("  {}\n", equipe_core::growth::outcome_line(card, f.after_days, o)));
            }
        }
    }
    out
}
