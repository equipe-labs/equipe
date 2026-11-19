//! The growth loop (docs/GROWTH.md, `equipe_protocol::growth`): once a week,
//! or when asked (`equipe growth run`), equipe reads a squad's product data
//! itself — its PostHog connection through read-only queries
//! (`crate::metrics::hogql`), the numbers it keeps, its signals, the health
//! checks a project declares — and hands what it counted, each number with
//! its query and days ([`Evidence`]), to the PM in one message. The PM turns
//! it into at most [`CARDS`] recommendation cards (`propose_growth`); a card
//! without numbers isn't made ([`cards`]). The person answers each — accept,
//! later, no — on one decision card; an accepted one becomes a ticket serving
//! a request of its own, a "no" isn't proposed again for [`DECLINED_FOR`].
//! Once its ticket is live, the card's metric is read again after 7 and 14
//! days and judged worked, no change or worse ([`classify`]); worse brings a
//! card to undo it ([`revert_card`]).
//!
//! A project says what matters in [`FILE`] (`.equipe/growth.json`): key
//! events, funnels, audiences, the traffic to leave out, health checks, the
//! fields a card may name, the loop's caps ([`Config`]).
//!
//! Plain functions, tested without a runtime; the runtime gathers, keeps and
//! asks (`runtime/growth.rs`).

use std::path::{Path, PathBuf};

use equipe_protocol::decide::{Answer, Choice, Question};
use equipe_protocol::deploys::Deploy;
use equipe_protocol::growth::{Better, Card, Evidence, FollowUp, Measure, Outcome, Risk, Size, Status, Verdict};
use equipe_protocol::{Ticket, TicketStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::Lang;
use crate::metrics::day_string;
use crate::routines::Clock;

/// The PM's tool.
pub const PROPOSE: &str = "propose_growth";
/// What a project declares, in its repository.
pub const FILE: &str = ".equipe/growth.json";
/// Where the squad's cards are kept, in its data folder.
pub const STORE: &str = "growth.json";

pub const DAY: u64 = 24 * 60 * 60;
pub const WEEK: u64 = 7 * DAY;
/// Cards open at once (proposed and not answered, or "later"), unless the
/// project says otherwise.
pub const CARDS: usize = 5;
/// The loop's tokens in any 7 days, by equipe's estimate, unless the project
/// says otherwise.
pub const WEEKLY_TOKENS: u64 = 150_000;
/// What the PM's one turn on a run is counted as, beside its message.
pub const TURN_ALLOWANCE: u64 = 20_000;
/// A "no" isn't proposed again for this long.
pub const DECLINED_FOR: u64 = 60 * DAY;
/// A "later" card is asked again with the next run's cards — until this old.
pub const LATER_KEPT: u64 = 30 * DAY;
/// A run's numbers may be cited on cards this long after it.
pub const EVIDENCE_FRESH: u64 = WEEK;
/// The days after going live a card's metric is read again.
pub const FOLLOW_UPS: [u32; 2] = [7, 14];
/// A change of less than this, either way, is no change (percent).
pub const THRESHOLD_PCT: f64 = 5.0;
/// Fewer people or events than this, before or after: too few to say.
pub const MIN_SAMPLE: f64 = 20.0;
/// The weekly run: Mondays from 09:00, the daemon machine's local time.
pub const RUN_WEEKDAY: u8 = 1;
pub const RUN_HOUR: u8 = 9;
/// The period a run reads, beside the one before it (days).
pub const PERIOD: i64 = 7;
/// Queries one run sends at most.
pub const QUERIES: usize = 8;
/// Evidence one run hands over at most.
pub const EVIDENCE: usize = 24;
/// Steps of a funnel read, at most.
const STEPS: usize = 5;
const TITLE_CHARS: usize = 120;
const CHANGE_CHARS: usize = 600;

/// `propose_growth`, the PM's.
pub fn tool() -> Value {
    let text = |what: &str| serde_json::json!({ "type": "string", "description": what });
    serde_json::json!({
        "name": PROPOSE,
        "description": "Put growth recommendation cards to the person, after equipe hands you a growth run's numbers \
(\"[Growth run · …]\"). One call with all the cards, at most as many as the run says. Each card stands on the run's \
evidence, cited by id (E1…) — a card without numbers isn't made — and says the change, the metric it should move (one \
of its evidence's), the effect you expect on it, its size and risk. Counts only: never name a person. The person \
answers accept, later or no; equipe makes the accepted ones' tickets and measures them 7 and 14 days after they go live.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "cards": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": text("The recommendation in a few words."),
                            "change": text("The proposed change, in a sentence or two."),
                            "evidence": { "type": "array", "items": { "type": "string" }, "description": "The run's evidence ids it stands on: E1, E3." },
                            "metric": text("The metric it should move: one of its evidence's `metric`s (default: the first's)."),
                            "expected": text("The effect expected on that metric: \"+5 points of conversion\"."),
                            "size": { "type": "string", "enum": ["S", "M", "L"] },
                            "risk": { "type": "string", "enum": ["low", "medium", "high"] },
                            "project": text("The project it changes, when the squad has several."),
                        },
                        "required": ["title", "change", "evidence", "expected", "size", "risk"],
                    },
                },
            },
            "required": ["cards"],
        },
    })
}

