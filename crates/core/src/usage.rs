//! Where a squad's tokens went, and which of that looks wasted
//! (`equipe usage`, the daily usage note, docs/USAGE.md).
//!
//! Everything is read from the squad's journal: `turns` (one row per member
//! turn, with its tokens by kind), `tickets` (title and status, for the
//! tickets a turn ran on) and `turn_endings` (turns that ended other than as
//! they should). [`summarize`] is pure — rows in, [`Report`] out — so it is
//! tested on fixture rows; [`read`] and [`crate::journal::Journal::usage_input`]
//! only fetch the rows.
//!
//! Tokens here are the engines' own sums: most of a long-running squad's
//! tokens are *cache reads* (the whole conversation re-read on every model
//! call), so the number that says how heavy a turn is isn't its total but
//! its **context per call**: what one request carried in, on average
//! (`input + cache read + cache write`, over the turn's calls). Writes kept
//! for an hour are part of the cache writes already, not added again.
//!
//! The *waste signals* ([`Signal`]) are the few measures that were worth
//! acting on in practice, each reported only over its threshold constant.
//! They are hints, not verdicts: a big context can be what a big task needs.
//!
//! Days are UTC calendar days, as everywhere in the retro numbers.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::i18n::Catalog;
use crate::runtime::turn_ending;

const DAY: u64 = 24 * 60 * 60;

// ── thresholds ──

/// Below this many tokens in the window, the share-based signals say
/// nothing: a quiet squad's percentages are noise.
pub const MIN_TOKENS_FOR_SHARES: u64 = 50_000_000;
/// The PM desk (its turns with no ticket) is a signal when it reads more
/// than this per call on average …
pub const PM_DESK_CONTEXT: u64 = 400_000;
/// … and is at least this share of all tokens (in percent).
pub const PM_DESK_MIN_SHARE_PERCENT: f64 = 5.0;
/// A turn is *heavy* when its context per call is over this.
pub const HEAVY_CONTEXT: u64 = 400_000;
/// Heavy turns are a signal when they hold at least this share of all
/// tokens (in percent).
pub const HEAVY_SHARE_PERCENT: f64 = 25.0;
/// Tokens spent on tickets that were cancelled afterwards are a signal at
/// this share of all tokens (in percent).
pub const CANCELLED_SHARE_PERCENT: f64 = 3.0;
/// A ticket with this many turns (or more) in the window is a rework loop.
pub const REWORK_TURNS: u64 = 15;
/// Rework loops are a signal when together they hold at least this share of
/// all tokens (in percent).
pub const REWORK_SHARE_PERCENT: f64 = 5.0;
/// Abnormal turn endings (limited, failed fast, handed back) are a signal at
/// this many in the window.
pub const ENDINGS_MIN: u64 = 5;
/// Context-per-call bucket edges, in tokens: under the first, up to the
/// second, up to the third, and above [`HEAVY_CONTEXT`] (the last edge).
pub const CONTEXT_EDGES: [u64; 3] = [100_000, 200_000, HEAVY_CONTEXT];
/// How many tickets the top lists keep.
pub const TOP_TICKETS: usize = 10;
/// How many tickets a signal's sentence names.
const NAMED_TICKETS: usize = 3;
/// How many days before the last day the daily note compares it with.
pub const DIGEST_COMPARE_DAYS: u64 = 7;

// ── rows in ──

/// One member turn, as kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnRow {
    pub member: String,
    pub role: String,
    pub ticket: Option<u64>,
    pub ended: u64,
    /// The turn's sum, as kept for every turn.
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// Everything written to the cache, the hour-long writes included.
    pub cache_write: u64,
    pub calls: u64,
    pub engine: Option<String>,
    pub model: Option<String>,
    /// The turn's tokens were kept by kind (turns from before that are only a sum).
    pub measured: bool,
}

impl TurnRow {
    /// What one request of the turn carried in, on average; `None` for a
    /// turn kept only as a sum or with no calls counted.
    pub fn context_per_call(&self) -> Option<u64> {
        (self.measured && self.calls > 0).then(|| self.context() / self.calls)
    }

    fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

/// A ticket's title and status, as the board has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketInfo {
    pub title: String,
    pub status: String,
}

/// Everything [`summarize`] reads.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub turns: Vec<TurnRow>,
    pub tickets: HashMap<u64, TicketInfo>,
    /// Turn endings in the window: kind and how many.
    pub endings: Vec<(String, u64)>,
    /// The records of tickets handed in or accepted in the window, from
    /// [`EVENT_KINDS`] only, oldest first.
    pub events: Vec<EventRow>,
    /// Every turn and token a ticket took, whenever they ran: `(turns, tokens)`.
    pub ticket_totals: HashMap<u64, (u64, u64)>,
    /// Sends-back in the window, by reason.
    pub send_backs: Vec<(String, u64)>,
    /// Design QA runs begun in the window: verdict (`none` while open), decision (`none`), count.
    pub design_qa: Vec<(String, String, u64)>,
}

/// One entry of a ticket's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub ticket: u64,
    pub at: u64,
    pub kind: String,
}

/// What sends handed-in work back to its member (the same as `quality`'s).
const SENT_BACK: [&str; 3] = ["reopened", "sent back", "design QA: changes"];
/// The record entries the quality numbers read; `accepted*` also matches
/// `accepted by the PM`.
pub const EVENT_KINDS: [&str; 7] = ["created", "assigned", "handed in", "accepted", "accepted by the PM", "reopened", "sent back"];

// ── the report ──

/// Tokens by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Split {
    /// The sum over every turn, those kept only as a sum included.
    pub total: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Of `total`, from turns kept only as a sum (no split known).
    pub unsplit: u64,
}

/// A line of a breakdown: a name, its turns and tokens, and its share.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Row {
    pub name: String,
    pub turns: u64,
    pub tokens: u64,
    pub share_percent: f64,
    /// Lane members folded into this (members only).
    #[serde(skip_serializing_if = "is_zero")]
    pub lanes: u64,
    /// Calls counted, and the average context per call.
    #[serde(skip_serializing_if = "is_zero")]
    pub calls: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_per_call: Option<u64>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One bucket of context per call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bucket {
    /// `<100k`, `100-200k`, `200-400k`, `>400k`, or `unknown` (kept only as a sum).
    pub label: String,
    pub turns: u64,
    pub tokens: u64,
    pub share_percent: f64,
}

