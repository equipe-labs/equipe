//! The squad's status at a glance (`squad_status`, the desktop's Status
//! screen): is the work going well, what slows or stops it, and what can the
//! person do about it — without reading logs.
//!
//! Two kinds of input, both cheap:
//!
//! - [`Facts`], what only the running daemon knows: the board, the squad's
//!   breaker, each engine's plan and pace, which runs are mid-turn, what waits
//!   for the person. The runtime gathers them in one pass
//!   (`runtime/status.rs`).
//! - [`Rows`], read from the squad's journal without writing to it: what was
//!   accepted today, when each ticket's last turn ended, the turn endings of
//!   the last hour, the pinned tickets, and the usage rows of the last seven
//!   days ([`crate::usage`]).
//!
//! [`compute`] is pure — facts and rows in, [`SquadStatus`] out — so every
//! rule is tested on fixtures. It answers in facts, never sentences: the
//! screen puts them in the person's language.
//!
//! The color: **red** when the squad's breaker is open (nothing starts) or
//! every engine members run on is out of its limit; **amber** when something
//! slows the work — a row of amber severity, or the squad running fewer at
//! once to make a plan last; else **green**. What only waits for the person
//! is quiet (info) while the work goes on, and amber once nothing runs
//! because of it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use equipe_protocol::status::{
    Action, Block, Cause, EngineBudget, Goto, Handling, Mood, Note, Severity, SquadStatus, Tiles, Waste, WasteKey, WasteMetric, Watched, WatchedList,
};
use equipe_protocol::{Engine, PauseWhy, Priority, Ticket, TicketId, TicketStatus};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::importance;
use crate::runtime::turn_ending;
use crate::usage::{self, Input, Report};

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// How long an answer is kept before the journal is read again.
pub const CACHE_FOR: Duration = Duration::from_secs(15);
/// A ticket in progress with no turn for this long is quiet.
pub const QUIET_AFTER: u64 = 2 * HOUR;
/// Failed turns in the last hour that make the failures a row.
pub const FAILING_MIN: u64 = 3;
/// The failures counted: the engine failed, or the turn ended with no end.
const FAILURES: [&str; 4] = [turn_ending::FAILED, turn_ending::FAILED_FAST, turn_ending::NO_END, turn_ending::TO_PM];
/// Rows of the blocked list; the rest is "+N more".
pub const BLOCKED_SHOWN: usize = 8;
/// Tickets of the watched list; the rest is "+N more".
pub const WATCHED_SHOWN: usize = 8;
/// Items of what waits on the person the status lists; the rest is "+N more".
pub const WAITING_SHOWN: usize = 5;
/// Tickets a row names.
const NAMED: usize = 3;
/// A plan this used makes the squad run fewer at once ([`crate::lanes`]).
pub const PACING_AT: u8 = 70;

/// What the squad's breaker says: why it opened, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breaker {
    /// What the engine said, in a line.
    pub cause: String,
    pub since: u64,
}

/// What only the running daemon knows.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub squad: String,
    /// Every ticket of the board.
    pub tickets: Vec<Ticket>,
    /// Members besides the PM.
    pub members: usize,
    pub breaker: Option<Breaker>,
    /// Each engine's plan, as read, with its pace and the squad's pacing.
    pub engines: Vec<EngineBudget>,
    /// The engines members run on.
    pub in_use: Vec<Engine>,
    /// Tickets whose run is mid-turn (or making its workspace) now.
    pub working: HashSet<TicketId>,
    /// How many runs the squad may have at once, by its engines' plans.
    pub runs_max: u32,
    /// Requests for permission waiting for the person.
    pub approvals: u32,
    /// Questions, proposals, cards to confirm and secrets waiting for the person.
    pub decisions: u32,
    /// Engines that need signing in again.
    pub signed_out: Vec<Engine>,
    /// The PM accepts handed-in work: the person sees only what it puts to them.
    pub pm_accepts: bool,
    /// What waits on the person besides tickets: requests, questions, cards,
    /// secrets — with the tickets they are about, for [`importance`].
    pub asks: Vec<importance::Ask>,
}

/// What the squad's journal says.
#[derive(Debug, Clone, Default)]
pub struct Rows {
    pub accepted_today: u32,
    pub accepted_2h: u32,
    /// When each ticket's last turn ended.
    pub last_turn: HashMap<TicketId, u64>,
    /// Turns that failed in the last hour.
    pub failures_hour: u64,
    /// The tickets the person pinned, in the order they pinned them.
    pub watched: Vec<TicketId>,
    /// The turns and tickets of the last seven days.
    pub usage: Input,
}

/// The cause's place on the list among rows of one severity.
fn rank(cause: Cause) -> u8 {
    match cause {
        Cause::BreakerOpen => 0,
        Cause::EngineOut => 1,
        Cause::SignIn => 2,
        Cause::RunsPaused => 3,
        Cause::Failing => 4,
        Cause::Quiet => 5,
        Cause::HandedBack => 6,
        Cause::Approvals => 7,
        Cause::Decisions => 8,
    }
}

/// The squad's status at unix time `now`.
pub fn compute(facts: &Facts, rows: &Rows, now: u64) -> SquadStatus {
    let tiles = tiles(facts, rows);
    let mut blocked = blocks(facts, rows, &tiles, now);
    let all_out = !facts.in_use.is_empty() && facts.in_use.iter().all(|e| facts.engines.iter().any(|b| b.engine == *e && b.out_until.is_some()));
    if all_out {
        for b in blocked.iter_mut().filter(|b| b.cause == Cause::EngineOut) {
            b.severity = Severity::Red;
        }
    }
    blocked.sort_by(|a, b| b.severity.cmp(&a.severity).then(rank(a.cause).cmp(&rank(b.cause))).then(b.count.cmp(&a.count)));
    let blocked_more = blocked.len().saturating_sub(BLOCKED_SHOWN) as u32;
    blocked.truncate(BLOCKED_SHOWN);

    let pacing = facts.engines.iter().any(|e| e.pacing_at_once.is_some());
    let mood = if facts.breaker.is_some() || all_out {
        Mood::Stopped
    } else if pacing || blocked.iter().any(|b| b.severity >= Severity::Amber) {
        Mood::Slowed
    } else {
        Mood::Good
    };
    let mut notes: Vec<Note> = facts.engines.iter().filter_map(|e| e.out_until.map(|at| Note::Out { engine: e.engine, back_at: (at > 0).then_some(at) })).collect();
    notes.extend(facts.engines.iter().filter_map(|e| e.pacing_at_once.map(|at_once| Note::Pacing { engine: e.engine, at_once })));

    let mut waiting = importance::waiting_list(&facts.tickets, &facts.asks, facts.pm_accepts);
    let waiting_more = waiting.len().saturating_sub(WAITING_SHOWN) as u32;
    waiting.truncate(WAITING_SHOWN);

    SquadStatus {
        squad: facts.squad.clone(),
        at: now,
        mood,
        notes,
        tiles,
        blocked,
        blocked_more,
        engines: facts.engines.clone(),
        waste: waste(&rows.usage, now),
        watched: watched(facts, rows),
        waiting,
        waiting_more,
        empty: facts.tickets.is_empty() && facts.members == 0,
    }
}

