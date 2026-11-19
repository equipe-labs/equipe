//! Waste stops: equipe notices work that burns tokens without moving, by
//! itself, instead of an operator noticing hours later (docs/HEALTH.md).
//!
//! - **A ticket's budget** ([`limit`], [`trip`]): once the turns on one ticket
//!   have used more tokens than the squad allows a ticket
//!   ([`Settings::ticket_budget`], [`DEFAULT_TICKET_BUDGET`] unless set),
//!   nothing more runs on it after the turn under way. It goes to the PM as a
//!   proposal with what it cost ([`Card`]); the PM puts it back to todo (it
//!   gets another budget), narrows what is left into a new ticket, or cancels
//!   it. The same member taking turn after turn on it without a new commit
//!   ([`repeats`]) stops it sooner.
//! - **The squad's spend rate** ([`rate`], [`RateState`]): the last hour's
//!   tokens against the squad's own hourly median over the week before. Past
//!   [`RATE_FACTOR`] times it, one line to the PM and the person naming the
//!   tickets that spent most — at most once an hour, and only while it stays high.
//! - **Plan headroom** ([`headroom_level`], [`headroom_step`]): when every
//!   signed-in engine's weekly window is [`HEADROOM_LEVELS`]`[0]`% used or
//!   more, the person and the PM are told once per level, before the squad stops.
//!
//! Everything here is pure: numbers and records in, decisions out. The
//! runtime (`runtime::guards`) feeds it and carries the decisions out.

use std::collections::HashMap;

use equipe_protocol::{Engine, Ticket, TicketEntry, TicketId};
use serde::{Deserialize, Serialize};

use crate::usage::amount;

/// Tokens one ticket's turns may use before it goes to the PM.
pub const DEFAULT_TICKET_BUDGET: u64 = 30_000_000;
/// Turns in a row by the same member on one ticket, with no new commit on
/// its branch, that stop it before its budget.
pub const DEFAULT_REPEAT_TURNS: u32 = 6;
/// The ticket record's word for a stop; its text starts with the tokens
/// spent when it stopped ([`stopped_at`]).
pub const STOP: &str = "budget stop";
/// The last hour is too fast past this many times the hourly median.
pub const RATE_FACTOR: u64 = 2;
/// Hours with any spend the week before needs for a median worth comparing with.
pub const RATE_MIN_HOURS: usize = 24;
/// Below this many tokens in the last hour the rate is never news.
pub const RATE_MIN_TOKENS: u64 = 2_000_000;
/// How far back the median looks.
pub const RATE_WINDOW: u64 = 7 * 24 * HOUR;
/// A high rate is told again at most this often.
pub const RATE_TELL_EVERY: u64 = HOUR;
/// How many tickets the rate's line names.
pub const RATE_TOP: usize = 3;
/// The levels of a week's use, in percent, each told once.
pub const HEADROOM_LEVELS: [u8; 2] = [90, 95];
/// How much of a hand-in or hand-off note the PM's card quotes.
const NOTE_KEPT: usize = 600;

const HOUR: u64 = 3600;

/// A squad's spend guards, as set for it (`equipe health budget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Tokens a ticket's turns may use before the PM decides; 0: no limit.
    #[serde(default = "default_budget")]
    pub ticket_budget: u64,
    /// Turns in a row without a new commit that stop a ticket; 0: never.
    #[serde(default = "default_repeats")]
    pub repeat_turns: u32,
}

fn default_budget() -> u64 {
    DEFAULT_TICKET_BUDGET
}

fn default_repeats() -> u32 {
    DEFAULT_REPEAT_TURNS
}

impl Default for Settings {
    fn default() -> Self {
        Settings { ticket_budget: DEFAULT_TICKET_BUDGET, repeat_turns: DEFAULT_REPEAT_TURNS }
    }
}

