//! The sprint loop (docs/SPRINTS.md): once a product is live, the squad works
//! on it the way a startup team does — in sprints. Each sprint has a goal tied
//! to the product's north star, what "done" looks like, the items it ships
//! and the hypotheses they test. Every hypothesis carries a decision rule
//! fixed before its change goes live ("signup conversion +10% within 7 days,
//! at least 200 people"); once the window has passed, equipe reads the numbers
//! through the squad's product-data connection and says worked, didn't or
//! inconclusive, with the evidence. At the sprint's end the PM writes a short
//! retro from what equipe counted and proposes the next sprint as ranked
//! items the person starts with one tap.
//!
//! The daemon keeps the loop per squad; screens read it with `sprint.get`
//! ([`SprintView`]: "This sprint", "What we learned", "Next sprint
//! (proposed)"), and are told to read it again by `Event::SprintChanged`.
//! Only the person's own clients end a sprint early or change the loop's
//! settings (`sprint.end`, `sprint.set`).

use serde::{Deserialize, Serialize};

use crate::TicketId;
use crate::growth::{Better, Risk, Size};

/// The methods (JSON-RPC requests over the daemon's socket).
pub const GET: &str = "sprint.get";
/// The person ends the sprint under way now: its review starts.
pub const END: &str = "sprint.end";
/// The person changes the loop's settings ([`SetParams`]).
pub const SET: &str = "sprint.set";

/// `sprint.get` and `sprint.end`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SprintParams {
    pub squad: String,
}

/// `sprint.set`: each field given is set, the rest stay. Answered with the
/// [`SprintView`] as it is then.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SetParams {
    pub squad: String,
    /// On, off, or back to automatic (`null` isn't distinguishable from left
    /// out: `auto: true` puts it back to automatic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
    /// The review's day (1 Monday … 7 Sunday) and hour, the daemon machine's local time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekday: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<u8>,
    /// Days a sprint runs: 7 or 14.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_days: Option<u32>,
    /// Tokens one sprint may use, by equipe's count of its tickets' turns; 0: equipe's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u64>,
}

/// `sprint.end`'s answer: the review started, or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    pub started: bool,
    /// Why not, in English.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// The loop's settings, as the person set them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    /// `None`: automatic — on once the squad has a PM and a product live (a
    /// deploy seen) or a PostHog connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// The review's slot; `None`: the weekly retro routine's, else Friday 17:00.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekday: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<u8>,
    /// Days a sprint runs; `0`: 7.
    #[serde(default)]
    pub length_days: u32,
    /// Tokens a sprint may use; `0`: equipe's default.
    #[serde(default)]
    pub budget_tokens: u64,
}

/// What a product's numbers are read as: how many times something happened,
/// or the share of people who did one thing and then another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Metric {
    /// How many `event` events.
    Event {
        event: String,
        /// Only the events matching this condition (`prop=value`,
        /// `prop!=value`, `prop~text`), as `.equipe/growth.json`'s `exclude` reads them.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        filter: String,
    },
    /// The share (%) of the people who did `from` that then did `to` within `days`.
    Conversion {
        from: String,
        to: String,
        days: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        filter: String,
    },
}

impl Metric {
    /// In a few words, in English: `signup events`, `visit → signup within 1 day`.
    pub fn label(&self) -> String {
        let with = |f: &str| if f.is_empty() { String::new() } else { format!(" ({f})") };
        match self {
            Metric::Event { event, filter } => format!("{event} events{}", with(filter)),
            Metric::Conversion { from, to, days, filter } => format!("{from} → {to} within {days} day{}{}", if *days == 1 { "" } else { "s" }, with(filter)),
        }
    }

    /// A share of people, not a count.
    pub fn is_share(&self) -> bool {
        matches!(self, Metric::Conversion { .. })
    }
}

/// How a hypothesis is judged, fixed before its change goes live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Which way is better.
    #[serde(default)]
    pub better: Better,
    /// The change it must make, the better way, in percent of what it was: `10` for +10%.
    pub min_change_pct: f64,
    /// Days after going live it is read over (and as many before, beside it).
    pub window_days: u32,
    /// The fewest counted — events, or people for a share — before and
    /// after (each arm, for a split) for any verdict but inconclusive.
    pub min_sample: u64,
}

/// An A/B split by a feature flag the product reads: the people in each
/// variant compared with those in `control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    pub flag: String,
    #[serde(default = "control")]
    pub control: String,
}

fn control() -> String {
    "control".into()
}

/// What a hypothesis found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// It moved by the rule's change, the better way, beyond chance.
    Worked,
    /// It didn't reach the rule's change (or went the wrong way: `worse`).
    DidNot,
    /// Too few counted, too close to call, or the numbers couldn't be read.
    Inconclusive,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Worked => "worked",
            Verdict::DidNot => "didn't",
            Verdict::Inconclusive => "inconclusive",
        }
    }
}

