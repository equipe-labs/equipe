//! The growth loop (docs/GROWTH.md): equipe reads a squad's product data,
//! the PM turns what it found into recommendation cards — each with its
//! numbers — the person answers each (accept, later, no), an accepted card
//! becomes a ticket, and once it is live equipe measures the card's metric
//! again after 7 and 14 days and says whether it worked.
//!
//! The cards and their outcomes are kept by the daemon per squad; screens
//! read them with `growth.list` and start a run with `growth.run`.

use serde::{Deserialize, Serialize};

/// The methods (JSON-RPC requests over the daemon's socket).
pub const LIST: &str = "growth.list";
pub const RUN: &str = "growth.run";

/// `growth.list` → [`Growth`]; `growth.run` → [`RunStarted`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GrowthParams {
    pub squad: String,
}

/// `growth.run`'s answer: the run started (its numbers are gathered off the
/// daemon's loop, then handed to the PM), or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStarted {
    pub started: bool,
    /// Why it didn't start (no PM, over the week's budget, one running), in English.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A squad's growth loop as a screen shows it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Growth {
    /// Every card, newest first.
    pub cards: Vec<Card>,
    /// The latest run, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<u64>,
    /// Tokens the loop spent (by equipe's estimate) in the last 7 days, and its weekly cap.
    #[serde(default)]
    pub spent: u64,
    #[serde(default)]
    pub cap: u64,
}

/// One number a card stands on: what was counted, over which days, by which query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// `E1`, `E2`… within its run.
    pub id: String,
    /// The metric it measures, as cards name it: `event:question_sent`,
    /// `funnel:question_sent>answer_received`, `health:api`, `visits`.
    pub metric: String,
    /// In a few words, in English: "question_sent → answer_received conversion".
    pub label: String,
    pub value: f64,
    /// The same over the period before, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<f64>,
    /// `events`, `%`, `people`, `ms`, `page_loads`…
    pub unit: String,
    /// The days counted, `YYYY-MM-DD` (UTC), first and last.
    pub from: String,
    pub to: String,
    /// What it was read with: the HogQL as sent, `GET <url>`, a snapshot.
    pub query: String,
    /// Where: `posthog m1`, `health`, `metrics m2`, `equipe signals`.
    pub source: String,
    /// Whether more is better (a conversion) or less (errors, slowness).
    #[serde(default)]
    pub better: Better,
    /// How it is measured again after a change goes live; `None`: it can't be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<Measure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Better {
    #[default]
    Up,
    Down,
}

/// How a metric is read again over any days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measure {
    /// How many `event` events (PostHog), the run's exclusions applied.
    Event { connection: String, event: String, exclude: String },
    /// The share of people who did `from` and then `to` within `days` (PostHog).
    Funnel {
        connection: String,
        from: String,
        to: String,
        days: u32,
        exclude: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        audience: String,
    },
    /// A connection's kept daily metric (`metrics` snapshots), summed over the days.
    Snapshot { connection: String, metric: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    S,
    M,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// Where a card stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting on the person's answer.
    Open,
    /// "Later": asked again with the next run's cards.
    Later,
    /// "No": not proposed again for a while.
    Declined,
    /// "Accept": a ticket does it; measured once it is live.
    Accepted,
    /// Its ticket is live: its follow-ups are due.
    Live,
    /// Both follow-ups measured.
    Measured,
    /// Put away without an answer, or its ticket was cancelled.
    Dropped,
}

impl Status {
    /// Waiting on the person: counts against the open cards' cap.
    pub fn is_open(self) -> bool {
        matches!(self, Status::Open | Status::Later)
    }
}

/// What a follow-up found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Worked,
    NoChange,
    Worse,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Worked => "worked",
            Verdict::NoChange => "no change",
            Verdict::Worse => "worse",
        }
    }
}

/// A measurement of a live card's metric, `after_days` after it went live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUp {
    /// 7 or 14.
    pub after_days: u32,
    /// When it is due, unix seconds.
    pub due_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub verdict: Verdict,
    /// The metric over the `after_days` before it went live, and after.
    pub before: f64,
    pub after: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// The days of each, `YYYY-MM-DD`, and the query that read them.
    pub before_from: String,
    pub before_to: String,
    pub after_from: String,
    pub after_to: String,
    pub query: String,
    pub at: u64,
    /// Why it couldn't be measured, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

/// A recommendation the person answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// `G-1`, `G-2`… within the squad.
    pub id: u32,
    pub title: String,
    /// The proposed change, in a sentence or two.
    pub change: String,
    /// The metric it should move (an evidence's `metric`), and how.
    pub metric: String,
    pub expected: String,
    pub size: Size,
    pub risk: Risk,
    /// The numbers it stands on — never none.
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub status: Status,
    pub proposed_at: u64,
    /// The decision set it was asked on, and which question it is there.
    #[serde(default)]
    pub set: u64,
    #[serde(default)]
    pub question: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<u64>,
    /// The ticket that does it, and the request it serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<FollowUp>,
    /// A card equipe made itself after a follow-up said "worse": the card it undoes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverts: Option<u32>,
}

impl Card {
    /// The latest follow-up measured, when one is.
    pub fn latest(&self) -> Option<&Outcome> {
        self.follow_ups.iter().rev().find_map(|f| f.outcome.as_ref())
    }
}
