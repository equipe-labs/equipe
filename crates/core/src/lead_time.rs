//! How long the person waits for what they ask (`equipe flow`, docs/FLOW.md):
//! from a request to the first work on it, its first merge and production,
//! with how many tickets and hand-offs it took on the way, how much of the
//! board was meta work (checks, verifications, design QA, polish), and how
//! long ready tickets sat before anyone took them.
//!
//! Everything is read from the squad's journal, opened without writing.
//! [`summarize`] is pure — rows in, [`Report`] out — so it is tested on
//! fixture rows; [`read`] only fetches them.
//!
//! **Requests.** A journal with the person's requests (`requests`,
//! `request_tickets`, docs/REQUESTS.md) says which tickets serve which
//! request. An older journal has none; then they are worked out the way the
//! runtime records them today: a ticket a member put on the board within
//! [`ASKED_WITHIN`] of the person's words, while not at work on another ticket
//! (or one the person put there), starts a request — every such ticket from
//! the same words is the same request — and the tickets made while working
//! one of its tickets, its design follow-ups and its design QA join it. The
//! report says which way it was read ([`Report::source`]).
//!
//! **Production** is when the last of a request's code went live by a deploy
//! signal equipe saw (a production deployment or a deploy workflow on main,
//! `crate::deploys`); else a deploy ticket of the request done; otherwise
//! the moment its last ticket was finished — code merged into main, other
//! work done — with nothing of it left open.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

pub use crate::requests::ASKED_WITHIN;

/// The creator equipe writes on the tickets it makes itself (design QA,
/// reviews, audits).
pub const EQUIPE: &str = "equipe";
/// The creator on tickets the person put on the board.
pub const PERSON: &str = "you";

/// A ticket event that moves a ticket's active baton to another member
/// without a new ticket (`runtime::flow`). Its text starts with who holds it now.
pub const BATON: &str = "baton";
/// A ticket event: its own member landed it, its gates passed (`runtime::flow`).
pub const SELF_LANDED: &str = "landed by its member";

/// A ticket as the report needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketRow {
    pub id: u64,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub label: Option<String>,
    pub created_by: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub merged: Option<bool>,
    /// The ticket a design QA checks, when this one is (`tickets.design_qa`).
    pub design_qa: Option<u64>,
}

/// One line of a ticket's history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventRow {
    pub ticket: u64,
    pub at: u64,
    pub by: String,
    pub kind: String,
    pub text: String,
}

/// One member turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnRow {
    pub member: String,
    pub ticket: Option<u64>,
    pub began: u64,
    pub ended: u64,
    pub tokens: u64,
}

/// A request as the journal keeps it: when it was asked, and its tickets
/// with how each joined (`asked`, `made`, `follow_up`, `check`, `linked`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestRow {
    pub id: u64,
    pub title: String,
    pub asked_at: u64,
    pub tickets: Vec<(u64, String)>,
}

/// Everything [`summarize`] reads.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub tickets: Vec<TicketRow>,
    pub events: Vec<EventRow>,
    /// `(ticket, parent)`: design follow-ups.
    pub follow_ups: Vec<(u64, u64)>,
    /// `(ticket, kind, severity)` of each design follow-up.
    pub follow_up_kinds: Vec<(u64, String, Option<String>)>,
    pub turns: Vec<TurnRow>,
    /// When the person said something in a conversation.
    pub person_said: Vec<u64>,
    /// `None`: the journal keeps no requests.
    pub requests: Option<Vec<RequestRow>>,
    /// When each ticket's work first went live by a deploy signal (a
    /// production deployment, a deploy workflow) — not merely on main.
    pub live: HashMap<u64, u64>,
}

/// A median and a 90th percentile, of durations (seconds) or counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Stat {
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p90: Option<u64>,
}

impl Stat {
    pub fn of(mut values: Vec<u64>) -> Stat {
        values.sort_unstable();
        let at = |q: f64| -> Option<u64> {
            if values.is_empty() {
                return None;
            }
            // Nearest rank.
            let rank = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
            Some(values[rank - 1])
        };
        Stat { count: values.len(), median: at(0.5), p90: at(0.9) }
    }
}

/// One request, as the report lists the slowest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RequestLine {
    pub title: String,
    pub asked_at: u64,
    pub tickets: usize,
    pub handoffs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_first_merge: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_production: Option<u64>,
    /// A decision or an instruction answered: all its work finished and
    /// none of it merged or deployed anything — production doesn't apply.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub decided: bool,
}

