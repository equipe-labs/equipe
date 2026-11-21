//! The sprint loop (docs/SPRINTS.md, `equipe_protocol::sprint`): once a
//! squad's product is live, it works in sprints, the way a startup team does.
//!
//! ```text
//! plan ─▶ person taps ─▶ active ─▶ ship ─▶ live ─▶ judged by its rule
//!   ▲                       │
//!   └── PM: retro + next ◀── review (facts, results, numbers)
//! ```
//!
//! - **Plan.** The PM proposes a sprint with [`PLAN`]: a goal tied to the
//!   north star, what done looks like, and at most [`ITEMS`] items, each
//!   standing on numbers, a result or what users said, with the change it
//!   makes, its expected impact, size and risk — and, where its effect can be
//!   measured, a hypothesis with its decision rule fixed now, before it ships.
//!   equipe ranks the items by impact against cost ([`rank`]) and recommends
//!   those that fit the sprint's budget; the person starts them with one tap
//!   ([`questions`], [`picked`]). Small low-risk items may start on their own
//!   when the person's autonomy setting lets them ([`Item::may_auto_start`]).
//! - **Measure.** An item's ticket going live locks its hypothesis' rule; the
//!   day after, equipe looks whether its events arrive; once its window has
//!   passed, it reads the numbers through the squad's product data
//!   ([`data::ProductData`]) and judges them by the rule ([`judge`],
//!   [`judge_split`]): worked, didn't or inconclusive.
//! - **Review.** At the sprint's end equipe counts what it can — tickets
//!   done, what shipped, what rubbed, the waits, the tokens, the results, the
//!   product's numbers — and hands it to the PM in one message
//!   ([`review_message`]); the PM writes a short retro for the person and
//!   proposes the next sprint in the same [`PLAN`] call.
//!
//! Plain functions, tested without a runtime; the runtime gathers, keeps and
//! asks (`runtime/sprint.rs`).

use std::path::{Path, PathBuf};

use equipe_protocol::decide::{Answer, Choice, Question};
use equipe_protocol::growth::{Better, Evidence, Measure, Risk, Size};
use equipe_protocol::sprint::{
    Arm, Experiment, ExperimentStatus, Facts, Hypothesis, Impact, Item, ItemKind, ItemStatus, Market, Metric, NorthStar, Phase, Reading, Retro, Rule, Settings, Split, Sprint, Verdict,
};
use equipe_protocol::{Ticket, TicketId, TicketStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::growth::{self, alike, number, personal};
use crate::i18n::Lang;
use crate::routines::Clock;

pub mod data;
pub mod platform;

/// The PM's tool: the next sprint, and the retro of the one ending.
pub const PLAN: &str = "plan_sprint";
/// The PM's and the data analyst's: a hypothesis' metric and rule, before it goes live.
pub const DEFINE: &str = "define_experiment";
/// Where a squad's loop is kept, in its data folder.
pub const STORE: &str = "sprint.json";

pub const HOUR: u64 = 60 * 60;
pub const DAY: u64 = 24 * HOUR;
/// Days a sprint runs, unless the person says 14.
pub const LENGTH: u32 = 7;
/// The review: Fridays from 17:00, the daemon machine's local time — the
/// weekly retro's slot, which it takes over.
pub const REVIEW_WEEKDAY: u8 = 5;
pub const REVIEW_HOUR: u8 = 17;
/// Tokens a sprint may use, by equipe's count of its tickets' turns, unless set.
pub const BUDGET: u64 = 25_000_000;
/// Items a sprint proposes at most, and recommends at most.
pub const ITEMS: usize = 5;
pub const RECOMMENDED: usize = 4;
/// How long after a review equipe waits for the PM's plan before it closes
/// the sprint on its facts alone.
pub const PATIENCE: u64 = DAY;
/// A planning ask the PM didn't answer is asked again no sooner than this.
pub const PLAN_AGAIN: u64 = 6 * DAY;
/// Budget levels told once each, in percent.
pub const BUDGET_LEVELS: [u8; 2] = [80, 100];
/// Hypotheses judged before shown to the PM as "tried before", at most.
const TRIED_SHOWN: usize = 8;
/// Lines of each part of the retro, and their length, at most.
pub const RETRO_LINES: usize = 3;
const LINE_CHARS: usize = 200;
const TITLE_CHARS: usize = 120;
const TEXT_CHARS: usize = 600;
/// A move this many standard errors from none is beyond chance (95%).
pub const Z: f64 = 1.96;
/// Evidence a review hands over at most.
pub const EVIDENCE: usize = 16;

/// What an item of `size` is expected to cost, in tokens (everything its
/// turns process, as equipe counts a ticket's spend).
pub fn cost(size: Size) -> u64 {
    match size {
        Size::S => 2_000_000,
        Size::M => 6_000_000,
        Size::L => 15_000_000,
    }
}

// ---- Settings ----

/// Days a sprint runs: 7, or 14 when set so.
pub fn length(s: &Settings) -> u32 {
    if s.length_days >= 14 { 14 } else { LENGTH }
}

pub fn budget(s: &Settings) -> u64 {
    if s.budget_tokens > 0 { s.budget_tokens } else { BUDGET }
}

/// The review's slot: as set, else the weekly retro routine's, else Friday 17:00.
pub fn slot(s: &Settings, retro: Option<(u8, u8)>) -> (u8, u8) {
    let (weekday, hour) = retro.unwrap_or((REVIEW_WEEKDAY, REVIEW_HOUR));
    (s.weekday.filter(|d| (1..=7).contains(d)).unwrap_or(weekday), s.hour.filter(|h| *h < 24).unwrap_or(hour))
}

/// When a sprint started at `start` is reviewed: the first slot after half
/// its length — a week's sprint started on a Thursday runs to the next
/// Friday, not the next day.
pub fn ends_at(start: u64, (weekday, hour): (u8, u8), length_days: u32, clock: &impl Clock) -> u64 {
    let when = equipe_protocol::routines::Schedule::Weekly { weekday, hour, minute: 0 };
    let from = start + u64::from(length_days) * DAY / 2;
    let first = crate::routines::next_run(when, from, clock);
    // A fortnight's sprint takes the slot of its second week.
    if length_days >= 14 && first < start + 10 * DAY { crate::routines::next_run(when, first, clock) } else { first }
}

/// Whether the loop runs for a squad: the person's word, else on once it has
/// a PM and a product live (or analytics connected). `Err`: why not.
pub fn enabled(s: &Settings, has_pm: bool, live: bool) -> Result<(), String> {
    match s.on {
        Some(false) => Err("the person turned the sprint loop off".into()),
        _ if !has_pm => Err("the squad has no PM to run its sprints".into()),
        Some(true) => Ok(()),
        None if live => Ok(()),
        None => Err("nothing is live yet: sprints start once the product is".into()),
    }
}

// ---- What is kept ----

/// A squad's loop, as kept ([`STORE`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub next_sprint: u32,
    #[serde(default)]
    pub next_item: u32,
    #[serde(default)]
    pub next_experiment: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub north_star: Option<NorthStar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<Market>,
    #[serde(default)]
    pub sprints: Vec<Sprint>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub experiments: Vec<Experiment>,
    /// The latest review's product numbers: what items may cite (`E1`…).
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// When the PM was last asked to plan (a first sprint, or after a "not now").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning_asked: Option<u64>,
    /// When the proposal's small items start on their own (decide-and-tell).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_at: Option<u64>,
    /// Budget levels told for the sprint under way.
    #[serde(default)]
    pub budget_told: u8,
    /// The loop's own PM turns: when, and what each was counted as.
    #[serde(default)]
    pub turns: Vec<(u64, u64)>,
}