// ── the four numbers ──

/// Tickets that wait for the person: a member put one down for them, a run
/// stalled with no hand-in, work handed in where the person accepts it.
fn waits_for_person(t: &Ticket, pm_accepts: bool) -> bool {
    match t.status {
        TicketStatus::Waiting => true,
        TicketStatus::InProgress => t.execution.as_ref().is_some_and(|e| e.state == equipe_protocol::ExecutionState::Stalled),
        TicketStatus::Review => !pm_accepts && t.ready_for_review(),
        _ => false,
    }
}

fn tiles(facts: &Facts, rows: &Rows) -> Tiles {
    let open = |status: TicketStatus| facts.tickets.iter().filter(move |t| t.status == status);
    let waiting = open(TicketStatus::Todo).count() as u32;
    let waiting_urgent = open(TicketStatus::Todo).filter(|t| t.priority == Priority::Urgent).count() as u32;
    let on_you = facts.tickets.iter().filter(|t| waits_for_person(t, facts.pm_accepts)).count() as u32;
    let decisions = facts.decisions + on_you;
    Tiles {
        accepted_today: rows.accepted_today,
        accepted_2h: rows.accepted_2h,
        in_progress: open(TicketStatus::InProgress).count() as u32,
        runs_active: facts.working.len() as u32,
        runs_max: facts.runs_max,
        needs_you: decisions + facts.approvals,
        decisions,
        approvals: facts.approvals,
        waiting,
        waiting_urgent,
    }
}

// ── what's blocked ──

/// Runs paused for one reason on one engine.
#[derive(Default)]
struct Paused {
    count: u32,
    tickets: Vec<TicketId>,
    resumes: Vec<u64>,
    cause: Option<String>,
}

fn blocks(facts: &Facts, rows: &Rows, tiles: &Tiles, now: u64) -> Vec<Block> {
    let mut out = Vec::new();
    let block = |cause, severity, count: u32, handling, goto| Block { cause, severity, count, why: None, engine: None, at: None, detail: None, tickets: Vec::new(), handling, goto };

    // Runs paused, grouped by why and engine, in the order they were first seen.
    let mut groups: Groups = Vec::new();
    for t in facts.tickets.iter().filter(|t| !t.status.is_closed()) {
        let Some(pause) = t.execution.as_ref().and_then(|e| e.pause.as_ref()) else { continue };
        let key = (pause.why, pause.engine);
        let at = match groups.iter().position(|(k, _)| *k == key) {
            Some(at) => at,
            None => {
                groups.push((key, Paused::default()));
                groups.len() - 1
            }
        };
        let g = &mut groups[at].1;
        g.count += 1;
        if g.tickets.len() < NAMED {
            g.tickets.push(t.id);
        }
        g.resumes.extend(pause.resumes_at);
        g.cause = g.cause.take().or_else(|| pause.cause.clone());
    }
    type Groups = Vec<((PauseWhy, Option<Engine>), Paused)>;
    fn take(groups: &mut Groups, why: PauseWhy, engine: Option<Engine>) -> Option<Paused> {
        let at = groups.iter().position(|(k, _)| *k == (why, engine))?;
        Some(groups.remove(at).1)
    }

    // The breaker: the squad starts nothing. The runs it holds are part of its row.
    if let Some(breaker) = &facts.breaker {
        let mut held = Paused::default();
        let engines: Vec<Option<Engine>> = groups.iter().filter(|((why, _), _)| *why == PauseWhy::SquadPaused).map(|((_, e), _)| *e).collect();
        for engine in engines {
            if let Some(g) = take(&mut groups, PauseWhy::SquadPaused, engine) {
                held.count += g.count;
                held.tickets.extend(g.tickets);
            }
        }
        held.tickets.truncate(NAMED);
        out.push(Block {
            at: Some(breaker.since),
            detail: Some(breaker.cause.clone()).filter(|c| !c.is_empty()),
            tickets: held.tickets,
            ..block(Cause::BreakerOpen, Severity::Red, held.count, Handling::Resolving, None)
        });
    }

    // An engine out of its limit, with the runs waiting on it.
    for e in facts.engines.iter().filter(|e| e.out_until.is_some()) {
        let held = take(&mut groups, PauseWhy::EngineLimit, Some(e.engine)).unwrap_or_default();
        let back = e.out_until.filter(|at| *at > 0).or_else(|| held.resumes.iter().max().copied());
        out.push(Block { engine: Some(e.engine), at: back, tickets: held.tickets, ..block(Cause::EngineOut, Severity::Amber, held.count, Handling::Fixed, None) });
    }

    // An engine to sign in to again, with the runs that wait for it.
    for engine in &facts.signed_out {
        let held = take(&mut groups, PauseWhy::SignIn, Some(*engine)).unwrap_or_default();
        out.push(Block { engine: Some(*engine), tickets: held.tickets, ..block(Cause::SignIn, Severity::Amber, held.count, Handling::You { action: Action::Open }, Some(Goto::Logins)) });
    }

    // The rest of the paused runs, by why.
    for ((why, engine), g) in groups {
        let (severity, handling, goto, at) = match why {
            PauseWhy::EngineLimit => (Severity::Amber, Handling::Fixed, None, g.resumes.iter().max().copied()),
            PauseWhy::Retrying => (Severity::Amber, Handling::Resolving, None, g.resumes.iter().min().copied()),
            PauseWhy::SquadPaused => (Severity::Red, Handling::Resolving, None, None),
            PauseWhy::SignIn => (Severity::Amber, Handling::You { action: Action::Open }, Some(Goto::Logins), None),
        };
        out.push(Block { why: Some(why), engine, at, detail: g.cause, tickets: g.tickets, ..block(Cause::RunsPaused, severity, g.count, handling, goto) });
    }

    // Turns keep failing.
    if rows.failures_hour >= FAILING_MIN {
        out.push(block(Cause::Failing, Severity::Amber, rows.failures_hour.min(u64::from(u32::MAX)) as u32, Handling::Resolving, None));
    }

    // In progress, and nothing has touched it: stalled for the person, or quiet for two hours.
    let mut quiet: Vec<(u64, TicketId)> = facts
        .tickets
        .iter()
        .filter(|t| t.status == TicketStatus::InProgress && !facts.working.contains(&t.id))
        .filter(|t| t.execution.as_ref().is_none_or(|e| e.pause.is_none()))
        .filter_map(|t| {
            let stalled = t.execution.as_ref().is_some_and(|e| e.state == equipe_protocol::ExecutionState::Stalled);
            let last = rows.last_turn.get(&t.id).copied().unwrap_or(0).max(t.updated_at.unwrap_or(0));
            (stalled || now.saturating_sub(last) >= QUIET_AFTER).then_some((last, t.id))
        })
        .collect();
    quiet.sort();
    if let Some(&(since, first)) = quiet.first() {
        out.push(Block {
            at: Some(since).filter(|s| *s > 0),
            tickets: quiet.iter().take(NAMED).map(|(_, id)| *id).collect(),
            ..block(Cause::Quiet, Severity::Amber, quiet.len() as u32, Handling::You { action: Action::Look }, Some(Goto::Ticket(first)))
        });
    }

    // Handed back, or couldn't start: back among the proposals.
    let mut back: Vec<(u64, TicketId)> = facts.tickets.iter().filter(|t| !t.status.is_closed() && (t.handed_back() || t.could_not_start())).map(|t| (t.updated_at.unwrap_or(0), t.id)).collect();
    back.sort();
    if let Some(&(since, first)) = back.first() {
        out.push(Block {
            at: Some(since).filter(|s| *s > 0),
            tickets: back.iter().take(NAMED).map(|(_, id)| *id).collect(),
            ..block(Cause::HandedBack, Severity::Amber, back.len() as u32, Handling::You { action: Action::Look }, Some(Goto::Ticket(first)))
        });
    }

    // What waits for the person: quiet while the work goes on, amber once it holds it up.
    let stuck = tiles.runs_active == 0 && (tiles.in_progress > 0 || tiles.waiting > 0);
    let waiting = if stuck { Severity::Amber } else { Severity::Info };
    if tiles.approvals > 0 {
        out.push(block(Cause::Approvals, waiting, tiles.approvals, Handling::You { action: Action::Answer }, Some(Goto::NeedsYou)));
    }
    if tiles.decisions > 0 {
        out.push(block(Cause::Decisions, waiting, tiles.decisions, Handling::You { action: Action::Answer }, Some(Goto::NeedsYou)));
    }
    out
}