/// A ticket's turns and tokens in the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TicketRow {
    pub id: u64,
    pub title: String,
    pub status: String,
    pub turns: u64,
    pub tokens: u64,
}

/// What kind of measure a [`Signal`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// The PM's desk conversation reads a huge context on every call.
    PmDesk,
    /// A big share of tokens went to turns with a huge context per call.
    HeavyContext,
    /// Tokens went to tickets that were cancelled afterwards.
    Cancelled,
    /// Tickets that took many turns.
    Rework,
    /// Turns that ended limited, failed fast or handed back.
    Endings,
}

impl SignalKind {
    /// The word it is under in the locale keys.
    pub fn code(self) -> &'static str {
        match self {
            Self::PmDesk => "pm_desk",
            Self::HeavyContext => "heavy_context",
            Self::Cancelled => "cancelled",
            Self::Rework => "rework",
            Self::Endings => "endings",
        }
    }
}

/// One waste signal that is over its threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Signal {
    pub kind: SignalKind,
    pub tokens: u64,
    pub share_percent: f64,
    /// Average context per call ([`SignalKind::PmDesk`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_per_call: Option<u64>,
    /// How many tickets or turns the signal counts.
    pub count: u64,
    /// The biggest tickets it names.
    pub tickets: Vec<TicketRow>,
    /// Turn endings by kind ([`SignalKind::Endings`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endings: Vec<(String, u64)>,
}

/// Everything `equipe usage` says about a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub since: u64,
    pub until: u64,
    pub turns: u64,
    pub tokens: Split,
    /// Per UTC day, oldest first (`name` is `YYYY-MM-DD`).
    pub by_day: Vec<Row>,
    /// Per engine and model (`claude · claude-sonnet-5`), most tokens first.
    pub by_model: Vec<Row>,
    /// Per member, lane members (`frontend-12`) folded into `frontend`.
    pub by_member: Vec<Row>,
    /// By [`kind`], in that order.
    pub by_kind: Vec<Row>,
    pub context: Vec<Bucket>,
    pub top_tickets: Vec<TicketRow>,
    /// Tokens on tickets now cancelled, and the biggest of them.
    pub cancelled_tokens: u64,
    pub cancelled_share_percent: f64,
    pub cancelled_tickets: Vec<TicketRow>,
    /// Tickets with [`REWORK_TURNS`] turns or more, most turns first.
    pub rework: Vec<TicketRow>,
    /// Turn endings by kind, most first.
    pub endings: Vec<(String, u64)>,
    pub signals: Vec<Signal>,
    pub quality: Quality,
}

/// Whether the work that was done was good enough, next to what it cost.
/// Everything is over the same window as the tokens; see [`quality`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Quality {
    /// Tickets the person (or the PM) accepted in the window.
    pub accepted: u64,
    /// Tickets first handed in during the window whose first verdict is in:
    /// accepted as handed in, or sent back.
    pub first_handins_decided: u64,
    pub first_handins_accepted: u64,
    /// Of those decided, the share accepted with no send-back, in percent.
    pub first_handin_accept_percent: Option<f64>,
    /// Send-backs (reopened, sent back, design QA changes) the accepted tickets went through.
    pub send_backs_on_accepted: u64,
    pub send_backs_per_accepted: Option<f64>,
    /// Every send-back kept in the window, by reason (`correctness`, `style`, …).
    pub send_backs_by_reason: Vec<(String, u64)>,
    /// Design QA runs begun in the window, by verdict (`pass`, `changes`, `none`).
    pub design_qa_verdicts: Vec<(String, u64)>,
    /// … and by what the gate decided (`auto_pass`, …, `none`).
    pub design_qa_decisions: Vec<(String, u64)>,
    /// From first assigned to accepted, for the accepted tickets, in seconds.
    pub time_to_done_median_secs: Option<u64>,
    pub time_to_done_p75_secs: Option<u64>,
    /// Turns, and tokens, a done ticket took on average.
    pub turns_per_done: Option<f64>,
    pub tokens_per_done: Option<u64>,
}

/// The kinds a turn is counted under.
pub mod kind {
    /// The PM's conversation with the person: a turn with no ticket.
    pub const PM_DESK: &str = "pm_desk";
    /// A member on a ticket.
    pub const MEMBER_ON_TICKET: &str = "member_on_ticket";
    /// A lane (`frontend-12`) on a ticket.
    pub const LANE_ON_TICKET: &str = "lane_on_ticket";
    /// A turn with no ticket by anyone but the PM.
    pub const OTHER: &str = "other_no_ticket";
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 }
}

/// The member a lane's name stands for: `frontend-12` → `frontend`.
fn base_member(name: &str) -> &str {
    crate::board::role_of(name)
}

fn kind_of(t: &TurnRow) -> &'static str {
    match t.ticket {
        None if t.role == crate::roles::PM => kind::PM_DESK,
        None => kind::OTHER,
        Some(_) if base_member(&t.member) != t.member => kind::LANE_ON_TICKET,
        Some(_) => kind::MEMBER_ON_TICKET,
    }
}

#[derive(Default)]
struct Acc {
    turns: u64,
    tokens: u64,
    calls: u64,
    context: u64,
    lanes: BTreeSet<String>,
}

impl Acc {
    fn add(&mut self, t: &TurnRow) {
        self.turns += 1;
        self.tokens += t.tokens;
        if t.measured {
            self.calls += t.calls;
            self.context += t.context();
        }
    }

    fn row(self, name: String, total: u64) -> Row {
        Row {
            name,
            turns: self.turns,
            tokens: self.tokens,
            share_percent: percent(self.tokens, total),
            lanes: self.lanes.len() as u64,
            calls: self.calls,
            context_per_call: (self.calls > 0).then(|| self.context / self.calls),
        }
    }
}

fn rows(map: BTreeMap<String, Acc>, total: u64) -> Vec<Row> {
    let mut out: Vec<Row> = map.into_iter().map(|(name, acc)| acc.row(name, total)).collect();
    out.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
    out
}