pub fn store_path(dir: &Path) -> PathBuf {
    dir.join(STORE)
}

pub fn load(dir: &Path) -> Store {
    std::fs::read_to_string(store_path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(dir: &Path, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    let partial = store_path(dir).with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, store_path(dir)).map_err(|e| e.to_string())
}

impl Store {
    /// The sprint under way, or in review.
    pub fn current(&self) -> Option<&Sprint> {
        self.sprints.iter().rev().find(|s| matches!(s.phase, Phase::Active | Phase::Review))
    }

    pub fn current_mut(&mut self) -> Option<&mut Sprint> {
        self.sprints.iter_mut().rev().find(|s| matches!(s.phase, Phase::Active | Phase::Review))
    }

    /// The proposal waiting on the person.
    pub fn proposed(&self) -> Option<&Sprint> {
        self.sprints.iter().rev().find(|s| s.phase == Phase::Proposed)
    }

    /// The last sprint closed.
    pub fn last_closed(&self) -> Option<&Sprint> {
        self.sprints.iter().rev().find(|s| s.phase == Phase::Closed)
    }

    pub fn sprint_mut(&mut self, id: u32) -> Option<&mut Sprint> {
        self.sprints.iter_mut().find(|s| s.id == id)
    }

    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: u32) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn experiment_mut(&mut self, id: u32) -> Option<&mut Experiment> {
        self.experiments.iter_mut().find(|e| e.id == id)
    }

    /// The hypothesis a ticket ships, not dropped.
    pub fn experiment_of(&self, ticket: TicketId) -> Option<&Experiment> {
        self.experiments.iter().find(|e| e.ticket == Some(ticket) && e.status != ExperimentStatus::Dropped)
    }

    /// The loop's own tokens in the 7 days before `now`.
    pub fn loop_spent(&self, now: u64) -> u64 {
        self.turns.iter().filter(|(at, _)| at + 7 * DAY > now).map(|(_, t)| t).sum()
    }

    /// A PM turn of the loop's, counted.
    pub fn count_turn(&mut self, now: u64, tokens: u64) {
        self.turns.retain(|(at, _)| at + 8 * 7 * DAY > now);
        self.turns.push((now, tokens));
    }
}

// ---- The tools ----

fn text(what: &str) -> Value {
    serde_json::json!({ "type": "string", "description": what })
}

fn metric_schema() -> Value {
    serde_json::json!({
        "description": "What is counted: `event:<name>` (how many), `<a> > <b>` (the share of people who did a then b within a day), \
an evidence's metric as the review lists it, or {kind: event, event, filter} / {kind: conversion, from, to, days, filter}; \
`filter` keeps only events matching `prop=value`, `prop!=value` or `prop~text`.",
    })
}

fn hypothesis_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Only where its effect can be measured: the belief it tests and its decision rule, fixed now, before it ships.",
        "properties": {
            "belief": text("What you believe, in the person's language: \"A shorter signup form gets more people to sign up\"."),
            "metric": metric_schema(),
            "rule": {
                "type": "object",
                "properties": {
                    "min_change_pct": { "type": "number", "description": "The change it must make the better way, in % of before: 10." },
                    "window_days": { "type": "integer", "minimum": 1, "maximum": 30, "description": "Days after it goes live it is read over (7)." },
                    "min_sample": { "type": "integer", "minimum": 1, "description": "Fewest events (or people, for a share) before and after for a verdict." },
                    "better": { "type": "string", "enum": ["up", "down"] },
                },
                "required": ["min_change_pct", "window_days", "min_sample"],
            },
            "split": { "type": "object", "description": "An A/B split the product reads from a feature flag; a share metric only.", "properties": { "flag": { "type": "string" }, "control": { "type": "string" } }, "required": ["flag"] },
            "events": { "type": "array", "items": { "type": "string" }, "description": "Events the change must send to be measured." },
        },
        "required": ["belief", "metric", "rule"],
    })
}

/// `plan_sprint`, the PM's.
pub fn tool() -> Value {
    serde_json::json!({
        "name": PLAN,
        "description": "Propose the squad's next sprint, when equipe asks (\"[Sprint review · …]\", \"[Sprint planning · …]\") or the \
person wants one and none runs — and, at a review, write its retro in the same call. A goal tied to the north star, what done looks \
like, and at most 5 items: each says what it stands on (cite the review's E1/H-2, or in `why` what users said), the change, the \
expected impact, size and risk; one whose effect can be measured carries a `hypothesis` with its decision rule. Kill or fix what \
didn't work, do more of what did. equipe ranks the items by impact against cost, and the person starts them with one tap; don't \
make their tickets yourself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "retro": {
                    "type": "object",
                    "description": "At a review: for a person who isn't a developer, at most 3 short lines each, no jargon, no ticket numbers.",
                    "properties": {
                        "shipped": { "type": "array", "items": { "type": "string" } },
                        "learned": { "type": "array", "items": { "type": "string" }, "description": "What the results and numbers taught." },
                        "wrong": { "type": "array", "items": { "type": "string" } },
                        "next": { "type": "array", "items": { "type": "string" }, "description": "What changes next sprint, and why." },
                    },
                },
                "north_star": {
                    "type": "object",
                    "description": "The one number that says the product works for its people; needed once, then kept.",
                    "properties": { "name": text("In the person's language."), "metric": metric_schema(), "better": { "type": "string", "enum": ["up", "down"] } },
                    "required": ["name"],
                },
                "goal": text("What the sprint is for, tied to the north star, in a line in the person's language."),
                "done_when": text("What done looks like, in a line."),
                "market_guess": { "type": "string", "enum": ["consumers", "businesses", "internal"], "description": "While the person hasn't said: is it sold to people, to businesses, or not sold? equipe asks them, your guess marked." },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "enum": ["iterate", "fix", "kill", "new", "marketing", "measure"] },
                            "title": text("A few words."),
                            "change": text("The change, in a sentence or two."),
                            "why": text("What it stands on, in a line: a result, the numbers, what users said."),
                            "evidence": { "type": "array", "items": { "type": "string" }, "description": "The review's ids it cites: E1, H-2." },
                            "expected": text("The effect expected: \"+10% sign-ups\"."),
                            "impact": { "type": "string", "enum": ["high", "medium", "low"] },
                            "size": { "type": "string", "enum": ["S", "M", "L"] },
                            "risk": { "type": "string", "enum": ["low", "medium", "high"] },
                            "owner": text("The role that does it: frontend, data, marketing…"),
                            "project": text("The project it changes, when the squad has several."),
                            "outside": { "type": "boolean", "description": "It publishes, posts, emails or spends money: that part waits for the person's card." },
                            "hypothesis": hypothesis_schema(),
                        },
                        "required": ["kind", "title", "change", "why", "expected", "impact", "size", "risk"],
                    },
                },
            },
            "required": ["goal", "done_when", "items"],
        },
    })
}

