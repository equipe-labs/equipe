//! The squad's status at a glance: is work going well, what slows or stops
//! it, what the person can do about it — one answer to one request
//! ([`crate::rpc::SQUAD_STATUS`]), so a screen on another machine needs no
//! logs and no second look.
//!
//! The daemon computes it from the squad's journal and its runtime state
//! (`equipe_core::status`) and sends *facts*, never sentences: which cause,
//! how many, which engine, until when. The screen puts them in the person's
//! language. A screen that meets a cause it doesn't know shows the row's
//! plain count and cause code, so a newer daemon never breaks an older app.
//!
//! Times are unix seconds, as everywhere in the protocol.

use serde::{Deserialize, Serialize};

use crate::{Engine, PauseWhy, SquadName, TicketId};

/// Whose squad, and which day is "today" for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusAsk {
    pub squad: SquadName,
    /// The person's offset from UTC, in seconds, so "today" starts at their
    /// midnight, not the daemon machine's. Omitted: UTC.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub utc_offset_secs: i32,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// How the squad is doing, as one color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    /// All good: green.
    #[default]
    Good,
    /// Going, but slowed: amber.
    Slowed,
    /// Stopped: red.
    Stopped,
}

/// How much a blocked row matters; the dot before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Waiting for the person, nothing broken: a quiet dot.
    #[default]
    Info,
    /// Slows the work.
    Amber,
    /// Stops it.
    Red,
}

/// What slows or stops the work — the kinds of row on the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// Several tickets failed fast at once: the squad starts nothing
    /// (`detail` is what the engine said, `at` when it opened).
    BreakerOpen,
    /// Runs paused, as many as `count`, for `why` (on `engine`); `at` is when
    /// they go on, if known.
    RunsPaused,
    /// Tickets their members handed back, or that couldn't start.
    HandedBack,
    /// Questions, proposals and cards waiting for the person.
    Decisions,
    /// Requests for permission waiting for the person.
    Approvals,
    /// Tickets in progress that no turn has touched for two hours.
    Quiet,
    /// An engine is out of its plan's limit until `at` (`count` runs wait on it).
    EngineOut,
    /// Turns keep failing: `count` in the last hour.
    Failing,
    /// An engine needs signing in again.
    SignIn,
}

/// Who handles a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handling {
    /// equipe handles it by itself once its time comes: "fixed by equipe".
    Fixed,
    /// equipe is trying now (retries, a breaker's test turns): "resolving".
    Resolving,
    /// The person can act: the button's word.
    You { action: Action },
}

/// The button on a row the person can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Open,
    Answer,
    Look,
}

/// Where a row's button leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "to", content = "ticket")]
pub enum Goto {
    /// The ticket's card.
    Ticket(TicketId),
    /// The list of what waits for the person.
    NeedsYou,
    /// Logins and engine sign-ins.
    Logins,
    /// The squad's progress.
    Progress,
}

/// One problem on the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub cause: Cause,
    pub severity: Severity,
    /// How many things it is about (runs, tickets, decisions); 1 for a single.
    pub count: u32,
    /// Why runs are paused ([`Cause::RunsPaused`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<PauseWhy>,
    /// The engine it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    /// Since when it has been so (the breaker), or until when (a limit's
    /// reset, the next retry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
    /// What the engine said, in a line, with anything like a key blanked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The first few tickets it is about, the likeliest to matter first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tickets: Vec<TicketId>,
    pub handling: Handling,
    /// Where its button leads, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goto: Option<Goto>,
}

/// What the second line of the status adds: a short note on an engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "note", rename_all = "snake_case")]
pub enum Note {
    /// The engine is out of its limit until `back_at`.
    Out { engine: Engine, back_at: Option<u64> },
    /// The squad runs fewer at once to make the plan last: `at_once` runs on this engine.
    Pacing { engine: Engine, at_once: u32 },
}

/// The four numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tiles {
    /// Tickets accepted since the person's midnight, and in the last two hours.
    pub accepted_today: u32,
    pub accepted_2h: u32,
    /// Tickets in progress; runs going now, and how many may at once.
    pub in_progress: u32,
    pub runs_active: u32,
    pub runs_max: u32,
    /// Everything waiting for the person, of which decisions and approvals.
    pub needs_you: u32,
    pub decisions: u32,
    pub approvals: u32,
    /// Tickets queued for a member, of which urgent.
    pub waiting: u32,
    pub waiting_urgent: u32,
}

/// One engine's plan, as the budget card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineBudget {
    pub engine: Engine,
    /// The plan as the engine names it (`max`, `plus`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// The week's window: percent used, and when it starts over. `None`: not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly: Option<crate::PlanWindow>,
    /// The five-hour window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<crate::PlanWindow>,
    /// Out of its limit now: back at this time (0: unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_until: Option<u64>,
    /// At the week's pace of late it runs out at this time, before it starts over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_out_at: Option<u64>,
    /// The pace says the week lasts until it starts over.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lasts: bool,
    /// The squad runs fewer at once on it to make it last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing_at_once: Option<u32>,
    /// The reading is old: a floor of the use, not the use now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

/// The measures of the token-waste card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasteKey {
    /// Tokens the PM's desk conversation carries in per call.
    PmDeskContext,
    /// The share of tokens in turns with a huge context per call (percent).
    HeavyShare,
    /// The share of tokens spent on tickets cancelled afterwards (percent).
    CancelledShare,
    /// The share of tokens in rework loops (percent).
    ReworkShare,
    /// Of tickets first handed in, the share accepted without a send-back
    /// (percent) — the quality check; higher is better.
    FirstHandInAccept,
}