/// A token count as the person types it: `30000000`, `30M`, `500k`, `1.5B`; `off` is 0.
pub fn parse_tokens(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase();
    if text == "off" || text == "0" {
        return Some(0);
    }
    let (number, scale) = match text.chars().last()? {
        'k' => (&text[..text.len() - 1], 1e3),
        'm' => (&text[..text.len() - 1], 1e6),
        'b' => (&text[..text.len() - 1], 1e9),
        _ => (text.as_str(), 1.0),
    };
    let n: f64 = number.trim().parse().ok()?;
    (n.is_finite() && n >= 0.0).then(|| (n * scale).round() as u64)
}

/// What a stop's record line says was spent when it stopped.
pub fn stopped_at(text: &str) -> Option<u64> {
    let digits: String = text.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// How many tokens a ticket whose record is `record` may have used before it
/// stops; `None`: the squad set no limit. Each stop the PM let go on from
/// gives it another budget, counted from what it had spent then.
pub fn limit(settings: &Settings, record: &[TicketEntry]) -> Option<u64> {
    if settings.ticket_budget == 0 {
        return None;
    }
    let from = record.iter().rev().find(|e| e.kind == STOP).and_then(|e| stopped_at(&e.text)).unwrap_or(0);
    Some(from.saturating_add(settings.ticket_budget))
}

/// How many turns in a row, at the end of `turns` (oldest first, each the
/// member that took it and the commit its branch was at after), one member
/// took on a ticket without its branch moving.
pub fn repeats(turns: &[(String, Option<String>)]) -> u32 {
    let Some((member, tip)) = turns.last() else { return 0 };
    turns.iter().rev().take_while(|(m, t)| m == member && t == tip).count() as u32
}

/// Why a ticket stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trip {
    /// Its turns used `spent` tokens, at or over `limit`.
    Budget { spent: u64, limit: u64 },
    /// `member` took `turns` turns in a row on it without a new commit.
    Repeats { member: String, turns: u32 },
}

/// Whether a ticket stops now: it spent `spent` against `limit`, and its
/// last turns repeated `repeated` times (by `member`).
pub fn trip(settings: &Settings, spent: u64, limit: Option<u64>, repeated: u32, member: &str) -> Option<Trip> {
    if let Some(limit) = limit
        && spent >= limit
    {
        return Some(Trip::Budget { spent, limit });
    }
    (settings.repeat_turns > 0 && repeated >= settings.repeat_turns).then(|| Trip::Repeats { member: member.to_owned(), turns: repeated })
}

/// The kinds of a ticket's record that send work back to its member.
pub const SEND_BACKS: [&str; 3] = ["sent back", "design QA: changes", "reopened"];
/// The kinds whose text is a hand-in's or a hand-off's note.
const NOTES: [&str; 5] = ["handed in", crate::lead_time::BATON, "handed back", "waiting on", "handoff"];

/// What the PM is told of a ticket that stopped: what it cost and how it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub ticket: TicketId,
    pub title: String,
    pub spent: u64,
    pub turns: u64,
    pub batons: usize,
    pub send_backs: usize,
    /// Its last hand-in or hand-off note, clipped.
    pub last_note: Option<String>,
    pub trip: Trip,
    /// What going on gives it.
    pub budget: u64,
}

impl Card {
    /// The card for `ticket`, whose record is `record`, that took `turns`
    /// turns and `spent` tokens and stopped for `trip`.
    pub fn new(ticket: &Ticket, record: &[TicketEntry], spent: u64, turns: u64, trip: Trip, settings: &Settings) -> Self {
        let batons = record.iter().filter(|e| e.kind == crate::lead_time::BATON).count();
        let send_backs = record.iter().filter(|e| SEND_BACKS.contains(&e.kind.as_str())).count();
        let last_note = record
            .iter()
            .rev()
            .filter(|e| NOTES.contains(&e.kind.as_str()))
            .map(|e| e.text.trim().to_owned())
            .find(|t| !t.is_empty())
            .or_else(|| ticket.note.clone().filter(|n| !n.trim().is_empty()))
            .map(|n| clip(&n, NOTE_KEPT));
        let budget = if settings.ticket_budget > 0 { settings.ticket_budget } else { DEFAULT_TICKET_BUDGET };
        Card { ticket: ticket.id, title: ticket.title.clone(), spent, turns, batons, send_backs, last_note, trip, budget }
    }