/// `define_experiment`, the PM's and the data analyst's.
pub fn define_tool() -> Value {
    serde_json::json!({
        "name": DEFINE,
        "description": "Set or refine the hypothesis a ticket tests — its metric and decision rule — before the ticket goes live; once \
it is live the rule is locked and equipe judges it by that rule when its window has passed (worked, didn't, inconclusive).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "ticket": { "type": "integer", "description": "The ticket that ships the change." },
                "belief": text("What it tests, in the person's language."),
                "metric": metric_schema(),
                "rule": hypothesis_schema()["properties"]["rule"].clone(),
                "split": hypothesis_schema()["properties"]["split"].clone(),
                "events": hypothesis_schema()["properties"]["events"].clone(),
            },
            "required": ["ticket", "belief", "metric", "rule"],
        },
    })
}

// ---- Reading what the PM gives ----

fn line(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// An event's name as given: a few words at most, nothing that isn't printable.
fn event_name(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 80 || s.chars().any(char::is_control) {
        return Err(format!("`{s}` isn't an event's name"));
    }
    Ok(s.to_owned())
}

fn filter_of(v: Option<&Value>) -> Result<String, String> {
    let f = line(v);
    if !f.is_empty() {
        growth::condition(&f)?;
    }
    Ok(f)
}

/// A metric as the PM gave it — `event:<name>`, `<a> > <b>`, an evidence's
/// metric (its days kept), or an object.
pub fn metric_of(v: &Value, evidence: &[Evidence]) -> Result<Metric, String> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if let Some(e) = evidence.iter().find(|e| e.metric == s) {
            match &e.measure {
                Some(Measure::Event { event, .. }) => return Ok(Metric::Event { event: event.clone(), filter: String::new() }),
                Some(Measure::Funnel { from, to, days, audience, .. }) if audience.is_empty() => return Ok(Metric::Conversion { from: from.clone(), to: to.clone(), days: *days, filter: String::new() }),
                _ => return Err(format!("{s} can't be read again after a change: pick an event or a conversion")),
            }
        }
        let rest = s.strip_prefix("funnel:").or_else(|| s.strip_prefix("conversion:"));
        if let Some((a, b)) = rest.unwrap_or(s).split_once(['>', '→']) {
            return Ok(Metric::Conversion { from: event_name(a.trim_end_matches('-'))?, to: event_name(b)?, days: 1, filter: String::new() });
        }
        if rest.is_some() {
            return Err(format!("`{s}` names no second step: `a > b`"));
        }
        return Ok(Metric::Event { event: event_name(s.strip_prefix("event:").unwrap_or(s))?, filter: String::new() });
    }
    let kind = line(v.get("kind"));
    match kind.as_str() {
        "event" | "" if v.get("event").is_some() => Ok(Metric::Event { event: event_name(&line(v.get("event")))?, filter: filter_of(v.get("filter"))? }),
        "conversion" | "funnel" | "" if v.get("from").is_some() => {
            let days = v.get("days").and_then(Value::as_u64).unwrap_or(1);
            if !(1..=30).contains(&days) {
                return Err("a conversion's `days` are 1 to 30".into());
            }
            Ok(Metric::Conversion { from: event_name(&line(v.get("from")))?, to: event_name(&line(v.get("to")))?, days: days as u32, filter: filter_of(v.get("filter"))? })
        }
        _ => Err("a `metric` is `event:<name>`, `<a> > <b>`, or {kind: event, event} / {kind: conversion, from, to, days}".into()),
    }
}

fn better_of(v: Option<&Value>) -> Better {
    if line(v).eq_ignore_ascii_case("down") { Better::Down } else { Better::Up }
}

pub fn rule_of(v: Option<&Value>) -> Result<Rule, String> {
    let v = v.filter(|v| v.is_object()).ok_or("a `rule`: min_change_pct, window_days, min_sample")?;
    let pct = v.get("min_change_pct").and_then(Value::as_f64).ok_or("the rule's `min_change_pct`: the change it must make, in % (10)")?;
    if !(pct > 0.0 && pct <= 1000.0) {
        return Err("the rule's `min_change_pct` is more than 0 and at most 1000".into());
    }
    let days = v.get("window_days").and_then(Value::as_u64).ok_or("the rule's `window_days`: 1 to 30")?;
    if !(1..=30).contains(&days) {
        return Err("the rule's `window_days` are 1 to 30".into());
    }
    let sample = v.get("min_sample").and_then(Value::as_u64).ok_or("the rule's `min_sample`: the fewest counted for a verdict")?;
    if sample == 0 || sample > 10_000_000 {
        return Err("the rule's `min_sample` is at least 1".into());
    }
    Ok(Rule { better: better_of(v.get("better")), min_change_pct: pct, window_days: days as u32, min_sample: sample })
}

pub fn split_of(v: Option<&Value>, metric: &Metric) -> Result<Option<Split>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else { return Ok(None) };
    let flag = line(v.get("flag"));
    let ok = |s: &str| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok(&flag) {
        return Err("a split's `flag` is the feature flag's key: letters, digits, - and _".into());
    }
    let control = Some(line(v.get("control"))).filter(|c| !c.is_empty()).unwrap_or_else(|| "control".into());
    if !ok(&control) {
        return Err("a split's `control` is the variant's key".into());
    }
    if !metric.is_share() {
        return Err("a split compares a share of people: give a conversion metric (`a > b`)".into());
    }
    Ok(Some(Split { flag, control }))
}

fn events_of(v: Option<&Value>) -> Result<Vec<String>, String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(event_name).collect(),
        _ => Ok(Vec::new()),
    }
}

/// A hypothesis as the PM or the data analyst gave it.
pub fn hypothesis_of(v: &Value, evidence: &[Evidence]) -> Result<Hypothesis, String> {
    let belief = line(v.get("belief"));
    if belief.is_empty() || belief.chars().count() > LINE_CHARS {
        return Err(format!("a `belief`: what it tests, in at most {LINE_CHARS} characters"));
    }
    let metric = metric_of(v.get("metric").ok_or("a `metric`: what is counted")?, evidence)?;
    let rule = rule_of(v.get("rule"))?;
    let split = split_of(v.get("split"), &metric)?;
    Ok(Hypothesis { belief, metric, rule, split, events: events_of(v.get("events"))? })
}

fn kind_of(v: Option<&Value>) -> Option<ItemKind> {
    Some(match line(v).to_ascii_lowercase().as_str() {
        "iterate" => ItemKind::Iterate,
        "fix" => ItemKind::Fix,
        "kill" => ItemKind::Kill,
        "new" => ItemKind::New,
        "marketing" | "go-to-market" | "gtm" => ItemKind::Marketing,
        "measure" | "analytics" => ItemKind::Measure,
        _ => return None,
    })
}

fn impact_of(v: Option<&Value>) -> Option<Impact> {
    Some(match line(v).to_ascii_lowercase().as_str() {
        "high" => Impact::High,
        "medium" => Impact::Medium,
        "low" => Impact::Low,
        _ => return None,
    })
}

pub fn market_of(s: &str) -> Option<Market> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "consumers" | "b2c" | "people" => Market::Consumers,
        "businesses" | "b2b" => Market::Businesses,
        "internal" | "not sold" | "none" => Market::Internal,
        _ => return None,
    })
}