fn bucket_label(context: Option<u64>) -> &'static str {
    let [a, b, c] = CONTEXT_EDGES;
    match context {
        None => "unknown",
        Some(n) if n < a => "<100k",
        Some(n) if n < b => "100-200k",
        Some(n) if n <= c => "200-400k",
        Some(_) => ">400k",
    }
}

#[derive(Default)]
struct PerTicket {
    turns: u64,
    tokens: u64,
}

/// Every measure over `input`'s rows, for the window `since`..`until`.
pub fn summarize(input: &Input, since: u64, until: u64) -> Report {
    let mut r = Report { since, until, turns: input.turns.len() as u64, ..Report::default() };
    let mut days: BTreeMap<String, Acc> = BTreeMap::new();
    let mut models: BTreeMap<String, Acc> = BTreeMap::new();
    let mut members: BTreeMap<String, Acc> = BTreeMap::new();
    let mut kinds: BTreeMap<String, Acc> = BTreeMap::new();
    let mut buckets: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut tickets: BTreeMap<u64, PerTicket> = BTreeMap::new();
    let mut heavy = (0u64, 0u64);

    for t in &input.turns {
        r.tokens.total += t.tokens;
        if t.measured {
            r.tokens.input += t.input;
            r.tokens.output += t.output;
            r.tokens.cache_read += t.cache_read;
            r.tokens.cache_write += t.cache_write;
        } else {
            r.tokens.unsplit += t.tokens;
        }
        days.entry(crate::squad_memory::date(t.ended)).or_default().add(t);
        let model = match (&t.engine, &t.model) {
            (Some(e), Some(m)) => format!("{e} · {m}"),
            (Some(e), None) => e.clone(),
            _ => "unknown".to_owned(),
        };
        models.entry(model).or_default().add(t);
        let base = base_member(&t.member).to_owned();
        let member = members.entry(base.clone()).or_default();
        member.add(t);
        if base != t.member {
            member.lanes.insert(t.member.clone());
        }
        kinds.entry(kind_of(t).to_owned()).or_default().add(t);
        let context = t.context_per_call();
        let bucket = buckets.entry(bucket_label(context)).or_default();
        bucket.0 += 1;
        bucket.1 += t.tokens;
        if context.is_some_and(|c| c > HEAVY_CONTEXT) {
            heavy.0 += 1;
            heavy.1 += t.tokens;
        }
        if let Some(id) = t.ticket {
            let p = tickets.entry(id).or_default();
            p.turns += 1;
            p.tokens += t.tokens;
        }
    }

    let total = r.tokens.total;
    r.by_day = {
        let mut v = rows(days, total);
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    };
    r.by_model = rows(models, total);
    r.by_member = rows(members, total);
    let order = [kind::PM_DESK, kind::MEMBER_ON_TICKET, kind::LANE_ON_TICKET, kind::OTHER];
    r.by_kind = rows(kinds, total);
    r.by_kind.sort_by_key(|row| order.iter().position(|k| *k == row.name).unwrap_or(order.len()));
    r.context = ["<100k", "100-200k", "200-400k", ">400k", "unknown"]
        .iter()
        .filter_map(|label| {
            let (turns, tokens) = buckets.get(label).copied()?;
            Some(Bucket { label: (*label).to_owned(), turns, tokens, share_percent: percent(tokens, total) })
        })
        .collect();

    let by_tokens = |a: &TicketRow, b: &TicketRow| b.tokens.cmp(&a.tokens).then(a.id.cmp(&b.id));
    let mut all: Vec<TicketRow> = tickets
        .iter()
        .map(|(id, p)| {
            let info = input.tickets.get(id);
            TicketRow { id: *id, title: info.map(|i| i.title.clone()).unwrap_or_default(), status: info.map(|i| i.status.clone()).unwrap_or_default(), turns: p.turns, tokens: p.tokens }
        })
        .collect();
    all.sort_by(by_tokens);
    r.top_tickets = all.iter().take(TOP_TICKETS).cloned().collect();
    let cancelled: Vec<TicketRow> = all.iter().filter(|t| t.status == "cancelled").cloned().collect();
    r.cancelled_tokens = cancelled.iter().map(|t| t.tokens).sum();
    r.cancelled_share_percent = percent(r.cancelled_tokens, total);
    r.cancelled_tickets = cancelled.iter().take(TOP_TICKETS).cloned().collect();
    let mut rework: Vec<TicketRow> = all.iter().filter(|t| t.turns >= REWORK_TURNS).cloned().collect();
    rework.sort_by(|a, b| b.turns.cmp(&a.turns).then_with(|| by_tokens(a, b)));
    r.rework = rework.iter().take(TOP_TICKETS).cloned().collect();
    r.endings = input.endings.clone();
    r.signals = signals(&r, heavy, &cancelled, &rework);
    r.quality = quality(input, since, until);
    r
}

