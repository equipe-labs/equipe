//! When to move members between engines, and which — the rules, without a
//! clock or a daemon: what each engine's plan says, the time, the team, and
//! the squad's choice go in; what to tell the person, what to hold and which
//! members to move come out. The runtime (`runtime/engines.rs`) makes it so,
//! at ticket boundaries only. The numbers are docs/research/claude-codex-together.md §6.
//!
//! - **Watch** (a window ≥ 70%, or the burn so far runs it out before it
//!   starts over): told once; nothing moves.
//! - **Shift** (five hours ≥ 85% with more than 30 minutes left, a week ≥ 85%
//!   — or ≥ 75% with two days left and the burn running it out — or the
//!   engine's own warning): the implementation roles move, one batch.
//! - **Drain** (five hours ≥ 95%, a week ≥ 92%): every role that can move does,
//!   the PM last.
//! - **Exhausted** (100%, or refused): no new turns. A reset within the hour is
//!   waited for; one further off hands the work over.
//! - Five hours nearly gone with 30 minutes or less to its reset and a week
//!   under 75%: waited for rather than moved — a new engine reads everything
//!   again, which costs more than half an hour.
//! - The other engine must be no worse than Watch and have 30 points more room;
//!   when both run low, nothing moves: a lighter model or less thinking on the
//!   same engine, and no new tickets.
//! - Back when the original engine's five hours are under 50% and its week
//!   under 75% for ten minutes.
//! - A member stays at least an hour, moves at most once in three; batches
//!   are 15 minutes apart.
//! - A reading older than 30 minutes is a floor (usage only grows until a
//!   reset), so it may still move members away — never toward its engine,
//!   and back only once its limits have started over.

use std::collections::HashMap;

use equipe_protocol::engines::{Move, Switching, Why, Window};
use equipe_protocol::{Engine, EnginePlan, PlanWindow};

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;

pub const WATCH_AT: u8 = 70;
pub const SHIFT_AT: u8 = 85;
pub const DRAIN_FIVE_HOUR_AT: u8 = 95;
pub const DRAIN_WEEKLY_AT: u8 = 92;
pub const EXHAUSTED_AT: u8 = 100;
/// A week this used, with this long left and the burn running it out, shifts.
pub const LONG_WEEK_AT: u8 = 75;
pub const LONG_WEEK_LEFT: i64 = 48 * HOUR;
/// Five hours this close to starting over are waited for — while the week is under `WAIT_WEEKLY_BELOW`.
pub const WAIT_FOR_RESET: i64 = 30 * MINUTE;
pub const WAIT_WEEKLY_BELOW: u8 = 75;
/// An exhausted engine whose reset is this near is waited for; further, the work is handed over.
pub const HAND_OVER_AFTER: i64 = HOUR;
/// How much more room the engine moved to must have, in points.
pub const HEADROOM: u8 = 30;
pub const BACK_FIVE_HOUR_BELOW: u8 = 50;
pub const BACK_WEEKLY_BELOW: u8 = 75;
/// A stale week counts as recovered only below this.
pub const BACK_WEEKLY_STALE_BELOW: u8 = 65;
pub const BACK_AFTER: i64 = 10 * MINUTE;
pub const MIN_STAY: i64 = HOUR;
pub const COOLDOWN: i64 = 3 * HOUR;
pub const BATCH_GAP: i64 = 15 * MINUTE;
pub const STALE_AFTER: i64 = 30 * MINUTE;
/// The burn rate's half-life, in seconds.
const RATE_HALF_LIFE: f64 = 20. * 60.;

/// How an engine's plan stands, worst first last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Ok,
    Watch,
    Shift,
    Drain,
    Exhausted,
}

/// One engine's plan as read, with what the engine said beyond the numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub engine: Engine,
    pub five_hour: Option<PlanWindow>,
    pub weekly: Option<PlanWindow>,
    /// When it was read (unix seconds).
    pub at: i64,
    /// The engine warns the plan runs low (Claude's `allowed_warning`).
    pub warning: bool,
    /// The engine refuses turns until a reset.
    pub rejected: bool,
}

impl Reading {
    pub fn of(plan: &EnginePlan) -> Self {
        Reading { engine: plan.engine, five_hour: plan.usage.five_hour, weekly: plan.usage.seven_day, at: plan.at, warning: false, rejected: false }
    }