fn lines_of(v: Option<&Value>, what: &str, problems: &mut Vec<String>) -> Vec<String> {
    let all: Vec<String> = match v {
        Some(Value::Array(a)) => a.iter().map(|l| line(Some(l))).filter(|l| !l.is_empty()).collect(),
        Some(Value::String(s)) => s.lines().map(|l| l.trim().trim_start_matches(['-', '•', '*']).trim().to_owned()).filter(|l| !l.is_empty()).collect(),
        _ => Vec::new(),
    };
    if all.len() > RETRO_LINES {
        problems.push(format!("retro `{what}`: at most {RETRO_LINES} lines"));
    }
    if all.iter().any(|l| l.chars().count() > LINE_CHARS) {
        problems.push(format!("retro `{what}`: each line at most {LINE_CHARS} characters"));
    }
    all
}

/// What the PM's call is checked against.
#[derive(Debug, Clone)]
pub struct PlanCtx<'a> {
    pub store: &'a Store,
    /// The squad's projects' names.
    pub projects: &'a [String],
    /// The roles and names on the team.
    pub team: &'a [String],
    /// A sprint in review whose retro this call writes.
    pub reviewing: Option<u32>,
    pub now: u64,
}

/// A `plan_sprint` call, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub retro: Option<Retro>,
    pub north_star: Option<NorthStar>,
    pub goal: String,
    pub done_when: String,
    pub market_guess: Option<Market>,
    /// Numbered from the store's next, in the PM's order (ranked later).
    pub items: Vec<Item>,
}

/// `plan_sprint`, checked — all or nothing, each problem said: a goal and
/// what done looks like; the north star once; at most [`ITEMS`] items, each
/// with what it stands on (its `why`, and any ids it cites known), the
/// change, the expected impact, size and risk, naming no one; a hypothesis
/// read as [`hypothesis_of`]; at a review, the retro.
pub fn plan(args: &Value, ctx: &PlanCtx<'_>) -> Result<Plan, String> {
    let store = ctx.store;
    if let Some(p) = store.proposed() {
        return Err(format!("S-{} waits on the person already: they start it, or say not now, on its card.", p.id));
    }
    if let Some(c) = store.current().filter(|c| c.phase == Phase::Active) {
        return Err(format!("S-{} runs until its review; the next sprint is planned then (the person may end it sooner).", c.id));
    }
    let mut problems = Vec::new();
    let goal = line(args.get("goal"));
    let done_when = line(args.get("done_when"));
    if goal.is_empty() || goal.chars().count() > LINE_CHARS {
        problems.push(format!("a `goal` tied to the north star, in at most {LINE_CHARS} characters"));
    }
    if done_when.is_empty() || done_when.chars().count() > LINE_CHARS {
        problems.push(format!("`done_when`: what done looks like, in at most {LINE_CHARS} characters"));
    }
    let retro = match args.get("retro").filter(|r| r.is_object()) {
        Some(r) => {
            let retro = Retro {
                shipped: lines_of(r.get("shipped"), "shipped", &mut problems),
                learned: lines_of(r.get("learned"), "learned", &mut problems),
                wrong: lines_of(r.get("wrong"), "wrong", &mut problems),
                next: lines_of(r.get("next"), "next", &mut problems),
            };
            (!retro.is_empty()).then_some(retro)
        }
        None => None,
    };
    if ctx.reviewing.is_some() && retro.is_none() {
        problems.push("a `retro` of the sprint ending: shipped, learned, wrong, next — a few short lines each".into());
    }
    let north_star = match args.get("north_star").filter(|n| n.is_object()) {
        Some(n) => {
            let name = line(n.get("name"));
            if name.is_empty() || name.chars().count() > LINE_CHARS {
                problems.push("the north star's `name`, in a line".into());
            }
            let metric = match n.get("metric").filter(|m| !m.is_null()) {
                Some(m) => match metric_of(m, &store.evidence) {
                    Ok(m) => Some(m),
                    Err(why) => {
                        problems.push(format!("the north star's metric: {why}"));
                        None
                    }
                },
                None => None,
            };
            Some(NorthStar { name, metric, better: better_of(n.get("better")), value: None, previous: None })
        }
        None => None,
    };
    if north_star.is_none() && store.north_star.is_none() {
        problems.push("the `north_star`: the one number that says the product works for its people".into());
    }
    let market_guess = args.get("market_guess").and_then(Value::as_str).and_then(market_of);
    let asked = match args.get("items") {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    if asked.is_empty() {
        problems.push("`items`: at least one".into());
    }
    if asked.len() > ITEMS {
        problems.push(format!("{} items: at most {ITEMS} — propose the strongest", asked.len()));
    }
    let mut items = Vec::new();
    for (n, v) in asked.iter().take(ITEMS).enumerate() {
        let n = n + 1;
        let (title, change, why, expected) = (line(v.get("title")), line(v.get("change")), line(v.get("why")), line(v.get("expected")));
        if title.is_empty() || title.chars().count() > TITLE_CHARS {
            problems.push(format!("item {n}: a `title` of at most {TITLE_CHARS} characters"));
        }
        if change.is_empty() || change.chars().count() > TEXT_CHARS {
            problems.push(format!("item {n}: the `change`, in at most {TEXT_CHARS} characters"));
        }
        if why.is_empty() || why.chars().count() > TEXT_CHARS {
            problems.push(format!("item {n}: `why` — what it stands on (a result, the numbers, what users said)"));
        }
        if expected.is_empty() || expected.chars().count() > LINE_CHARS {
            problems.push(format!("item {n}: the `expected` effect, in a line"));
        }
        let kind = kind_of(v.get("kind"));
        let impact = impact_of(v.get("impact"));
        let size = growth::size_of(v.get("size"));
        let risk = growth::risk_of(v.get("risk"));
        if kind.is_none() {
            problems.push(format!("item {n}: a `kind`: iterate, fix, kill, new, marketing or measure"));
        }
        if impact.is_none() {
            problems.push(format!("item {n}: an `impact`: high, medium or low"));
        }
        if size.is_none() {
            problems.push(format!("item {n}: a `size`: S, M or L"));
        }
        if risk.is_none() {
            problems.push(format!("item {n}: a `risk`: low, medium or high"));
        }
        if kind == Some(ItemKind::Marketing) && store.market == Some(Market::Internal) {
            problems.push(format!("item {n}: the person said the product isn't sold — no go-to-market work"));
        }
        if let Some(what) = [&title, &change, &why, &expected].iter().find_map(|t| personal(t)) {
            problems.push(format!("item {n}: it names {what} — items carry counts only, never who"));
        }
        let mut cited = Vec::new();
        for id in match v.get("evidence") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_ascii_uppercase()).collect(),
            Some(Value::String(s)) => s.split([',', ' ']).map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect(),
            _ => Vec::<String>::new(),
        } {
            if let Some(e) = store.evidence.iter().find(|e| e.id == id) {
                cited.push(format!("{} · {}", e.id, growth::evidence_line(e)));
            } else if let Some(x) = id.strip_prefix("H-").or_else(|| id.strip_prefix('H')).and_then(|h| h.parse::<u32>().ok()).and_then(|h| store.experiments.iter().find(|x| x.id == h)) {
                cited.push(experiment_line(x));
            } else {
                problems.push(format!("item {n}: there is no {id} to cite — cite the review's E… and H-… only"));
            }
        }
        let owner = Some(line(v.get("owner"))).filter(|o| !o.is_empty());
        if let Some(o) = &owner
            && !ctx.team.iter().any(|t| t.eq_ignore_ascii_case(o))
        {
            problems.push(format!("item {n}: there is no {o} on the team — name a role on it, or leave `owner` out for the PM to give"));
        }
        let project = Some(line(v.get("project"))).filter(|p| !p.is_empty());
        if let Some(p) = &project
            && !ctx.projects.contains(p)
        {
            problems.push(format!("item {n}: the squad has no project \"{p}\""));
        }
        let hypothesis = match v.get("hypothesis").filter(|h| h.is_object()) {
            Some(h) => match hypothesis_of(h, &store.evidence) {
                Ok(h) => Some(h),
                Err(why) => {
                    problems.push(format!("item {n}'s hypothesis: {why}"));
                    None
                }
            },
            None => None,
        };
        if let (Some(kind), Some(impact), Some(size), Some(risk)) = (kind, impact, size, risk) {
            let outside = v.get("outside").and_then(Value::as_bool).unwrap_or(false);
            items.push(Item {
                id: store.next_item + items.len() as u32 + 1,
                sprint: store.next_sprint + 1,
                kind,
                title,
                change,
                why,
                evidence: cited,
                expected,
                impact,
                size,
                risk,
                cost_tokens: cost(size),
                owner,
                project,
                hypothesis,
                outside,
                rank: 0,
                recommended: false,
                may_auto_start: false,
                status: ItemStatus::Proposed,
                auto_started: false,
                ticket: None,
                experiment: None,
            });
        }
    }
    for (a, b) in items.iter().enumerate().flat_map(|(i, a)| items[i + 1..].iter().map(move |b| (a, b))) {
        if alike(&a.title, &b.title) {
            problems.push(format!("\"{}\" and \"{}\" are the same item: make it one", a.title, b.title));
        }
    }
    if !problems.is_empty() {
        return Err(format!("No sprint was proposed — fix these and call {PLAN} again:\n- {}", problems.join("\n- ")));
    }
    Ok(Plan { retro, north_star, goal, done_when, market_guess, items })
}