// ---- What a project declares ----

/// `.equipe/growth.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Events counted each run, this week beside last: `question_sent`.
    pub key_events: Vec<String>,
    /// The funnels that matter: `"question_sent → answer_received"`, or
    /// `{ "name", "steps": [...], "windowDays" }`.
    pub funnels: Vec<Funnel>,
    /// Audiences the first funnel is also read for: `{ "name", "where": "$browser_language~ko" }`.
    pub audiences: Vec<Audience>,
    /// Traffic left out of every query: `prop=value`, `prop!=value`, `prop~text`.
    pub exclude: Vec<String>,
    /// Leave out what equipe's own browsers do ([`EQUIPE_TRAFFIC`]); on unless false.
    pub exclude_equipe: bool,
    /// Health checks: `{ "name", "url" }` (an https GET) or `{ "name", "hogql" }` (one number).
    pub health: Vec<Health>,
    /// What a card may name that otherwise refuses it: `email`, `phone` —
    /// none by default: cards carry counts only.
    pub allowed_fields: Vec<String>,
    /// The PostHog connection to read (`m1`); the squad's only one otherwise.
    pub connection: Option<String>,
    pub max_cards: Option<usize>,
    pub weekly_tokens: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            key_events: Vec::new(),
            funnels: Vec::new(),
            audiences: Vec::new(),
            exclude: Vec::new(),
            exclude_equipe: true,
            health: Vec::new(),
            allowed_fields: Vec::new(),
            connection: None,
            max_cards: None,
            weekly_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "FunnelIn")]