    /// Why it stopped, in the PM's words.
    pub fn why(&self) -> String {
        match &self.trip {
            Trip::Budget { limit, .. } => format!("its turns used {} tokens, past its budget of {}", amount(self.spent), amount(*limit)),
            Trip::Repeats { member, turns } => format!("{member} took {turns} turns in a row on it without a new commit"),
        }
    }

    /// The ticket record's line for the stop: the tokens spent first ([`stopped_at`]).
    pub fn record_line(&self) -> String {
        format!("{} tokens over {} turns — {}", self.spent, self.turns, self.why())
    }

    /// What the PM is told: what it cost, how it went, and its three ways on.
    pub fn pm_line(&self) -> String {
        let note = self.last_note.as_deref().map(|n| format!(" Its last hand-in or hand-off said: \"{n}\".")).unwrap_or_default();
        format!(
            "T-{id} \"{title}\" was stopped before another turn: {why}. In all: {spent} tokens over {turns} turns, {batons} hand-overs \
between members, {sends} send-backs.{note} Nothing more runs on it until you decide, and its branch and worktree are kept: \
go on — put it back with `update_ticket` status todo, and it gets another {budget} tokens; narrow it — put what is really \
left in a new, smaller ticket and cancel this one; or cancel it.",
            id = self.ticket,
            title = self.title,
            why = self.why(),
            spent = amount(self.spent),
            turns = self.turns,
            batons = self.batons,
            sends = self.send_backs,
            budget = amount(self.budget),
        )
    }
}

fn clip(text: &str, most: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= most {
        return text;
    }
    let mut cut: String = text.chars().take(most).collect();
    cut.push('…');
    cut
}

// ── the squad's spend rate ──

/// The last hour against the week before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    /// Tokens of the turns that ended in the last hour.
    pub last_hour: u64,
    /// The median of the week's hours that spent anything, before the last
    /// hour; `None` when too few hours did ([`RATE_MIN_HOURS`]).
    pub median: Option<u64>,
    /// The tickets that spent most in the last hour, most first.
    pub top: Vec<(TicketId, u64)>,
}

impl Rate {
    /// Whether the last hour spent too fast.
    pub fn high(&self) -> bool {
        self.median.is_some_and(|m| m > 0 && self.last_hour >= RATE_MIN_TOKENS && self.last_hour > m.saturating_mul(RATE_FACTOR))
    }
}

/// The squad's rate at `now` from its turns (when each ended, its tokens,
/// its ticket) of the last [`RATE_WINDOW`].
pub fn rate(turns: &[(u64, u64, Option<TicketId>)], now: u64) -> Rate {
    let hour_ago = now.saturating_sub(HOUR);
    let since = now.saturating_sub(RATE_WINDOW);
    let mut last_hour = 0;
    let mut by_ticket: HashMap<TicketId, u64> = HashMap::new();
    // Hours before the last one, counted back from it: hour 0 ends where the last hour begins.
    let mut hours: HashMap<u64, u64> = HashMap::new();
    for &(ended, tokens, ticket) in turns {
        if ended > now || ended < since {
            continue;
        }
        if ended > hour_ago {
            last_hour += tokens;
            if let Some(t) = ticket {
                *by_ticket.entry(t).or_default() += tokens;
            }
        } else {
            *hours.entry((hour_ago - ended) / HOUR).or_default() += tokens;
        }
    }
    let mut spent: Vec<u64> = hours.into_values().filter(|t| *t > 0).collect();
    let median = (spent.len() >= RATE_MIN_HOURS).then(|| {
        spent.sort_unstable();
        let mid = spent.len() / 2;
        if spent.len().is_multiple_of(2) { (spent[mid - 1] + spent[mid]) / 2 } else { spent[mid] }
    });
    let mut top: Vec<(TicketId, u64)> = by_ticket.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(RATE_TOP);
    Rate { last_hour, median, top }
}