fn nearest_rank(sorted: &[u64], percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (percent * sorted.len()).div_ceil(100).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

fn counted(map: BTreeMap<String, u64>) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// The quality numbers of the window `since`..`until`, from the tickets'
/// records. A ticket is *accepted* by its first `accepted` entry in the
/// window; its time to done runs from its first `assigned` (its `created`
/// when it never was) to that entry. A first hand-in is the first `handed in`
/// entry in the window, *decided* by the first accept or send-back after it
/// (one still waiting counts for neither side, so a window's last days don't
/// look worse than they are).
pub fn quality(input: &Input, since: u64, until: u64) -> Quality {
    let inside = |at: u64| at >= since && at < until;
    let mut records: BTreeMap<u64, Vec<&EventRow>> = BTreeMap::new();
    for e in &input.events {
        records.entry(e.ticket).or_default().push(e);
    }
    let mut q = Quality::default();
    let (mut durations, mut turns, mut tokens) = (Vec::new(), 0u64, 0u64);
    let is_accept = |k: &str| k.starts_with("accepted");
    for (id, record) in &records {
        // Accepted in the window.
        if let Some(accept) = record.iter().find(|e| is_accept(&e.kind) && inside(e.at)) {
            q.accepted += 1;
            q.send_backs_on_accepted += record.iter().filter(|e| SENT_BACK.contains(&e.kind.as_str()) && e.at <= accept.at).count() as u64;
            let start = record.iter().find(|e| e.kind == "assigned").or_else(|| record.iter().find(|e| e.kind == "created")).map(|e| e.at);
            if let Some(start) = start {
                durations.push(accept.at.saturating_sub(start));
            }
            let (t, k) = input.ticket_totals.get(id).copied().unwrap_or_default();
            turns += t;
            tokens += k;
        }
        // First handed in within the window, and how that went.
        if let Some(first) = record.iter().position(|e| e.kind == "handed in").filter(|i| inside(record[*i].at)) {
            match record[first..].iter().find(|e| is_accept(&e.kind) || SENT_BACK.contains(&e.kind.as_str())) {
                Some(e) if is_accept(&e.kind) => {
                    q.first_handins_decided += 1;
                    q.first_handins_accepted += 1;
                }
                Some(_) => q.first_handins_decided += 1,
                None => {}
            }
        }
    }
    if q.first_handins_decided > 0 {
        q.first_handin_accept_percent = Some(percent(q.first_handins_accepted, q.first_handins_decided));
    }
    if q.accepted > 0 {
        q.send_backs_per_accepted = Some(q.send_backs_on_accepted as f64 / q.accepted as f64);
        q.turns_per_done = Some(turns as f64 / q.accepted as f64);
        q.tokens_per_done = Some(tokens / q.accepted);
    }
    durations.sort_unstable();
    q.time_to_done_median_secs = nearest_rank(&durations, 50);
    q.time_to_done_p75_secs = nearest_rank(&durations, 75);
    q.send_backs_by_reason = input.send_backs.clone();
    let mut verdicts: BTreeMap<String, u64> = BTreeMap::new();
    let mut decisions: BTreeMap<String, u64> = BTreeMap::new();
    for (verdict, decision, n) in &input.design_qa {
        *verdicts.entry(verdict.clone()).or_default() += n;
        *decisions.entry(decision.clone()).or_default() += n;
    }
    q.design_qa_verdicts = counted(verdicts);
    q.design_qa_decisions = counted(decisions);
    q
}

/// One number compared across two windows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    /// What it measures: a code the screens name (`tokens`, `tokens_per_done`, …).
    pub name: &'static str,
    pub before: Option<f64>,
    pub after: Option<f64>,
    /// `after` against `before`, in percent; `None` when either is missing or `before` is 0.
    pub change_percent: Option<f64>,
}

/// The token and quality numbers of `after` against `before` (the earlier window).
pub fn compare(before: &Report, after: &Report) -> Vec<Change> {
    let change = |name: &'static str, b: Option<f64>, a: Option<f64>| Change { name, before: b, after: a, change_percent: b.zip(a).filter(|(b, _)| *b != 0.0).map(|(b, a)| (a - b) * 100.0 / b) };
    let n = |v: u64| Some(v as f64);
    let (bq, aq) = (&before.quality, &after.quality);
    let share = |r: &Report| r.by_kind.iter().find(|row| row.name == kind::PM_DESK).map_or(0.0, |row| row.share_percent);
    vec![
        change("tokens", n(before.tokens.total), n(after.tokens.total)),
        change("turns", n(before.turns), n(after.turns)),
        change("pm_desk_share_percent", Some(share(before)), Some(share(after))),
        change("accepted", n(bq.accepted), n(aq.accepted)),
        change("first_handin_accept_percent", bq.first_handin_accept_percent, aq.first_handin_accept_percent),
        change("send_backs_per_accepted", bq.send_backs_per_accepted, aq.send_backs_per_accepted),
        change("time_to_done_median_secs", bq.time_to_done_median_secs.and_then(n), aq.time_to_done_median_secs.and_then(n)),
        change("time_to_done_p75_secs", bq.time_to_done_p75_secs.and_then(n), aq.time_to_done_p75_secs.and_then(n)),
        change("turns_per_done", bq.turns_per_done, aq.turns_per_done),
        change("tokens_per_done", bq.tokens_per_done.and_then(n), aq.tokens_per_done.and_then(n)),
    ]
}

/// The turn endings that count toward [`SignalKind::Endings`].
const ABNORMAL: [&str; 3] = [turn_ending::LIMITED, turn_ending::FAILED_FAST, turn_ending::HANDED_BACK];

fn signals(r: &Report, heavy: (u64, u64), cancelled: &[TicketRow], rework: &[TicketRow]) -> Vec<Signal> {
    let total = r.tokens.total;
    let mut out = Vec::new();
    let base = |kind: SignalKind, tokens: u64| Signal { kind, tokens, share_percent: percent(tokens, total), context_per_call: None, count: 0, tickets: Vec::new(), endings: Vec::new() };
    let named = |tickets: &[TicketRow]| tickets.iter().take(NAMED_TICKETS).cloned().collect::<Vec<_>>();
    if total >= MIN_TOKENS_FOR_SHARES {
        if let Some(desk) = r.by_kind.iter().find(|k| k.name == kind::PM_DESK)
            && let Some(context) = desk.context_per_call
            && context > PM_DESK_CONTEXT
            && desk.share_percent >= PM_DESK_MIN_SHARE_PERCENT
        {
            out.push(Signal { context_per_call: Some(context), count: desk.turns, ..base(SignalKind::PmDesk, desk.tokens) });
        }
        if percent(heavy.1, total) >= HEAVY_SHARE_PERCENT {
            out.push(Signal { count: heavy.0, ..base(SignalKind::HeavyContext, heavy.1) });
        }
        if r.cancelled_share_percent >= CANCELLED_SHARE_PERCENT {
            out.push(Signal { count: cancelled.len() as u64, tickets: named(cancelled), ..base(SignalKind::Cancelled, r.cancelled_tokens) });
        }
        let rework_tokens: u64 = rework.iter().map(|t| t.tokens).sum();
        if percent(rework_tokens, total) >= REWORK_SHARE_PERCENT {
            out.push(Signal { count: rework.len() as u64, tickets: named(rework), ..base(SignalKind::Rework, rework_tokens) });
        }
    }
    let abnormal: Vec<(String, u64)> = r.endings.iter().filter(|(k, _)| ABNORMAL.contains(&k.as_str())).cloned().collect();
    let count: u64 = abnormal.iter().map(|(_, n)| n).sum();
    if count >= ENDINGS_MIN {
        out.push(Signal { count, endings: abnormal, ..base(SignalKind::Endings, 0) });
    }
    out
}