/// One measure: the last day's value against the seven days'.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WasteMetric {
    pub key: WasteKey,
    /// The last 24 hours. `None`: nothing to measure it on yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<f64>,
    /// The last seven days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week: Option<f64>,
    /// The day's value is on the wrong side of its threshold.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bad: bool,
}

/// `equipe usage`'s signals, as the status shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Waste {
    /// Tokens and member turns in the last 24 hours; 0: no usage data yet.
    pub tokens_day: u64,
    pub turns_day: u64,
    pub metrics: Vec<WasteMetric>,
}

/// A ticket the person watches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watched {
    pub ticket: TicketId,
    pub title: String,
    pub status: crate::TicketStatus,
    /// Who it is for, or who works on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Since when it has been in this status (its last change).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(default, skip_serializing_if = "crate::Priority::is_normal")]
    pub priority: crate::Priority,
    /// The run is paused, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<PauseWhy>,
}

/// The watched list, with how it was made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchedList {
    /// The person pinned these; else the list is the squad's urgent tickets.
    pub pinned: bool,
    pub tickets: Vec<Watched>,
    /// Every ticket the person pinned, in the order they pinned them — even
    /// one closed or cut off the list: a ticket's toggle reads it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<TicketId>,
    /// Watched tickets cut off the list for being too many.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub more: u32,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// Everything the Status screen shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SquadStatus {
    pub squad: String,
    /// When it was worked out.
    pub at: u64,
    pub mood: Mood,
    /// Notes for the status line's second row, the worst first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    pub tiles: Tiles,
    /// What slows or stops the work, the worst first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<Block>,
    /// Rows cut off the list for being too many.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub blocked_more: u32,
    /// Engines whose plans were read, or that members run on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<EngineBudget>,
    pub waste: Waste,
    pub watched: WatchedList,
    /// What waits on the person, the most blocking first (the first few; the
    /// rest is `waiting_more`). Left out by an older daemon.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<crate::importance::Waiting>,
    /// Items cut off `waiting` for being too many.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub waiting_more: u32,
    /// The squad has no members or tickets yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub empty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_status_survives_the_wire_and_an_older_daemons_missing_parts_read_as_empty() {
        let status = SquadStatus {
            squad: "acme".into(),
            at: 1_760_000_000,
            mood: Mood::Slowed,
            notes: vec![Note::Out { engine: Engine::Codex, back_at: Some(1_760_259_200) }],
            blocked: vec![Block {
                cause: Cause::RunsPaused,
                severity: Severity::Amber,
                count: 12,
                why: Some(PauseWhy::EngineLimit),
                engine: Some(Engine::Codex),
                at: Some(1_760_259_200),
                detail: None,
                tickets: vec![4, 9],
                handling: Handling::Fixed,
                goto: None,
            }],
            ..SquadStatus::default()
        };
        let back: SquadStatus = serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        assert_eq!(back, status);
        let waiting = SquadStatus {
            waiting: vec![crate::importance::Waiting {
                kind: crate::importance::WaitKind::Ticket,
                ticket: Some(3),
                status: Some(crate::TicketStatus::Waiting),
                title: "AWS admin sign-in".into(),
                since: None,
                impact: crate::importance::Impact { score: 55, blocks: 5, ..Default::default() },
            }],
            waiting_more: 2,
            ..status.clone()
        };
        let back: SquadStatus = serde_json::from_str(&serde_json::to_string(&waiting).unwrap()).unwrap();
        assert_eq!(back, waiting, "what waits on the person, most blocking first, survives the wire");
        assert!(serde_json::to_value(&status).unwrap().get("waiting").is_none(), "left out when none: an older screen reads it as before");
        let old: SquadStatus = serde_json::from_value(json!({
            "squad": "acme", "at": 1, "mood": "good", "tiles": {}, "waste": { "tokens_day": 0, "turns_day": 0, "metrics": [] }, "watched": { "pinned": false, "tickets": [] }
        }))
        .unwrap();
        assert_eq!(old.mood, Mood::Good);
        assert!(old.blocked.is_empty() && old.engines.is_empty() && old.waiting.is_empty() && old.waiting_more == 0);
    }

    #[test]
    fn handling_says_who_acts_in_plain_words_on_the_wire() {
        assert_eq!(serde_json::to_value(Handling::Fixed).unwrap(), json!("fixed"));
        assert_eq!(serde_json::to_value(Handling::You { action: Action::Answer }).unwrap(), json!({ "you": { "action": "answer" } }));
        assert_eq!(serde_json::to_value(Goto::Ticket(7)).unwrap(), json!({ "to": "ticket", "ticket": 7 }));
        assert_eq!(serde_json::to_value(Goto::NeedsYou).unwrap(), json!({ "to": "needs_you" }));
    }

    #[test]
    fn a_change_is_told_to_the_squads_screens_without_figures() {
        let told = crate::Event::StatusChanged { squad: "acme".into() };
        assert_eq!((told.squad(), told.session(), told.for_everyone()), (Some("acme"), None, false));
        assert_eq!(serde_json::to_value(&told).unwrap(), json!({ "type": "status_changed", "squad": "acme" }));
        let ask: StatusAsk = serde_json::from_value(json!({ "squad": "acme" })).unwrap();
        assert_eq!(ask.utc_offset_secs, 0, "an older screen sends no offset");
        let watch: crate::Command = serde_json::from_value(json!({ "type": "watch_ticket", "squad": "acme", "ticket": 7, "watch": true })).unwrap();
        assert!(matches!(watch, crate::Command::WatchTicket { ticket: 7, watch: true, .. }));
    }

    #[test]
    fn severity_orders_from_quiet_to_stopping() {
        assert!(Severity::Info < Severity::Amber && Severity::Amber < Severity::Red);
    }
}