    fn windows(&self) -> impl Iterator<Item = (Window, PlanWindow)> {
        [(Window::FiveHour, self.five_hour), (Window::Weekly, self.weekly)].into_iter().filter_map(|(k, w)| Some((k, w?)))
    }

    /// Its windows that haven't started over since.
    fn live(&self, now: i64) -> impl Iterator<Item = (Window, PlanWindow)> {
        self.windows().filter(move |(_, w)| w.resets_at > now)
    }
}

/// Where an engine stands now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    pub engine: Engine,
    pub level: Level,
    /// The window that set the level, how used, when it starts over.
    pub window: Option<Window>,
    pub used: u8,
    pub resets_at: i64,
    /// The five-hour exception: wait for its reset rather than move.
    pub wait: bool,
    /// The reading is older than [`STALE_AFTER`]: a floor, not the usage now.
    pub stale: bool,
    /// When, at the burn so far, the window runs out — before it starts over.
    pub runs_out_at: Option<i64>,
    /// The most any live window is used: its room is 100 less.
    pub worst: u8,
}

impl Assessment {
    fn room(&self) -> u8 {
        100u8.saturating_sub(self.worst)
    }
}

/// Burn rate per window, from the readings so far.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rate {
    used: u8,
    at: i64,
    resets_at: i64,
    /// Points per second, smoothed.
    per_sec: Option<f64>,
}

/// What the rules remember between calls, per squad.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    /// When the last batch of moves was proposed or made.
    pub last_batch: Option<i64>,
    /// Since when each engine, fresh, has looked recovered.
    pub back_since: HashMap<Engine, i64>,
    /// What the person was told already: an engine's level, until its window starts over.
    pub told: Vec<(Engine, Level, i64)>,
    /// What the person turned down, until its window starts over.
    pub declined: Vec<(Engine, Why, i64)>,
    /// Engines stepped down already, until their window starts over.
    pub stepped: Vec<(Engine, i64)>,
    rates: HashMap<(Engine, Window), Rate>,
}

impl Memory {
    /// A new reading: the burn rate, and whether its engine looks recovered.
    pub fn observe(&mut self, r: &Reading) {
        for (kind, w) in r.windows() {
            let key = (r.engine, kind);
            let per_sec = match self.rates.get(&key) {
                Some(prev) if prev.resets_at == w.resets_at && r.at > prev.at => {
                    let dt = (r.at - prev.at) as f64;
                    let now = (f64::from(w.used_percent) - f64::from(prev.used)).max(0.) / dt;
                    let alpha = 1. - 0.5f64.powf(dt / RATE_HALF_LIFE);
                    Some(prev.per_sec.map_or(now, |old| old + alpha * (now - old)))
                }
                Some(prev) if prev.resets_at == w.resets_at => prev.per_sec,
                _ => None,
            };
            self.rates.insert(key, Rate { used: w.used_percent, at: r.at, resets_at: w.resets_at, per_sec });
        }
        if recovered(r, r.at) {
            self.back_since.entry(r.engine).or_insert(r.at);
        } else {
            self.back_since.remove(&r.engine);
        }
    }

    fn rate(&self, engine: Engine, kind: Window, w: PlanWindow) -> Option<f64> {
        self.rates.get(&(engine, kind)).filter(|r| r.resets_at == w.resets_at).and_then(|r| r.per_sec)
    }

    /// The person said no to moves off `engine` for `why`: not proposed again
    /// until its window starts over.
    pub fn decline(&mut self, engine: Engine, why: Why, resets_at: i64) {
        self.declined.push((engine, why, resets_at));
    }

    fn forget_past(&mut self, now: i64) {
        self.told.retain(|t| t.2 > now);
        self.declined.retain(|d| d.2 > now);
        self.stepped.retain(|s| s.1 > now);
    }
}

/// Whether a fresh reading's limits are low enough to come back to.
fn recovered(r: &Reading, now: i64) -> bool {
    let ok = |w: Option<PlanWindow>, below: u8| w.is_none_or(|w| w.used_percent < below || w.resets_at <= now);
    !r.rejected && !r.warning && ok(r.five_hour, BACK_FIVE_HOUR_BELOW) && ok(r.weekly, BACK_WEEKLY_BELOW)
}