// ── rows from the journal ──

fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else { return false };
    let Ok(names) = stmt.query_map([], |r| r.get::<_, String>(1)) else { return false };
    names.flatten().any(|n| n == column)
}

fn has_table(conn: &Connection, table: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |_| Ok(())).optional().ok().flatten().is_some()
}

/// The rows for the turns that ended in `since..until`, from an open journal.
pub fn select(conn: &Connection, since: u64, until: u64) -> rusqlite::Result<Input> {
    let sql = if has_column(conn, "turns", "calls") {
        "SELECT member, role, ticket, ended, tokens, engine, model, input, output, cache_read, cache_write, calls FROM turns WHERE ended >= ?1 AND ended < ?2 ORDER BY ended, rowid"
    } else {
        "SELECT member, role, ticket, ended, tokens, NULL, NULL, NULL, NULL, NULL, NULL, NULL FROM turns WHERE ended >= ?1 AND ended < ?2 ORDER BY ended, rowid"
    };
    let n = |v: Option<i64>| v.unwrap_or(0).max(0) as u64;
    let window = [since.min(i64::MAX as u64) as i64, until.min(i64::MAX as u64) as i64];
    let mut stmt = conn.prepare(sql)?;
    let turns = stmt
        .query_map(window, |r| {
            let input: Option<i64> = r.get(7)?;
            Ok(TurnRow {
                member: r.get(0)?,
                role: r.get(1)?,
                ticket: r.get::<_, Option<i64>>(2)?.map(|t| t.max(0) as u64),
                ended: n(Some(r.get(3)?)),
                tokens: n(Some(r.get(4)?)),
                engine: r.get(5)?,
                model: r.get(6)?,
                // A turn kept before tokens were kept by kind has no input value.
                measured: input.is_some(),
                input: n(input),
                output: n(r.get(8)?),
                cache_read: n(r.get(9)?),
                cache_write: n(r.get(10)?),
                calls: n(r.get(11)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tickets = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, title, status FROM tickets")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, TicketInfo { title: r.get(1)?, status: r.get(2)? })))? {
        let (id, info) = row?;
        tickets.insert(id.max(0) as u64, info);
    }
    let endings = if has_table(conn, "turn_endings") {
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM turn_endings WHERE at >= ?1 AND at < ?2 GROUP BY kind ORDER BY COUNT(*) DESC, kind")?;
        stmt.query_map(window, |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    // The records of the tickets handed in or accepted in the window, from the few entries that matter.
    let kinds = EVENT_KINDS.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
    let mut events = Vec::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT ticket, at, kind FROM ticket_events
         WHERE kind IN ({kinds}) AND ticket IN (SELECT ticket FROM ticket_events WHERE (kind = 'handed in' OR kind LIKE 'accepted%') AND at >= ?1 AND at < ?2)
         ORDER BY ticket, seq"
    ))?;
    for row in stmt.query_map(window, |r| Ok(EventRow { ticket: r.get::<_, i64>(0)?.max(0) as u64, at: r.get::<_, i64>(1)?.max(0) as u64, kind: r.get(2)? }))? {
        events.push(row?);
    }
    let mut ticket_totals = HashMap::new();
    let mut stmt = conn.prepare("SELECT ticket, COUNT(*), COALESCE(SUM(tokens), 0) FROM turns WHERE ticket IS NOT NULL GROUP BY ticket")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (id, turns, tokens) = row?;
        ticket_totals.insert(id.max(0) as u64, (turns.max(0) as u64, tokens.max(0) as u64));
    }
    let send_backs = if has_table(conn, "send_backs") {
        let mut stmt = conn.prepare("SELECT reason, COUNT(*) FROM send_backs WHERE at >= ?1 AND at < ?2 GROUP BY reason ORDER BY COUNT(*) DESC, reason")?;
        stmt.query_map(window, |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let design_qa = if has_table(conn, "design_qa_runs") {
        let mut stmt = conn.prepare("SELECT COALESCE(verdict, 'none'), COALESCE(decision, 'none'), COUNT(*) FROM design_qa_runs WHERE created_at >= ?1 AND created_at < ?2 GROUP BY 1, 2")?;
        stmt.query_map(window, |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?.max(0) as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok(Input { turns, tickets, endings, events, ticket_totals, send_backs, design_qa })
}

/// A squad's rows from its journal file, read without writing to it.
pub fn read(db: &Path, since: u64, until: u64) -> Result<Input, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("{}: {e}", db.display()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    select(&conn, since, until).map_err(|e| e.to_string())
}

// ── the daily note ──

/// What the daily note says: the last day against the days before it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Digest {
    /// The last 24 hours.
    pub last: Report,
    /// The seven days before them: their tokens per day, on average.
    pub prior_daily_tokens: u64,
}

impl Digest {
    /// Worth saying only when a waste signal is over its threshold.
    pub fn worth_saying(&self) -> bool {
        !self.last.signals.is_empty()
    }
}

/// The last 24 hours up to `now` (`last`), and the days before them (`prior`).
pub fn digest(last: &Input, prior: &Input, now: u64) -> Digest {
    let prior_tokens: u64 = prior.turns.iter().map(|t| t.tokens).sum();
    Digest { last: summarize(last, now.saturating_sub(DAY), now), prior_daily_tokens: prior_tokens / DIGEST_COMPARE_DAYS }
}

/// The two windows the daily note reads, as `(last, before)`: the 24 hours up
/// to `now`, and the [`DIGEST_COMPARE_DAYS`] days before them.
pub fn digest_windows(now: u64) -> ((u64, u64), (u64, u64)) {
    let last = now.saturating_sub(DAY);
    ((last, now), (last.saturating_sub(DIGEST_COMPARE_DAYS * DAY), last))
}

// ── words ──

/// A token count as people read it: 640k, 12.3M, 1.2B.
pub fn amount(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.0}k", n as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

/// `T-12 Title (12.3M)`: a ticket in a sentence.
fn name_ticket(t: &TicketRow) -> String {
    let title: String = t.title.chars().take(40).collect();
    let title = title.trim_end();
    if title.is_empty() { format!("T-{} ({})", t.id, amount(t.tokens)) } else { format!("T-{} {title} ({})", t.id, amount(t.tokens)) }
}

fn ending_word(words: &Catalog, kind: &str) -> String {
    match kind {
        turn_ending::LIMITED => words.get("usage.ending.limited").to_owned(),
        turn_ending::FAILED_FAST => words.get("usage.ending.failed_fast").to_owned(),
        turn_ending::HANDED_BACK => words.get("usage.ending.handed_back").to_owned(),
        other => other.replace('_', " "),
    }
}

/// A signal as one plain sentence, and what to do about it.
pub fn say_signal(words: &Catalog, s: &Signal) -> (String, String) {
    let key = |part: &str| format!("usage.signal.{}.{part}", s.kind.code());
    let limit = if s.kind == SignalKind::PmDesk { PM_DESK_CONTEXT } else { HEAVY_CONTEXT };
    let args = [
        ("share", format!("{:.1}", s.share_percent)),
        ("tokens", amount(s.tokens)),
        ("context", amount(s.context_per_call.unwrap_or(0))),
        ("limit", amount(limit)),
        ("count", s.count.to_string()),
        ("turns", REWORK_TURNS.to_string()),
        ("tickets", s.tickets.iter().map(name_ticket).collect::<Vec<_>>().join(", ")),
        ("endings", s.endings.iter().map(|(k, n)| format!("{} {n}", ending_word(words, k))).collect::<Vec<_>>().join(", ")),
    ];
    (words.fill(&key("what"), &args), words.fill(&key("todo"), &args))
}

/// The daily note: a line with the numbers, then each signal's sentence.
pub fn say_digest(words: &Catalog, d: &Digest) -> String {
    let mut lines = vec![words.fill("usage.digest.head", &[("tokens", amount(d.last.tokens.total)), ("average", amount(d.prior_daily_tokens)), ("turns", d.last.turns.to_string())])];
    for s in &d.last.signals {
        let (what, todo) = say_signal(words, s);
        lines.push(format!("- {what} {todo}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{Journal, TurnUse};
    use equipe_protocol::{Engine, Usage};

    const T0: u64 = 1_794_000_000;

    fn row(member: &str, role: &str, ticket: Option<u64>, ended: u64, read: u64, calls: u64) -> TurnRow {
        TurnRow { member: member.into(), role: role.into(), ticket, ended, tokens: read + 100, input: 0, output: 100, cache_read: read, cache_write: 0, calls, engine: Some("claude".into()), model: Some("claude-sonnet-5".into()), measured: true }
    }

    fn ticket(id: u64, title: &str, status: &str) -> (u64, TicketInfo) {
        (id, TicketInfo { title: title.into(), status: status.into() })
    }

    #[test]
    fn context_per_call_is_what_one_request_carried_in() {
        let t = TurnRow { input: 1_000, ..row("pm", "pm", None, T0, 999_000, 1) };
        assert_eq!(t.context_per_call(), Some(1_000_000));
        let two = TurnRow { cache_write: 100_000, ..row("pm", "pm", None, T0, 900_000, 2) };
        assert_eq!(two.context_per_call(), Some(500_000));
        assert_eq!(TurnRow { measured: false, ..t.clone() }.context_per_call(), None);
        assert_eq!(TurnRow { calls: 0, ..t }.context_per_call(), None);
    }

    #[test]
    fn tokens_split_by_day_member_kind_and_context() {
        let tickets: HashMap<u64, TicketInfo> = [ticket(1, "Add a page", "done")].into();
        let turns = vec![
            row("pm", "pm", None, T0, 600_000_000, 1_000),                           // desk: 600k per call
            row("frontend", "frontend", Some(1), T0 + 10, 50_000_000, 1_000),        // 50k per call
            row("frontend-12", "frontend", Some(1), T0 + 20, 150_000_000, 1_000),    // 150k per call
            row("frontend-13", "frontend", Some(1), T0 + DAY, 300_000_000, 1_000),   // 300k per call, next day
            row("qa", "qa", None, T0 + DAY, 100_000, 10),                            // no ticket, not the PM
            TurnRow { measured: false, tokens: 5_000, ..row("qa", "qa", Some(1), T0 + DAY, 0, 0) },
        ];
        let r = summarize(&Input { turns, tickets, ..Input::default() }, T0, T0 + 2 * DAY);
        assert_eq!(r.turns, 6);
        assert_eq!(r.tokens.total, 600_000_100 + 50_000_100 + 150_000_100 + 300_000_100 + 100_100 + 5_000);
        assert_eq!(r.tokens.unsplit, 5_000);
        assert_eq!(r.by_day.len(), 2);
        assert!(r.by_day[0].name < r.by_day[1].name);
        let frontend = r.by_member.iter().find(|m| m.name == "frontend").unwrap();
        assert_eq!((frontend.turns, frontend.lanes), (3, 2), "frontend-12 and frontend-13 fold into frontend");
        assert!(!r.by_member.iter().any(|m| m.name.contains("-1")));
        let kinds: Vec<(&str, u64)> = r.by_kind.iter().map(|k| (k.name.as_str(), k.turns)).collect();
        assert_eq!(kinds, [("pm_desk", 1), ("member_on_ticket", 2), ("lane_on_ticket", 2), ("other_no_ticket", 1)]);
        assert_eq!(r.by_kind[0].context_per_call, Some(600_000));
        let buckets: Vec<(&str, u64)> = r.context.iter().map(|b| (b.label.as_str(), b.turns)).collect();
        assert_eq!(buckets, [("<100k", 2), ("100-200k", 1), ("200-400k", 1), (">400k", 1), ("unknown", 1)]);
        assert_eq!((r.top_tickets[0].id, r.top_tickets[0].title.as_str(), r.top_tickets[0].turns), (1, "Add a page", 4));
    }

    #[test]
    fn signals_appear_only_over_their_thresholds() {
        let tickets: HashMap<u64, TicketInfo> = [ticket(7, "Dropped idea", "cancelled"), ticket(8, "Stuck", "in_progress")].into();
        // A PM desk at 1M per call with 29% of the tokens, a cancelled ticket, a rework loop.
        let mut turns = vec![row("pm", "pm", None, T0, 290_000_000, 290), row("backend", "backend", Some(7), T0, 40_000_000, 400)];
        for i in 0..REWORK_TURNS {
            turns.push(row("backend", "backend", Some(8), T0 + i, 4_000_000, 100));
        }
        turns.push(row("backend", "backend", Some(9), T0, 590_000_000, 5_900));
        let endings = vec![("limited".to_owned(), 3), ("failed_fast".to_owned(), 2), ("nudged".to_owned(), 50)];
        let r = summarize(&Input { turns, tickets: tickets.clone(), endings, ..Input::default() }, T0, T0 + DAY);
        let kinds: Vec<SignalKind> = r.signals.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [SignalKind::PmDesk, SignalKind::HeavyContext, SignalKind::Cancelled, SignalKind::Rework, SignalKind::Endings], "{:#?}", r.signals);
        assert_eq!(r.signals[0].context_per_call, Some(1_000_000));
        assert!((r.signals[0].share_percent - 29.6).abs() < 0.1, "{}", r.signals[0].share_percent);
        assert_eq!(r.signals[1].tokens, r.signals[0].tokens, "the PM desk is the only heavy turn here");
        assert_eq!(r.signals[2].tickets[0].id, 7);
        assert_eq!(r.signals[3].tickets[0].id, 8);
        assert_eq!(r.signals[4].count, 5, "nudged isn't one of the abnormal endings");
        assert_eq!(r.cancelled_tokens, 40_000_100);
        assert_eq!(r.rework.len(), 1);

        // One ending short of the threshold, and a quiet squad: nothing to say.
        let few = vec![("limited".to_owned(), ENDINGS_MIN - 1)];
        let quiet = summarize(&Input { turns: vec![row("pm", "pm", None, T0, 1_000_000, 1)], tickets, endings: few, ..Input::default() }, T0, T0 + DAY);
        assert!(quiet.signals.is_empty(), "{:#?}", quiet.signals);

        // A heavy share is its own signal.
        let turns = vec![row("backend", "backend", Some(9), T0, 900_000_000, 1_000), row("backend", "backend", Some(9), T0, 100_000_000, 1_000)];
        let heavy = summarize(&Input { turns, ..Input::default() }, T0, T0 + DAY);
        assert_eq!(heavy.signals.iter().map(|s| s.kind).collect::<Vec<_>>(), [SignalKind::HeavyContext]);
        assert!((heavy.signals[0].share_percent - 90.0).abs() < 0.1);
    }

    #[test]
    fn a_journal_is_read_without_writing_to_it() {
        let home = std::env::temp_dir().join(format!("equipe-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let j = Journal::for_squad(&home, "acme").unwrap();
        let new = |title: &str, status| equipe_protocol::Ticket {
            id: 0,
            title: title.into(),
            body: String::new(),
            status,
            kind: equipe_protocol::TicketKind::Task,
            owner: None,
            assignee: None,
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            plan: None,
            model: None,
            created_by: "equipe".into(),
            note: None,
            asks: vec![],
            remaining: vec![],
            images: vec![],
            updated_at: None,
            priority: Default::default(),
            rank: 0,
        };
        let kept = u64::from(j.create_ticket(&new("Keep", equipe_protocol::TicketStatus::Todo)).unwrap());
        let dropped = u64::from(j.create_ticket(&new("Drop", equipe_protocol::TicketStatus::Cancelled)).unwrap());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let usage = |read| Usage { input_tokens: Some(1_000), output_tokens: Some(200), cache_read_tokens: Some(read), cache_write_tokens: Some(5_000), cache_write_long_tokens: Some(5_000), calls: Some(2), ..Default::default() };
        let turn = |member: &str, role: &str, ticket: Option<u64>, used: &Usage| {
            let ticket = ticket.map(|t| t as u32);
            let u = TurnUse::of(Engine::Claude, Some("claude-sonnet-5".into()), None, used);
            let tokens = u.tokens.input + u.tokens.output + u.tokens.cache_read + u.tokens.cache_write;
            j.record_turn_use(&crate::retro::Turn { member: member.into(), role: role.into(), ticket, began: now - 60, ended: now - 30, outcome: "completed".into(), tokens }, Some(&u)).unwrap();
        };
        turn("pm", "pm", None, &usage(1_994_000));
        turn("frontend-2", "frontend", Some(kept), &usage(100_000));
        turn("backend", "backend", Some(dropped), &usage(100_000));
        // One kept only as a sum, and one outside the window.
        let old = |ended: u64, tokens| crate::retro::Turn { member: "qa".into(), role: "qa".into(), ticket: None, began: ended - 5, ended, outcome: "completed".into(), tokens };
        j.record_turn(&old(now - 20, 777)).unwrap();
        j.record_turn(&old(now - 10 * DAY, 999)).unwrap();
        j.count_turn_ending("limited", Some(kept as u32), "frontend-2").unwrap();
        j.record_send_back(kept as u32, "pm", equipe_protocol::tiers::SendBackReason::Style).unwrap();
        for kind in ["assigned", "handed in", "sent back", "handed in", "accepted"] {
            j.ticket_event(kept as u32, "equipe", kind, "").unwrap();
        }

        let db = crate::journal::squad_dir(&home, "acme").join("squad.db");
        let input = read(&db, now - 2 * DAY, now + 1).unwrap();
        assert_eq!(input.turns.len(), 4);
        let r = summarize(&input, now - 2 * DAY, now + 1);
        assert_eq!(r.tokens.unsplit, 777);
        assert_eq!(r.endings, [("limited".to_owned(), 1)]);
        let desk = r.by_kind.iter().find(|k| k.name == "pm_desk").unwrap();
        assert_eq!(desk.context_per_call, Some((1_000 + 1_994_000 + 5_000) / 2), "an hour-long write is part of the writes, not added again");
        assert_eq!(r.cancelled_tickets.iter().map(|t| t.id).collect::<Vec<_>>(), [dropped]);
        let q = &r.quality;
        assert_eq!((q.accepted, q.send_backs_on_accepted, q.first_handins_decided, q.first_handins_accepted), (1, 1, 1, 0), "{q:?}");
        assert_eq!((q.turns_per_done, q.send_backs_by_reason.clone()), (Some(1.0), vec![("style".to_owned(), 1)]));
        assert_eq!(j.usage_input(now - 2 * DAY, now + 1).unwrap().turns.len(), 4, "the daemon reads the same rows through its open journal");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_daily_note_is_worth_saying_only_with_a_signal() {
        assert!(!digest(&Input::default(), &Input::default(), T0).worth_saying());
        let last = Input { turns: vec![row("pm", "pm", None, T0 - 10, 900_000_000, 900)], ..Input::default() };
        let prior = Input { turns: vec![row("pm", "pm", None, T0 - 3 * DAY, 700_000_000, 700)], ..Input::default() };
        let d = digest(&last, &prior, T0);
        assert!(d.worth_saying());
        assert_eq!(d.prior_daily_tokens, 700_000_100 / 7);
        let text = say_digest(crate::i18n::core(crate::i18n::Lang::En), &d);
        assert!(text.contains("900.0M") && text.contains("100.0M"), "{text}");
        assert!(text.contains("1.0M tokens per call"), "{text}");
        let ((from, to), (before_from, before_to)) = digest_windows(T0);
        assert_eq!((to - from, before_to, before_to - before_from), (DAY, from, 7 * DAY));
    }

    fn ev(ticket: u64, at: u64, kind: &str) -> EventRow {
        EventRow { ticket, at, kind: kind.into() }
    }

    #[test]
    fn quality_counts_accepts_first_time_send_backs_time_to_done_and_cost_per_done_ticket() {
        let h = 3_600;
        let events = vec![
            // T1: handed in once, accepted as handed in; 4 hours from assigned.
            ev(1, T0, "assigned"), ev(1, T0 + 3 * h, "handed in"), ev(1, T0 + 4 * h, "accepted"),
            // T2: sent back twice, then accepted by the PM; 10 hours.
            ev(2, T0, "assigned"), ev(2, T0 + h, "handed in"), ev(2, T0 + 2 * h, "sent back"), ev(2, T0 + 5 * h, "reopened"), ev(2, T0 + 10 * h, "accepted by the PM"),
            // T3: created, never assigned; handed in and still waiting; not accepted.
            ev(3, T0, "created"), ev(3, T0 + h, "handed in"),
            // T4: accepted in the window, first handed in before it; 2 hours from created.
            ev(4, T0 - 5 * DAY, "handed in"), ev(4, T0 + h, "created"), ev(4, T0 + 3 * h, "accepted"),
        ];
        let ticket_totals = [(1, (4, 1_000)), (2, (10, 5_000)), (3, (2, 99)), (4, (6, 3_000))].into();
        let send_backs = vec![("correctness".to_owned(), 2), ("style".to_owned(), 1)];
        let design_qa = vec![("pass".to_owned(), "auto_pass".to_owned(), 3), ("changes".to_owned(), "none".to_owned(), 1), ("none".to_owned(), "none".to_owned(), 1)];
        let input = Input { events, ticket_totals, send_backs, design_qa, ..Input::default() };
        let q = quality(&input, T0 - DAY, T0 + DAY);
        assert_eq!(q.accepted, 3);
        assert_eq!((q.first_handins_decided, q.first_handins_accepted), (2, 1), "T3 is still waiting, T4's hand-in was before the window");
        assert_eq!(q.first_handin_accept_percent, Some(50.0));
        assert_eq!((q.send_backs_on_accepted, q.send_backs_per_accepted), (2, Some(2.0 / 3.0)));
        // 4h, 10h and 2h: sorted 2, 4, 10 → median 4h, p75 10h.
        assert_eq!((q.time_to_done_median_secs, q.time_to_done_p75_secs), (Some(4 * h), Some(10 * h)));
        assert_eq!((q.turns_per_done, q.tokens_per_done), (Some(20.0 / 3.0), Some(3_000)));
        assert_eq!(q.send_backs_by_reason, [("correctness".to_owned(), 2), ("style".to_owned(), 1)]);
        assert_eq!(q.design_qa_verdicts, [("pass".to_owned(), 3), ("changes".to_owned(), 1), ("none".to_owned(), 1)]);
        assert_eq!(q.design_qa_decisions, [("auto_pass".to_owned(), 3), ("none".to_owned(), 2)]);
        // A window with no accepts says nothing of rates.
        let none = quality(&input, T0 + 30 * DAY, T0 + 31 * DAY);
        assert_eq!((none.accepted, none.first_handin_accept_percent, none.tokens_per_done, none.time_to_done_median_secs), (0, None, None, None));
    }

    #[test]
    fn two_windows_compare_in_percent_and_leave_out_what_has_no_base() {
        let q = |accepted, tokens_per_done| Quality { accepted, tokens_per_done: Some(tokens_per_done), first_handin_accept_percent: Some(80.0), ..Quality::default() };
        let mut before = Report { quality: q(4, 1_000), ..Report::default() };
        before.tokens.total = 1_000;
        let mut after = Report { quality: q(5, 750), ..Report::default() };
        after.tokens.total = 1_500;
        let changes = compare(&before, &after);
        let get = |name: &str| changes.iter().find(|c| c.name == name).unwrap();
        assert_eq!(get("tokens").change_percent, Some(50.0));
        assert_eq!(get("tokens_per_done").change_percent, Some(-25.0));
        assert_eq!(get("accepted").change_percent, Some(25.0));
        assert_eq!(get("first_handin_accept_percent").change_percent, Some(0.0));
        assert_eq!(get("time_to_done_median_secs").change_percent, None, "nothing to compare");
        assert_eq!(get("turns").change_percent, None, "no turns before: no base");
    }

    #[test]
    fn amounts_read_as_people_say_them() {
        assert_eq!([amount(999), amount(12_345), amount(1_000_000), amount(2_345_000_000)], ["999", "12k", "1.0M", "2.3B"]);
    }
}