// ---- Ranking ----

fn score(item: &Item) -> f64 {
    let impact = match item.impact {
        Impact::High => 3.0,
        Impact::Medium => 2.0,
        Impact::Low => 1.0,
    };
    let risk = match item.risk {
        Risk::Low => 1.0,
        Risk::Medium => 0.8,
        Risk::High => 0.6,
    };
    impact * risk / (item.cost_tokens.max(1) as f64 / 1_000_000.0).sqrt()
}

/// The items in order of expected impact against cost (and risk), the PM's
/// order breaking ties; the strongest that fit `budget` recommended (at most
/// [`RECOMMENDED`], the first always); the small low-risk ones that aren't
/// go-to-market may start on their own.
pub fn rank(items: &mut Vec<Item>, budget: u64) {
    let mut order: Vec<(usize, f64)> = items.iter().enumerate().map(|(i, it)| (i, score(it))).collect();
    order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    let mut ranked: Vec<Item> = order.into_iter().map(|(i, _)| items[i].clone()).collect();
    let mut left = budget;
    let mut picked = 0;
    for (n, item) in ranked.iter_mut().enumerate() {
        item.rank = n as u32 + 1;
        let fits = item.cost_tokens <= left;
        item.recommended = picked < RECOMMENDED && (fits || n == 0);
        if item.recommended {
            picked += 1;
            left = left.saturating_sub(item.cost_tokens);
        }
        item.may_auto_start = item.size == Size::S && item.risk == Risk::Low && item.kind != ItemKind::Marketing && !item.outside;
    }
    *items = ranked;
}

/// Items that start on their own now: recommended and small enough, while
/// the sprint has budget left.
pub fn auto_starting(items: &[Item], spent: u64, budget: u64) -> Vec<&Item> {
    if spent >= budget {
        return Vec::new();
    }
    items.iter().filter(|i| i.status == ItemStatus::Proposed && i.recommended && i.may_auto_start).collect()
}

// ---- The person's card ----

fn impact_word(lang: Lang, i: Impact) -> String {
    let w = crate::i18n::core(lang);
    w.get(match i {
        Impact::High => "sprint.impact_high",
        Impact::Medium => "sprint.impact_medium",
        Impact::Low => "sprint.impact_low",
    })
    .to_owned()
}

fn risk_word(lang: Lang, r: Risk) -> String {
    let w = crate::i18n::core(lang);
    w.get(match r {
        Risk::Low => "growth.risk_low",
        Risk::Medium => "growth.risk_medium",
        Risk::High => "growth.risk_high",
    })
    .to_owned()
}

fn size_word(s: Size) -> &'static str {
    match s {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

/// Tokens, short: `2M`, `9.5M`, `750k`.
pub fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = format!("{:.1}", n as f64 / 1_000_000.0);
        format!("{}M", m.strip_suffix(".0").unwrap_or(&m))
    } else {
        crate::retro::tokens(n)
    }
}

/// The proposal as the person's card: whether to start it (the recommended
/// items, those they pick, or not now), the items to pick from — the
/// recommended marked, those already started on their own said so — and,
/// while they haven't said, whether the product is sold.
pub fn questions(sprint: &Sprint, items: &[&Item], ask_market: bool, guess: Option<Market>, lang: Lang) -> Vec<Question> {
    let w = crate::i18n::core(lang);
    let recommended: Vec<String> = items.iter().filter(|i| i.recommended).map(|i| format!("P-{} {}", i.id, i.title)).collect();
    let choice = |key: &str, recommended: bool| Choice { label: w.get(key).to_owned(), recommended, ..Choice::default() };
    let start = Question {
        question: w.fill(
            "sprint.q_start",
            &[
                ("id", sprint.id.to_string()),
                ("goal", sprint.goal.clone()),
                ("done", sprint.done_when.clone()),
                ("budget", tokens(sprint.budget_tokens)),
                ("recommended", recommended.join("\n")),
            ],
        ),
        header: w.fill("sprint.header", &[("id", sprint.id.to_string())]),
        options: vec![choice("sprint.start_recommended", true), choice("sprint.start_picked", false), choice("sprint.not_now", false)],
        ..Question::default()
    };
    let options = items
        .iter()
        .map(|i| {
            let mut description = w.fill(
                "sprint.item_line",
                &[("expected", i.expected.clone()), ("impact", impact_word(lang, i.impact)), ("size", size_word(i.size).to_owned()), ("risk", risk_word(lang, i.risk)), ("why", i.why.clone())],
            );
            if i.auto_started {
                description = format!("{description}\n{}", w.get("sprint.started_on_own"));
            } else if i.outside {
                description = format!("{description}\n{}", w.get("sprint.outside"));
            }
            Choice { label: format!("P-{} {}", i.id, i.title), description, recommended: i.recommended, ..Choice::default() }
        })
        .collect();
    let mut out = vec![start, Question { question: w.get("sprint.q_items").to_owned(), header: w.get("sprint.items_header").to_owned(), options, multi_select: true, ..Question::default() }];
    if ask_market {
        let m = |key: &str, market: Market| choice(key, guess == Some(market));
        out.push(Question {
            question: w.get("sprint.q_market").to_owned(),
            header: w.get("sprint.market_header").to_owned(),
            options: vec![m("sprint.market_consumers", Market::Consumers), m("sprint.market_businesses", Market::Businesses), m("sprint.market_internal", Market::Internal)],
            ..Question::default()
        });
    }
    out
}

/// What the person's answers to a proposal's card start.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Picked {
    pub items: Vec<u32>,
    pub market: Option<Market>,
    /// They said not now (or closed the card).
    pub not_now: bool,
}