/// Since when `engine` has looked fit to come back to, as far as `r` and
/// what was seen before show — `None`: not now. Fresh, as observed; stale,
/// only once its five hours started over (and its week was well below).
fn back_since(r: &Reading, memory: &Memory, now: i64) -> Option<i64> {
    if now - r.at <= STALE_AFTER {
        return recovered(r, now).then(|| memory.back_since.get(&r.engine).copied().unwrap_or(r.at));
    }
    let mut since = None;
    if let Some(w) = r.five_hour {
        if w.resets_at > now {
            return None;
        }
        since = Some(w.resets_at);
    }
    if let Some(w) = r.weekly {
        if w.resets_at <= now {
            since = Some(since.map_or(w.resets_at, |s: i64| s.max(w.resets_at)));
        } else if w.used_percent >= BACK_WEEKLY_STALE_BELOW {
            return None;
        }
    }
    if r.rejected { None } else { since }
}

/// One window's level, and when it runs out at `rate` (points a second).
fn window_level(kind: Window, w: PlanWindow, now: i64, rate: Option<f64>) -> (Level, Option<i64>) {
    let left = w.resets_at - now;
    let u = w.used_percent;
    let runs_out = rate.filter(|r| *r > 0.).map(|r| now + ((100. - f64::from(u)).max(0.) / r) as i64).filter(|at| *at < w.resets_at);
    let out = runs_out.is_some();
    let level = if u >= EXHAUSTED_AT {
        Level::Exhausted
    } else {
        match kind {
            Window::FiveHour if u >= DRAIN_FIVE_HOUR_AT => Level::Drain,
            Window::FiveHour if u >= SHIFT_AT && left > WAIT_FOR_RESET => Level::Shift,
            Window::Weekly if u >= DRAIN_WEEKLY_AT => Level::Drain,
            Window::Weekly if u >= SHIFT_AT || (u >= LONG_WEEK_AT && left > LONG_WEEK_LEFT && out) => Level::Shift,
            _ if u >= WATCH_AT || out => Level::Watch,
            _ => Level::Ok,
        }
    };
    (level, runs_out)
}

/// Where `r`'s engine stands at `now`.
pub fn assess(r: &Reading, memory: &Memory, now: i64) -> Assessment {
    let stale = now - r.at > STALE_AFTER;
    let mut out = Assessment { engine: r.engine, level: Level::Ok, window: None, used: 0, resets_at: 0, wait: false, stale, runs_out_at: None, worst: 0 };
    for (kind, w) in r.live(now) {
        // A stale reading's burn is unknown: no projection.
        let rate = if stale { None } else { memory.rate(r.engine, kind, w) };
        let (level, runs_out) = window_level(kind, w, now, rate);
        out.worst = out.worst.max(w.used_percent.min(100));
        if (level, w.used_percent) > (out.level, out.used) {
            (out.level, out.window, out.used, out.resets_at, out.runs_out_at) = (level, Some(kind), w.used_percent, w.resets_at, runs_out);
        }
    }
    // The engine's own word: refused is exhausted, a warning at least a shift.
    if r.rejected || (r.warning && out.level < Level::Shift) {
        if let Some((kind, w)) = r.live(now).max_by_key(|(_, w)| w.used_percent) {
            (out.window, out.used, out.resets_at) = (Some(kind), w.used_percent, w.resets_at);
        }
        out.level = if r.rejected { Level::Exhausted } else { Level::Shift };
    }
    // Five hours nearly gone, about to start over, with the week to spare: wait.
    if out.window == Some(Window::FiveHour) && out.level >= Level::Drain && out.resets_at - now <= WAIT_FOR_RESET {
        out.wait = r.weekly.is_none_or(|w| w.resets_at <= now || w.used_percent < WAIT_WEEKLY_BELOW);
    }
    out
}

/// A member, as the rules see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mate {
    pub name: String,
    pub role: String,
    pub engine: Engine,
    /// The engine it belongs on — as it joined, or as the setup put it — and its model there.
    pub home: Engine,
    pub home_model: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// It changes a project's files on its tickets.
    pub edits: bool,
    /// The person fixed its engine: never moved.
    pub pinned: bool,
    /// When it moved between engines, oldest first (unix seconds).
    pub switched: Vec<i64>,
    /// A move already waits for it.
    pub waiting: bool,
}

impl Mate {
    /// It may move now: not pinned, not waiting, an hour since it last moved and no move in three.
    fn may_move(&self, now: i64) -> bool {
        let last = self.switched.iter().max().copied();
        !self.pinned && !self.waiting && last.is_none_or(|t| now - t >= MIN_STAY.max(COOLDOWN))
    }
}