pub struct Funnel {
    pub name: String,
    pub steps: Vec<String>,
    pub window_days: u32,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FunnelIn {
    Text(String),
    Full {
        #[serde(default)]
        name: Option<String>,
        steps: Vec<String>,
        #[serde(default, rename = "windowDays")]
        window_days: Option<u32>,
    },
}

impl From<FunnelIn> for Funnel {
    fn from(f: FunnelIn) -> Self {
        let (name, steps, window) = match f {
            FunnelIn::Text(text) => (None, text.split(['→', '>']).map(|s| s.trim().trim_end_matches('-').trim().to_owned()).collect(), None),
            FunnelIn::Full { name, steps, window_days } => (name, steps, window_days),
        };
        let steps: Vec<String> = steps.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).take(STEPS).collect();
        Funnel { name: name.unwrap_or_else(|| steps.join(" → ")), steps, window_days: window.unwrap_or(1).clamp(1, 30) }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Audience {
    pub name: String,
    #[serde(rename = "where")]
    pub condition: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Health {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub hogql: Option<String>,
    /// For a query: whether more is better (`up`, the default) or less (`down`).
    #[serde(default)]
    pub better: Better,
}

/// The project's declaration, when it has one; `Err` when it can't be read.
pub fn config(project: &Path) -> Result<Option<Config>, String> {
    let path = project.join(FILE);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    serde_json::from_str(&text).map(Some).map_err(|e| format!("{FILE} isn't readable: {e}"))
}

impl Config {
    pub fn cards(&self) -> usize {
        self.max_cards.unwrap_or(CARDS).clamp(1, 10)
    }

    /// Whether cards may name what [`personal`] found: `email` or `phone`
    /// listed in `allowedFields`.
    pub fn allows(&self, what: &str) -> bool {
        let field = if what.contains("email") { "email" } else { "phone" };
        self.allowed_fields.iter().any(|f| f.trim().trim_start_matches('$').eq_ignore_ascii_case(field))
    }

    pub fn weekly_tokens(&self) -> u64 {
        self.weekly_tokens.unwrap_or(WEEKLY_TOKENS)
    }

    /// Several projects' declarations as one: lists joined, the first cap said kept.
    pub fn merged(all: impl IntoIterator<Item = Config>) -> Config {
        let mut out = Config::default();
        for c in all {
            out.key_events.extend(c.key_events.into_iter().filter(|e| !out.key_events.contains(e)).collect::<Vec<_>>());
            out.funnels.extend(c.funnels);
            out.audiences.extend(c.audiences);
            out.exclude.extend(c.exclude);
            out.exclude_equipe &= c.exclude_equipe;
            out.health.extend(c.health);
            out.allowed_fields.extend(c.allowed_fields);
            out.connection = out.connection.or(c.connection);
            out.max_cards = out.max_cards.or(c.max_cards);
            out.weekly_tokens = out.weekly_tokens.or(c.weekly_tokens);
        }
        out
    }
}

// ---- Leaving traffic out ----

/// What equipe's own browsers are known by, left out unless a project says
/// `"excludeEquipe": false` (docs/GROWTH.md, "Traffic left out"): an event
/// flagged `internal` or `equipe_agent`, or a user agent that says equipe.
pub const EQUIPE_TRAFFIC: [&str; 3] = ["internal=true", "equipe_agent=true", "$raw_user_agent~equipe"];

/// An event property as HogQL names it.
fn property(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
        return Err(format!("`{name}` isn't an event property"));
    }
    Ok(format!("properties.{name}"))
}

/// A string as HogQL quotes it.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// One condition — `prop=value`, `prop!=value`, `prop~text` (contains, any
/// case) — as HogQL. A property an event lacks is the empty string.
pub fn condition(text: &str) -> Result<String, String> {
    let text = text.trim();
    let (name, op, value) = if let Some((n, v)) = text.split_once("!=") {
        (n, "!=", v)
    } else if let Some((n, v)) = text.split_once('~') {
        (n, "~", v)
    } else if let Some((n, v)) = text.split_once('=') {
        (n, "=", v)
    } else {
        return Err(format!("`{text}` isn't prop=value, prop!=value or prop~text"));
    };
    let field = format!("coalesce(toString({}), '')", property(name)?);
    let value = value.trim();
    Ok(match op {
        "~" => format!("positionCaseInsensitive({field}, {}) > 0", quote(value)),
        op => format!("{field} {op} {}", quote(value)),
    })
}

/// The `AND NOT (…)` every query of a run ends its `WHERE` with: the
/// project's exclusions, and equipe's own traffic unless it says otherwise.
/// Empty when nothing is left out.
pub fn exclusion(config: &Config) -> Result<String, String> {
    let equipe: &[&str] = if config.exclude_equipe { &EQUIPE_TRAFFIC } else { &[] };
    let all: Vec<String> = equipe.iter().map(|s| (*s).to_owned()).chain(config.exclude.iter().cloned()).map(|c| condition(&c)).collect::<Result<_, _>>()?;
    Ok(if all.is_empty() { String::new() } else { format!(" AND NOT ({})", all.join(" OR ")) })
}

// ---- What a run reads ----

/// Days `from..=to` (day numbers) as a `timestamp` range.
fn range(from: i64, to: i64) -> String {
    format!("timestamp >= toDateTime('{} 00:00:00') AND timestamp < toDateTime('{} 00:00:00')", day_string(from), day_string(to + 1))
}

/// An event's count over `from..=to`.
pub fn event_query(event: &str, from: i64, to: i64, exclude: &str) -> String {
    format!("SELECT count() AS n FROM events WHERE event = {} AND {}{exclude}", quote(event), range(from, to))
}

/// The people who did `a` in `from..=to`, and those of them who then did `b`
/// within `days` — `entered`, `converted`.
pub fn funnel_query(a: &str, b: &str, days: u32, from: i64, to: i64, exclude: &str, audience: &str) -> String {
    let first = format!("event = {} AND timestamp < toDateTime('{} 00:00:00')", quote(a), day_string(to + 1));
    format!(
        "SELECT count() AS entered, countIf(arrayExists(x -> x >= t1 AND x <= t1 + toIntervalDay({days}), seen)) AS converted FROM (\
         SELECT person_id, minIf(timestamp, {first}) AS t1, groupArrayIf(timestamp, event = {b}) AS seen FROM events \
         WHERE event IN ({qa}, {b}) AND {range}{exclude}{audience} GROUP BY person_id HAVING countIf({first}) > 0)",
        b = quote(b),
        qa = quote(a),
        range = range(from, to + i64::from(days)),
    )
}

/// The query that reads `measure` over `from..=to`; `None` for one read from
/// equipe's own snapshots.
pub fn measure_query(measure: &Measure, from: i64, to: i64) -> Option<String> {
    match measure {
        Measure::Event { event, exclude, .. } => Some(event_query(event, from, to, exclude)),
        Measure::Funnel { from: a, to: b, days, exclude, audience, .. } => Some(funnel_query(a, b, *days, from, to, exclude, audience)),
        Measure::Snapshot { .. } => None,
    }
}

/// A measure's value from its query's first row, and how many it counted
/// (`None`: the rows weren't what it asks).
pub fn read_value(measure: &Measure, rows: &[Vec<Value>]) -> Option<(f64, f64)> {
    let row = rows.first()?;
    let n = |i: usize| row.get(i).and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()));
    match measure {
        Measure::Event { .. } => n(0).map(|v| (v, v)),
        Measure::Funnel { .. } => {
            let (entered, converted) = (n(0)?, n(1)?);
            Some((if entered > 0.0 { round1(converted / entered * 100.0) } else { 0.0 }, entered))
        }
        Measure::Snapshot { .. } => None,
    }
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// One query a run sends: what it measures and how it is called.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub measure: Measure,
}

