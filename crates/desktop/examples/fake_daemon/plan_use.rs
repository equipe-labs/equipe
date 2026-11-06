//! Where the plans went, made up: a week of hours on each engine — the
//! person's own Claude Code sessions in three folders, the squad's tickets in
//! web, api and app, the PM's desk — added up by equipe core's own report
//! (`equipe_core::plan_use::report`), so the Usage screen shows what the
//! daemon would. T-42 burns fast three hours ago (1,204 calls re-reading its
//! long context) and the person's `~/code/site` session spiked last night.
//!
//! `FAKE_BURN=1` (also with `FAKE_USAGE=week61`) tells T-42's burn to a
//! screen that attaches, as the notice over every screen.

use std::time::Duration;

use equipe_core::plan_use::ledger::Used;
use equipe_core::plan_use::report::{self, Place, Row, Squads, TicketInfo, Whose};
use equipe_core::protocol::plan_use::{Burn, Mostly, Span, Tokens, Who};
use equipe_core::protocol::{Command, Engine, Event, PlanWindow};
use tokio::sync::broadcast;

const H: i64 = 3600;
const SQUAD: &str = "demo";

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty() && v != "0")
}

pub fn burning() -> bool {
    env("FAKE_BURN").is_some() || env("FAKE_USAGE").as_deref() == Some("week61")
}

fn squads() -> Squads {
    let mut s = Squads { home: "/fake/.equipe".into(), user_home: Some("/fake".into()), ..Default::default() };
    for (id, title, who, project) in [
        (42, "Checkout page payment method choice", "frontend", "web"),
        (47, "Product detail page image gallery", "frontend", "web"),
        (70, "Orders API pagination", "backend", "api"),
        (60, "Three onboarding screen drafts", "designer", "app"),
        (72, "Order search index", "backend", "api"),
    ] {
        s.tickets.insert((SQUAD.into(), id), TicketInfo { title: title.into(), assignee: Some(who.into()), project: Some(project.into()) });
    }
    for role in ["pm", "frontend", "backend", "designer"] {
        s.roles.insert((SQUAD.into(), role.into()), role.into());
    }
    s
}

fn ticket(id: u32, member: &str, project: &str) -> Place {
    Place::Equipe(Whose { squad: Some(SQUAD.into()), member: Some(member.into()), ticket: Some(id), project: Some(project.into()), home: false })
}

fn desk() -> Place {
    Place::Equipe(Whose { squad: Some(SQUAD.into()), member: Some("pm".into()), home: true, ..Default::default() })
}

/// A long conversation's request mix: mostly cache reads, some writes, a little output.
fn reread(read: u64, calls: u64) -> Used {
    Used { tokens: Tokens { input: read / 4000, output: read / 120, cache_write: read / 40, cache_read: read }, long: read / 40, calls }
}

/// A fresh one: more context loaded, more written.
fn loading(read: u64, calls: u64) -> Used {
    Used { tokens: Tokens { input: read / 1000, output: read / 60, cache_write: read / 6, cache_read: read }, long: read / 6, calls }
}

/// A week of hours on `engine`: during the day the squad works, evenings the person's own sessions.
fn rows(engine: Engine, now: i64) -> Vec<Row> {
    let top = now - now.rem_euclid(H);
    let mut rows = Vec::new();
    let model = if engine == Engine::Claude { "claude-opus-5-5" } else { "gpt-5.6-sol" };
    let row = |hours_ago: i64, place: Place, used: Used| Row { hour: top - hours_ago * H, model: model.into(), place, used };
    let scale = if engine == Engine::Claude { 1 } else { 4 };
    for h in 0..(7 * 24) {
        // A steady hum, quieter at night.
        let quiet = (h % 24) >= 14;
        let base = if quiet { 400_000 } else { 2_400_000 } / scale;
        match h % 5 {
            0 => rows.push(row(h, ticket(47, "frontend", "web"), reread(base, 12))),
            1 => rows.push(row(h, ticket(70, "backend", "api"), reread(base * 3 / 2, 18))),
            2 => rows.push(row(h, desk(), reread(base / 2, 6))),
            3 => rows.push(row(h, ticket(60, "designer", "app"), loading(base, 9))),
            _ => rows.push(row(h, ticket(72, "backend", "api"), reread(base, 10))),
        }
        // The person's own Claude Code, every evening, and a long one last night.
        if engine == Engine::Claude {
            let own = if (10..15).contains(&h) { 9_000_000 } else if (h % 24) < 6 { 2_600_000 } else { 0 };
            if own > 0 {
                rows.push(row(h, Place::Outside("~/code/site".into()), reread(own, 40 + own / 100_000)));
            }
            if h % 24 == 7 {
                rows.push(row(h, Place::Outside("~/notes".into()), loading(1_200_000, 14)));
            }
            if h % 48 == 30 {
                rows.push(row(h, Place::Outside("~/scratch/scripts".into()), reread(800_000, 9)));
            }
        }
    }
    // T-42, three hours ago: a long context read again and again.
    for (h, read, calls) in [(3, 22_000_000 / scale, 402), (2, 26_000_000 / scale, 431), (1, 19_000_000 / scale, 371)] {
        rows.push(row(h, ticket(42, "frontend", "web"), reread(read, calls)));
    }
    rows.sort_by_key(|r| r.hour);
    rows
}

fn week(engine: Engine) -> Option<PlanWindow> {
    super::switching::plans().into_iter().find(|p| p.engine == engine).and_then(|p| p.usage.seven_day)
}

/// The look a screen asked for.
pub fn report(engine: Engine, span: Span) -> equipe_core::protocol::plan_use::Report {
    let now = now();
    let rows = rows(engine, now);
    let squads = squads();
    report::build(&report::Input { engine, span, now, week: week(engine), rows: &rows, squads: &squads, reading: false })
}

pub fn on_command(command: &Command, events: &broadcast::Sender<Event>) -> bool {
    let Command::PlanUse { engine, span } = command else { return false };
    let (engine, span, events) = (*engine, *span, events.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = events.send(Event::PlanUse { report: Box::new(report(engine, span)) });
    });
    true
}

/// T-42's burn, told as the daemon tells it.
pub fn attached(events: broadcast::Sender<Event>) {
    if !burning() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        // As the daemon finds it, from the same hours; told as made up when the plan isn't.
        let now = now();
        let found = report::burns(Engine::Claude, &rows(Engine::Claude, now), &squads(), week(Engine::Claude), now).into_iter().next();
        let burn = found.unwrap_or(Burn {
            id: "claude:demo/T-42".into(),
            engine: Engine::Claude,
            who: Who::Ticket { squad: Some(SQUAD.into()), ticket: 42 },
            label: Some("Checkout page payment method choice".into()),
            week_bp: 720,
            hours: 3,
            at: now,
            calls: 1204,
            mostly: Mostly::Rereading,
        });
        let _ = events.send(Event::PlanBurn { burn });
    });
}
