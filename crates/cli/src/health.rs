//! `equipe health`: what the squad's self-watch finds now (`equipe_core::health`,
//! docs/HEALTH.md) — stalled tickets, an idle squad and what it waits on, main
//! not pushed or not live, what waits on the person, tickets going in circles.
//! Worked out from the squad's journal and its projects' git, without the
//! daemon: what runs now is read from the runs' leases. `--json` hands the
//! findings over whole. `equipe health budget [<tokens>|off]` shows or sets
//! how many tokens one ticket may use before the PM decides (`equipe_core::spend`).

use clap::{Args, Subcommand};
use equipe_core::health::{self, Live, Snapshot};
use equipe_core::journal::Journal;
use equipe_core::spend;
use equipe_core::t;
use serde_json::json;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HealthArgs {
    #[command(subcommand)]
    pub action: Option<HealthAction>,
    /// The squad (default: the one this folder belongs to, else your first)
    #[arg(long)]
    pub squad: Option<String>,
    /// The findings as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum HealthAction {
    /// How many tokens one ticket may use before it goes to the PM: `30M`, `500k`, `off`, or nothing to see it
    Budget {
        tokens: Option<String>,
        /// The squad (default: the one this folder belongs to, else your first)
        #[arg(long)]
        squad: Option<String>,
    },
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn command(args: HealthArgs) -> Result<(), String> {
    match args.action {
        Some(HealthAction::Budget { tokens, squad }) => budget(squad, tokens),
        None => report(args.squad, args.json),
    }
}

fn journal_of(named: Option<String>) -> Result<(String, Journal), String> {
    let home = equipe_providers::credentials::equipe_home();
    let cwd = std::env::current_dir().unwrap_or_default();
    let squad = crate::home::squad_for(&home, &cwd, named).ok_or_else(|| t!("design.qa.no_squad").to_owned())?;
    if !equipe_core::journal::squad_dir(&home, &squad).join("squad.db").is_file() {
        return Err(t!("design.qa.no_journal", squad = squad));
    }
    let journal = Journal::for_squad(&home, &squad).map_err(|e| e.to_string())?;
    Ok((squad, journal))
}

fn report(squad: Option<String>, as_json: bool) -> Result<(), String> {
    let (squad, journal) = journal_of(squad)?;
    let now = now();
    let tickets = journal.tickets().map_err(|e| e.to_string())?;
    let live = Live::from_leases(&tickets, now);
    let watch = journal.deploy_watch();
    let deploys = health::DeployWatch(&watch);
    let repos = journal.projects().unwrap_or_default().iter().map(|p| health::repo_facts(p, &deploys)).collect();
    let findings = health::detect(&Snapshot::gather(&journal, tickets, now, live, &[], repos));
    let settings = journal.spend_settings();
    if as_json {
        let out = json!({ "squad": squad, "at": now, "findings": findings, "spend_guards": { "ticket_budget": settings.ticket_budget, "repeat_turns": settings.repeat_turns } });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    let words = equipe_core::i18n::core(crate::i18n::lang());
    println!("{}", words.fill("health.title", &[("squad", squad)]));
    if findings.is_empty() {
        println!("  {}", words.get("health.clear"));
    }
    for finding in &findings {
        println!("  - {}", finding.said(words));
    }
    println!("\n{}", budget_line(&settings));
    Ok(())
}

fn budget_line(settings: &spend::Settings) -> String {
    let words = equipe_core::i18n::core(crate::i18n::lang());
    let turns = settings.repeat_turns.to_string();
    if settings.ticket_budget == 0 {
        words.fill("health.budget_off", &[("turns", turns)])
    } else {
        words.fill("health.budget", &[("tokens", equipe_core::usage::amount(settings.ticket_budget)), ("turns", turns)])
    }
}

fn budget(squad: Option<String>, tokens: Option<String>) -> Result<(), String> {
    let (_, journal) = journal_of(squad)?;
    let mut settings = journal.spend_settings();
    if let Some(text) = tokens {
        let words = equipe_core::i18n::core(crate::i18n::lang());
        settings.ticket_budget = spend::parse_tokens(&text).ok_or_else(|| words.get("health.bad_budget").to_owned())?;
        journal.set_spend_settings(&settings).map_err(|e| e.to_string())?;
    }
    println!("{}", budget_line(&settings));
    Ok(())
}