/// What a run asks PostHog `connection`, from the project's declaration: each
/// key event, each step of each funnel and the whole of it, the first funnel
/// for each audience — at most [`QUERIES`].
pub fn asks(config: &Config, connection: &str) -> Result<Vec<Ask>, String> {
    let exclude = exclusion(config)?;
    let mut out = Vec::new();
    for event in config.key_events.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        out.push(Ask {
            metric: format!("event:{event}"),
            label: format!("{event} events"),
            unit: "events".into(),
            measure: Measure::Event { connection: connection.into(), event: event.into(), exclude: exclude.clone() },
        });
    }
    let funnel = |a: &str, b: &str, days: u32, audience: Option<&Audience>| -> Result<Ask, String> {
        let (suffix, cond) = match audience {
            Some(au) => (format!(" ({})", au.name), format!(" AND {}", condition(&au.condition)?)),
            None => (String::new(), String::new()),
        };
        Ok(Ask {
            metric: format!("funnel:{a}>{b}{}", audience.map(|au| format!("@{}", au.name)).unwrap_or_default()),
            label: format!("{a} → {b} conversion within {days} day{}{suffix}", if days == 1 { "" } else { "s" }),
            unit: "%".into(),
            measure: Measure::Funnel { connection: connection.into(), from: a.into(), to: b.into(), days, exclude: exclude.clone(), audience: cond },
        })
    };
    for (n, f) in config.funnels.iter().enumerate() {
        if f.steps.len() < 2 {
            return Err(format!("the funnel \"{}\" needs two steps at least", f.name));
        }
        for pair in f.steps.windows(2) {
            out.push(funnel(&pair[0], &pair[1], f.window_days, None)?);
        }
        if f.steps.len() > 2 {
            out.push(funnel(&f.steps[0], &f.steps[f.steps.len() - 1], f.window_days, None)?);
        }
        if n == 0 {
            for au in &config.audiences {
                out.push(funnel(&f.steps[0], &f.steps[f.steps.len() - 1], f.window_days, Some(au))?);
            }
        }
    }
    out.truncate(QUERIES);
    Ok(out)
}

/// The period a run reads, and the one before: the last [`PERIOD`] whole
/// days before `today` (day numbers).
pub fn periods(today: i64) -> ((i64, i64), (i64, i64)) {
    let to = today - 1;
    let from = to - PERIOD + 1;
    ((from, to), (from - PERIOD, from - 1))
}

/// An ask's two readings as evidence numbered `n`.
pub fn evidence(n: usize, ask: &Ask, now: (f64, f64), before: Option<(f64, f64)>, period: (i64, i64), source: &str) -> Evidence {
    let query = measure_query(&ask.measure, period.0, period.1).unwrap_or_default();
    let sample = match &ask.measure {
        Measure::Funnel { .. } => format!(" of {} people", now.1),
        _ => String::new(),
    };
    Evidence {
        id: format!("E{n}"),
        metric: ask.metric.clone(),
        label: format!("{}{sample}", ask.label),
        value: now.0,
        previous: before.map(|b| b.0),
        unit: ask.unit.clone(),
        from: day_string(period.0),
        to: day_string(period.1),
        query,
        source: source.into(),
        better: Better::Up,
        measure: Some(ask.measure.clone()),
    }
}

// ---- Cards ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declined {
    pub card: u32,
    pub metric: String,
    pub title: String,
    pub until: u64,
}

/// One run: when, what it was counted as, what it found, who asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub at: u64,
    pub tokens: u64,
    pub evidence: usize,
    /// `weekly` or `asked`.
    pub by: String,
}

/// A squad's loop, as kept ([`STORE`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub next: u32,
    #[serde(default)]
    pub cards: Vec<Card>,
    #[serde(default)]
    pub declined: Vec<Declined>,
    #[serde(default)]
    pub runs: Vec<Run>,
    /// The latest run's evidence: what cards may cite.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub last_run: Option<u64>,
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
    pub fn open(&self) -> usize {
        self.cards.iter().filter(|c| c.status.is_open()).count()
    }

    /// Tokens the loop was counted in the 7 days before `now`.
    pub fn spent(&self, now: u64) -> u64 {
        self.runs.iter().filter(|r| r.at + WEEK > now).map(|r| r.tokens).sum()
    }

    /// The card numbered `id`.
    pub fn card(&mut self, id: u32) -> Option<&mut Card> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    /// Whether the person said no to something like this lately: that card.
    fn declined_like(&self, metric: &str, title: &str, now: u64) -> Option<&Declined> {
        self.declined.iter().find(|d| d.until > now && d.metric == metric && alike(&d.title, title))
    }
}

