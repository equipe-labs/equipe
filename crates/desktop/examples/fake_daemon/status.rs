//! The Status screen's data (`squad_status`), made up. `FAKE_STATUS`:
//! `healthy` (the default) — work going, a few things waiting for you;
//! `slowed` — Codex out of its limit with runs paused, Claude pacing, one
//! ticket handed back and one gone quiet; `stopped` — the squad's breaker is
//! open (the disk is full), nothing starts; `empty` — a squad with no work
//! and no usage yet; `old` — a daemon that predates the request ("unknown
//! method": the sidebar's Status row hides); `waiting` — five runs going
//! while 14 decisions block 31 tickets and one ticket is ready: the speed
//! isn't what holds the work (`FAKE_PARALLEL=max` shows full speed); `held` —
//! six ready tickets held by the cap. Pinning a ticket to watch
//! (`Command::WatchTicket`) is kept and listed.

use equipe_core::protocol::status::{Action, Block, Cause, EngineBudget, Flow, Goto, Handling, Limit, Mood, Note, Outlook, Severity, Speed, SpeedOutlook, SquadStatus, Tiles, Waste, WasteKey, WasteMetric, Watched, WatchedList};
use equipe_core::protocol::{Engine, PauseWhy, PlanWindow, Priority, TicketId, TicketStatus};

const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;

pub fn mode() -> String {
    std::env::var("FAKE_STATUS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "healthy".into())
}

/// Whether the daemon is one that doesn't know the request.
pub fn old() -> bool {
    mode() == "old"
}

fn pins() -> &'static std::sync::Mutex<Vec<TicketId>> {
    static PINS: std::sync::OnceLock<std::sync::Mutex<Vec<TicketId>>> = std::sync::OnceLock::new();
    PINS.get_or_init(Default::default)
}

/// The person pinned a ticket to watch, or let it go.
pub fn pin(ticket: TicketId, watch: bool) {
    let mut pins = pins().lock().unwrap();
    pins.retain(|t| *t != ticket);
    if watch {
        pins.push(ticket);
    }
}

fn block(cause: Cause, severity: Severity, count: u32, handling: Handling) -> Block {
    Block { cause, severity, count, why: None, engine: None, at: None, detail: None, tickets: Vec::new(), handling, goto: None }
}

fn engine(engine: Engine, plan: &str, used: u8, resets_in: u64, now: u64) -> EngineBudget {
    EngineBudget {
        engine,
        plan_type: Some(plan.into()),
        weekly: Some(PlanWindow { used_percent: used, resets_at: (now + resets_in) as i64 }),
        five_hour: Some(PlanWindow { used_percent: used / 3, resets_at: (now + 2 * HOUR) as i64 }),
        out_until: None,
        runs_out_at: None,
        lasts: true,
        pacing_at_once: None,
        stale: false,
    }
}

fn watched(title: &str, id: TicketId, status: TicketStatus, owner: &str, since_ago: u64, priority: Priority, now: u64) -> Watched {
    Watched { ticket: id, title: title.into(), status, owner: Some(owner.into()), since: Some(now - since_ago), priority, paused: None }
}

fn metric(key: WasteKey, day: Option<f64>, week: Option<f64>, bad: bool) -> WasteMetric {
    WasteMetric { key, day, week, bad }
}

fn urgent(now: u64) -> Vec<Watched> {
    vec![
        watched("Product detail page image gallery", 47, TicketStatus::InProgress, "frontend", 95 * 60, Priority::Urgent, now),
        watched("Go to order history after paying", 45, TicketStatus::Todo, "frontend", 5 * HOUR, Priority::Urgent, now),
    ]
}