/// What each engine can take on now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    /// The engines the person is signed in to.
    pub ready: Vec<Engine>,
    /// Codex members can edit a project's files.
    pub codex_edits: bool,
    /// Codex members have the board's tools (what the PM works with).
    pub codex_board: bool,
}

impl Reach {
    pub fn takes(&self, engine: Engine, mate: &Mate) -> bool {
        self.ready.contains(&engine)
            && match engine {
                Engine::Claude => true,
                Engine::Codex => (!mate.edits || self.codex_edits) && (mate.role != crate::roles::PM || self.codex_board),
            }
    }
}

/// Which roles move first: those that spend the most and care least which
/// engine. Off Claude: backend, then mobile and frontend…; off Codex: data and
/// qa first. The PM always last. A role of the person's own sits before the designer.
pub fn rank(to: Engine, role: &str) -> usize {
    const TO_CODEX: [&str; 8] = ["backend", "mobile", "frontend", "data", "qa", "security", "designer", "pm"];
    const TO_CLAUDE: [&str; 8] = ["data", "qa", "security", "backend", "mobile", "frontend", "designer", "pm"];
    let order = if to == Engine::Codex { TO_CODEX } else { TO_CLAUDE };
    let designer = order.iter().position(|r| *r == "designer").unwrap_or(0);
    order.iter().position(|r| *r == role).map_or(designer * 2 - 1, |at| at * 2)
}

/// The roles a Shift moves: the implementation roles off Claude, the reviewers and data off Codex.
fn shifts(to: Engine, role: &str) -> bool {
    match to {
        Engine::Codex => matches!(role, "backend" | "mobile" | "frontend"),
        Engine::Claude => matches!(role, "data" | "qa" | "security"),
    }
}

/// One step lighter on the same engine: Claude's Opus to Sonnet, Codex's Astra
/// to Sol, else one notch less thinking. `None`: as light as it goes.
pub fn step_down(engine: Engine, model: Option<&str>, effort: Option<&str>) -> Option<(Option<String>, Option<String>)> {
    let model_down = match (engine, model) {
        (Engine::Claude, Some(m)) if m.contains("opus") => Some("sonnet"),
        (Engine::Codex, Some(m)) if m.contains("astra") => Some("gpt-6.1-sol"),
        _ => None,
    };
    if let Some(lighter) = model_down {
        return Some((Some(lighter.to_owned()), effort.map(str::to_owned)));
    }
    const LEVELS: [&str; 7] = ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
    let floor = if engine == Engine::Claude { 1 } else { 0 };
    let now = LEVELS.iter().position(|l| Some(*l) == effort).unwrap_or(2);
    (now > floor).then(|| (model.map(str::to_owned), Some(LEVELS[now - 1].to_owned())))
}

/// Something to tell the person once: an engine reached a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    pub engine: Engine,
    pub level: Level,
    pub window: Option<Window>,
    pub used: u8,
    pub resets_at: i64,
    pub runs_out_at: Option<i64>,
    /// Waited for rather than moved (the five-hour exception, or an exhausted engine's reset within the hour).
    pub wait: bool,
}

/// Moves to propose, or make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub why: Why,
    pub engine: Engine,
    pub window: Option<Window>,
    pub used: u8,
    pub resets_at: i64,
    pub moves: Vec<Move>,
}

/// No new tickets for members on `engine` until `until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hold {
    pub engine: Engine,
    pub until: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decision {
    pub assessments: Vec<Assessment>,
    pub notices: Vec<Notice>,
    pub batch: Option<Batch>,
    pub hold: Vec<Hold>,
    /// Engines whose usage should be read again before anything moves toward them.
    pub refresh: Vec<Engine>,
}

/// What the rules see of a squad.
pub struct Squad<'a> {
    pub readings: &'a [Reading],
    pub mates: &'a [Mate],
    pub reach: &'a Reach,
    pub mode: Switching,
    pub now: i64,
}

fn other(engine: Engine) -> Engine {
    match engine {
        Engine::Claude => Engine::Codex,
        Engine::Codex => Engine::Claude,
    }
}