/// The flow of a window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub since: u64,
    pub until: u64,
    /// `requests`: the journal's own; `reconstructed`: worked out from the
    /// person's words and when tickets were made (an older journal).
    pub source: &'static str,
    /// Requests asked in the window.
    pub requests: usize,
    /// Request → first work on any of its tickets.
    pub to_start: Stat,
    /// Request → its first merge into main.
    pub to_first_merge: Stat,
    /// Request → production (see the module's words); decided requests left out.
    pub to_production: Stat,
    /// Requests answered with nothing to ship — a decision, an instruction —
    /// apart from production ([`RequestLine::decided`]).
    pub decided: usize,
    /// How many of `to_production` were a deploy — signal or ticket (the rest: last merge).
    pub production_by_deploy: usize,
    /// Every ticket of a request, its checks too.
    pub tickets_per_request: Stat,
    /// How many times the work of a request changed hands.
    pub handoffs_per_request: Stat,
    /// Tickets made in the window → their first assignment.
    pub queue_wait: Stat,
    /// Tickets made in the window → their work merged into main.
    pub ticket_to_merge: Stat,
    pub tickets_created: usize,
    pub meta_tickets: usize,
    /// Meta tickets of all made, in percent (one decimal, times ten).
    pub meta_share_permille: u64,
    pub tokens: u64,
    pub meta_tokens: u64,
    pub meta_token_share_permille: u64,
    /// Who made the window's tickets, the most first.
    pub by_creator: Vec<(String, usize)>,
    /// Baton passes inside tickets ([`BATON`]).
    pub baton_passes: usize,
    /// Work its member landed itself ([`SELF_LANDED`]).
    pub self_landed: usize,
    /// The slowest requests to production (or still open), at most five.
    pub slowest: Vec<RequestLine>,
}

/// Whether a ticket is meta work — about other work rather than the change
/// itself: a check (QA, verifying, reviewing), equipe's own (design QA,
/// reviews, audits), or a design follow-up that is polish.
pub fn is_meta(t: &TicketRow, polish: &HashSet<u64>) -> bool {
    t.label.as_deref() == Some("check") || t.created_by == EQUIPE || polish.contains(&t.id)
}