// ── the watched tickets ──

fn watch(t: &Ticket) -> Watched {
    Watched {
        ticket: t.id,
        title: t.title.clone(),
        status: t.status,
        owner: t.assignee.clone().or_else(|| t.owner.clone()),
        since: t.updated_at,
        priority: t.priority,
        paused: t.execution.as_ref().and_then(|e| e.pause.as_ref()).map(|p| p.why),
    }
}

/// The tickets the person pinned — or, with none pinned, the squad's urgent
/// ones still open, those under way first.
fn watched(facts: &Facts, rows: &Rows) -> WatchedList {
    let pins: Vec<TicketId> = rows.watched.clone();
    let mut list: Vec<&Ticket> = if pins.is_empty() {
        let mut urgent: Vec<&Ticket> = facts.tickets.iter().filter(|t| t.priority == Priority::Urgent && !t.status.is_closed() && !matches!(t.status, TicketStatus::Idea | TicketStatus::Proposed)).collect();
        urgent.sort_by_key(|t| (t.status != TicketStatus::InProgress, t.id));
        urgent
    } else {
        pins.iter().filter_map(|id| facts.tickets.iter().find(|t| t.id == *id)).collect()
    };
    let more = list.len().saturating_sub(WATCHED_SHOWN) as u32;
    list.truncate(WATCHED_SHOWN);
    WatchedList { pinned: !pins.is_empty(), pins, tickets: list.into_iter().map(watch).collect(), more }
}

// ── token waste ──

/// The waste signals of the last day against the last week.
fn waste(input: &Input, now: u64) -> Waste {
    let day_from = now.saturating_sub(DAY);
    let week = usage::summarize(input, now.saturating_sub(7 * DAY), now.saturating_add(1));
    let day_input = Input {
        turns: input.turns.iter().filter(|t| t.ended >= day_from).cloned().collect(),
        tickets: input.tickets.clone(),
        endings: Vec::new(),
        events: input.events.clone(),
        ticket_totals: HashMap::new(),
        send_backs: Vec::new(),
        design_qa: Vec::new(),
    };
    let day = usage::summarize(&day_input, day_from, now.saturating_add(1));
    let shares_say_something = day.tokens.total >= usage::MIN_TOKENS_FOR_SHARES;
    let value = |r: &Report, f: &dyn Fn(&Report) -> Option<f64>| if r.tokens.total == 0 { None } else { f(r) };
    let desk = |r: &Report| r.by_kind.iter().find(|k| k.name == usage::kind::PM_DESK).and_then(|k| k.context_per_call).map(|c| c as f64);
    let heavy = |r: &Report| Some(r.context.iter().find(|b| b.label == ">400k").map_or(0.0, |b| b.share_percent));
    let cancelled = |r: &Report| Some(r.cancelled_share_percent);
    let rework = |r: &Report| Some(r.rework.iter().map(|t| t.tokens).sum::<u64>() as f64 * 100.0 / r.tokens.total.max(1) as f64);
    let accept = |r: &Report| r.quality.first_handin_accept_percent;
    let metric = |key, f: &dyn Fn(&Report) -> Option<f64>, over: &dyn Fn(f64) -> bool| {
        let (d, w) = (value(&day, f), value(&week, f));
        WasteMetric { key, day: d, week: w, bad: d.is_some_and(over) }
    };
    let mut metrics = vec![
        metric(WasteKey::PmDeskContext, &desk, &|v| v > usage::PM_DESK_CONTEXT as f64),
        metric(WasteKey::HeavyShare, &heavy, &|v| shares_say_something && v >= usage::HEAVY_SHARE_PERCENT),
        metric(WasteKey::CancelledShare, &cancelled, &|v| shares_say_something && v >= usage::CANCELLED_SHARE_PERCENT),
        metric(WasteKey::ReworkShare, &rework, &|v| shares_say_something && v >= usage::REWORK_SHARE_PERCENT),
    ];
    // The quality check stands on tickets, not tokens: a day with no hand-in has none.
    let (d, w) = (accept(&day), accept(&week));
    metrics.push(WasteMetric { key: WasteKey::FirstHandInAccept, day: d, week: w, bad: d.is_some_and(|d| d < 50.0 || w.is_some_and(|w| d + 15.0 <= w)) });
    Waste { tokens_day: day.tokens.total, turns_day: day.turns, metrics }
}