/// What to do now. Readings are each engine's latest; `memory` the squad's,
/// kept between calls (each reading [`Memory::observe`]d as it came).
pub fn decide(squad: &Squad<'_>, memory: &mut Memory) -> Decision {
    let now = squad.now;
    memory.forget_past(now);
    let assessments: Vec<Assessment> = squad.readings.iter().map(|r| assess(r, memory, now)).collect();
    let mut out = Decision { assessments: assessments.clone(), ..Decision::default() };
    if squad.mode == Switching::Off {
        return out;
    }
    let of = |engine: Engine| assessments.iter().find(|a| a.engine == engine);

    // Told once per level and window.
    for a in assessments.iter().filter(|a| a.level >= Level::Watch) {
        let key = (a.engine, a.level, a.resets_at);
        if !memory.told.contains(&key) {
            memory.told.push(key);
            let wait = a.wait || (a.level == Level::Exhausted && a.resets_at - now <= HAND_OVER_AFTER);
            out.notices.push(Notice { engine: a.engine, level: a.level, window: a.window, used: a.used, resets_at: a.resets_at, runs_out_at: a.runs_out_at, wait });
        }
    }

    let gap_ok = memory.last_batch.is_none_or(|t| now - t >= BATCH_GAP);
    let mut worst: Vec<&Assessment> = assessments.iter().filter(|a| a.level >= Level::Shift).collect();
    worst.sort_by_key(|a| std::cmp::Reverse((a.level, a.used)));
    for source in worst {
        let exhausted = source.level == Level::Exhausted;
        // An exhausted engine takes no new turns; nor one waited for.
        if source.wait || (exhausted && source.resets_at - now <= HAND_OVER_AFTER) {
            out.hold.push(Hold { engine: source.engine, until: source.resets_at });
            continue;
        }
        if exhausted {
            out.hold.push(Hold { engine: source.engine, until: source.resets_at });
        }
        if out.batch.is_some() || !(gap_ok || exhausted) {
            continue;
        }
        let why = match source.level {
            Level::Shift => Why::Shift,
            Level::Drain => Why::Drain,
            _ => Why::Exhausted,
        };
        if memory.declined.contains(&(source.engine, why, source.resets_at)) {
            continue;
        }
        let to = other(source.engine);
        let target = of(to).filter(|t| !t.stale && squad.reach.ready.contains(&to));
        let Some(target) = target else {
            // Nothing known of where they'd go — or nothing current: read it first.
            if squad.reach.ready.contains(&to) {
                out.refresh.push(to);
            }
            continue;
        };
        let fits = target.level <= Level::Watch && target.room() >= 100u8.saturating_sub(source.used).saturating_add(HEADROOM);
        if !fits && target.level < Level::Shift {
            // Not enough room there to be worth the move: stay, and say so.
            continue;
        }
        if !fits {
            // Both run low: lighter on the same engine, and nothing new to start.
            out.hold.push(Hold { engine: source.engine, until: source.resets_at });
            if memory.stepped.contains(&(source.engine, source.resets_at)) {
                continue;
            }
            let moves: Vec<Move> = squad
                .mates
                .iter()
                .filter(|m| m.engine == source.engine && !m.pinned && !m.waiting)
                .filter_map(|m| {
                    let (model, effort) = step_down(m.engine, m.model.as_deref(), m.effort.as_deref())?;
                    Some(Move { member: m.name.clone(), role: m.role.clone(), from: m.engine, to: m.engine, model, effort, was_model: m.model.clone(), done: false })
                })
                .collect();
            if !moves.is_empty() && memory.declined.iter().all(|d| d != &(source.engine, Why::StepDown, source.resets_at)) {
                memory.stepped.push((source.engine, source.resets_at));
                out.batch = Some(Batch { why: Why::StepDown, engine: source.engine, window: source.window, used: source.used, resets_at: source.resets_at, moves });
            }
            continue;
        }
        let mut movers: Vec<&Mate> = squad
            .mates
            .iter()
            .filter(|m| m.engine == source.engine && m.may_move(now) && squad.reach.takes(to, m))
            .filter(|m| why != Why::Shift || shifts(to, &m.role))
            .collect();
        movers.sort_by_key(|m| (rank(to, &m.role), m.name.clone()));
        if movers.is_empty() {
            continue;
        }
        let moves = movers
            .into_iter()
            .map(|m| Move { member: m.name.clone(), role: m.role.clone(), from: m.engine, to, model: None, effort: None, was_model: m.model.clone(), done: false })
            .collect();
        memory.last_batch = Some(now);
        out.batch = Some(Batch { why, engine: source.engine, window: source.window, used: source.used, resets_at: source.resets_at, moves });
    }
    if out.batch.is_none() && gap_ok {
        out.batch = back_home(squad, memory);
        if out.batch.is_some() {
            memory.last_batch = Some(now);
        }
    }
    out
}