/// What the loop is counted for a run handing over `message`: its words at
/// four characters a token, and the PM's turn.
pub fn estimate(message: &str) -> u64 {
    message.chars().count() as u64 / 4 + TURN_ALLOWANCE
}

/// Whether a run counted `estimate` fits the week's `cap`; why not.
pub fn may_spend(store: &Store, cap: u64, estimate: u64, now: u64) -> Result<(), String> {
    let spent = store.spent(now);
    if spent + estimate > cap {
        return Err(format!("the growth loop has used {spent} of its {cap} tokens this week; a run needs about {estimate}"));
    }
    Ok(())
}

/// Two titles about the same thing: most of their words shared.
fn alike(a: &str, b: &str) -> bool {
    let words = |s: &str| s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 2).map(str::to_owned).collect::<std::collections::BTreeSet<_>>();
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let shared = a.intersection(&b).count() as f64;
    shared / (a.len().max(b.len()) as f64) >= 0.6
}

/// Text that names a person: an email address or a phone number.
pub fn personal(text: &str) -> Option<&'static str> {
    let email = text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '.');
        w.split_once('@').is_some_and(|(user, host)| !user.is_empty() && host.contains('.') && !host.starts_with('.'))
    });
    if email {
        return Some("an email address");
    }
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 9 {
                return Some("a phone number");
            }
        } else if !matches!(c, ' ' | '-' | '(' | ')' | '+') {
            run = 0;
        }
    }
    None
}

fn size_of(v: Option<&Value>) -> Option<Size> {
    match v?.as_str()?.trim().to_ascii_uppercase().as_str() {
        "S" | "SMALL" => Some(Size::S),
        "M" | "MEDIUM" => Some(Size::M),
        "L" | "LARGE" => Some(Size::L),
        _ => None,
    }
}

fn risk_of(v: Option<&Value>) -> Option<Risk> {
    match v?.as_str()?.trim().to_ascii_lowercase().as_str() {
        "low" => Some(Risk::Low),
        "medium" => Some(Risk::Medium),
        "high" => Some(Risk::High),
        _ => None,
    }
}

/// `propose_growth`'s cards, checked: each cites the run's evidence by id
/// and has numbers — a card without them isn't made — names the metric it
/// should move (one of its evidence's), says the effect it expects, its size
/// and risk, names no one, and isn't one the person said no to lately or
/// already open. All or none: any card wrong refuses the call, each problem
/// said. Numbered from `store.next + 1`.
pub fn cards(args: &Value, store: &Store, config: &Config, projects: &[String], now: u64) -> Result<Vec<Card>, String> {
    let fresh = store.last_run.is_some_and(|at| at + EVIDENCE_FRESH > now) && !store.evidence.is_empty();
    if !fresh {
        return Err("There are no fresh numbers to propose from: cards stand on a growth run's evidence. Wait for the weekly run, or the person starts one (`equipe growth run`).".into());
    }
    let asked = args.get("cards").and_then(Value::as_array).ok_or("Give `cards`: a list, each with title, change, evidence (ids like E1), metric, expected, size and risk.")?;
    if asked.is_empty() {
        return Err("No cards given. If the numbers show nothing worth changing, say so in one line and propose none.".into());
    }
    let room = config.cards().saturating_sub(store.open());
    if asked.len() > room {
        return Err(format!(
            "{} cards, but only {room} more may be open (at most {} wait on the person at once). Propose the {room} strongest.",
            asked.len(),
            config.cards()
        ));
    }
    let mut problems = Vec::new();
    let mut out = Vec::new();
    for (n, c) in asked.iter().enumerate() {
        let n = n + 1;
        let text = |k: &str| c.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
        let (title, change, expected) = (text("title"), text("change"), text("expected"));
        if title.is_empty() || title.chars().count() > TITLE_CHARS {
            problems.push(format!("card {n}: a `title` of at most {TITLE_CHARS} characters"));
        }
        if change.is_empty() || change.chars().count() > CHANGE_CHARS {
            problems.push(format!("card {n}: the proposed `change`, in at most {CHANGE_CHARS} characters"));
        }
        if expected.is_empty() {
            problems.push(format!("card {n}: the `expected` effect on its metric"));
        }
        let ids: Vec<String> = match c.get("evidence") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_ascii_uppercase()).collect(),
            Some(Value::String(s)) => s.split([',', ' ']).map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect(),
            _ => Vec::new(),
        };
        let cited: Vec<Evidence> = ids.iter().filter_map(|id| store.evidence.iter().find(|e| &e.id == id)).filter(|e| e.value.is_finite()).cloned().collect();
        if ids.is_empty() {
            problems.push(format!("card {n}: no `evidence` — a card stands on the run's numbers (ids like E1); without numbers it isn't made"));
        } else if let Some(unknown) = ids.iter().find(|id| !store.evidence.iter().any(|e| &e.id == *id)) {
            problems.push(format!("card {n}: there is no evidence {unknown} in the latest run"));
        } else if cited.is_empty() {
            problems.push(format!("card {n}: its evidence has no numbers — without numbers it isn't made"));
        }
        let metric = c.get("metric").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty()).map(str::to_owned).or_else(|| cited.first().map(|e| e.metric.clone())).unwrap_or_default();
        if !cited.is_empty() && !cited.iter().any(|e| e.metric == metric) {
            problems.push(format!("card {n}: its `metric` ({metric}) is none of its evidence's — name the one it should move"));
        }
        let size = size_of(c.get("size"));
        let risk = risk_of(c.get("risk"));
        if size.is_none() {
            problems.push(format!("card {n}: a `size`: S, M or L"));
        }
        if risk.is_none() {
            problems.push(format!("card {n}: a `risk`: low, medium or high"));
        }
        if let Some(what) = [&title, &change, &expected].iter().find_map(|t| personal(t)).filter(|what| !config.allows(what)) {
            problems.push(format!("card {n}: it names {what} — cards carry counts only, never who"));
        }
        let project = c.get("project").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned);
        if let Some(p) = &project
            && !projects.contains(p)
        {
            problems.push(format!("card {n}: the squad has no project \"{p}\""));
        }
        if let Some(d) = store.declined_like(&metric, &title, now) {
            problems.push(format!("card {n}: the person said no to G-{} (\"{}\") — not again before {}", d.card, d.title, day_string((d.until / DAY) as i64)));
        }
        if store.cards.iter().any(|o| o.status.is_open() && o.metric == metric && alike(&o.title, &title)) || out.iter().any(|o: &Card| o.metric == metric && alike(&o.title, &title)) {
            problems.push(format!("card {n}: a card like it is already open"));
        }
        if let (Some(size), Some(risk)) = (size, risk)
            && !cited.is_empty()
        {
            out.push(Card {
                id: store.next + out.len() as u32 + 1,
                title,
                change,
                metric,
                expected,
                size,
                risk,
                evidence: cited,
                project,
                status: Status::Open,
                proposed_at: now,
                set: 0,
                question: 0,
                answered_at: None,
                ticket: None,
                request: None,
                live_at: None,
                follow_ups: Vec::new(),
                reverts: None,
            });
        }
    }
    if !problems.is_empty() {
        return Err(format!("No card was made — fix these and propose again:\n- {}", problems.join("\n- ")));
    }
    Ok(out)
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