/// The person's answers to [`questions`]: the items they ticked win; else
/// "start the recommended" starts the recommended; "not now", a closed card
/// or nothing picked starts none.
pub fn picked(answers: Option<&[Answer]>, items: &[&Item], ask_market: bool) -> Picked {
    let Some(answers) = answers else { return Picked { not_now: true, ..Picked::default() } };
    let start = answers.first().and_then(|a| a.picked.first().copied());
    let ticked: Vec<u32> = answers.get(1).map(|a| a.picked.iter().filter_map(|n| items.get(*n)).map(|i| i.id).collect()).unwrap_or_default();
    let market = if ask_market {
        answers.get(2).and_then(|a| a.picked.first()).and_then(|n| [Market::Consumers, Market::Businesses, Market::Internal].get(*n).copied())
    } else {
        None
    };
    let chosen = match start {
        Some(2) => Vec::new(),
        _ if !ticked.is_empty() => ticked,
        Some(0) => items.iter().filter(|i| i.recommended).map(|i| i.id).collect(),
        _ => Vec::new(),
    };
    Picked { not_now: chosen.is_empty(), items: chosen, market }
}

// ---- Judging ----

/// How far `after` moved from `before`, in standard errors: a share's
/// two-proportion z, or a count's (as two Poisson counts over equal days).
fn z(share: bool, before: Reading, after: Reading) -> f64 {
    if share {
        let (n1, n2) = (before.sample, after.sample);
        let (s1, s2) = (before.value / 100.0 * n1, after.value / 100.0 * n2);
        let p = (s1 + s2) / (n1 + n2);
        let se = (p * (1.0 - p) * (1.0 / n1 + 1.0 / n2)).sqrt();
        if se > 0.0 { (s2 / n2 - s1 / n1) / se } else { 0.0 }
    } else {
        let total = before.value + after.value;
        if total > 0.0 { (after.value - before.value) / total.sqrt() } else { 0.0 }
    }
}

/// A verdict, why, whether it went the wrong way, and the change in percent.
pub type Judgement = (Verdict, String, bool, Option<f64>);

fn pct_text(p: Option<f64>) -> String {
    p.map(|p| format!("{}{}%", if p >= 0.0 { "+" } else { "" }, number(growth::round1(p)))).unwrap_or_else(|| "from none".into())
}

/// `after` beside `before` by `rule`: fewer counted than its `min_sample`
/// on either side — inconclusive; moved by its change the better way beyond
/// chance — worked (by it but within chance: inconclusive); the wrong way by
/// it beyond chance — didn't, worse; anything else — didn't, short of the rule.
pub fn judge(rule: &Rule, share: bool, before: Reading, after: Reading) -> Judgement {
    let fewest = before.sample.min(after.sample);
    let pct = (before.value != 0.0).then(|| (after.value - before.value) / before.value.abs() * 100.0);
    if fewest < rule.min_sample as f64 {
        return (Verdict::Inconclusive, format!("too few: {} counted, the rule needs {}", number(fewest), rule.min_sample), false, pct);
    }
    let way = if rule.better == Better::Up { 1.0 } else { -1.0 };
    let lift = match pct {
        Some(p) => p * way,
        None if after.value > 0.0 => f64::INFINITY * way,
        None => 0.0,
    };
    let beyond = z(share, before, after).abs() >= Z;
    let moved = pct_text(pct);
    if lift >= rule.min_change_pct {
        if beyond {
            (Verdict::Worked, format!("moved {moved}, the rule asked {}", rule_change(rule)), false, pct)
        } else {
            (Verdict::Inconclusive, format!("moved {moved}, but that could be chance at these numbers"), false, pct)
        }
    } else if lift <= -rule.min_change_pct && beyond {
        (Verdict::DidNot, format!("went the wrong way: {moved}"), true, pct)
    } else {
        (Verdict::DidNot, format!("moved {moved}, short of the {} the rule asked", rule_change(rule)), false, pct)
    }
}

/// The rule's change, the better way: `+10%`, `-5%`.
pub fn rule_change(rule: &Rule) -> String {
    format!("{}{}%", if rule.better == Better::Up { "+" } else { "-" }, number(rule.min_change_pct))
}

/// A split by `rule`: the strongest variant beside `control`, each arm with
/// at least its `min_sample` people; judged as [`judge`] judges a share.
pub fn judge_split(rule: &Rule, arms: &[Arm], control: &str) -> Judgement {
    let Some(base) = arms.iter().find(|a| a.variant == control) else {
        return (Verdict::Inconclusive, format!("no one was counted in the `{control}` variant"), false, None);
    };
    let way = if rule.better == Better::Up { 1.0 } else { -1.0 };
    let Some(best) = arms.iter().filter(|a| a.variant != control && !a.variant.is_empty()).max_by(|a, b| (a.value * way).partial_cmp(&(b.value * way)).unwrap_or(std::cmp::Ordering::Equal)) else {
        return (Verdict::Inconclusive, "no one was counted in any variant but control".into(), false, None);
    };
    let (verdict, why, worse, pct) = judge(rule, true, Reading { value: base.value, sample: base.sample }, Reading { value: best.value, sample: best.sample });
    (verdict, format!("{} beside {control}: {why}", best.variant), worse, pct)
}

// ---- Hypotheses ----

/// A hypothesis registered for `ticket` (from `item`), waiting for it to go live.
pub fn experiment(id: u32, sprint: u32, h: &Hypothesis, ticket: Option<TicketId>, item: Option<u32>, by: &str, now: u64) -> Experiment {
    Experiment {
        id,
        sprint,
        belief: h.belief.clone(),
        metric: h.metric.clone(),
        rule: h.rule.clone(),
        split: h.split.clone(),
        events: h.events.clone(),
        ticket,
        item,
        defined_by: by.to_owned(),
        defined_at: now,
        status: ExperimentStatus::Waiting,
        live_at: None,
        due_at: None,
        checked: false,
        result: None,
    }
}

/// Its ticket went live at `at`: the rule locked, its judging due after its window.
pub fn went_live(x: &mut Experiment, at: u64) {
    x.status = ExperimentStatus::Live;
    x.live_at = Some(at);
    x.due_at = Some(at + u64::from(x.rule.window_days) * DAY);
}

/// Live a day, and not yet looked whether its events arrive.
pub fn check_due(x: &Experiment, now: u64) -> bool {
    x.status == ExperimentStatus::Live && !x.checked && x.live_at.is_some_and(|at| at + DAY <= now) && x.due_at.is_some_and(|d| d > now)
}

/// Its window has passed: judge it.
pub fn judge_due(x: &Experiment, now: u64) -> bool {
    x.status == ExperimentStatus::Live && x.due_at.is_some_and(|d| d <= now)
}

/// A hypothesis in a line, for the PM and the person.
pub fn experiment_line(x: &Experiment) -> String {
    let rule = format!("rule: {} in {} days, at least {}", rule_change(&x.rule), x.rule.window_days, x.rule.min_sample);
    let split = x.split.as_ref().map(|s| format!(", A/B on {}", s.flag)).unwrap_or_default();
    let state = match (&x.status, &x.result) {
        (_, Some(r)) => format!("{} — {}", r.verdict.as_str(), r.why),
        (ExperimentStatus::Waiting, _) => "not live yet".into(),
        (ExperimentStatus::Live, _) => format!("measuring until {}", x.due_at.map(|d| crate::metrics::day_string((d / DAY) as i64)).unwrap_or_default()),
        (ExperimentStatus::Dropped, _) => "dropped".into(),
        (ExperimentStatus::Judged, None) => "judged".into(),
    };
    let numbers = x.result.as_ref().map(result_numbers).filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default();
    format!("H-{} \"{}\" · {} · {rule}{split}: {state}{numbers}", x.id, x.belief, x.metric.label())
}