/// Members away from their engine go back once it has recovered for ten minutes.
fn back_home(squad: &Squad<'_>, memory: &Memory) -> Option<Batch> {
    let now = squad.now;
    for reading in squad.readings {
        let home = reading.engine;
        let Some(since) = back_since(reading, memory, now) else { continue };
        if now - since < BACK_AFTER {
            continue;
        }
        let mut movers: Vec<&Mate> = squad.mates.iter().filter(|m| m.home == home && m.engine != home && m.may_move(now) && squad.reach.takes(home, m)).collect();
        movers.sort_by_key(|m| (rank(home, &m.role), m.name.clone()));
        if movers.is_empty() {
            continue;
        }
        let a = assess(reading, memory, now);
        let moves = movers
            .into_iter()
            .map(|m| Move { member: m.name.clone(), role: m.role.clone(), from: m.engine, to: home, model: m.home_model.clone(), effort: None, was_model: m.model.clone(), done: false })
            .collect();
        let window = reading.five_hour.map(|_| Window::FiveHour).or(reading.weekly.map(|_| Window::Weekly));
        let used = reading.five_hour.or(reading.weekly).filter(|w| w.resets_at > now).map_or(0, |w| w.used_percent);
        return Some(Batch { why: Why::Return, engine: home, window, used, resets_at: a.resets_at.max(reading.five_hour.map_or(0, |w| w.resets_at)), moves });
    }
    None
}

// ── the setup for both engines ──

/// Where a role sits in the recommended setup: Claude writes, Codex reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub engine: Engine,
    pub model: &'static str,
    pub effort: &'static str,
}

/// The recommended seat of a built-in role (research §5.1, "Claude main"):
/// the PM, the designer and the hard implementation roles on Opus, frontend
/// on Sonnet, the reviewer and data on Codex's Sol. `None`: the person's own role.
pub fn seat(role: &str) -> Option<Seat> {
    let s = |engine, model| Some(Seat { engine, model, effort: "medium" });
    match role {
        "pm" | "designer" | "backend" | "mobile" => s(Engine::Claude, "opus"),
        "frontend" => s(Engine::Claude, "sonnet"),
        "qa" | "security" | "data" => s(Engine::Codex, "gpt-6.1-sol"),
        _ => None,
    }
}

/// Whether `model` is `wanted`'s family (`claude-opus-5-5` is `opus`).
fn same_model(model: Option<&str>, wanted: &str) -> bool {
    let tail = wanted.rsplit('-').next().unwrap_or(wanted);
    model.is_some_and(|m| m == wanted || m.contains(tail))
}

/// The moves that put the team in the recommended setup, as far as each
/// engine can take its member now.
pub fn setup(mates: &[Mate], reach: &Reach) -> Vec<Move> {
    let mut moves: Vec<Move> = mates
        .iter()
        .filter(|m| !m.pinned)
        .filter_map(|m| {
            let seat = seat(&m.role)?;
            if !reach.takes(seat.engine, m) || (m.engine == seat.engine && same_model(m.model.as_deref(), seat.model)) {
                return None;
            }
            Some(Move { member: m.name.clone(), role: m.role.clone(), from: m.engine, to: seat.engine, model: Some(seat.model.to_owned()), effort: Some(seat.effort.to_owned()), was_model: m.model.clone(), done: false })
        })
        .collect();
    moves.sort_by_key(|m| (rank(m.to, &m.role), m.member.clone()));
    moves
}

/// Whether to propose the setup: signed in to both, the whole team on one
/// engine, and the setup would put someone on the other.
pub fn wants_setup(mates: &[Mate], reach: &Reach) -> bool {
    let both = reach.ready.contains(&Engine::Claude) && reach.ready.contains(&Engine::Codex);
    let one = mates.first().is_some_and(|first| mates.iter().all(|m| m.engine == first.engine));
    both && one && setup(mates, reach).iter().any(|m| !m.in_place())
}

/// The model to ask Codex for: `hint` if the account has it, else the newest
/// of its family (`sol`), else the engine's own choice (`None`).
pub fn codex_model(hint: &str, catalog: &[String]) -> Option<String> {
    if catalog.iter().any(|m| m == hint) {
        return Some(hint.to_owned());
    }
    let family = hint.rsplit('-').next().unwrap_or(hint);
    catalog.iter().find(|m| m.ends_with(&format!("-{family}"))).cloned()
}

#[cfg(test)]
mod tests;