fn with_unit(v: f64, unit: &str) -> String {
    if unit == "%" { format!("{}%", number(v)) } else { format!("{} {unit}", number(v)) }
}

/// One evidence in a line: what, how much, which days, the period before.
pub fn evidence_line(e: &Evidence) -> String {
    let before = e.previous.map(|p| format!("; before: {}", with_unit(p, &e.unit))).unwrap_or_default();
    format!("{}: {} ({} to {}{before})", e.label, with_unit(e.value, &e.unit), e.from, e.to)
}

/// A card as a question on the person's decision card: its title, the
/// change, its numbers, what it should do, its size and risk; accept, later or no.
pub fn question(card: &Card, lang: Lang) -> Question {
    let words = crate::i18n::core(lang);
    let evidence: Vec<String> = card.evidence.iter().map(evidence_line).collect();
    let size = match card.size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    };
    let risk = match card.risk {
        Risk::Low => words.get("growth.risk_low"),
        Risk::Medium => words.get("growth.risk_medium"),
        Risk::High => words.get("growth.risk_high"),
    };
    let text = words.fill(
        "growth.question",
        &[
            ("title", card.title.clone()),
            ("change", card.change.clone()),
            ("evidence", evidence.join("\n")),
            ("expected", card.expected.clone()),
            ("metric", card.metric.clone()),
            ("size", size.to_owned()),
            ("risk", risk.to_owned()),
        ],
    );
    let choice = |key: &str| Choice { label: words.get(key).to_owned(), ..Choice::default() };
    Question {
        question: text,
        header: words.fill("growth.header", &[("id", card.id.to_string())]),
        options: vec![choice("growth.accept"), choice("growth.later"), choice("growth.no")],
        ..Question::default()
    }
}

/// What the person's answer to a card's question says: accept, later or no
/// (skipped: later).
pub fn answered(answer: Option<&Answer>) -> Status {
    match answer.and_then(|a| a.picked.first()) {
        Some(0) => Status::Accepted,
        Some(2) => Status::Declined,
        _ => Status::Later,
    }
}

/// The person said no to `card`: remembered until [`DECLINED_FOR`] from `now`.
pub fn decline(store: &mut Store, card: &Card, now: u64) {
    store.declined.retain(|d| d.until > now);
    store.declined.push(Declined { card: card.id, metric: card.metric.clone(), title: card.title.clone(), until: now + DECLINED_FOR });
}