/// A result's numbers in a few words: `18.2% → 21.0% of 640 people`.
pub fn result_numbers(r: &equipe_protocol::sprint::Judged) -> String {
    match (&r.before, &r.after) {
        (Some(b), Some(a)) => format!("{} → {}, {} counted", number(growth::round1(b.value)), number(growth::round1(a.value)), number(b.sample.min(a.sample))),
        _ => r.arms.iter().map(|a| format!("{} {} of {}", a.variant, number(growth::round1(a.value)), number(a.sample))).collect::<Vec<_>>().join(" · "),
    }
}

// ---- Tickets ----

/// A started item's ticket: its title and body — with the hypothesis it
/// tests, the events it must send and the rule it is judged by.
pub fn ticket_draft(item: &Item, x: Option<&Experiment>) -> (String, String) {
    let title = format!("S-{} P-{}: {}", item.sprint, item.id, item.title);
    let mut body = format!("{}\n\nWhy: {}\nExpected: {} (impact {:?}, size {:?}, risk {:?}).", item.change, item.why, item.expected, item.impact, item.size, item.risk);
    if !item.evidence.is_empty() {
        body.push_str(&format!("\n\nIt stands on:\n{}", item.evidence.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n")));
    }
    if let Some(x) = x {
        body.push_str(&format!(
            "\n\nHypothesis H-{}: {}\nMeasured by {}; judged {} days after it is live: worked if it moves {} or more the better way, with at least {} counted before and after.",
            x.id,
            x.belief,
            x.metric.label(),
            x.rule.window_days,
            rule_change(&x.rule),
            x.rule.min_sample
        ));
        if let Some(s) = &x.split {
            body.push_str(&format!(" An A/B split on the feature flag `{}` (control: `{}`): every event of the metric carries `$feature/{}`.", s.flag, s.control, s.flag));
        }
        let events: Vec<String> = x.events.iter().cloned().chain(metric_events(&x.metric)).fold(Vec::new(), |mut all, e| {
            if !all.contains(&e) {
                all.push(e);
            }
            all
        });
        body.push_str(&format!("\nInstrument it before it ships: the product sends {} (equipe's own traffic marked `equipe_agent: true`). The rule may be refined with `{DEFINE}` until then.", events.iter().map(|e| format!("`{e}`")).collect::<Vec<_>>().join(", ")));
    }
    if item.outside {
        body.push_str("\n\nWhatever it publishes, posts, emails or spends waits for the person's yes on that exact thing: draft it, then ask.");
    }
    (title, body)
}

/// The events a metric is counted from.
pub fn metric_events(m: &Metric) -> Vec<String> {
    match m {
        Metric::Event { event, .. } => vec![event.clone()],
        Metric::Conversion { from, to, .. } => vec![from.clone(), to.clone()],
    }
}

// ---- The review ----

/// What a sprint's facts count, from the retro's gathering of its days and
/// its tickets on the board.
pub fn facts(sprint: &Sprint, board: &[Ticket], week: &crate::retro::Facts, experiments: &[Experiment], tokens: u64) -> Facts {
    let mine: Vec<&Ticket> = board.iter().filter(|t| sprint.tickets.contains(&t.id)).collect();
    let judged: Vec<Verdict> = experiments.iter().filter(|x| sprint.experiments.contains(&x.id)).filter_map(|x| x.result.as_ref().map(|r| r.verdict)).collect();
    let count = |v: Verdict| judged.iter().filter(|j| **j == v).count() as u32;
    Facts {
        tickets: mine.len() as u32,
        done: mine.iter().filter(|t| t.status == TicketStatus::Done).count() as u32,
        shipped: week.shipped.len() as u32,
        troubles: week.troubles.len() as u32,
        waits: week.waits.len() as u32,
        waited_secs: week.waits.iter().map(|w| w.secs(week.to)).sum(),
        worked: count(Verdict::Worked),
        did_not: count(Verdict::DidNot),
        inconclusive: count(Verdict::Inconclusive),
        tokens,
    }
}

/// What a review hands the PM.
#[derive(Debug, Clone, Default)]
pub struct Review<'a> {
    pub sprint: Option<&'a Sprint>,
    pub from: String,
    pub to: String,
    /// Its tickets: `T-12 ✓ Pricing page`.
    pub tickets: Vec<String>,
    /// Its hypotheses and any others judged or measuring.
    pub experiments: Vec<String>,
    /// Hypotheses judged before, what didn't work first: not to repeat.
    pub tried: Vec<String>,
    pub north_star: Option<&'a NorthStar>,
    pub facts: Option<&'a Facts>,
    pub budget: u64,
    /// What rubbed, a line each (the retro's troubles), and each member's friction.
    pub troubles: Vec<String>,
    pub friction: Vec<String>,
    pub evidence: &'a [Evidence],
    pub gaps: Vec<String>,
    pub market: Option<Market>,
    pub marketer: bool,
    pub measurable: bool,
    /// Items the person left for later: offered again.
    pub later: Vec<String>,
    /// The product's numbers come from the equipe platform (a cloud team).
    pub platform: bool,
    /// Plain lines about the product beside its numbers (top sources, experiments running).
    pub notes: Vec<String>,
}

/// What the PM does to measure with the equipe platform: A/B tests are its
/// experiments, read back by key.
const PLATFORM_HOW: &str = "The product's numbers come from the equipe platform. For a hypothesis worth an A/B test set `split.flag` to a platform experiment key; its engineer creates it with the platform tool `create_experiment` (its `goal_event` is the conversion) and uses `equipe.variant(key)` and `equipe.track(event)` in the product. A hypothesis's events are sent with `equipe.track`.";

fn market_words(m: Option<Market>) -> &'static str {
    match m {
        Some(Market::Consumers) => "sold to people (B2C)",
        Some(Market::Businesses) => "sold to businesses (B2B)",
        Some(Market::Internal) => "not sold (an internal tool)",
        None => "not said yet",
    }
}

fn north_star_line(n: &NorthStar) -> String {
    let value = match (n.value, n.previous) {
        (Some(v), Some(p)) => format!(": {} (before: {})", number(growth::round1(v)), number(growth::round1(p))),
        (Some(v), None) => format!(": {}", number(growth::round1(v))),
        _ => String::new(),
    };
    let metric = n.metric.as_ref().map(|m| format!(" · {}", m.label())).unwrap_or_else(|| " · not measured yet".into());
    format!("North star: {}{metric}{value}", n.name)
}