// ── the journal ──

/// Where the person's day starts, as unix time: the last midnight at their
/// offset from UTC.
pub fn day_start(now: u64, utc_offset_secs: i32) -> u64 {
    let local = now as i64 + i64::from(utc_offset_secs);
    (now as i64 - local.rem_euclid(DAY as i64)).max(0) as u64
}

/// The squad's rows from its journal file, read without writing to it.
pub fn read_rows(db: &Path, now: u64, utc_offset_secs: i32) -> Result<Rows, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("{}: {e}", db.display()))?;
    let _ = conn.busy_timeout(Duration::from_secs(5));
    rows_of(&conn, now, day_start(now, utc_offset_secs)).map_err(|e| e.to_string())
}

fn count(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<u64> {
    Ok(conn.query_row(sql, args, |r| r.get::<_, i64>(0))?.max(0) as u64)
}

/// The rows, from an open journal.
pub fn rows_of(conn: &Connection, now: u64, since_midnight: u64) -> rusqlite::Result<Rows> {
    let int = |n: u64| n.min(i64::MAX as u64) as i64;
    let accepted = |from: u64| count(conn, "SELECT COUNT(DISTINCT ticket) FROM ticket_events WHERE kind LIKE 'accepted%' AND at >= ?1 AND at <= ?2", [int(from), int(now)]);
    let week = now.saturating_sub(7 * DAY);
    let mut last_turn = HashMap::new();
    let mut stmt = conn.prepare("SELECT ticket, MAX(ended) FROM turns WHERE ticket IS NOT NULL AND ended >= ?1 GROUP BY ticket")?;
    for row in stmt.query_map([int(week)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (ticket, ended) = row?;
        last_turn.insert(ticket.max(0) as TicketId, ended.max(0) as u64);
    }
    let failures_hour = if usage::has_table(conn, "turn_endings") {
        let kinds = FAILURES.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
        count(conn, &format!("SELECT COUNT(*) FROM turn_endings WHERE at >= ?1 AND kind IN ({kinds})"), [int(now.saturating_sub(HOUR))])?
    } else {
        0
    };
    let watched = conn.query_row("SELECT value FROM meta WHERE key = 'watched'", [], |r| r.get::<_, String>(0)).optional()?.and_then(|v| serde_json::from_str::<Vec<TicketId>>(&v).ok()).unwrap_or_default();
    Ok(Rows {
        accepted_today: accepted(since_midnight)? as u32,
        accepted_2h: accepted(now.saturating_sub(2 * HOUR))? as u32,
        last_turn,
        failures_hour,
        watched,
        usage: usage::select(conn, week, now.saturating_add(1))?,
    })
}

/// A squad's whole status from its journal file and the daemon's `facts`.
pub fn read(db: &Path, facts: &Facts, now: u64, utc_offset_secs: i32) -> Result<SquadStatus, String> {
    Ok(compute(facts, &read_rows(db, now, utc_offset_secs)?, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::{ExecutionState, RunPause, TicketExecution, TicketKind};

    const NOW: u64 = 1_794_000_000;

    fn ticket(id: TicketId, status: TicketStatus) -> Ticket {
        Ticket {
            id,
            title: format!("Ticket {id}"),
            body: String::new(),
            status,
            kind: TicketKind::Task,
            owner: Some("frontend".into()),
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None, remaining: vec![],
            images: vec![],
            priority: Priority::Normal,
            rank: 0,
            updated_at: Some(NOW - 10 * MINUTE),
            design_qa: None,
            plan: None,
            model: None,
        }
    }

    fn paused(id: TicketId, why: PauseWhy, engine: Engine, resumes_at: Option<u64>) -> Ticket {
        let mut t = ticket(id, TicketStatus::InProgress);
        t.execution = Some(Box::new(TicketExecution {
            state: ExecutionState::Active,
            session: 1,
            operation: "run".into(),
            generation: 1,
            heartbeat_at: NOW,
            lease_expires_at: NOW + 600,
            reason: None,
            pause: Some(RunPause { why, engine: Some(engine), resumes_at, moves_to: None, cause: None, attempt: 0 }),
            head: None,
            dirty: None,
        }));
        t
    }

    fn stalled(id: TicketId) -> Ticket {
        let mut t = paused(id, PauseWhy::Retrying, Engine::Claude, None);
        let e = t.execution.as_mut().unwrap();
        e.pause = None;
        e.state = ExecutionState::Stalled;
        t
    }

    fn budget(engine: Engine) -> EngineBudget {
        EngineBudget { engine, plan_type: None, weekly: None, five_hour: None, out_until: None, runs_out_at: None, lasts: false, pacing_at_once: None, stale: false }
    }

    fn healthy() -> Facts {
        Facts {
            squad: "acme".into(),
            tickets: vec![ticket(1, TicketStatus::InProgress), ticket(2, TicketStatus::Todo)],
            members: 3,
            engines: vec![budget(Engine::Claude), budget(Engine::Codex)],
            in_use: vec![Engine::Claude, Engine::Codex],
            working: [1].into(),
            runs_max: 8,
            pm_accepts: true,
            ..Facts::default()
        }
    }

    fn quiet_rows() -> Rows {
        Rows { last_turn: [(1, NOW - 5 * MINUTE)].into(), ..Rows::default() }
    }

    fn status(facts: &Facts, rows: &Rows) -> SquadStatus {
        compute(facts, rows, NOW)
    }

    fn causes(s: &SquadStatus) -> Vec<Cause> {
        s.blocked.iter().map(|b| b.cause).collect()
    }

    #[test]
    fn what_waits_on_the_person_is_listed_most_blocking_first_and_cut_to_a_few() {
        let mut waiting = ticket(1, TicketStatus::Waiting);
        waiting.title = "AWS admin sign-in".into();
        let mut quiet = ticket(2, TicketStatus::Waiting);
        quiet.title = "Pick a color".into();
        let mut tickets = vec![quiet, waiting];
        tickets.extend((10..13).map(|id| Ticket { depends_on: vec![1], ..ticket(id, TicketStatus::Todo) }));
        let facts = Facts { squad: "acme".into(), tickets, members: 1, ..healthy() };
        let s = status(&facts, &quiet_rows());
        assert_eq!(s.waiting.iter().map(|w| w.ticket).collect::<Vec<_>>(), vec![Some(1), Some(2)]);
        assert_eq!((s.waiting[0].impact.blocks, s.waiting[0].impact.reason.as_str()), (3, "blocks 3 tickets"));
        assert_eq!(s.waiting_more, 0);
        let many = Facts { tickets: (1..=8).map(|id| ticket(id, TicketStatus::Waiting)).collect(), ..facts };
        let s = status(&many, &quiet_rows());
        assert_eq!((s.waiting.len(), s.waiting_more), (WAITING_SHOWN, 3));
    }

    #[test]
    fn a_squad_at_work_with_nothing_wrong_is_green_with_an_empty_list() {
        let s = status(&healthy(), &quiet_rows());
        assert_eq!(s.mood, Mood::Good);
        assert!(s.blocked.is_empty() && s.notes.is_empty() && s.blocked_more == 0, "{s:#?}");
        assert_eq!((s.tiles.in_progress, s.tiles.runs_active, s.tiles.runs_max, s.tiles.waiting), (1, 1, 8, 1));
        assert!(!s.empty);
    }

    #[test]
    fn an_open_breaker_is_red_with_its_cause_since_when_and_the_runs_it_holds() {
        let mut facts = healthy();
        facts.tickets.push(paused(3, PauseWhy::SquadPaused, Engine::Claude, None));
        facts.tickets.push(paused(4, PauseWhy::SquadPaused, Engine::Codex, None));
        facts.working.clear();
        facts.breaker = Some(Breaker { cause: "No space left on device".into(), since: NOW - 20 * MINUTE });
        let s = status(&facts, &quiet_rows());
        assert_eq!(s.mood, Mood::Stopped);
        let first = &s.blocked[0];
        assert_eq!((first.cause, first.severity, first.count), (Cause::BreakerOpen, Severity::Red, 2));
        assert_eq!((first.at, first.detail.as_deref(), first.handling), (Some(NOW - 20 * MINUTE), Some("No space left on device"), Handling::Resolving));
        assert_eq!(first.tickets, [3, 4]);
        assert!(!causes(&s).contains(&Cause::RunsPaused), "the held runs are the breaker's row, not a second one: {:?}", causes(&s));
    }

    #[test]
    fn paused_runs_are_grouped_by_why_and_engine_with_who_handles_them() {
        let mut facts = healthy();
        let back = NOW + 3 * DAY;
        for id in 10..22 {
            facts.tickets.push(paused(id, PauseWhy::EngineLimit, Engine::Codex, Some(back)));
        }
        facts.tickets.push(paused(30, PauseWhy::Retrying, Engine::Claude, Some(NOW + 600)));
        facts.tickets.push(paused(31, PauseWhy::Retrying, Engine::Claude, Some(NOW + 120)));
        facts.tickets.push(paused(32, PauseWhy::SignIn, Engine::Claude, None));
        let s = status(&facts, &quiet_rows());
        let row = |why| s.blocked.iter().find(|b| b.why == Some(why)).unwrap();
        let limit = row(PauseWhy::EngineLimit);
        assert_eq!((limit.count, limit.engine, limit.at, limit.handling, limit.severity), (12, Some(Engine::Codex), Some(back), Handling::Fixed, Severity::Amber));
        assert_eq!(limit.tickets, [10, 11, 12], "a row names a few tickets, not twelve");
        let retry = row(PauseWhy::Retrying);
        assert_eq!((retry.count, retry.at, retry.handling), (2, Some(NOW + 120), Handling::Resolving), "the next retry is the soonest");
        let sign_in = row(PauseWhy::SignIn);
        assert_eq!((sign_in.handling, sign_in.goto), (Handling::You { action: Action::Open }, Some(Goto::Logins)));
        assert_eq!(s.mood, Mood::Slowed);
    }

    #[test]
    fn an_engine_out_of_its_limit_is_one_row_with_the_runs_that_wait_on_it() {
        let mut facts = healthy();
        facts.engines[1].out_until = Some(NOW + DAY);
        for id in 10..13 {
            facts.tickets.push(paused(id, PauseWhy::EngineLimit, Engine::Codex, Some(NOW + DAY)));
        }
        let s = status(&facts, &quiet_rows());
        let rows: Vec<_> = s.blocked.iter().filter(|b| matches!(b.cause, Cause::EngineOut | Cause::RunsPaused)).collect();
        assert_eq!(rows.len(), 1, "{rows:#?}");
        assert_eq!((rows[0].cause, rows[0].engine, rows[0].count, rows[0].at, rows[0].severity), (Cause::EngineOut, Some(Engine::Codex), 3, Some(NOW + DAY), Severity::Amber));
        assert_eq!(s.mood, Mood::Slowed, "one engine out, the other works");
        assert_eq!(s.notes, [Note::Out { engine: Engine::Codex, back_at: Some(NOW + DAY) }]);
    }

    #[test]
    fn every_engine_out_stops_the_squad_and_one_out_with_none_in_use_does_not() {
        let mut facts = healthy();
        facts.engines[0].out_until = Some(NOW + HOUR);
        facts.engines[1].out_until = Some(NOW + DAY);
        let s = status(&facts, &quiet_rows());
        assert_eq!(s.mood, Mood::Stopped);
        assert!(s.blocked.iter().filter(|b| b.cause == Cause::EngineOut).all(|b| b.severity == Severity::Red));
        // Members run on Claude only; Codex being out changes nothing for them.
        let mut only = healthy();
        only.in_use = vec![Engine::Claude];
        only.engines[1].out_until = Some(NOW + DAY);
        assert_eq!(status(&only, &quiet_rows()).mood, Mood::Slowed, "still a row to see, not a stop");
        only.engines[0].out_until = Some(NOW + HOUR);
        assert_eq!(status(&only, &quiet_rows()).mood, Mood::Stopped);
    }

    #[test]
    fn a_sign_in_to_make_is_one_row_the_person_opens_with_its_waiting_runs() {
        let mut facts = healthy();
        facts.signed_out = vec![Engine::Claude];
        facts.tickets.push(paused(5, PauseWhy::SignIn, Engine::Claude, None));
        facts.tickets.push(paused(6, PauseWhy::SignIn, Engine::Claude, None));
        let s = status(&facts, &quiet_rows());
        let sign_in: Vec<_> = s.blocked.iter().filter(|b| b.cause == Cause::SignIn || b.why == Some(PauseWhy::SignIn)).collect();
        assert_eq!(sign_in.len(), 1);
        assert_eq!((sign_in[0].cause, sign_in[0].count, sign_in[0].engine), (Cause::SignIn, 2, Some(Engine::Claude)));
        assert_eq!(sign_in[0].handling, Handling::You { action: Action::Open });
    }

    #[test]
    fn tickets_handed_back_are_a_row_for_the_person_to_look_at() {
        let mut facts = healthy();
        let mut back = ticket(7, TicketStatus::Proposed);
        back.note = Some(format!("{}can't be done as asked", equipe_protocol::HANDED_BACK));
        back.updated_at = Some(NOW - 3 * HOUR);
        let mut late = ticket(8, TicketStatus::Proposed);
        late.note = back.note.clone();
        late.updated_at = Some(NOW - HOUR);
        facts.tickets.extend([late, back, ticket(9, TicketStatus::Proposed)]);
        let s = status(&facts, &quiet_rows());
        let row = s.blocked.iter().find(|b| b.cause == Cause::HandedBack).unwrap();
        assert_eq!((row.count, row.tickets.clone(), row.at), (2, vec![7, 8], Some(NOW - 3 * HOUR)));
        assert_eq!((row.handling, row.goto, row.severity), (Handling::You { action: Action::Look }, Some(Goto::Ticket(7)), Severity::Amber));
    }

    #[test]
    fn decisions_and_approvals_are_quiet_while_work_goes_on_and_amber_once_nothing_runs() {
        let mut facts = healthy();
        facts.decisions = 2;
        facts.approvals = 1;
        let s = status(&facts, &quiet_rows());
        assert_eq!((s.tiles.needs_you, s.tiles.decisions, s.tiles.approvals), (3, 2, 1));
        assert!(s.blocked.iter().all(|b| b.severity == Severity::Info));
        assert_eq!(s.mood, Mood::Good, "waiting for the person, with the squad at work, isn't slow");
        assert_eq!(causes(&s), [Cause::Approvals, Cause::Decisions]);
        assert_eq!(s.blocked[0].handling, Handling::You { action: Action::Answer });
        facts.working.clear();
        let held = status(&facts, &quiet_rows());
        assert!(held.blocked.iter().all(|b| b.severity == Severity::Amber), "{:?}", held.blocked);
        assert_eq!(held.mood, Mood::Slowed);
        // Nothing to run and a proposal waiting: not slow, just waiting.
        let mut idle = Facts { squad: "acme".into(), members: 2, decisions: 1, ..Facts::default() };
        idle.tickets.push(ticket(1, TicketStatus::Done));
        assert_eq!(status(&idle, &Rows::default()).mood, Mood::Good);
    }

    #[test]
    fn waiting_tickets_stalled_runs_and_work_to_accept_count_as_needing_the_person() {
        let mut facts = healthy();
        facts.tickets.push(ticket(3, TicketStatus::Waiting));
        facts.tickets.push(stalled(4));
        facts.tickets.push(ticket(5, TicketStatus::Review));
        facts.pm_accepts = false;
        assert_eq!(status(&facts, &quiet_rows()).tiles.needs_you, 3);
        facts.pm_accepts = true;
        assert_eq!(status(&facts, &quiet_rows()).tiles.needs_you, 2, "where the PM accepts, a hand-in is the PM's");
    }

    #[test]
    fn a_ticket_in_progress_with_no_turn_for_two_hours_is_quiet_unless_it_is_mid_turn_or_paused() {
        let mut facts = healthy();
        let mut old = ticket(3, TicketStatus::InProgress);
        old.updated_at = Some(NOW - 5 * HOUR);
        facts.tickets.extend([old, ticket(4, TicketStatus::InProgress), paused(5, PauseWhy::EngineLimit, Engine::Codex, None)]);
        // 3: its last turn ended 3h ago; 4: 90 minutes ago; 1 is mid-turn; 5 is paused.
        let rows = Rows { last_turn: [(1, NOW - 4 * HOUR), (3, NOW - 3 * HOUR), (4, NOW - 90 * MINUTE)].into(), ..Rows::default() };
        let s = status(&facts, &rows);
        let row = s.blocked.iter().find(|b| b.cause == Cause::Quiet).unwrap();
        assert_eq!((row.count, row.tickets.clone(), row.at, row.goto), (1, vec![3], Some(NOW - 3 * HOUR), Some(Goto::Ticket(3))));
        // Without a single turn on record, the ticket's own last change is when it went quiet.
        let none = status(&facts, &Rows::default());
        assert_eq!(none.blocked.iter().find(|b| b.cause == Cause::Quiet).map(|b| b.count), Some(1), "ticket 3, changed 5h ago; 4 changed 10 minutes ago");
        // A stalled run is quiet at once, whatever the clock says.
        facts.tickets = vec![stalled(9)];
        facts.working.clear();
        let s = status(&facts, &Rows::default());
        assert_eq!(s.blocked.iter().find(|b| b.cause == Cause::Quiet).map(|b| b.tickets.clone()), Some(vec![9]));
    }

    #[test]
    fn repeated_failures_in_the_last_hour_are_a_row_from_three() {
        let few = Rows { failures_hour: FAILING_MIN - 1, ..quiet_rows() };
        assert!(status(&healthy(), &few).blocked.is_empty());
        let many = Rows { failures_hour: 7, ..quiet_rows() };
        let s = status(&healthy(), &many);
        let row = &s.blocked[0];
        assert_eq!((row.cause, row.count, row.handling, row.severity), (Cause::Failing, 7, Handling::Resolving, Severity::Amber));
        assert_eq!(s.mood, Mood::Slowed);
    }

    #[test]
    fn running_fewer_at_once_to_make_a_plan_last_is_amber_with_a_note_and_no_row() {
        let mut facts = healthy();
        facts.engines[0].pacing_at_once = Some(4);
        let s = status(&facts, &quiet_rows());
        assert_eq!(s.mood, Mood::Slowed);
        assert!(s.blocked.is_empty());
        assert_eq!(s.notes, [Note::Pacing { engine: Engine::Claude, at_once: 4 }]);
    }

    #[test]
    fn rows_come_worst_first_and_a_long_list_is_cut_with_how_many_were_left_out() {
        let mut facts = healthy();
        facts.breaker = Some(Breaker { cause: String::new(), since: NOW - 60 });
        facts.signed_out = vec![Engine::Claude, Engine::Codex];
        facts.engines[0].out_until = Some(NOW + HOUR);
        let mut back = ticket(7, TicketStatus::Proposed);
        back.note = Some(format!("{}x", equipe_protocol::HANDED_BACK));
        facts.tickets.push(back);
        facts.tickets.push(paused(20, PauseWhy::Retrying, Engine::Claude, None));
        facts.tickets.push(paused(21, PauseWhy::Retrying, Engine::Codex, None));
        facts.tickets.push(paused(22, PauseWhy::SignIn, Engine::Codex, None));
        facts.decisions = 1;
        facts.approvals = 1;
        let rows = Rows { failures_hour: 4, ..quiet_rows() };
        let s = status(&facts, &rows);
        assert_eq!(s.blocked.len(), BLOCKED_SHOWN);
        assert!(s.blocked_more > 0, "{:?}", causes(&s));
        assert_eq!(s.blocked[0].cause, Cause::BreakerOpen, "red first");
        assert!(s.blocked.windows(2).all(|w| w[0].severity >= w[1].severity));
        assert_eq!(s.blocked.len() as u32 + s.blocked_more, 10, "{:?}", causes(&s));
    }

    #[test]
    fn the_four_numbers_count_what_was_accepted_what_runs_and_what_waits() {
        let mut facts = healthy();
        let mut urgent = ticket(3, TicketStatus::Todo);
        urgent.priority = Priority::Urgent;
        facts.tickets.extend([urgent, ticket(4, TicketStatus::Done)]);
        let rows = Rows { accepted_today: 5, accepted_2h: 2, ..quiet_rows() };
        let t = status(&facts, &rows).tiles;
        assert_eq!((t.accepted_today, t.accepted_2h), (5, 2));
        assert_eq!((t.in_progress, t.runs_active, t.runs_max), (1, 1, 8));
        assert_eq!((t.waiting, t.waiting_urgent), (2, 1));
    }

    #[test]
    fn an_empty_squad_and_one_with_no_engine_readings_say_so_without_failing() {
        let s = status(&Facts { squad: "new".into(), ..Facts::default() }, &Rows::default());
        assert!(s.empty && s.engines.is_empty() && s.blocked.is_empty());
        assert_eq!(s.mood, Mood::Good);
        assert_eq!(s.waste.tokens_day, 0);
        assert!(s.waste.metrics.iter().all(|m| m.day.is_none() && !m.bad), "no usage data yet: nothing measured, nothing flagged");
        assert!(s.watched.tickets.is_empty() && !s.watched.pinned);
    }

    #[test]
    fn the_watched_list_is_the_pinned_tickets_or_else_the_urgent_ones() {
        let mut facts = healthy();
        let mut a = ticket(3, TicketStatus::Todo);
        a.priority = Priority::Urgent;
        let mut b = ticket(4, TicketStatus::InProgress);
        b.priority = Priority::Urgent;
        b.assignee = Some("backend-2".into());
        let mut done = ticket(5, TicketStatus::Done);
        done.priority = Priority::Urgent;
        let mut idea = ticket(6, TicketStatus::Idea);
        idea.priority = Priority::Urgent;
        facts.tickets.extend([a, b, done, idea]);
        let list = status(&facts, &quiet_rows()).watched;
        assert!(!list.pinned);
        assert_eq!(list.tickets.iter().map(|t| t.ticket).collect::<Vec<_>>(), [4, 3], "under way first; closed ideas and done work are not urgent anymore");
        assert_eq!((list.tickets[0].owner.as_deref(), list.tickets[0].since), (Some("backend-2"), Some(NOW - 10 * MINUTE)));
        // Pinned: exactly those, in the order pinned, closed ones too; one that is gone is skipped.
        let rows = Rows { watched: vec![5, 2, 99], ..quiet_rows() };
        let list = status(&facts, &rows).watched;
        assert!(list.pinned);
        assert_eq!(list.tickets.iter().map(|t| t.ticket).collect::<Vec<_>>(), [5, 2]);
        assert_eq!(list.pins, [5, 2, 99]);
        // A long list is cut.
        facts.tickets = (1..=12).map(|id| ticket(id, TicketStatus::Todo)).collect();
        let rows = Rows { watched: (1..=12).collect(), ..quiet_rows() };
        let list = status(&facts, &rows).watched;
        assert_eq!((list.tickets.len(), list.more), (WATCHED_SHOWN, 4));
    }

    fn turn(ticket: Option<u64>, role: &str, ended: u64, read: u64, calls: u64) -> usage::TurnRow {
        usage::TurnRow { member: role.into(), role: role.into(), ticket, ended, tokens: read + 100, input: 0, output: 100, cache_read: read, cache_write: 0, calls, engine: Some("claude".into()), model: None, measured: true }
    }

    #[test]
    fn the_waste_card_sets_the_last_day_against_the_week_and_flags_what_is_over_its_threshold() {
        let big = 900_000_000;
        // A PM desk that reads 900k per call today, 300k per call earlier in the week.
        let mut turns = vec![turn(None, "pm", NOW - HOUR, big, 1_000)];
        turns.push(turn(None, "pm", NOW - 3 * DAY, 300_000 * 10, 10));
        turns.push(turn(Some(1), "backend", NOW - 2 * HOUR, 100_000_000, 1_000));
        let events = vec![
            usage::EventRow { ticket: 1, at: NOW - 3 * HOUR, kind: "handed in".into() },
            usage::EventRow { ticket: 1, at: NOW - 2 * HOUR, kind: "accepted".into() },
        ];
        let tickets = [(1, usage::TicketInfo { title: "A".into(), status: "done".into() })].into();
        let rows = Rows { usage: Input { turns, tickets, events, ..Input::default() }, ..quiet_rows() };
        let w = status(&healthy(), &rows).waste;
        assert!(w.tokens_day > 900_000_000 && w.turns_day == 2);
        let get = |key| w.metrics.iter().find(|m| m.key == key).unwrap();
        let desk = get(WasteKey::PmDeskContext);
        assert!(desk.bad && desk.day.unwrap() > 800_000.0 && (desk.week.unwrap() - 600_000.0).abs() < 400_000.0, "{desk:?}");
        assert_eq!(get(WasteKey::FirstHandInAccept).day, Some(100.0));
        assert!(!get(WasteKey::FirstHandInAccept).bad);
        assert!(get(WasteKey::HeavyShare).bad, "the desk's turn is over 400k a call and is most of the tokens");
    }

    #[test]
    fn a_falling_first_hand_in_accept_rate_is_flagged_as_the_quality_check() {
        let ev = |ticket, at, kind: &str| usage::EventRow { ticket, at, kind: kind.into() };
        // Four first hand-ins this week, three accepted early in the week; today's one was sent back.
        let events = vec![
            ev(1, NOW - 5 * DAY, "handed in"), ev(1, NOW - 5 * DAY + 60, "accepted"),
            ev(2, NOW - 4 * DAY, "handed in"), ev(2, NOW - 4 * DAY + 60, "accepted"),
            ev(3, NOW - 3 * DAY, "handed in"), ev(3, NOW - 3 * DAY + 60, "accepted"),
            ev(4, NOW - 2 * HOUR, "handed in"), ev(4, NOW - HOUR, "sent back"),
        ];
        let turns = vec![turn(Some(4), "backend", NOW - 2 * HOUR, 1_000, 2)];
        let rows = Rows { usage: Input { turns, events, ..Input::default() }, ..quiet_rows() };
        let m = status(&healthy(), &rows).waste.metrics.into_iter().find(|m| m.key == WasteKey::FirstHandInAccept).unwrap();
        assert_eq!((m.day, m.week), (Some(0.0), Some(75.0)));
        assert!(m.bad);
    }

    #[test]
    fn the_persons_day_starts_at_their_midnight() {
        let utc_midnight = 1_794_009_600 / DAY * DAY;
        assert_eq!(day_start(utc_midnight + 5 * HOUR, 0), utc_midnight);
        // Seoul, UTC+9: 03:00 UTC is noon there, so today began at 15:00 UTC the day before.
        assert_eq!(day_start(utc_midnight + 3 * HOUR, 9 * 3600), utc_midnight - 9 * HOUR);
        // New York, UTC-5: 03:00 UTC is 22:00 the evening before.
        assert_eq!(day_start(utc_midnight + 3 * HOUR, -5 * 3600), utc_midnight - DAY + 5 * HOUR);
    }

    #[test]
    fn the_journal_gives_accepts_last_turns_failures_and_pins_without_being_written_to() {
        let home = std::env::temp_dir().join(format!("equipe-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let j = crate::journal::Journal::for_squad(&home, "acme").unwrap();
        let new = |title: &str, status| Ticket { id: 0, title: title.into(), ..ticket(0, status) };
        let a = j.create_ticket(&new("A", TicketStatus::InProgress)).unwrap();
        let b = j.create_ticket(&new("B", TicketStatus::InProgress)).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        j.ticket_event(a, "equipe", "accepted", "").unwrap();
        j.ticket_event(b, "equipe", "accepted by the PM", "").unwrap();
        j.ticket_event(b, "equipe", "accepted", "").unwrap();
        let turn = |ticket: Option<u32>, ended: u64| crate::retro::Turn { member: "frontend".into(), role: "frontend".into(), ticket, began: ended - 5, ended, outcome: "completed".into(), tokens: 10 };
        j.record_turn(&turn(Some(a), now - 3 * HOUR)).unwrap();
        j.record_turn(&turn(Some(a), now - 2 * HOUR)).unwrap();
        j.record_turn(&turn(None, now - 60)).unwrap();
        for _ in 0..3 {
            j.count_turn_ending("failed_fast", Some(a), "frontend").unwrap();
        }
        j.count_turn_ending("limited", Some(a), "frontend").unwrap();
        j.set_watched(b, true).unwrap();
        j.set_watched(a, true).unwrap();
        j.set_watched(b, true).unwrap();

        let db = crate::journal::squad_dir(&home, "acme").join("squad.db");
        let rows = read_rows(&db, now, 0).unwrap();
        assert_eq!((rows.accepted_today, rows.accepted_2h), (2, 2), "a ticket accepted twice counts once; its PM accept too");
        assert_eq!(rows.last_turn.get(&a), Some(&(now - 2 * HOUR)));
        assert_eq!(rows.failures_hour, 3, "a limit isn't a failure");
        assert_eq!(rows.watched, [a, b], "pinned once each, in order, the second pin of b changing nothing");
        assert_eq!(rows.usage.turns.len(), 3);
        j.set_watched(a, false).unwrap();
        assert_eq!(read_rows(&db, now, 0).unwrap().watched, [b]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