/// A metric over some days: its value and how many it counted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub value: f64,
    /// Events for a count; people who entered for a share.
    pub sample: f64,
}

/// One arm of a split, over the window after going live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub variant: String,
    pub value: f64,
    pub sample: f64,
}

/// A hypothesis judged: the verdict and the numbers behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judged {
    pub verdict: Verdict,
    /// Why, in a short English line: "too few: 80 people, the rule needs 200".
    pub why: String,
    /// It went the wrong way, beyond chance.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worse: bool,
    /// Before and after going live (a before/after reading).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Reading>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Reading>,
    /// Each variant (a split).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arms: Vec<Arm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// The days read, `YYYY-MM-DD`: after going live, and before it (none for a split).
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before_from: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before_to: String,
    /// What read them: the query as sent, and where (`posthog m1`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub at: u64,
}

/// Where a hypothesis stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    /// Defined; its change isn't live yet — its rule may still be refined.
    Waiting,
    /// Live: its rule is locked; judged once its window has passed.
    Live,
    Judged,
    /// Its ticket was cancelled.
    Dropped,
}

/// A hypothesis a change tests, with the rule it is judged by (`H-3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    pub id: u32,
    /// The sprint it was registered in.
    pub sprint: u32,
    /// What the team believes, in the person's language: "A shorter signup
    /// form gets more people to sign up".
    pub belief: String,
    pub metric: Metric,
    pub rule: Rule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Split>,
    /// The events the change must send for it to be measured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// The ticket that ships it, and the item it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<u32>,
    /// Who set its rule last (the PM, the data analyst), and when.
    pub defined_by: String,
    pub defined_at: u64,
    pub status: ExperimentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_at: Option<u64>,
    /// When it is judged: its window after going live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<u64>,
    /// The day after going live equipe looked whether its events arrive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Judged>,
}

/// The sort of change an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// More of what worked.
    Iterate,
    /// Mend what didn't work, or broke.
    Fix,
    /// Take out what didn't work.
    Kill,
    /// Something new, grounded in usage or feedback.
    New,
    /// Go-to-market: positioning, copy, content, launches, acquisition.
    Marketing,
    /// Measuring the product: analytics, events.
    Measure,
}

/// How much an item is expected to move the goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    Low,
    Medium,
    High,
}

/// Where a proposed item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// Waiting on the person.
    Proposed,
    /// A ticket does it.
    Started,
    /// Not this sprint: offered again when the next is planned.
    Later,
    /// Let go: its proposal was replaced, or its ticket cancelled.
    Dropped,
}

/// The hypothesis an item proposes to test, as the PM proposed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub belief: String,
    pub metric: Metric,
    pub rule: Rule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Split>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

/// One item proposed for a sprint (`P-7`), ranked by its expected impact
/// against its cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    /// The sprint it is proposed for.
    pub sprint: u32,
    pub kind: ItemKind,
    pub title: String,
    /// The change, in a sentence or two.
    pub change: String,
    /// What it stands on, in a line: a result, the numbers, what users said.
    pub why: String,
    /// The evidence it cites, each in a line (`E2 · signups: 120 (2026-11-09 to …)`, `H-3 · worked …`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// The effect expected, in words: "+10% sign-ups".
    pub expected: String,
    pub impact: Impact,
    pub size: Size,
    pub risk: Risk,
    /// What it is expected to cost, in tokens (equipe's estimate by size).
    pub cost_tokens: u64,
    /// The role or member that does it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypothesis: Option<Hypothesis>,
    /// Publishes, posts, emails or spends money: whatever it drafts, that
    /// part waits for the person's card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
    /// 1 first.
    pub rank: u32,
    /// Within the sprint's budget, the strongest: started by "start the recommended".
    pub recommended: bool,
    /// Small and low-risk: may start on its own under the person's autonomy setting.
    pub may_auto_start: bool,
    pub status: ItemStatus,
    /// Started on its own (the person's autonomy setting), not by their tap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<u32>,
}

/// Where a sprint stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Proposed: its items wait on the person.
    Proposed,
    /// Under way until `ends_at`.
    Active,
    /// Ended: its facts went to the PM for the retro and the next sprint.
    Review,
    /// Reviewed: its retro written (or its review let go).
    Closed,
    /// The person said not now, or a newer proposal replaced it.
    Declined,
}

/// The retro, for a person who isn't a developer: a few short lines each.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Retro {
    #[serde(default)]
    pub shipped: Vec<String>,
    #[serde(default)]
    pub learned: Vec<String>,
    #[serde(default)]
    pub wrong: Vec<String>,
    #[serde(default)]
    pub next: Vec<String>,
}