/// The shared part of a review's and a planning's message: the product's
/// numbers, the hypotheses tried, go-to-market, what isn't measured.
fn context_parts(r: &Review<'_>, out: &mut String) {
    if let Some(n) = r.north_star {
        out.push_str(&format!("\n{}", north_star_line(n)));
    }
    if !r.evidence.is_empty() {
        out.push_str("\n\nProduct numbers (cite by id):");
        for e in r.evidence {
            out.push_str(&format!("\n- {} · `{}` · {}", e.id, e.metric, growth::evidence_line(e)));
        }
    }
    if !r.notes.is_empty() {
        for n in &r.notes {
            out.push_str(&format!("\n{n}"));
        }
    }
    if r.platform {
        out.push_str(&format!("\n\n{PLATFORM_HOW}"));
    }
    if !r.tried.is_empty() {
        out.push_str("\n\nTried before (don't repeat what didn't work without a new reason):");
        for t in &r.tried {
            out.push_str(&format!("\n- {t}"));
        }
    }
    if !r.later.is_empty() {
        out.push_str("\n\nLeft for later by the person (propose again only if still right):");
        for l in &r.later {
            out.push_str(&format!("\n- {l}"));
        }
    }
    out.push_str(&format!("\n\nGo-to-market: {}.", market_words(r.market)));
    match r.market {
        Some(m) if m.is_sold() && r.marketer => out.push_str(" The marketing member's work belongs in the plan: positioning, who it is for, channels, the launch, landing copy, a content calendar, posts — as items, and acquisition experiments with hypotheses. Their drafts are theirs to make; publishing, posting, emailing and spending always wait for the person's card."),
        Some(m) if m.is_sold() => out.push_str(" The team has no marketer: if growth needs one, propose adding the marketing role with `propose_team_change`."),
        None => out.push_str(" equipe asks the person on the sprint's card; give your guess as `market_guess`."),
        _ => {}
    }
    if !r.measurable {
        out.push_str("\nNo product analytics are connected, so no hypothesis can be judged: make measuring the product an item (kind `measure`: connect analytics, send the key events).");
    }
    if !r.gaps.is_empty() {
        out.push_str(&format!("\nNot read this time: {}.", r.gaps.join("; ")));
    }
}

const ASK: &str = "Call `plan_sprint` once with: the next sprint's goal tied to the north star, `done_when`, and at most 5 items. \
Each item says what it stands on (cite E…/H-…, or in `why` what users said), the change, the expected impact, size and risk; one whose \
effect can be measured carries a `hypothesis`: belief, metric, and the decision rule fixed now (the change it must make, the days, the \
fewest counted). Kill or fix what didn't work, do more of what worked, add what usage or feedback asks for. equipe ranks them and the \
person starts them with one tap: don't make tickets. Then tell the person in at most 5 lines what happened and that the next sprint \
waits for their tap.";

/// What the PM is handed at a sprint's end: its facts and results, the
/// product's numbers, and what to do in one call.
pub fn review_message(r: &Review<'_>) -> String {
    let Some(s) = r.sprint else { return planning_message(r) };
    let mut out = format!(
        "[Sprint review · S-{} · {} – {}]\nequipe counted this sprint itself; nothing here is guessed. Write its retro for the person in the \
same `plan_sprint` call (`retro`: shipped, learned, wrong, next — at most 3 short lines each, plain words a non-developer reads in half \
a minute, no jargon or ticket numbers). {ASK}\n\nGoal: {}\nDone when: {}",
        s.id, r.from, r.to, s.goal, s.done_when
    );
    if let Some(f) = r.facts {
        out.push_str(&format!(
            "\nTickets: {} of {} done · shipped {} · rubbed {} · waited on the person {}× ({}) · used {} of {} tokens",
            f.done,
            f.tickets,
            f.shipped,
            f.troubles,
            f.waits,
            crate::retro::span(f.waited_secs),
            tokens(f.tokens),
            tokens(r.budget)
        ));
    }
    for t in &r.tickets {
        out.push_str(&format!("\n- {t}"));
    }
    if !r.experiments.is_empty() {
        out.push_str("\n\nHypotheses:");
        for x in &r.experiments {
            out.push_str(&format!("\n- {x}"));
        }
    }
    if !r.troubles.is_empty() || !r.friction.is_empty() {
        out.push_str("\n\nWhat rubbed (equipe's own flow, the team's inputs):");
        for t in r.troubles.iter().chain(&r.friction) {
            out.push_str(&format!("\n- {t}"));
        }
    }
    context_parts(r, &mut out);
    out
}

/// What the PM is handed to plan a sprint when none ran before (or the
/// person said not now): the numbers, and what to do.
pub fn planning_message(r: &Review<'_>) -> String {
    let mut out = format!(
        "[Sprint planning · {}]\nThe product is live, so the squad now works on it in sprints: each with a goal, items the person starts \
with one tap, hypotheses judged by rules fixed before they ship, and a short retro at the end. Name the `north_star` once if it isn't \
set. {ASK}",
        r.to
    );
    context_parts(r, &mut out);
    out
}

/// The sprint in a few lines for the PM's turns — said once per
/// conversation, then only as it changes (`turn_context`).
pub fn context(store: &Store, done: u32, now: u64) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(s) = store.current() {
        let left = s.ends_at.map(|e| if e > now { format!("review in {}", crate::retro::span(e - now)) } else { "in review".into() }).unwrap_or_default();
        lines.push(format!("S-{} ({left}): {}", s.id, s.goal));
        lines.push(format!("Done when: {} · {} of {} tickets done · {} of {} tokens", s.done_when, done, s.tickets.len(), tokens(s.spent_tokens), tokens(s.budget_tokens)));
        for x in store.experiments.iter().filter(|x| s.experiments.contains(&x.id) && x.status != ExperimentStatus::Dropped) {
            let state = match (&x.status, &x.result) {
                (_, Some(r)) => r.verdict.as_str().to_owned(),
                (ExperimentStatus::Waiting, _) => "not live".into(),
                _ => "measuring".into(),
            };
            lines.push(format!("H-{} {}: {state}", x.id, x.belief));
        }
    }
    if let Some(p) = store.proposed() {
        lines.push(format!("S-{} proposed, waiting on the person: {}", p.id, p.goal));
    }
    (!lines.is_empty()).then(|| format!("The squad's sprint (docs: `plan_sprint`):\n{}", lines.join("\n")))
}

/// Hypotheses judged, those that didn't work first, at most [`TRIED_SHOWN`].
pub fn tried(experiments: &[Experiment]) -> Vec<String> {
    let mut judged: Vec<&Experiment> = experiments.iter().filter(|x| x.result.is_some()).collect();
    judged.sort_by_key(|x| (x.result.as_ref().map(|r| r.verdict != Verdict::DidNot), std::cmp::Reverse(x.id)));
    judged.into_iter().take(TRIED_SHOWN).map(experiment_line).collect()
}

/// How many of a sprint's tickets are done on `board`.
pub fn done(sprint: &Sprint, board: &[Ticket]) -> u32 {
    sprint.tickets.iter().filter(|t| board.iter().any(|tk| tk.id == **t && tk.status == TicketStatus::Done)).count() as u32
}

/// A ticket of the sprint in a line: done, cancelled, or where it is.
pub fn ticket_line(t: &Ticket) -> String {
    match t.status {
        TicketStatus::Done => format!("T-{} ✓ {}", t.id, t.title),
        TicketStatus::Cancelled => format!("T-{} ✗ {}", t.id, t.title),
        other => format!("T-{} … {} ({})", t.id, t.title, other.label()),
    }
}

/// A budget level reached for the first time: `spent` of `budget` crossed
/// one of [`BUDGET_LEVELS`] above `told`.
pub fn budget_step(told: u8, spent: u64, budget: u64) -> Option<u8> {
    let used = spent.saturating_mul(100).checked_div(budget).unwrap_or(0).min(255) as u8;
    BUDGET_LEVELS.iter().rev().find(|l| used >= **l && **l > told).copied()
}

#[cfg(test)]
mod tests;