/// What a squad's rate alarm remembers: when it last told.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateState {
    pub told_at: Option<u64>,
}

impl RateState {
    /// Whether `rate`, looked at `now`, is told: high, and not told within the hour.
    pub fn step(&mut self, rate: &Rate, now: u64) -> bool {
        if !rate.high() || self.told_at.is_some_and(|at| now.saturating_sub(at) < RATE_TELL_EVERY) {
            return false;
        }
        self.told_at = Some(now);
        true
    }
}

// ── plan headroom ──

/// One signed-in engine's weekly window, as last read; `None`: not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Room {
    pub engine: Engine,
    pub week: Option<equipe_protocol::PlanWindow>,
}

impl Room {
    /// The week's use in percent at `now`: a window past its reset is spent no more.
    pub fn used(&self, now: i64) -> Option<u8> {
        self.week.map(|w| if w.resets_at > 0 && w.resets_at <= now { 0 } else { w.used_percent })
    }
}

/// The level every signed-in engine's week is at or past, of
/// [`HEADROOM_LEVELS`]; `None` when one is under the first, or wasn't read.
pub fn headroom_level(rooms: &[Room], now: i64) -> Option<u8> {
    if rooms.is_empty() {
        return None;
    }
    let least = rooms.iter().map(|r| r.used(now)).collect::<Option<Vec<u8>>>()?.into_iter().min()?;
    HEADROOM_LEVELS.iter().rev().copied().find(|level| least >= *level)
}