/// An accepted card's ticket: its title and body, for the PM to give an owner.
pub fn ticket_draft(card: &Card) -> (String, String) {
    let title = format!("Growth G-{}: {}", card.id, card.title);
    let evidence: Vec<String> = card.evidence.iter().map(|e| format!("- {} — `{}` ({})", evidence_line(e), e.query.replace('\n', " "), e.source)).collect();
    let body = format!(
        "{}\n\nExpected: {} on `{}`. Size {:?}, risk {:?}.\n\nEvidence:\n{}\n\nequipe reads `{}` again 7 and 14 days after this is live and reports whether it worked.",
        card.change,
        card.expected,
        card.metric,
        card.size,
        card.risk,
        evidence.join("\n"),
        card.metric
    );
    (title, body)
}

// ---- Live, and measured again ----

/// When `ticket` went live: the first deploy that brought it — or, for work
/// that lives in the squad's folder (no project), when it was done (`now`, the
/// first time it is seen done). `None`: not yet.
pub fn live_at(ticket: &Ticket, deploys: &[Deploy], now: u64) -> Option<u64> {
    deploys
        .iter()
        .filter(|d| d.tickets.contains(&ticket.id))
        .map(|d| d.at)
        .min()
        .or_else(|| (ticket.project.is_none() && ticket.status == TicketStatus::Done).then_some(now))
}

/// `card` went live at `at`: its follow-ups scheduled, 7 and 14 days on.
pub fn went_live(card: &mut Card, at: u64) {
    card.live_at = Some(at);
    card.status = Status::Live;
    card.follow_ups = FOLLOW_UPS.iter().map(|d| FollowUp { after_days: *d, due_at: at + u64::from(*d) * DAY, outcome: None }).collect();
}

/// The follow-ups of `card` due at `now` and not measured yet.
pub fn due(card: &Card, now: u64) -> Vec<u32> {
    if card.status != Status::Live {
        return Vec::new();
    }
    card.follow_ups.iter().filter(|f| f.outcome.is_none() && f.due_at <= now).map(|f| f.after_days).collect()
}

/// The days a follow-up `after_days` on compares, for a card live at
/// `live`: the same number of days before it went live, and from then on.
pub fn windows(live: u64, after_days: u32) -> ((i64, i64), (i64, i64)) {
    let first = (live / DAY) as i64;
    let n = i64::from(after_days);
    ((first - n, first - 1), (first, first + n - 1))
}

/// Worked, no change or worse: `after` beside `before`, by more than
/// [`THRESHOLD_PCT`] the way that is better; too few counted (under
/// [`MIN_SAMPLE`]) is no change. The change in percent, when there is one.
pub fn classify(before: f64, after: f64, sample: f64, better: Better) -> (Verdict, Option<f64>) {
    let pct = if before != 0.0 { Some(round1((after - before) / before.abs() * 100.0)) } else { None };
    if sample < MIN_SAMPLE {
        return (Verdict::NoChange, pct);
    }
    let moved = match pct {
        Some(p) if p >= THRESHOLD_PCT => 1,
        Some(p) if p <= -THRESHOLD_PCT => -1,
        Some(_) => 0,
        None if after > 0.0 => 1,
        None => 0,
    };
    let verdict = match (moved, better) {
        (0, _) => Verdict::NoChange,
        (1, Better::Up) | (-1, Better::Down) => Verdict::Worked,
        _ => Verdict::Worse,
    };
    (verdict, pct)
}

/// A follow-up's outcome, kept on its card; `true` when it was the last.
pub fn measured(card: &mut Card, after_days: u32, outcome: Outcome) -> bool {
    if let Some(f) = card.follow_ups.iter_mut().find(|f| f.after_days == after_days) {
        f.outcome = Some(outcome);
    }
    let all = card.follow_ups.iter().all(|f| f.outcome.is_some());
    if all {
        card.status = Status::Measured;
    }
    all
}

/// A follow-up in a line, for the person and the PM.
pub fn outcome_line(card: &Card, after_days: u32, o: &Outcome) -> String {
    if let Some(why) = &o.failed {
        return format!("G-{} \"{}\", {after_days} days live: couldn't be measured ({why}).", card.id, card.title);
    }
    let pct = o.change_pct.map(|p| format!(", {}{}%", if p >= 0.0 { "+" } else { "" }, number(p))).unwrap_or_default();
    format!(
        "G-{} \"{}\", {after_days} days live: {} — {} {} ({} to {}) beside {} ({} to {}){pct}.",
        card.id,
        card.title,
        o.verdict.as_str(),
        card.metric,
        number(o.after),
        o.after_from,
        o.after_to,
        number(o.before),
        o.before_from,
        o.before_to
    )
}