/// The status of the squad `squad`, in the mode `FAKE_STATUS` names.
pub fn status(squad: &str, now: u64) -> SquadStatus {
    let mode = mode();
    let mut s = SquadStatus { squad: squad.into(), at: now, ..SquadStatus::default() };
    s.tiles = Tiles { accepted_today: 7, accepted_2h: 2, in_progress: 3, runs_active: 3, runs_max: 8, needs_you: 4, decisions: 3, approvals: 1, waiting: 3, waiting_urgent: 1 };
    s.engines = vec![engine(Engine::Claude, "max", 38, 4 * DAY + 3 * HOUR, now), engine(Engine::Codex, "plus", 22, 5 * DAY, now)];
    s.waste = Waste {
        tokens_day: 412_000_000,
        turns_day: 96,
        metrics: vec![
            metric(WasteKey::PmDeskContext, Some(182_000.0), Some(204_000.0), false),
            metric(WasteKey::HeavyShare, Some(6.0), Some(8.0), false),
            metric(WasteKey::CancelledShare, Some(1.0), Some(1.0), false),
            metric(WasteKey::ReworkShare, Some(2.0), Some(3.0), false),
            metric(WasteKey::FirstHandInAccept, Some(78.0), Some(74.0), false),
        ],
    };
    s.blocked = vec![
        Block { goto: Some(Goto::NeedsYou), ..block(Cause::Approvals, Severity::Info, 1, Handling::You { action: Action::Answer }) },
        Block { goto: Some(Goto::NeedsYou), ..block(Cause::Decisions, Severity::Info, 3, Handling::You { action: Action::Answer }) },
    ];
    s.watched = WatchedList { pinned: false, tickets: urgent(now), ..WatchedList::default() };
    match mode.as_str() {
        "slowed" => {
            s.mood = Mood::Slowed;
            s.tiles = Tiles { accepted_today: 4, accepted_2h: 0, in_progress: 14, runs_active: 2, runs_max: 4, needs_you: 5, decisions: 4, approvals: 1, waiting: 9, waiting_urgent: 3 };
            s.engines[0].weekly = Some(PlanWindow { used_percent: 83, resets_at: (now + 2 * DAY + 5 * HOUR) as i64 });
            s.engines[0].runs_out_at = Some(now + DAY + 2 * HOUR);
            s.engines[0].lasts = false;
            s.engines[0].pacing_at_once = Some(4);
            s.engines[1].weekly = Some(PlanWindow { used_percent: 100, resets_at: (now + 3 * DAY) as i64 });
            s.engines[1].out_until = Some(now + 3 * DAY);
            s.engines[1].runs_out_at = None;
            s.engines[1].lasts = false;
            s.notes = vec![Note::Out { engine: Engine::Codex, back_at: Some(now + 3 * DAY) }, Note::Pacing { engine: Engine::Claude, at_once: 4 }];
            s.blocked = vec![
                Block { engine: Some(Engine::Codex), at: Some(now + 3 * DAY), tickets: vec![70, 72], ..block(Cause::EngineOut, Severity::Amber, 12, Handling::Fixed) },
                Block { goto: Some(Goto::Ticket(61)), at: Some(now - 3 * HOUR), tickets: vec![61], ..block(Cause::HandedBack, Severity::Amber, 1, Handling::You { action: Action::Look }) },
                Block { goto: Some(Goto::Ticket(47)), at: Some(now - 150 * 60), tickets: vec![47], ..block(Cause::Quiet, Severity::Amber, 1, Handling::You { action: Action::Look }) },
                block(Cause::Failing, Severity::Amber, 4, Handling::Resolving),
                Block { why: Some(PauseWhy::Retrying), engine: Some(Engine::Claude), at: Some(now + 120), tickets: vec![60], ..block(Cause::RunsPaused, Severity::Amber, 2, Handling::Resolving) },
                Block { goto: Some(Goto::NeedsYou), ..block(Cause::Approvals, Severity::Amber, 1, Handling::You { action: Action::Answer }) },
                Block { goto: Some(Goto::NeedsYou), ..block(Cause::Decisions, Severity::Amber, 4, Handling::You { action: Action::Answer }) },
            ];
            s.waste.metrics = vec![
                metric(WasteKey::PmDeskContext, Some(628_000.0), Some(410_000.0), true),
                metric(WasteKey::HeavyShare, Some(31.0), Some(22.0), true),
                metric(WasteKey::CancelledShare, Some(2.0), Some(2.0), false),
                metric(WasteKey::ReworkShare, Some(7.0), Some(4.0), true),
                metric(WasteKey::FirstHandInAccept, Some(52.0), Some(71.0), true),
            ];
        }
        "stopped" => {
            s.mood = Mood::Stopped;
            s.tiles = Tiles { accepted_today: 3, accepted_2h: 0, in_progress: 3, runs_active: 0, runs_max: 8, needs_you: 1, decisions: 1, approvals: 0, waiting: 5, waiting_urgent: 1 };
            s.blocked = vec![
                Block { detail: Some("No space left on device (os error 28)".into()), at: Some(now - 25 * 60), tickets: vec![47, 60, 70], ..block(Cause::BreakerOpen, Severity::Red, 3, Handling::Resolving) },
                Block { goto: Some(Goto::NeedsYou), ..block(Cause::Decisions, Severity::Amber, 1, Handling::You { action: Action::Answer }) },
            ];
        }
        "empty" => {
            s.empty = true;
            s.tiles = Tiles::default();
            s.blocked = Vec::new();
            s.engines = Vec::new();
            s.waste = Waste::default();
            s.watched = WatchedList::default();
        }
        _ => {}
    }
    // How the speed would change the week: at full speed Claude's runs out on the
    // weekend, balanced it lasts until it starts over; Codex's pace is the work's.
    let outlook = vec![
        SpeedOutlook { engine: Engine::Claude, full: Some(Outlook { runs_out_at: Some(now + 2 * DAY + 3 * HOUR), lasts: false }), balanced: Some(Outlook { runs_out_at: Some(now + 3 * DAY + 20 * HOUR), lasts: false }) },
        SpeedOutlook { engine: Engine::Codex, full: Some(Outlook { runs_out_at: None, lasts: true }), balanced: Some(Outlook { runs_out_at: None, lasts: true }) },
    ];
    s.speed = Some(Speed { parallel: super::switching::parallel(), outlook });
    s.flow = Some(match mode.as_str() {
        "waiting" | "slowed" => Flow { running: 5, starting: 1, on_tickets: 15, on_you: 14, behind_you: 31, limit: Limit::You, ..Flow::default() },
        "held" => Flow { running: 8, held_by_cap: 6, on_tickets: 12, on_you: 3, behind_you: 2, limit: Limit::Cap, ..Flow::default() },
        "stopped" => Flow { running: 0, starting: 0, on_tickets: 4, on_you: 1, limit: Limit::Tickets, ..Flow::default() },
        "empty" => Flow { limit: Limit::Idle, ..Flow::default() },
        _ => Flow { running: 3, starting: 1, on_tickets: 6, on_you: 2, behind_you: 1, limit: Limit::Tickets, ..Flow::default() },
    });
    match mode.as_str() {
        "waiting" => {
            s.tiles = Tiles { accepted_today: 9, accepted_2h: 3, in_progress: 5, runs_active: 5, runs_max: 8, needs_you: 14, decisions: 13, approvals: 1, waiting: 47, waiting_urgent: 1 };
            s.blocked = vec![
                Block { goto: Some(Goto::NeedsYou), ..block(Cause::Approvals, Severity::Info, 1, Handling::You { action: Action::Answer }) },
                Block { goto: Some(Goto::NeedsYou), ..block(Cause::Decisions, Severity::Info, 13, Handling::You { action: Action::Answer }) },
            ];
            s.engines[0].weekly = Some(PlanWindow { used_percent: 61, resets_at: (now + 4 * DAY + 3 * HOUR) as i64 });
            s.engines[0].lasts = false;
            s.engines[0].runs_out_at = Some(now + 3 * DAY + 20 * HOUR);
            s.engines[1].out_until = Some(now + 5 * DAY);
            s.notes = vec![Note::Out { engine: Engine::Codex, back_at: Some(now + 5 * DAY) }];
        }
        "held" => {
            s.tiles = Tiles { accepted_today: 6, accepted_2h: 1, in_progress: 8, runs_active: 8, runs_max: 8, needs_you: 3, decisions: 3, approvals: 0, waiting: 18, waiting_urgent: 2 };
            s.mood = Mood::Slowed;
            s.blocked = vec![Block { goto: Some(Goto::NeedsYou), ..block(Cause::Decisions, Severity::Info, 3, Handling::You { action: Action::Answer }) }];
            s.engines[0].pacing_at_once = Some(8);
            s.notes = vec![Note::Pacing { engine: Engine::Claude, at_once: 8 }];
        }
        _ => {}
    }
    let pinned = pins().lock().unwrap().clone();
    if !pinned.is_empty() && mode != "empty" {
        let known = [
            watched("Product detail page image gallery", 47, TicketStatus::InProgress, "frontend", 95 * 60, Priority::Urgent, now),
            watched("Go to order history after paying", 45, TicketStatus::Todo, "frontend", 5 * HOUR, Priority::Urgent, now),
            watched("Checkout page payment method choice", 42, TicketStatus::Review, "frontend", 40 * 60, Priority::High, now),
            watched("Orders API pagination", 70, TicketStatus::InProgress, "backend", 3 * HOUR, Priority::Normal, now),
        ];
        s.watched = WatchedList { pinned: true, pins: pinned.clone(), tickets: pinned.iter().filter_map(|id| known.iter().find(|w| w.ticket == *id).cloned()).collect(), more: 0 };
    }
    s
}