impl Retro {
    pub fn is_empty(&self) -> bool {
        self.shipped.is_empty() && self.learned.is_empty() && self.wrong.is_empty() && self.next.is_empty()
    }
}

/// What equipe counted of a sprint at its review — nobody's word.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Facts {
    /// Its tickets, and those done.
    pub tickets: u32,
    pub done: u32,
    /// Work accepted or merged in its days.
    pub shipped: u32,
    /// Sent back, failed, stuck, late — work that rubbed.
    pub troubles: u32,
    /// Times members waited on the person, and how long in all (seconds).
    pub waits: u32,
    pub waited_secs: u64,
    /// Its hypotheses judged by now: worked, didn't, inconclusive.
    pub worked: u32,
    pub did_not: u32,
    pub inconclusive: u32,
    /// Tokens its tickets' turns used.
    pub tokens: u64,
}

/// A sprint (`S-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sprint {
    pub id: u32,
    /// What it is for, tied to the north star, in the person's language.
    pub goal: String,
    /// What "done" looks like, in a line.
    pub done_when: String,
    pub phase: Phase,
    pub proposed_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// When its review is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
    /// The items proposed for it, its tickets (carried over ones too), the hypotheses it tests.
    #[serde(default)]
    pub items: Vec<u32>,
    #[serde(default)]
    pub tickets: Vec<TicketId>,
    #[serde(default)]
    pub experiments: Vec<u32>,
    /// Tokens it may use, and what its tickets' turns used (as last counted).
    pub budget_tokens: u64,
    #[serde(default)]
    pub spent_tokens: u64,
    /// The decision set its proposal was asked on.
    #[serde(default)]
    pub set: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retro: Option<Retro>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts: Option<Facts>,
}

/// The one number that says the product works for its people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NorthStar {
    /// In the person's language: "people who get an answer each week".
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<Metric>,
    #[serde(default)]
    pub better: Better,
    /// The latest reading over a sprint's days, and the one before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<f64>,
}

/// Whether the product is sold, and to whom — the person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    /// Sold to people (B2C).
    Consumers,
    /// Sold to businesses (B2B).
    Businesses,
    /// Not sold: an internal tool, or for the person alone.
    Internal,
}

impl Market {
    pub fn is_sold(self) -> bool {
        self != Market::Internal
    }
}

/// A squad's loop as a screen shows it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SprintView {
    /// Whether the loop runs for the squad, and why not when it doesn't (English).
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_off: Option<String>,
    pub settings: Settings,
    /// The review's slot and length in force: the settings, else equipe's defaults.
    pub weekday: u8,
    pub hour: u8,
    pub length_days: u32,
    pub budget_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub north_star: Option<NorthStar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<Market>,
    /// "This sprint": the one under way, or in review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Sprint>,
    /// Its tickets done, of all.
    #[serde(default)]
    pub current_done: u32,
    /// "What we learned": the last sprint closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<Sprint>,
    /// "Next sprint (proposed)": waiting on the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed: Option<Sprint>,
    /// The items of those three sprints.
    #[serde(default)]
    pub items: Vec<Item>,
    /// Their hypotheses, and any other still measuring, newest first.
    #[serde(default)]
    pub experiments: Vec<Experiment>,
    /// When the sprint under way is reviewed, or the next planning is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_review: Option<u64>,
    /// Whether a product-data connection can read the hypotheses' numbers.
    #[serde(default)]
    pub measurable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_read_back_from_json_and_say_what_they_count() {
        let m: Metric = serde_json::from_str(r#"{"kind":"conversion","from":"visit","to":"signup","days":1}"#).unwrap();
        assert_eq!(m, Metric::Conversion { from: "visit".into(), to: "signup".into(), days: 1, filter: String::new() });
        assert_eq!(m.label(), "visit → signup within 1 day");
        assert!(m.is_share());
        let e = Metric::Event { event: "signup".into(), filter: "utm_source=newsletter".into() };
        assert_eq!(e.label(), "signup events (utm_source=newsletter)");
        assert_eq!(serde_json::to_value(&e).unwrap()["kind"], "event");
    }

    #[test]
    fn a_split_compares_with_control_unless_it_names_another() {
        let s: Split = serde_json::from_str(r#"{"flag":"short-signup"}"#).unwrap();
        assert_eq!(s.control, "control");
        assert_eq!(Verdict::DidNot.as_str(), "didn't");
        assert!(Market::Businesses.is_sold() && !Market::Internal.is_sold());
    }

    #[test]
    fn a_view_from_an_older_daemon_reads_with_defaults() {
        let v: SprintView = serde_json::from_str(r#"{"on":false,"settings":{},"weekday":5,"hour":17,"length_days":7,"budget_tokens":1}"#).unwrap();
        assert!(v.current.is_none() && v.items.is_empty() && !v.measurable);
    }
}