/// Whether `level` is told, given the highest level told so far (`told`,
/// 0: none): each level once, again only after the weeks dropped below the first.
pub fn headroom_step(told: &mut u8, level: Option<u8>) -> Option<u8> {
    match level {
        Some(level) if level > *told => {
            *told = level;
            Some(level)
        }
        Some(_) => None,
        None => {
            *told = 0;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::{PlanWindow, TicketKind, TicketStatus};

    fn entry(kind: &str, text: &str) -> TicketEntry {
        TicketEntry { at: 0, by: "equipe".into(), kind: kind.into(), text: text.into() }
    }

    fn ticket() -> Ticket {
        Ticket {
            id: 7,
            title: "Verify the checkout flow".into(),
            body: String::new(),
            status: TicketStatus::InProgress,
            kind: TicketKind::Code,
            owner: Some("qa".into()),
            assignee: Some("qa".into()),
            project: None,
            depends_on: vec![],
            branch: None,
            worktree: None,
            execution: None,
            merged: None,
            off_main: None,
            design_qa: None,
            landing: false,
            plan: None,
            model: None,
            flow: None,
            created_by: "pm".into(),
            note: None,
            asks: vec![],
            label: None,
            remaining: vec![],
            priority: Default::default(),
            rank: 0,
            updated_at: None,
            images: Default::default(),
        }
    }

    #[test]
    fn a_ticket_stops_at_its_budget_and_each_go_on_gives_it_another() {
        let s = Settings::default();
        assert_eq!(limit(&s, &[]), Some(DEFAULT_TICKET_BUDGET));
        assert_eq!(trip(&s, DEFAULT_TICKET_BUDGET - 1, limit(&s, &[]), 0, "qa"), None);
        assert_eq!(
            trip(&s, DEFAULT_TICKET_BUDGET, limit(&s, &[]), 0, "qa"),
            Some(Trip::Budget { spent: DEFAULT_TICKET_BUDGET, limit: DEFAULT_TICKET_BUDGET })
        );
        // Stopped at 31M, then put back: another 30M from there.
        let record = [entry("created", ""), entry(STOP, "31000000 tokens over 27 turns — its turns used 31.0M tokens")];
        assert_eq!(limit(&s, &record), Some(61_000_000));
        assert_eq!(trip(&s, 40_000_000, limit(&s, &record), 0, "qa"), None);
        // A squad that turned it off never stops on tokens.
        let off = Settings { ticket_budget: 0, ..s };
        assert_eq!(limit(&off, &record), None);
        assert_eq!(trip(&off, u64::MAX, None, 0, "qa"), None);
    }

    #[test]
    fn the_same_member_turning_without_a_commit_stops_it_sooner() {
        let s = Settings { repeat_turns: 3, ..Settings::default() };
        let t = |m: &str, tip: &str| (m.to_owned(), Some(tip.to_owned()));
        let turns = vec![t("qa", "a"), t("dev", "a"), t("dev", "b"), t("dev", "b"), t("dev", "b")];
        assert_eq!(repeats(&turns), 3);
        assert_eq!(trip(&s, 10, Some(1_000), repeats(&turns), "dev"), Some(Trip::Repeats { member: "dev".into(), turns: 3 }));
        // A new commit, or another member, starts the count again.
        assert_eq!(repeats(&[t("dev", "b"), t("dev", "c")]), 1);
        assert_eq!(repeats(&[t("dev", "b"), t("qa", "b")]), 1);
        assert_eq!(repeats(&[]), 0);
        assert_eq!(trip(&Settings { repeat_turns: 0, ..s }, 10, None, 99, "dev"), None, "off");
    }

    #[test]
    fn the_pms_card_says_what_it_cost_how_it_went_and_the_three_ways_on() {
        let record = vec![
            entry("created", ""),
            entry(crate::lead_time::BATON, "qa · checked 3 of 5"),
            entry("sent back", "the login step fails"),
            entry(crate::lead_time::BATON, "frontend · fixed the login step"),
            entry("handed in", "All five steps pass on staging."),
        ];
        let spent = 31_200_000;
        let s = Settings::default();
        let card = Card::new(&ticket(), &record, spent, 27, trip(&s, spent, limit(&s, &record), 0, "qa").unwrap(), &s);
        assert_eq!((card.batons, card.send_backs, card.turns), (2, 1, 27));
        assert_eq!(card.last_note.as_deref(), Some("All five steps pass on staging."));
        let line = card.pm_line();
        for said in ["T-7", "Verify the checkout flow", "31.2M tokens", "27 turns", "2 hand-overs", "1 send-backs", "All five steps pass", "status todo", "another 30.0M", "cancel"] {
            assert!(line.contains(said), "{said}: {line}");
        }
        // Its record line starts with what it spent, for the next budget.
        assert_eq!(stopped_at(&card.record_line()), Some(spent));
    }

    #[test]
    fn tokens_are_read_as_people_type_them() {
        assert_eq!(parse_tokens("30M"), Some(30_000_000));
        assert_eq!(parse_tokens("500k"), Some(500_000));
        assert_eq!(parse_tokens("1.5b"), Some(1_500_000_000));
        assert_eq!(parse_tokens("1234"), Some(1234));
        assert_eq!(parse_tokens("off"), Some(0));
        assert_eq!(parse_tokens("lots"), None);
    }

    const NOW: u64 = 1_800_000_000;

    /// A week of hours each spending `per_hour` (on T-1), the last hour `last` split over three tickets.
    fn week(per_hour: u64, last: [u64; 4]) -> Vec<(u64, u64, Option<TicketId>)> {
        let mut turns: Vec<(u64, u64, Option<TicketId>)> = (1..7 * 24).map(|h| (NOW - h * HOUR - 60, per_hour, Some(1))).collect();
        for (i, tokens) in last.iter().enumerate() {
            turns.push((NOW - 60 * (i as u64 + 1), *tokens, Some(10 + i as TicketId)));
        }
        turns
    }

    #[test]
    fn the_spend_rate_compares_the_last_hour_with_the_weeks_hourly_median() {
        let normal = rate(&week(3_000_000, [1_000_000, 1_000_000, 1_000_000, 1_000_000]), NOW);
        assert_eq!(normal.median, Some(3_000_000));
        assert_eq!(normal.last_hour, 4_000_000);
        assert!(!normal.high(), "under twice the median");
        let fast = rate(&week(3_000_000, [5_000_000, 1_000_000, 4_000_000, 500_000]), NOW);
        assert!(fast.high());
        assert_eq!(fast.top, vec![(10, 5_000_000), (12, 4_000_000), (11, 1_000_000)], "the top three by spend");
        // Idle hours don't drag the median down: only hours that spent count.
        let mut sparse = week(3_000_000, [5_000_000, 0, 0, 0]);
        sparse.retain(|(ended, _, _)| ((NOW - ended) / HOUR).is_multiple_of(3) || NOW - ended < HOUR);
        assert_eq!(rate(&sparse, NOW).median, Some(3_000_000));
        // Too few hours of history: no median, no alarm.
        let young: Vec<_> = week(1, [9_000_000, 0, 0, 0]).into_iter().filter(|(ended, _, _)| NOW - ended < 10 * HOUR).collect();
        assert_eq!(rate(&young, NOW).median, None);
        assert!(!rate(&young, NOW).high());
    }

    #[test]
    fn a_high_rate_is_told_at_most_once_an_hour_and_only_while_it_stays_high() {
        let high = rate(&week(1_000_000, [3_000_000, 0, 0, 0]), NOW);
        let low = rate(&week(1_000_000, [1_000_000, 0, 0, 0]), NOW);
        assert!(high.high() && !low.high());
        let mut state = RateState::default();
        assert!(state.step(&high, NOW));
        assert!(!state.step(&high, NOW + 600), "within the hour");
        assert!(!state.step(&high, NOW + 3000));
        assert!(state.step(&high, NOW + HOUR), "still high an hour on");
        assert!(!state.step(&low, NOW + 2 * HOUR + 1), "back to normal: nothing");
        assert!(state.step(&high, NOW + 3 * HOUR + 1));
    }

    #[test]
    fn plan_headroom_is_told_once_per_level_when_every_signed_in_engine_is_nearly_out() {
        let now = 1_800_000_000_i64;
        let room = |engine, used| Room { engine, week: Some(PlanWindow { used_percent: used, resets_at: now + 86_400 }) };
        let mut told = 0;
        // One engine still has room: nothing.
        assert_eq!(headroom_level(&[room(Engine::Claude, 100), room(Engine::Codex, 80)], now), None);
        assert_eq!(headroom_step(&mut told, headroom_level(&[room(Engine::Claude, 100), room(Engine::Codex, 80)], now)), None);
        // Both past 90: told once.
        let level = headroom_level(&[room(Engine::Claude, 100), room(Engine::Codex, 91)], now);
        assert_eq!(level, Some(90));
        assert_eq!(headroom_step(&mut told, level), Some(90));
        assert_eq!(headroom_step(&mut told, Some(90)), None, "not again at the same level");
        // Past 95: told once more.
        let level = headroom_level(&[room(Engine::Claude, 100), room(Engine::Codex, 96)], now);
        assert_eq!(headroom_step(&mut told, level), Some(95));
        assert_eq!(headroom_step(&mut told, Some(95)), None);
        // The week reset: the next time it fills, told again.
        let reset = [Room { engine: Engine::Claude, week: Some(PlanWindow { used_percent: 100, resets_at: now - 1 }) }, room(Engine::Codex, 96)];
        assert_eq!(headroom_level(&reset, now), None, "a window past its reset is spent no more");
        assert_eq!(headroom_step(&mut told, None), None);
        assert_eq!(headroom_step(&mut told, Some(90)), Some(90));
        // An engine never read: unknown, so nothing.
        assert_eq!(headroom_level(&[room(Engine::Claude, 99), Room { engine: Engine::Codex, week: None }], now), None);
        assert_eq!(headroom_level(&[], now), None);
    }
}