/// After a follow-up said worse: a card to undo `card`'s ticket, standing on
/// that follow-up's numbers.
pub fn revert_card(card: &Card, o: &Outcome, id: u32, now: u64) -> Card {
    let base = card.evidence.iter().find(|e| e.metric == card.metric).or(card.evidence.first());
    let unit = base.map_or_else(String::new, |e| e.unit.clone());
    let evidence = Evidence {
        id: "F1".into(),
        metric: card.metric.clone(),
        label: format!("{} after G-{} went live", base.map_or(card.metric.as_str(), |e| e.label.as_str()), card.id),
        value: o.after,
        previous: Some(o.before),
        unit,
        from: o.after_from.clone(),
        to: o.after_to.clone(),
        query: o.query.clone(),
        source: base.map_or_else(String::new, |e| e.source.clone()),
        better: base.map_or(Better::Up, |e| e.better),
        measure: base.and_then(|e| e.measure.clone()),
    };
    let ticket = card.ticket.map(|t| format!(" (T-{t})")).unwrap_or_default();
    Card {
        id,
        title: format!("Undo G-{}: {}", card.id, card.title),
        change: format!("Revert what G-{}{ticket} shipped, or try another way: since it went live, {} went the wrong way.", card.id, card.metric),
        metric: card.metric.clone(),
        expected: format!("{} back to about {}", card.metric, number(o.before)),
        size: Size::S,
        risk: Risk::Low,
        evidence: vec![evidence],
        project: card.project.clone(),
        status: Status::Open,
        proposed_at: now,
        set: 0,
        question: 0,
        answered_at: None,
        ticket: None,
        request: None,
        live_at: None,
        follow_ups: Vec::new(),
        reverts: Some(card.id),
    }
}

// ---- The weekly run ----

/// Whether the weekly run is due at `now`: Monday from [`RUN_HOUR`], and
/// none in the last six days.
pub fn weekly_due(last_run: Option<u64>, now: u64, clock: &impl Clock) -> bool {
    let p = clock.parts(now);
    p.weekday == RUN_WEEKDAY && p.hour >= RUN_HOUR && last_run.is_none_or(|at| at + 6 * DAY <= now)
}

/// What the PM is handed for a run: the evidence, each number with its days
/// and query; the cards still open; how to propose.
pub fn message(evidence: &[Evidence], open: &[&Card], gaps: &[String], cards: usize, today: &str) -> String {
    let mut out = format!(
        "[Growth run · {today}] equipe read the squad's product data itself (aggregate counts only; automated and equipe's own \
traffic left out). Turn what matters into at most {cards} recommendation cards with `propose_growth` — one call, all cards. \
Each card cites the evidence it stands on by id (E1…), names the metric it should move (an evidence's metric), the change, \
the effect you expect on that metric, its size (S/M/L) and risk (low/medium/high). A card without numbers isn't made; never \
name a person. If nothing here is worth changing, propose nothing and say so in one line. Don't make tickets yourself: the \
person accepts cards, and equipe makes the tickets and measures them after they go live.\n\nEvidence:\n"
    );
    for e in evidence {
        out.push_str(&format!("- {} · `{}` · {} — {}\n", e.id, e.metric, evidence_line(e), e.source));
        if !e.query.is_empty() {
            out.push_str(&format!("  query: {}\n", e.query.replace('\n', " ")));
        }
    }
    if !open.is_empty() {
        out.push_str("\nCards still waiting on the person (don't propose them again):\n");
        for c in open {
            out.push_str(&format!("- G-{} {} (`{}`)\n", c.id, c.title, c.metric));
        }
    }
    if !gaps.is_empty() {
        out.push_str(&format!("\nNot read this time: {}.\n", gaps.join("; ")));
    }
    out
}

/// What the PM is told once the person answered a set of cards.
pub fn answers_message(lines: &[String]) -> String {
    format!(
        "The person answered the growth cards:\n{}\nequipe made the tickets of the accepted ones (each serves a request of its \
own) and measures them 7 and 14 days after they go live. Don't make tickets for these cards yourself; give the new tickets \
an owner if they have none.",
        lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")
    )
}

/// A card's outcomes for `equipe growth list` and the PM: one line each.
pub fn card_line(card: &Card) -> String {
    let status = match card.status {
        Status::Open => "waiting on the person".to_owned(),
        Status::Later => "later".to_owned(),
        Status::Declined => "declined".to_owned(),
        Status::Accepted => card.ticket.map_or_else(|| "accepted".to_owned(), |t| format!("accepted · T-{t}")),
        Status::Live | Status::Measured => match card.latest() {
            Some(o) if o.failed.is_none() => format!("live · {}", o.verdict.as_str()),
            _ => "live · measuring".to_owned(),
        },
        Status::Dropped => "dropped".to_owned(),
    };
    format!("G-{} {} — {status}", card.id, card.title)
}

#[cfg(test)]
mod tests;