/// `T-123` in `text`: the first ticket it names.
fn named_ticket(text: &str) -> Option<u64> {
    let at = text.find("T-")?;
    let digits: String = text[at + 2..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Each ticket's parent: the ticket a design QA checks, a design follow-up's
/// ticket, or the ticket its maker was at work on when it was made.
fn parents(input: &Input) -> HashMap<u64, u64> {
    let mut parent: HashMap<u64, u64> = HashMap::new();
    for (ticket, of) in &input.follow_ups {
        parent.insert(*ticket, *of);
    }
    for t in &input.tickets {
        if let Some(qa) = t.design_qa.filter(|qa| *qa != t.id) {
            parent.entry(qa).or_insert(t.id);
        }
    }
    for e in input.events.iter().filter(|e| e.kind == "created" && e.by == EQUIPE) {
        if let Some(of) = named_ticket(&e.text).filter(|of| *of != e.ticket) {
            parent.entry(e.ticket).or_insert(of);
        }
    }
    // The turn its maker was in when it was made.
    let mut turns: HashMap<&str, Vec<&TurnRow>> = HashMap::new();
    for turn in input.turns.iter().filter(|t| t.ticket.is_some()) {
        turns.entry(turn.member.as_str()).or_default().push(turn);
    }
    for t in &input.tickets {
        if parent.contains_key(&t.id) || t.created_by == EQUIPE || t.created_by == PERSON {
            continue;
        }
        let at_work = turns.get(t.created_by.as_str()).and_then(|ts| ts.iter().find(|turn| turn.began <= t.created_at && t.created_at <= turn.ended + 5 && turn.ticket != Some(t.id)));
        if let Some(on) = at_work.and_then(|turn| turn.ticket) {
            parent.insert(t.id, on);
        }
    }
    parent
}

/// A request as the report weighs it: when it was asked, its title, its tickets.
type Asked = (u64, String, Vec<u64>);

/// The requests of an older journal, worked out (the module's words).
fn reconstruct(input: &Input, parent: &HashMap<u64, u64>, since: u64, until: u64) -> Vec<Asked> {
    let mut said = input.person_said.clone();
    said.sort_unstable();
    // The words each root answers: keyed by when they were said.
    let mut by_words: BTreeMap<u64, (String, Vec<u64>)> = BTreeMap::new();
    for t in &input.tickets {
        if parent.contains_key(&t.id) || t.created_by == EQUIPE || t.created_at < since.saturating_sub(ASKED_WITHIN) || t.created_at > until {
            continue;
        }
        let asked = if t.created_by == PERSON {
            Some(t.created_at)
        } else {
            let before = said.partition_point(|at| *at <= t.created_at);
            before.checked_sub(1).map(|i| said[i]).filter(|at| t.created_at - at <= ASKED_WITHIN)
        };
        let Some(asked) = asked.filter(|at| (since..=until).contains(at)) else { continue };
        // The person's own tickets are a request each.
        let key = if t.created_by == PERSON { asked * 1_000_000 + t.id } else { asked * 1_000_000 };
        by_words.entry(key).or_insert_with(|| (t.title.clone(), Vec::new())).1.push(t.id);
    }
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    for (child, of) in parent {
        children.entry(*of).or_default().push(*child);
    }
    by_words
        .into_iter()
        .map(|(key, (title, roots))| {
            let mut all = Vec::new();
            let mut seen = HashSet::new();
            let mut next = roots;
            while let Some(t) = next.pop() {
                if seen.insert(t) {
                    all.push(t);
                    next.extend(children.get(&t).into_iter().flatten().copied());
                }
            }
            all.sort_unstable();
            (key / 1_000_000, title, all)
        })
        .collect()
}

/// When a ticket's work first reached main.
fn merged_at(events: &[&EventRow]) -> Option<u64> {
    events.iter().filter(|e| e.kind == "merged" && !e.text.starts_with("not")).map(|e| e.at).min()
}

/// When work on a ticket first started: assigned, taken on, or a baton passed to someone.
fn started_at(events: &[&EventRow]) -> Option<u64> {
    events.iter().filter(|e| matches!(e.kind.as_str(), "assigned" | "taken on" | BATON)).map(|e| e.at).min()
}

fn closed(status: &str) -> bool {
    matches!(status, "done" | "cancelled")
}

/// The flow of `[since, until]`.
pub fn summarize(input: &Input, since: u64, until: u64) -> Report {
    let tickets: HashMap<u64, &TicketRow> = input.tickets.iter().map(|t| (t.id, t)).collect();
    let mut events: HashMap<u64, Vec<&EventRow>> = HashMap::new();
    for e in &input.events {
        events.entry(e.ticket).or_default().push(e);
    }
    for list in events.values_mut() {
        list.sort_by_key(|e| e.at);
    }
    let polish: HashSet<u64> = input
        .follow_up_kinds
        .iter()
        .filter(|(_, kind, severity)| kind == "polish" || kind == "backlog" || matches!(severity.as_deref(), Some("s3" | "s4" | "S3" | "S4")))
        .map(|(t, _, _)| *t)
        .collect();
    let meta = |id: u64| tickets.get(&id).is_some_and(|t| is_meta(t, &polish));
    let no_events: Vec<&EventRow> = Vec::new();
    let events_of = |id: u64| events.get(&id).unwrap_or(&no_events);

    let parent = parents(input);
    let (source, requests): (&'static str, Vec<Asked>) = match &input.requests {
        Some(rows) if !rows.is_empty() => (
            "requests",
            rows.iter().filter(|r| (since..=until).contains(&r.asked_at)).map(|r| (r.asked_at, r.title.clone(), r.tickets.iter().map(|(t, _)| *t).collect())).collect(),
        ),
        _ => ("reconstructed", reconstruct(input, &parent, since, until)),
    };

    let mut report = Report { since, until, source, requests: requests.len(), ..Default::default() };
    let (mut to_start, mut to_merge, mut to_production, mut sizes, mut handoffs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut lines = Vec::new();
    for (asked, title, ids) in &requests {
        let work: Vec<u64> = {
            let w: Vec<u64> = ids.iter().copied().filter(|id| !meta(*id)).collect();
            if w.is_empty() { ids.clone() } else { w }
        };
        let start = work.iter().filter_map(|id| started_at(events_of(*id))).min();
        let first_merge = ids.iter().filter_map(|id| merged_at(events_of(*id))).min();
        // Production: a deploy ticket done, else everything of it finished.
        let deploy_ticket = work.iter().filter_map(|id| tickets.get(id)).filter(|t| t.label.as_deref() == Some("deploy") && t.status == "done").map(|t| t.updated_at).min();
        // Every piece of its code live by a deploy signal: the last of them.
        let code: Vec<u64> = work.iter().copied().filter(|id| tickets.get(id).is_some_and(|t| t.kind == "code" && t.status == "done" && t.merged == Some(true))).collect();
        let signalled = (!code.is_empty() && code.iter().all(|id| input.live.contains_key(id))).then(|| code.iter().filter_map(|id| input.live.get(id).copied()).max()).flatten();
        let deploy = signalled.or(deploy_ticket);
        let finished = work.iter().all(|id| tickets.get(id).is_none_or(|t| closed(&t.status) && (t.kind != "code" || t.status == "cancelled" || t.merged == Some(true))))
            && work.iter().any(|id| tickets.get(id).is_some_and(|t| t.status == "done"));
        // Finished, and nothing of it merged, waits to merge or deployed: a
        // decision or an instruction answered — its production is not pending.
        let merged_any = ids.iter().any(|id| merged_at(events_of(*id)).is_some() || tickets.get(id).is_some_and(|t| t.merged == Some(true)));
        let awaiting_merge = work.iter().any(|id| tickets.get(id).is_some_and(|t| t.kind == "code" && t.status == "done" && t.merged == Some(false)));
        let all_closed = work.iter().all(|id| tickets.get(id).is_none_or(|t| closed(&t.status)));
        let decided = deploy.is_none() && !merged_any && !awaiting_merge && all_closed;
        let live = if decided {
            report.decided += 1;
            None
        } else if deploy.is_some() {
            report.production_by_deploy += 1;
            deploy
        } else if finished {
            work.iter()
                .filter_map(|id| tickets.get(id))
                .filter(|t| t.status == "done")
                .map(|t| if t.kind == "code" { merged_at(events_of(t.id)).unwrap_or(t.updated_at) } else { t.updated_at })
                .max()
        } else {
            None
        };
        // Who held the work, in order: each change is a hand-off.
        let mut moves: Vec<(u64, String)> = Vec::new();
        for id in &work {
            for e in events_of(*id) {
                match e.kind.as_str() {
                    "assigned" | "taken on" => moves.push((e.at, e.text.trim().to_owned())),
                    BATON => moves.push((e.at, e.text.split([' ', '·', ':']).next().unwrap_or_default().to_owned())),
                    _ => {}
                }
            }
        }
        moves.sort();
        let changes = moves.windows(2).filter(|w| w[0].1 != w[1].1).count();
        let since_asked = |at: Option<u64>| at.map(|at| at.saturating_sub(*asked));
        let line = RequestLine {
            title: title.clone(),
            asked_at: *asked,
            tickets: ids.len(),
            handoffs: changes,
            to_start: since_asked(start),
            to_first_merge: since_asked(first_merge),
            to_production: since_asked(live),
            decided,
        };
        to_start.extend(line.to_start);
        to_merge.extend(line.to_first_merge);
        to_production.extend(line.to_production);
        sizes.push(ids.len() as u64);
        handoffs.push(changes as u64);
        lines.push(line);
    }
    report.to_start = Stat::of(to_start);
    report.to_first_merge = Stat::of(to_merge);
    report.to_production = Stat::of(to_production);
    report.tickets_per_request = Stat::of(sizes);
    report.handoffs_per_request = Stat::of(handoffs);
    // The slowest: still open ones (measured to `until`) and the longest to production.
    // A decided one never waited for production.
    lines.retain(|l| !l.decided);
    lines.sort_by_key(|l| std::cmp::Reverse(l.to_production.unwrap_or(until.saturating_sub(l.asked_at).saturating_add(1))));
    report.slowest = lines.into_iter().take(5).collect();

    // The window's tickets.
    let made: Vec<&TicketRow> = input.tickets.iter().filter(|t| (since..=until).contains(&t.created_at)).collect();
    report.tickets_created = made.len();
    report.meta_tickets = made.iter().filter(|t| is_meta(t, &polish)).count();
    report.meta_share_permille = permille(report.meta_tickets as u64, report.tickets_created as u64);
    let mut creators: BTreeMap<String, usize> = BTreeMap::new();
    for t in &made {
        *creators.entry(t.created_by.clone()).or_default() += 1;
    }
    let mut by_creator: Vec<(String, usize)> = creators.into_iter().collect();
    by_creator.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.by_creator = by_creator;
    report.queue_wait = Stat::of(made.iter().filter_map(|t| events_of(t.id).iter().find(|e| e.kind == "assigned").map(|e| e.at.saturating_sub(t.created_at))).collect());
    report.ticket_to_merge = Stat::of(made.iter().filter_map(|t| merged_at(events_of(t.id)).map(|at| at.saturating_sub(t.created_at))).collect());
    for turn in input.turns.iter().filter(|t| (since..=until).contains(&t.ended)) {
        report.tokens += turn.tokens;
        if turn.ticket.is_some_and(meta) {
            report.meta_tokens += turn.tokens;
        }
    }
    report.meta_token_share_permille = permille(report.meta_tokens, report.tokens);
    let in_window = |e: &&EventRow| (since..=until).contains(&e.at);
    report.baton_passes = input.events.iter().filter(in_window).filter(|e| e.kind == BATON).count();
    report.self_landed = input.events.iter().filter(in_window).filter(|e| e.kind == SELF_LANDED).count();
    report
}

fn permille(part: u64, whole: u64) -> u64 {
    (part * 1000 + whole / 2).checked_div(whole).unwrap_or(0)
}

/// `permille` as people read a share: `43.1`.
pub fn percent(permille: u64) -> String {
    format!("{}.{}", permille / 10, permille % 10)
}

/// A span of seconds as people read it: `48m`, `3h05m`, `2d4h`.
pub fn span(secs: u64) -> String {
    let m = secs / 60;
    match m {
        0..60 => format!("{m}m"),
        60..1440 => format!("{}h{:02}m", m / 60, m % 60),
        _ => format!("{}d{}h", m / 1440, (m % 1440) / 60),
    }
}

// ── reading ──

/// Whether `conn` has `table`.
fn has_table(conn: &Connection, table: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |_| Ok(())).is_ok()
}

/// Whether the person wrote `text` (not equipe, a tool's output or a note it
/// sent in their place): what a request is made of in an older journal.
fn persons_words(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && !text.starts_with('<') && !text.starts_with('[')
}

/// The rows of `[since, until]` in the journal at `db`, opened read-only.
/// Tickets made a little before the window are read too: a request asked in
/// it may have started from words said just before.
pub fn read(db: &Path, since: u64, until: u64) -> Result<Input, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("{}: {e}", db.display()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    select(&conn, since, until).map_err(|e| e.to_string())
}

fn select(conn: &Connection, since: u64, until: u64) -> rusqlite::Result<Input> {
    let from = since.saturating_sub(ASKED_WITHIN) as i64;
    let mut input = Input::default();
    let mut stmt = conn.prepare(
        "SELECT id, title, status, kind, label, created_by, created_at, updated_at, merged, design_qa FROM tickets WHERE created_at >= ?1 AND created_at <= ?2",
    )?;
    input.tickets = stmt
        .query_map([from, until as i64], |r| {
            Ok(TicketRow {
                id: r.get::<_, i64>(0)? as u64,
                title: r.get(1)?,
                status: r.get(2)?,
                kind: r.get(3)?,
                label: r.get(4)?,
                created_by: r.get(5)?,
                created_at: r.get::<_, i64>(6)?.max(0) as u64,
                updated_at: r.get::<_, i64>(7)?.max(0) as u64,
                merged: r.get::<_, Option<i64>>(8)?.map(|m| m != 0),
                design_qa: r.get::<_, Option<i64>>(9)?.map(|t| t as u64),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let lowest = input.tickets.iter().map(|t| t.id).min().unwrap_or(0) as i64;
    let mut stmt = conn.prepare(
        "SELECT ticket, at, by, kind, text FROM ticket_events WHERE ticket >= ?1
         AND kind IN ('created', 'assigned', 'taken on', 'merged', 'baton', 'landed by its member', 'accepted', 'accepted by the PM')",
    )?;
    input.events = stmt
        .query_map([lowest], |r| Ok(EventRow { ticket: r.get::<_, i64>(0)? as u64, at: r.get::<_, i64>(1)?.max(0) as u64, by: r.get(2)?, kind: r.get(3)?, text: r.get(4)? }))?
        .collect::<rusqlite::Result<_>>()?;
    if has_table(conn, "design_follow_ups") {
        let mut stmt = conn.prepare("SELECT ticket, parent, kind, severity FROM design_follow_ups WHERE ticket >= ?1")?;
        let rows: Vec<(u64, u64, String, Option<String>)> =
            stmt.query_map([lowest], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        input.follow_ups = rows.iter().map(|(t, p, _, _)| (*t, *p)).collect();
        input.follow_up_kinds = rows.into_iter().map(|(t, _, k, s)| (t, k, s)).collect();
    }
    let mut stmt = conn.prepare("SELECT member, ticket, began, ended, tokens FROM turns WHERE ended >= ?1 AND began <= ?2")?;
    input.turns = stmt
        .query_map([from, until as i64], |r| {
            Ok(TurnRow {
                member: r.get(0)?,
                ticket: r.get::<_, Option<i64>>(1)?.map(|t| t as u64),
                began: r.get::<_, i64>(2)?.max(0) as u64,
                ended: r.get::<_, i64>(3)?.max(0) as u64,
                tokens: r.get::<_, i64>(4)?.max(0) as u64,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if has_table(conn, "requests") && has_table(conn, "request_tickets") {
        let mut stmt = conn.prepare("SELECT id, title, asked_at FROM requests WHERE asked_at >= ?1 AND asked_at <= ?2")?;
        let mut rows: Vec<RequestRow> = stmt
            .query_map([since as i64, until as i64], |r| Ok(RequestRow { id: r.get::<_, i64>(0)? as u64, title: r.get(1)?, asked_at: r.get::<_, i64>(2)?.max(0) as u64, tickets: Vec::new() }))?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = conn.prepare("SELECT ticket, link FROM request_tickets WHERE request = ?1")?;
        for row in &mut rows {
            row.tickets = stmt.query_map([row.id as i64], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<_>>()?;
        }
        // Words no ticket came of — thanks, a question answered — took no work to measure.
        rows.retain(|row| !row.tickets.is_empty());
        input.requests = Some(rows);
    }
    // What went live by a deploy signal (`crate::deploys`, kept in `meta`).
    if let Ok(json) = conn.query_row("SELECT value FROM meta WHERE key = 'deploys'", [], |r| r.get::<_, String>(0)) {
        let deploys: Vec<equipe_protocol::deploys::Deploy> = serde_json::from_str(&json).unwrap_or_default();
        for d in deploys.iter().filter(|d| d.source.is_live()) {
            for t in &d.tickets {
                let at = input.live.entry(*t as u64).or_insert(d.at);
                *at = (*at).min(d.at);
            }
        }
    }
    // An older journal: the person's words, for working its requests out.
    if input.requests.as_ref().is_none_or(Vec::is_empty) {
        let mut stmt = conn.prepare("SELECT at, body FROM entries WHERE at >= ?1 AND at <= ?2 AND body LIKE '{\"kind\":\"user\"%'")?;
        let rows: Vec<(i64, String)> = stmt.query_map([from, until as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        input.person_said = rows
            .into_iter()
            .filter(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(persons_words)).unwrap_or(false))
            .map(|(at, _)| at.max(0) as u64)
            .collect();
    }
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600;
    const M: u64 = 60;

    fn ticket(id: u64, by: &str, at: u64, status: &str, label: Option<&str>) -> TicketRow {
        TicketRow { id, title: format!("T{id}"), status: status.into(), kind: "code".into(), label: label.map(Into::into), created_by: by.into(), created_at: at, updated_at: at, merged: None, design_qa: None }
    }

    fn event(ticket: u64, at: u64, kind: &str, text: &str) -> EventRow {
        EventRow { ticket, at, by: "equipe".into(), kind: kind.into(), text: text.into() }
    }

    /// The relay the owner saw: the person asks the designer at 10:00; a spec
    /// ticket, its design QA, a frontend build made while the spec was worked,
    /// its design QA and a verify ticket — five tickets, three hands.
    fn relay() -> Input {
        let t0 = 10 * H;
        let mut spec = ticket(1, "designer", t0 + 2 * M, "done", Some("design"));
        spec.merged = Some(true);
        spec.updated_at = t0 + 60 * M;
        let mut build = ticket(3, "designer", t0 + 30 * M, "done", Some("feature"));
        build.merged = Some(true);
        build.updated_at = t0 + 185 * M;
        let qa1 = ticket(2, EQUIPE, t0 + 40 * M, "done", None);
        let qa2 = ticket(4, EQUIPE, t0 + 150 * M, "done", None);
        let verify = ticket(5, "pm", t0 + 190 * M, "done", Some("check"));
        Input {
            tickets: vec![spec, qa1, build, qa2, verify],
            events: vec![
                event(1, t0 + 50 * M, "assigned", "designer"),
                event(2, t0 + 40 * M, "created", "design QA of T-1"),
                event(2, t0 + 41 * M, "assigned", "designer"),
                event(1, t0 + 58 * M, "merged", "main"),
                event(3, t0 + 90 * M, "assigned", "frontend"),
                event(4, t0 + 150 * M, "created", "design QA of T-3"),
                event(3, t0 + 185 * M, "merged", "main"),
                event(5, t0 + 200 * M, "assigned", "qa"),
            ],
            turns: vec![
                TurnRow { member: "designer".into(), ticket: Some(1), began: t0 + 25 * M, ended: t0 + 35 * M, tokens: 100 },
                TurnRow { member: "designer".into(), ticket: Some(2), began: t0 + 41 * M, ended: t0 + 45 * M, tokens: 50 },
                TurnRow { member: "pm".into(), ticket: Some(3), began: t0 + 188 * M, ended: t0 + 195 * M, tokens: 50 },
            ],
            person_said: vec![t0],
            ..Default::default()
        }
    }

    #[test]
    fn an_older_journal_s_request_is_worked_out_from_the_person_s_words_and_lineage() {
        let r = summarize(&relay(), 0, 24 * H);
        assert_eq!(r.source, "reconstructed");
        assert_eq!(r.requests, 1);
        // The spec answered the person; the build was made during the spec's turn,
        // the QAs check them, the verify ticket was made while the PM was on T-3.
        assert_eq!(r.tickets_per_request.median, Some(5));
        assert_eq!(r.to_start.median, Some(50 * M));
        assert_eq!(r.to_first_merge.median, Some(58 * M));
        // Not live until the verify ticket (meta) — production reads the work, not the checks.
        assert_eq!(r.to_production.median, Some(185 * M));
        // designer → frontend.
        assert_eq!(r.handoffs_per_request.median, Some(1));
        assert_eq!(r.meta_tickets, 3);
        assert_eq!(r.meta_share_permille, 600);
        assert_eq!(r.meta_tokens, 50);
        assert_eq!(r.tokens, 200);
        assert_eq!(r.queue_wait.count, 4);
        assert_eq!(r.ticket_to_merge.count, 2);
    }

    #[test]
    fn a_member_s_ticket_long_after_the_person_spoke_is_no_request_of_theirs() {
        let mut input = relay();
        input.person_said = vec![5 * H];
        assert_eq!(summarize(&input, 0, 24 * H).requests, 0);
        // The person's own ticket always is.
        input.tickets.push(ticket(9, PERSON, 12 * H, "todo", None));
        let r = summarize(&input, 0, 24 * H);
        assert_eq!(r.requests, 1);
        assert_eq!(r.to_production.count, 0);
        assert_eq!(r.slowest[0].title, "T9");
    }

    #[test]
    fn the_journal_s_requests_are_read_as_they_are_and_a_baton_counts_as_a_hand_off() {
        let t0 = 10 * H;
        let mut main = ticket(1, "designer", t0 + M, "done", Some("feature"));
        main.merged = Some(true);
        let input = Input {
            tickets: vec![main],
            events: vec![
                event(1, t0 + 2 * M, "assigned", "designer"),
                event(1, t0 + 40 * M, BATON, "frontend · the prototype is in design/"),
                event(1, t0 + 41 * M, "assigned", "frontend"),
                event(1, t0 + 90 * M, SELF_LANDED, "tests and precheck passed"),
                event(1, t0 + 90 * M, "merged", "main"),
            ],
            requests: Some(vec![RequestRow { id: 1, title: "New input box".into(), asked_at: t0, tickets: vec![(1, "asked".into())] }]),
            ..Default::default()
        };
        let r = summarize(&input, 0, 24 * H);
        assert_eq!(r.source, "requests");
        assert_eq!(r.tickets_per_request.median, Some(1));
        assert_eq!(r.handoffs_per_request.median, Some(1));
        assert_eq!(r.to_first_merge.median, Some(90 * M));
        assert_eq!(r.to_production.median, Some(90 * M));
        assert_eq!((r.baton_passes, r.self_landed), (1, 1));
    }

    #[test]
    fn a_deploy_ticket_done_is_production() {
        let mut input = relay();
        let mut deploy = ticket(6, "designer", 10 * H + 31 * M, "done", Some("deploy"));
        deploy.updated_at = 10 * H + 300 * M;
        deploy.kind = "action".into();
        input.tickets.push(deploy);
        let r = summarize(&input, 0, 24 * H);
        assert_eq!(r.to_production.median, Some(300 * M));
        assert_eq!(r.production_by_deploy, 1);
    }

    #[test]
    fn a_deploy_signal_for_every_piece_of_its_code_is_production() {
        let mut input = relay();
        input.live = HashMap::from([(1, 10 * H + 70 * M), (3, 10 * H + 200 * M)]);
        let r = summarize(&input, 0, 24 * H);
        assert_eq!(r.to_production.median, Some(200 * M));
        assert_eq!(r.production_by_deploy, 1);
        // One piece not seen live: as before, its last merge.
        input.live.remove(&3);
        assert_eq!(summarize(&input, 0, 24 * H).to_production.median, Some(185 * M));
    }

    /// "Go with B", "skip the test and ship it": words that decide rather
    /// than ask for work. Its ticket finished with nothing merged: answered,
    /// apart from production — never "not in production yet", never in the
    /// production lead time.
    #[test]
    fn a_request_answered_with_nothing_merged_is_decided_not_waiting_for_production() {
        let t0 = 10 * H;
        let mut shipped = ticket(1, "frontend", t0 + M, "done", Some("feature"));
        shipped.merged = Some(true);
        let mut decision = ticket(2, "pm", t0 + 3 * H + M, "done", None);
        decision.kind = "task".into();
        decision.updated_at = t0 + 3 * H + 5 * M;
        let open = ticket(3, "pm", t0 + 5 * H + M, "in_progress", None);
        let mut waits_to_merge = ticket(4, "backend", t0 + 6 * H + M, "done", None);
        waits_to_merge.merged = Some(false);
        let input = Input {
            tickets: vec![shipped, decision, open, waits_to_merge],
            events: vec![event(1, t0 + 2 * M, "assigned", "frontend"), event(1, t0 + 50 * M, "merged", "main"), event(2, t0 + 3 * H + 2 * M, "assigned", "pm")],
            requests: Some(vec![
                RequestRow { id: 1, title: "New input box".into(), asked_at: t0, tickets: vec![(1, "asked".into())] },
                RequestRow { id: 2, title: "Go with B".into(), asked_at: t0 + 3 * H, tickets: vec![(2, "made".into())] },
                RequestRow { id: 3, title: "Still being worked".into(), asked_at: t0 + 5 * H, tickets: vec![(3, "made".into())] },
                RequestRow { id: 4, title: "Accepted, not merged yet".into(), asked_at: t0 + 6 * H, tickets: vec![(4, "made".into())] },
            ]),
            ..Default::default()
        };
        let r = summarize(&input, 0, 24 * H);
        assert_eq!(r.requests, 4);
        assert_eq!(r.decided, 1, "only the decision: open work and code waiting to merge may still ship");
        assert_eq!((r.to_production.count, r.to_production.median), (1, Some(50 * M)), "the decision's 5 minutes aren't a production time");
        assert!(!r.slowest.iter().any(|l| l.title == "Go with B"), "never listed as waiting for production: {:?}", r.slowest);
        assert!(r.slowest.iter().any(|l| l.title == "Still being worked" && l.to_production.is_none() && !l.decided));
        // A deploy ticket done is production, not a decision ("ship it now").
        let mut deploy = ticket(5, "pm", t0 + 8 * H + M, "done", Some("deploy"));
        deploy.kind = "action".into();
        deploy.updated_at = t0 + 8 * H + 30 * M;
        let mut with_deploy = input.clone();
        with_deploy.tickets.push(deploy);
        with_deploy.requests.as_mut().unwrap().push(RequestRow { id: 5, title: "Ship it now".into(), asked_at: t0 + 8 * H, tickets: vec![(5, "made".into())] });
        let r = summarize(&with_deploy, 0, 24 * H);
        assert_eq!((r.decided, r.production_by_deploy, r.to_production.count), (1, 1, 2));
    }

    #[test]
    fn stats_are_nearest_rank() {
        let s = Stat::of((1..=10).collect());
        assert_eq!((s.count, s.median, s.p90), (10, Some(5), Some(9)));
        assert_eq!(Stat::of(vec![]).median, None);
        assert_eq!(span(48 * M), "48m");
        assert_eq!(span(185 * M), "3h05m");
        assert_eq!(percent(431), "43.1");
    }

    #[test]
    fn it_reads_a_journal_without_requests_and_one_with_them() {
        let dir = std::env::temp_dir().join(format!("equipe-lead-time-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("squad.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE tickets (id INTEGER PRIMARY KEY, title TEXT, status TEXT, kind TEXT, label TEXT, created_by TEXT, created_at INTEGER, updated_at INTEGER, merged INTEGER, design_qa INTEGER);
             CREATE TABLE ticket_events (ticket INTEGER, seq INTEGER, at INTEGER, by TEXT, kind TEXT, text TEXT);
             CREATE TABLE turns (member TEXT, role TEXT, ticket INTEGER, began INTEGER, ended INTEGER, outcome TEXT, tokens INTEGER);
             CREATE TABLE entries (session INTEGER, seq INTEGER, at INTEGER, body TEXT);
             INSERT INTO tickets VALUES (1, 'Input box', 'done', 'code', 'feature', 'designer', 1100, 2000, 1, NULL);
             INSERT INTO ticket_events VALUES (1, 1, 1200, 'equipe', 'assigned', 'designer'), (1, 2, 1900, 'you', 'merged', 'main');
             INSERT INTO entries VALUES (7, 1, 1000, '{\"kind\":\"user\",\"text\":\"a new input box\"}'), (7, 2, 1050, '{\"kind\":\"user\",\"text\":\"[equipe] not theirs\"}');",
        )
        .unwrap();
        let r = summarize(&read(&db, 0, 5000).unwrap(), 0, 5000);
        assert_eq!((r.source, r.requests, r.to_first_merge.median), ("reconstructed", 1, Some(900)));
        crate::requests::create_table(&conn).unwrap();
        conn.execute_batch("INSERT INTO requests (id, title, words, asked_at) VALUES (1, 'Input box', 'a new input box', 1000); INSERT INTO request_tickets VALUES (1, 1, NULL, 'asked', 1100);").unwrap();
        let r = summarize(&read(&db, 0, 5000).unwrap(), 0, 5000);
        assert_eq!((r.source, r.requests, r.to_production.median), ("requests", 1, Some(900)));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
