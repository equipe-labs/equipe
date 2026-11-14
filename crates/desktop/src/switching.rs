//! Both engines' plans and members moving between them, as the window shows
//! them — how much of each plan is used, in one direction everywhere (the
//! meters at the sidebar's foot, the model picker's tabs, `/usage`, the title
//! bar's glance; Codex's own `/status` alone counts what is left, which its
//! tooltip says), each number labelled "used", warned from 80% and 95%, the
//! week with its even-pace mark — the card above the
//! composer (a proposal in `suggest`, what was done in `auto` with an undo,
//! the recommended setup once), the badge by each member — and the squad's
//! choice in settings. No UI types here, so the rules are tested without a
//! window. Only the person's click answers a card.

use equipe_core::protocol::engines::{Move, Proposal, Switching, Why, Window};
use equipe_core::protocol::lanes::{Parallel, ParallelMode};
use equipe_core::protocol::{Command, Engine, EnginePlan, PlanUsage, PlanWindow, Snapshot};
use equipe_core::t;

/// How long a reading stays sure: older, the meter says when it was read.
pub const STALE_AFTER: i64 = 30 * 60;

/// What the window knows of both plans, the squad's choice and its cards.
#[derive(Debug, Clone, Default)]
pub struct Engines {
    /// Each engine's plan as last read.
    pub plans: Vec<EnginePlan>,
    pub mode: Switching,
    proposals: Vec<Proposal>,
    /// Answered here, not yet closed by the daemon: hidden, so a second click can't send it twice.
    sent: Vec<u64>,
}

/// The person's answer to a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Move, Apply.
    Apply,
    /// Move now: at the end of each one's turn, a ticket in progress handed
    /// over — on a card whose moves are made, those still waiting.
    Now,
    /// Not this time, Later — or Confirm on moves already made.
    Leave,
    /// Undo.
    Undo,
}

impl Engines {
    /// The snapshot's plans, choice and cards replace everything.
    pub fn load(&mut self, snapshot: &Snapshot, now: i64) {
        self.plans = snapshot.plans.clone();
        // An older daemon says only Claude's.
        if self.plans.is_empty()
            && let Some(usage) = snapshot.plan
        {
            self.plans.push(EnginePlan { engine: Engine::Claude, usage, plan_type: None, at: now });
        }
        self.mode = snapshot.engine_switching;
        self.proposals = snapshot.engine_proposals.clone();
        self.sent.clear();
    }

    /// An engine's plan, just read.
    pub fn read(&mut self, engine: Engine, usage: PlanUsage, now: i64) {
        let plan_type = self.plan(engine).and_then(|p| p.plan_type.clone());
        self.plans.retain(|p| p.engine != engine);
        self.plans.push(EnginePlan { engine, usage, plan_type, at: now });
    }

    pub fn plan(&self, engine: Engine) -> Option<&EnginePlan> {
        self.plans.iter().find(|p| p.engine == engine)
    }

    /// A card shown, or shown again as its members move. Whether it is new here.
    pub fn propose(&mut self, proposal: Proposal) -> bool {
        match self.proposals.iter_mut().find(|p| p.id == proposal.id) {
            Some(shown) => {
                *shown = proposal;
                false
            }
            None => {
                self.proposals.push(proposal);
                true
            }
        }
    }

    pub fn close(&mut self, id: u64) {
        self.proposals.retain(|p| p.id != id);
        self.sent.retain(|s| *s != id);
    }

    /// What was sent into a dead link never came back: show it again.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
    }

    pub fn open(&self) -> impl Iterator<Item = &Proposal> {
        self.proposals.iter().filter(|p| !self.sent.contains(&p.id))
    }

    pub fn first(&self) -> Option<&Proposal> {
        self.open().next()
    }

    pub fn get(&self, id: u64) -> Option<&Proposal> {
        self.open().find(|p| p.id == id)
    }

    /// The person's answer to card `id`: the command to send, the card hidden
    /// at once. `None` when it no longer shows, or the answer isn't one of its.
    pub fn decide(&mut self, squad: &str, id: u64, answer: Answer) -> Option<Command> {
        let card = self.get(id)?;
        let squad = squad.to_owned();
        let command = match (answer, card.applied) {
            (Answer::Apply, false) => Command::ApplyEngineProposal { squad, id, now: false },
            (Answer::Now, _) => Command::ApplyEngineProposal { squad, id, now: true },
            (Answer::Undo, true) => Command::UndoEngineProposal { squad, id },
            (Answer::Leave, _) => Command::DismissEngineProposal { squad, id },
            _ => return None,
        };
        // A hurry leaves the card showing: its moves are on their way.
        if !(answer == Answer::Now && card.applied) {
            self.sent.push(id);
        }
        Some(command)
    }

    /// Both engines' meters: Claude, then Codex.
    pub fn meters(&self, now: i64) -> Vec<Meter> {
        [Engine::Claude, Engine::Codex]
            .into_iter()
            .map(|engine| {
                let plan = self.plan(engine);
                let gauges = plan
                    .map(|p| {
                        [(Window::FiveHour, p.usage.five_hour), (Window::Weekly, p.usage.seven_day)]
                            .into_iter()
                            .filter_map(|(window, w)| Some(gauge(window, w?, now)))
                            .collect()
                    })
                    .unwrap_or_default();
                Meter { engine, gauges, read_at: plan.map(|p| p.at), stale: plan.is_some_and(|p| now - p.at > STALE_AFTER) }
            })
            .collect()
    }
}

/// One engine's plan as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meter {
    pub engine: Engine,
    pub gauges: Vec<Gauge>,
    /// When it was read; `None`: never yet.
    pub read_at: Option<i64>,
    pub stale: bool,
}

/// Whether the sidebar shows the plans' use: once either plan was read —
/// calm too, so the person can pace it — or a switch waits on the person.
pub fn worth_showing(meters: &[Meter], waiting: bool) -> bool {
    waiting || meters.iter().any(|m| m.read_at.is_some())
}

/// One window of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauge {
    pub window: Window,
    /// Used, 0–100; a window past its reset starts over at 0.
    pub percent: u8,
    pub resets_at: i64,
    pub tone: Tone,
}

/// How a window's use reads, warned early: calm, near its limit (80%),
/// nearly out (95%), out (100%: "Limit reached"). Every place a plan's use
/// shows reads it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Calm,
    Near,
    Critical,
    Out,
}

pub fn tone(percent: u8) -> Tone {
    match percent {
        100.. => Tone::Out,
        95.. => Tone::Critical,
        80.. => Tone::Near,
        _ => Tone::Calm,
    }
}

fn gauge(window: Window, w: PlanWindow, now: i64) -> Gauge {
    let percent = if w.resets_at <= now { 0 } else { w.used_percent.min(100) };
    Gauge { window, percent, resets_at: w.resets_at, tone: tone(percent) }
}

/// How long a window runs, in seconds.
pub fn window_secs(window: Window) -> i64 {
    match window {
        Window::FiveHour => 5 * 3_600,
        Window::Weekly => 7 * 86_400,
    }
}

/// Where the window's use would be by now if it were spread evenly over it,
/// 0–1: the share of the window gone by. `None` when its reset isn't known,
/// or is past — the window started over, and when the new one ends isn't
/// known yet. A reset further off than the window is long reads as just started.
pub fn even_pace(window: Window, resets_at: i64, now: i64) -> Option<f32> {
    if resets_at <= 0 || resets_at <= now {
        return None;
    }
    let length = window_secs(window);
    let left = (resets_at - now).min(length);
    Some((length - left) as f32 / length as f32)
}

impl Gauge {
    /// What is left of it, 0–100: what Codex's own `/status` says.
    pub fn left(&self) -> u8 {
        100 - self.percent.min(100)
    }

    /// The even-pace mark a weekly bar wears (see [`even_pace`]); none on the five hours.
    pub fn pace(&self, now: i64) -> Option<f32> {
        (self.window == Window::Weekly).then(|| even_pace(self.window, self.resets_at, now)).flatten()
    }

    /// Used past the even-pace mark: going faster than even.
    pub fn ahead(&self, now: i64) -> bool {
        self.pace(now).is_some_and(|pace| f32::from(self.percent) > pace * 100.)
    }
}

/// Every engine that was read has run out of a window: no member can work
/// until the first of them comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllOut {
    /// The window each engine is held by is the week (not only the five hours).
    pub weekly: bool,
    /// When the first engine is usable again (unix seconds).
    pub resumes_at: i64,
}

/// Whether every engine in use is at its limit — the sidebar's one banner
/// over the per-engine lines. Engines never read are not in use; a reading
/// gone stale is not trusted to say "out", so it clears the banner.
pub fn all_out(meters: &[Meter], now: i64) -> Option<AllOut> {
    let read: Vec<&Meter> = meters.iter().filter(|m| m.read_at.is_some() && !m.gauges.is_empty()).collect();
    if read.is_empty() || read.iter().any(|m| m.stale) {
        return None;
    }
    let mut back: Vec<(i64, bool)> = Vec::new();
    for m in &read {
        // The engine is back once every window it is out of has reset.
        let held = m.gauges.iter().filter(|g| g.tone == Tone::Out && g.resets_at > now).max_by_key(|g| g.resets_at)?;
        back.push((held.resets_at, held.window == Window::Weekly));
    }
    let resumes_at = back.iter().map(|(at, _)| *at).min()?;
    Some(AllOut { weekly: back.iter().all(|(_, weekly)| *weekly), resumes_at })
}

/// The window used most of the plans read, with its engine — the one back
/// sooner when two are as used.
pub fn tightest(meters: &[Meter]) -> Option<(Engine, Gauge)> {
    meters.iter().flat_map(|m| m.gauges.iter().map(move |g| (m.engine, *g))).min_by_key(|(_, g)| (std::cmp::Reverse(g.percent), g.resets_at))
}

/// The title bar's glance at it, only while the sidebar — the meters with
/// it — is away: shown there, it would say the same twice.
pub fn header_glance(meters: &[Meter], sidebar_hidden: bool) -> Option<(Engine, Gauge)> {
    tightest(meters).filter(|_| sidebar_hidden)
}

/// "Claude 58% used"; out, "Claude · Limit reached".
pub fn glance(engine: Engine, g: &Gauge) -> String {
    match g.tone {
        Tone::Out => t!("engines.glance_out", engine = engine_name(engine)),
        _ => t!("engines.glance", engine = engine_name(engine), used = g.percent),
    }
}

/// A window's name in words: "5-hour", "week".
pub fn window_name(window: Window) -> &'static str {
    match window {
        Window::FiveHour => t!("usage.five_hour"),
        Window::Weekly => t!("usage.week"),
    }
}

/// What a meter says beside its bar: "42% used"; out, "Limit reached ·
/// resets in 2h" — the reset in sight, not only on the pointer.
pub fn used_short(g: &Gauge, now: i64) -> String {
    match g.tone {
        Tone::Out => t!("engines.out_resets", span = span(g.resets_at, now)),
        _ => t!("engines.used_short", used = g.percent),
    }
}

/// A window on the pointer: "5-hour 42% used · resets in 2h 10m (17:40)";
/// out, "Limit reached"; past its reset, refilled (the next reset isn't
/// known yet). A weekly one says what its even-pace mark is, and Codex's
/// that its own `/status` counts what is left instead.
pub fn used_words(engine: Engine, g: &Gauge, now: i64, when: impl Fn(i64) -> String) -> String {
    let window = window_name(g.window);
    let mut lines = vec![if g.resets_at <= now {
        t!("engines.used_tip_fresh", window = window, used = g.percent)
    } else {
        let (span, at) = (until(g.resets_at, now), when(g.resets_at));
        match g.tone {
            Tone::Out => t!("engines.used_tip_out", window = window, span = span, at = at),
            _ => t!("engines.used_tip", window = window, used = g.percent, span = span, at = at),
        }
    }];
    if let Some(pace) = g.pace(now) {
        let pace = (pace * 100.).round() as u8;
        lines.push(if g.ahead(now) { t!("engines.pace_ahead", pace = pace) } else { t!("engines.pace_within", pace = pace) });
    }
    if engine == Engine::Codex {
        lines.push(t!("engines.codex_status_tip", left = g.left()));
    }
    lines.join("\n")
}

/// A meter's name to a screen reader: "Claude week: 42% used".
pub fn gauge_aria(engine: Engine, g: &Gauge) -> String {
    match g.tone {
        Tone::Out => t!("engines.gauge_aria_out", engine = engine_name(engine), window = window_name(g.window)),
        _ => t!("engines.gauge_aria", engine = engine_name(engine), window = window_name(g.window), used = g.percent),
    }
}

/// How long until `at`, as short as it says it: "35m", "2h", "2h 10m",
/// "3d", "3d 5h" — never under a minute.
pub fn until(at: i64, now: i64) -> String {
    let secs = (at - now).max(60);
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (days, hours, minutes) {
        (0, 0, m) => t!("engines.span_minutes", count = m),
        (0, h, 0) => t!("engines.span_hours", count = h),
        (0, h, m) => t!("usage.left_hours", hours = h, minutes = m),
        (d, 0, _) => t!("engines.span_days", count = d),
        (d, h, _) => t!("usage.left_days", days = d, hours = h),
    }
}

/// How old a reading is, once it is no longer sure ("read 2h ago").
pub fn age_words(m: &Meter, now: i64) -> Option<String> {
    m.read_at.filter(|_| m.stale).map(|at| t!("engines.read_ago", span = span(now + (now - at), now)))
}

/// An engine never read: what its row says, and on the pointer, how it gets read.
pub fn unread_words(engine: Engine) -> (&'static str, &'static str) {
    match engine {
        Engine::Claude => (t!("engines.unread_claude"), t!("engines.claude_not_read")),
        Engine::Codex => (t!("engines.unread_codex"), t!("engines.codex_unread_tip")),
    }
}

impl Meter {
    /// The worst of its windows.
    pub fn tone(&self) -> Tone {
        self.gauges.iter().map(|g| g.tone).max().unwrap_or(Tone::Calm)
    }
}

pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "Claude",
        Engine::Codex => "Codex",
    }
}

/// "5-hour", "Weekly".
pub fn window_word(window: Option<Window>) -> &'static str {
    match window {
        Some(Window::Weekly) => t!("switch.window_week"),
        _ => t!("engines.window_5h"),
    }
}

/// The meter's short label: "5h", "wk".
pub fn window_short(window: Window) -> &'static str {
    match window {
        Window::FiveHour => t!("engines.short_5h"),
        Window::Weekly => t!("engines.short_week"),
    }
}

/// "35m", "2h", "3d": how long until `at`.
pub fn span(at: i64, now: i64) -> String {
    let secs = (at - now).max(60) as u64;
    match secs {
        ..3_600 => t!("engines.span_minutes", count = secs / 60),
        3_600..172_800 => t!("engines.span_hours", count = (secs + 1_800) / 3_600),
        _ => t!("engines.span_days", count = (secs + 43_200) / 86_400),
    }
}

/// The meter's line on the pointer about when it was read.
pub fn read_words(m: &Meter, now: i64) -> String {
    match m.read_at {
        None if m.engine == Engine::Codex => t!("engines.codex_not_read").to_owned(),
        None => t!("engines.claude_not_read").to_owned(),
        Some(at) if m.stale => t!("engines.read_ago", span = span(now + (now - at), now)),
        Some(_) => t!("engines.read_now").to_owned(),
    }
}

pub fn mode_word(mode: Switching) -> &'static str {
    match mode {
        Switching::Off => t!("engines.mode_off"),
        Switching::Suggest => t!("engines.mode_suggest"),
        Switching::Auto => t!("engines.mode_auto"),
    }
}

pub fn mode_about(mode: Switching) -> &'static str {
    match mode {
        Switching::Off => t!("engines.mode_off_about"),
        Switching::Suggest => t!("engines.mode_suggest_about"),
        Switching::Auto => t!("engines.mode_auto_about"),
    }
}

/// The squad's parallel work as one word: auto or max.
pub fn parallel_word(mode: ParallelMode) -> &'static str {
    match mode {
        ParallelMode::Auto => t!("parallel.auto"),
        ParallelMode::Max => t!("parallel.max"),
    }
}

/// What the squad's parallel work does, in a line.
pub fn parallel_about(parallel: &Parallel) -> String {
    match parallel.mode {
        ParallelMode::Auto => t!("parallel.auto_about").to_owned(),
        ParallelMode::Max => t!("parallel.max_caps", per_member = parallel.per_member, per_engine = parallel.per_engine),
    }
}

/// The squad's model choice as a word: quality first, or off.
pub fn model_choice_word(choice: equipe_core::protocol::tiers::ModelChoice) -> &'static str {
    match choice {
        equipe_core::protocol::tiers::ModelChoice::QualityFirst => t!("model_choice.quality"),
        equipe_core::protocol::tiers::ModelChoice::Off => t!("model_choice.off"),
    }
}

/// What the squad's model choice does, in a sentence or two.
pub fn model_choice_about(choice: equipe_core::protocol::tiers::ModelChoice) -> &'static str {
    match choice {
        equipe_core::protocol::tiers::ModelChoice::QualityFirst => t!("model_choice.quality_about"),
        equipe_core::protocol::tiers::ModelChoice::Off => t!("model_choice.off_about"),
    }
}

/// The squad's parallel work in a few words, where the lanes are shown.
pub fn parallel_short(parallel: &Parallel) -> String {
    match parallel.mode {
        ParallelMode::Auto => t!("parallel.short_auto").to_owned(),
        ParallelMode::Max => t!("parallel.short_max", per_member = parallel.per_member, per_engine = parallel.per_engine),
    }
}

// ── the card's words ──

/// The card's small heading: what kind of card it is.
pub fn kind(p: &Proposal) -> &'static str {
    match (p.why, p.applied) {
        (Why::Setup, _) => t!("engines.kind_setup"),
        (Why::Person, _) => t!("engines.kind_person"),
        (Why::Return, true) => t!("engines.kind_back"),
        (_, true) => t!("engines.kind_made"),
        (_, false) => t!("engines.kind_ask"),
    }
}

/// What a card says first, in one line.
pub fn headline(p: &Proposal, now: i64) -> String {
    let names = p.moves.iter().map(|m| m.member.as_str()).collect::<Vec<_>>().join("·");
    let engine = p.engine.map(engine_name).unwrap_or_default();
    let to = p.to().map(engine_name).unwrap_or(engine);
    let (window, used, span) = (window_word(p.window), p.used_percent, span(p.resets_at, now));
    match (p.why, p.applied) {
        (Why::Setup, _) => t!("engines.setup_title").to_owned(),
        (Why::Person, _) => t!("engines.person_title", names = names, to = to),
        (Why::StepDown, false) => t!("engines.step_down_ask", names = names),
        (Why::StepDown, true) => t!("engines.step_down", names = names),
        (Why::Return, _) => t!("engines.return", engine = engine, window = window, used = used, names = names),
        (Why::Exhausted, false) => t!("engines.hand_over_ask", engine = engine, span = span, names = names, to = to),
        (Why::Exhausted, true) => t!("engines.hand_over", engine = engine, span = span, names = names, to = to),
        (_, false) => t!("engines.ask", engine = engine, window = window, used = used, span = span, names = names, to = to),
        (_, true) => t!("engines.made", engine = engine, window = window, used = used, names = names, to = to),
    }
}

/// The line under it: why, and what it costs.
pub fn subline(p: &Proposal) -> &'static str {
    match p.why {
        Why::Setup => t!("engines.setup_why"),
        Why::Person => t!("engines.person_why"),
        Why::StepDown => t!("engines.step_down_why"),
        Why::Return => t!("engines.return_why"),
        Why::Exhausted => t!("engines.hand_over_why"),
        Why::Shift | Why::Drain => t!("engines.move_why"),
    }
}

/// One move as the card's table shows it: who, now, after, whether done —
/// and when it happens, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub member: String,
    pub role: String,
    pub now: String,
    pub after: String,
    pub moves_engine: bool,
    pub done: bool,
    pub when: String,
}

/// Where a member stands, for when a move reaches it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stands {
    pub working: bool,
    pub on_ticket: bool,
}

/// When a move reaches its member, as the daemon's rule says: moved; at the
/// end of this turn with its ticket handed over (`now`); after its ticket;
/// when this turn ends; as soon as it's free.
pub fn when(m: &Move, s: Stands) -> &'static str {
    match () {
        _ if m.done => t!("engines.when_done"),
        _ if m.in_place() => t!("engines.when_turn"),
        _ if s.on_ticket && m.now => t!("engines.when_now"),
        _ if s.on_ticket => t!("engines.when_ticket"),
        _ if s.working => t!("engines.when_turn"),
        _ => t!("engines.when_free"),
    }
}

/// Whether the card offers "Move now": a move to the other engine waits for
/// a member on a ticket, not yet made to happen at its turn's end.
pub fn offers_now(p: &Proposal, stands: impl Fn(&str) -> Stands) -> bool {
    !matches!(p.why, Why::Setup | Why::StepDown | Why::Return) && p.moves.iter().any(|m| !m.done && !m.in_place() && !m.now && stands(&m.member).on_ticket)
}

/// The forecast behind a card, in words: "Claude runs out in about 1 day
/// 6 hours, 2 days before it resets — backend·frontend used 64% of its week".
pub fn forecast_words(p: &Proposal, now: i64) -> Option<String> {
    let f = p.forecast?;
    let engine = p.engine.map(engine_name).unwrap_or_default();
    let names = p.moves.iter().filter(|m| !m.in_place()).map(|m| m.member.as_str()).collect::<Vec<_>>().join("·");
    let pace = match f.runs_out_at {
        Some(at) => t!("engines.forecast_out", engine = engine, out = until(at, now), early = until(f.resets_at - at + now, now)),
        None => t!("engines.forecast_lasts", engine = engine, span = until(f.resets_at, now)),
    };
    Some(match f.load_bp {
        Some(bp) if bp > 0 && !names.is_empty() => format!("{pace} · {}", t!("engines.forecast_load", names = names, share = (bp + 50) / 100)),
        _ => pace,
    })
}

/// In `suggest`, the card's offer to let equipe make such moves itself.
pub fn offers_auto(p: &Proposal, mode: Switching) -> bool {
    mode == Switching::Suggest && !p.applied && matches!(p.why, Why::Shift | Why::Drain | Why::Exhausted)
}

/// "Claude · opus" — or the engine's own default.
fn seat(engine: Engine, model: Option<&str>, effort: Option<&str>) -> String {
    let model = model.map_or_else(|| t!("engines.default_model").to_owned(), short_model);
    match effort {
        Some(effort) => format!("{} · {model} · {effort}", engine_name(engine)),
        None => format!("{} · {model}", engine_name(engine)),
    }
}

/// `claude-opus-5-5` → `opus`; Codex's names stay.
fn short_model(model: &str) -> String {
    for family in ["opus", "sonnet", "haiku", "fable"] {
        if model.contains(family) {
            return family.to_owned();
        }
    }
    model.to_owned()
}

pub fn rows(p: &Proposal, stands: impl Fn(&str) -> Stands) -> Vec<Row> {
    p.moves
        .iter()
        .map(|m: &Move| Row {
            member: m.member.clone(),
            role: m.role.clone(),
            now: seat(m.from, m.was_model.as_deref(), None),
            after: seat(m.to, m.model.as_deref(), m.effort.as_deref()),
            moves_engine: !m.in_place(),
            done: m.done,
            when: when(m, stands(&m.member)).to_owned(),
        })
        .collect()
}

/// The buttons' words: the yes (or undo), the no.
pub fn buttons(p: &Proposal) -> (&'static str, &'static str) {
    match (p.why, p.applied) {
        (_, true) => (t!("engines.undo"), t!("engines.ok")),
        (Why::Setup, false) => (t!("engines.apply"), t!("engines.later")),
        (Why::StepDown, false) => (t!("engines.lighten"), t!("switch.keep")),
        (Why::Exhausted, false) => (t!("engines.hand_over_now"), t!("engines.wait")),
        (_, false) => (t!("engines.move"), t!("switch.keep")),
    }
}

/// How far along the moves are: "1/2 moved".
pub fn progress(p: &Proposal) -> Option<String> {
    let done = p.moves.iter().filter(|m| m.done).count();
    (p.applied && !p.moves.is_empty()).then(|| t!("engines.progress", done = done, all = p.moves.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000;

    fn meter(engine: Engine, gauges: &[(Window, u8, i64)], read_at: Option<i64>) -> Meter {
        let gauges = gauges.iter().map(|&(window, percent, resets_at)| Gauge { window, percent, resets_at, tone: tone(percent) }).collect();
        Meter { engine, gauges, read_at, stale: read_at.is_some_and(|at| NOW - at > STALE_AFTER) }
    }

    #[test]
    fn the_sidebar_shows_the_plans_use_once_read_calm_too() {
        let calm = meter(Engine::Claude, &[(Window::Weekly, 3, NOW + 3600)], Some(NOW));
        let unread = meter(Engine::Codex, &[], None);
        assert!(worth_showing(&[calm.clone(), unread.clone()], false), "calm: still shown, to pace it");
        assert!(worth_showing(&[unread.clone(), calm], false), "either engine read is enough");
        assert!(!worth_showing(&[unread.clone(), unread.clone()], false), "nothing read: the space stays the team's");
        assert!(worth_showing(&[unread], true), "a switch waits on the person");
        assert!(!worth_showing(&[], false));
    }

    #[test]
    fn all_engines_out_says_so_once_and_when_work_resumes() {
        let out = |engine, window, resets| meter(engine, &[(Window::FiveHour, 10, NOW + 100), (window, 100, resets)], Some(NOW));
        let claude = out(Engine::Claude, Window::Weekly, NOW + 5 * 86_400);
        let codex = out(Engine::Codex, Window::Weekly, NOW + 2 * 86_400);
        assert_eq!(all_out(&[claude.clone(), codex.clone()], NOW), Some(AllOut { weekly: true, resumes_at: NOW + 2 * 86_400 }), "the first engine back");
        let five = out(Engine::Codex, Window::FiveHour, NOW + 3_600);
        assert_eq!(all_out(&[claude.clone(), five], NOW), Some(AllOut { weekly: false, resumes_at: NOW + 3_600 }), "a five-hour hold is not called weekly");
        let calm = meter(Engine::Codex, &[(Window::Weekly, 40, NOW + 86_400)], Some(NOW));
        assert_eq!(all_out(&[claude.clone(), calm], NOW), None, "one engine with room: not all");
        let unread = meter(Engine::Codex, &[], None);
        assert!(all_out(&[claude.clone(), unread.clone()], NOW).is_some(), "an engine never read is not in use");
        assert_eq!(all_out(&[unread.clone(), unread], NOW), None, "nothing read");
        let mut old = claude;
        old.stale = true;
        assert_eq!(all_out(&[old, codex], NOW), None, "a stale reading is not trusted to say out");
    }

    #[test]
    fn a_window_says_how_much_is_used_and_when_it_starts_over() {
        let g = |window, percent, resets_at| Gauge { window, percent, resets_at, tone: tone(percent) };
        let five = g(Window::FiveHour, 42, NOW + 2 * 3600 + 10 * 60);
        assert_eq!(used_short(&five, NOW), "42% 사용", "always labelled: never a bare 42%");
        assert_eq!(used_words(Engine::Claude, &five, NOW, |_| "17:40".into()), "5시간 42% 사용 · 2시간 10분 뒤 초기화 (17:40)");
        assert_eq!(gauge_aria(Engine::Claude, &five), "Claude 5시간: 42% 사용");
        // At 100%: the limit, with when it comes back — in sight, not only on the pointer.
        let out = g(Window::FiveHour, 100, NOW + 3 * 3600);
        assert_eq!(used_short(&out, NOW), "한도 도달 · 3시간 뒤 초기화");
        assert_eq!(used_words(Engine::Claude, &out, NOW, |_| "20:00".into()), "5시간 한도 도달 · 3시간 뒤 초기화 (20:00)");
        assert_eq!(gauge_aria(Engine::Claude, &out), "Claude 5시간: 한도 도달");
        // As short as it says it: no "0m", no "0h".
        assert_eq!([until(NOW + 35 * 60, NOW), until(NOW + 2 * 3600, NOW), until(NOW + 3 * 86_400, NOW), until(NOW + 20, NOW)], ["35분", "2시간", "3일", "1분"]);
        let fresh = g(Window::FiveHour, 0, NOW - 60);
        assert_eq!(used_words(Engine::Claude, &fresh, NOW, |_| unreachable!("no reset known past it")), "5시간 0% 사용 · 다시 채워졌어요");
    }

    #[test]
    fn colours_warn_early_amber_from_80_red_from_95_and_out_at_100() {
        assert_eq!([tone(0), tone(79), tone(80), tone(94), tone(95), tone(99), tone(100)], [Tone::Calm, Tone::Calm, Tone::Near, Tone::Near, Tone::Critical, Tone::Critical, Tone::Out]);
        // Worst first, for a meter's colour and the folded heading.
        assert!(Tone::Calm < Tone::Near && Tone::Near < Tone::Critical && Tone::Critical < Tone::Out);
        let m = meter(Engine::Claude, &[(Window::FiveHour, 81, NOW + 3600), (Window::Weekly, 96, NOW + 86_400)], Some(NOW));
        assert_eq!(m.tone(), Tone::Critical);
    }

    #[test]
    fn the_weekly_bar_marks_where_an_even_pace_would_be_by_now() {
        const DAY: i64 = 86_400;
        let week = |percent, resets_at| Gauge { window: Window::Weekly, percent, resets_at, tone: tone(percent) };
        // Four of seven days gone: the mark at 4/7.
        let mid = week(70, NOW + 3 * DAY);
        let at = mid.pace(NOW).unwrap();
        assert!((at - 4. / 7.).abs() < 1e-6, "{at}");
        assert!(mid.ahead(NOW), "70% used against 57%: faster than even");
        assert!(!week(50, NOW + 3 * DAY).ahead(NOW), "50% against 57%: within it");
        // Just started: the mark at the start, so anything used is ahead of it.
        assert_eq!(week(0, NOW + 7 * DAY).pace(NOW), Some(0.));
        assert!(week(1, NOW + 7 * DAY).ahead(NOW));
        // A reset further off than a week (a clock off): read as just started, never past the start.
        assert_eq!(week(0, NOW + 9 * DAY).pace(NOW), Some(0.));
        // About to start over: the mark nearly at the end.
        assert!(week(10, NOW + 60).pace(NOW).unwrap() > 0.99);
        // Past its reset: started over, its new end not known yet — no mark.
        assert_eq!(week(0, NOW - 60).pace(NOW), None);
        assert_eq!(week(0, NOW).pace(NOW), None);
        // No reset time known: no mark.
        assert_eq!(week(30, 0).pace(NOW), None);
        assert_eq!(even_pace(Window::Weekly, -5, NOW), None);
        assert!(!week(30, 0).ahead(NOW), "no mark, never ahead of it");
        // The five hours wear none, though their pace is known.
        assert_eq!(Gauge { window: Window::FiveHour, ..mid }.pace(NOW), None);
        assert!((even_pace(Window::FiveHour, NOW + 3600, NOW).unwrap() - 0.8).abs() < 1e-6);
        // On the pointer: the mark explained.
        assert_eq!(
            used_words(Engine::Claude, &mid, NOW, |_| "11/10 09:00".into()),
            "주간 70% 사용 · 3일 뒤 초기화 (11/10 09:00)\n눈금은 고르게 쓸 때의 지금 위치: 57%. 넘었으니 이번 주는 고른 속도보다 빨라요"
        );
    }

    #[test]
    fn codex_says_its_own_status_counts_what_is_left() {
        let week = Gauge { window: Window::Weekly, percent: 61, resets_at: NOW + 2 * 86_400, tone: tone(61) };
        let words = used_words(Engine::Codex, &week, NOW, |_| "11/09 12:00".into());
        assert_eq!(words.lines().next(), Some("주간 61% 사용 · 2일 뒤 초기화 (11/09 12:00)"), "used first, as everywhere");
        assert_eq!(words.lines().last(), Some("Codex 자체 /status는 남은 양으로 보여요: 39% 남음"));
        assert_eq!(words.lines().nth(1), Some("눈금은 고르게 쓸 때의 지금 위치: 71%. 그 안이라 이번 주는 고른 속도 이내예요"));
        assert!(!used_words(Engine::Claude, &week, NOW, |_| String::new()).contains("/status"), "Claude's own counts used too");
    }

    #[test]
    fn the_glance_is_the_window_used_most() {
        let claude = meter(Engine::Claude, &[(Window::FiveHour, 42, NOW + 7200), (Window::Weekly, 76, NOW + 86_400)], Some(NOW));
        let codex = meter(Engine::Codex, &[(Window::FiveHour, 76, NOW + 3600), (Window::Weekly, 9, NOW + 86_400)], Some(NOW));
        let (engine, g) = tightest(&[claude.clone(), codex.clone()]).unwrap();
        assert_eq!((engine, g.window, g.percent), (Engine::Codex, Window::FiveHour, 76), "as used as Claude's week, and back sooner");
        assert_eq!(glance(engine, &g), "Codex 76% 사용");
        let out = meter(Engine::Claude, &[(Window::Weekly, 100, NOW + 86_400)], Some(NOW));
        assert_eq!(tightest(&[codex.clone(), out.clone()]).map(|(e, g)| glance(e, &g)).as_deref(), Some("Claude · 한도 도달"));
        assert_eq!(tightest(&[claude.clone(), meter(Engine::Codex, &[], None)]).map(|(e, g)| (e, g.percent)), Some((Engine::Claude, 76)));
        assert_eq!(tightest(&[meter(Engine::Claude, &[], None)]), None, "nothing read, nothing to glance at");
        // The title bar glances at it only while the sidebar, with the meters, is away.
        assert_eq!(header_glance(std::slice::from_ref(&claude), false), None);
        assert_eq!(header_glance(&[claude], true).map(|(e, g)| (e, g.window)), Some((Engine::Claude, Window::Weekly)));
    }

    #[test]
    fn a_reading_says_how_old_it_is_and_an_unread_one_how_to_get_it() {
        let fresh = meter(Engine::Claude, &[(Window::FiveHour, 10, NOW + 3600)], Some(NOW - 5 * 60));
        assert_eq!(age_words(&fresh, NOW), None, "just read: nothing to say");
        let old = meter(Engine::Claude, &[(Window::FiveHour, 10, NOW + 3600)], Some(NOW - 2 * 3600));
        assert_eq!(age_words(&old, NOW).as_deref(), Some("2시간 전에 읽음"));
        assert_eq!(age_words(&meter(Engine::Codex, &[], None), NOW), None, "never read is said otherwise");
        assert_eq!(unread_words(Engine::Claude), ("아직 모름 · 멤버가 일하면 보여요", "Claude 멤버가 다음 턴을 마치면 보여요"));
        assert_eq!(unread_words(Engine::Codex), ("아직 모름 · 로그인하면 보여요", "설정 › AI 플랫폼에서 Codex에 로그인하면 보여요"));
    }

    fn shift(applied: bool) -> Proposal {
        let mv = |name: &str, model: &str| Move {
            member: name.into(),
            role: name.into(),
            from: Engine::Claude,
            to: Engine::Codex,
            model: None,
            effort: None,
            was_model: Some(model.into()),
            done: false,
            now: false,
        };
        Proposal {
            id: 7,
            why: Why::Shift,
            engine: Some(Engine::Claude),
            window: Some(Window::FiveHour),
            used_percent: 86,
            resets_at: NOW + 2 * 3600,
            moves: vec![mv("backend", "claude-opus-5-5"), mv("frontend", "sonnet")],
            applied,
            proposed_at: NOW as u64,
            forecast: None,
        }
    }

    /// backend mid-ticket and at work, frontend free.
    fn stands(name: &str) -> Stands {
        Stands { working: name == "backend", on_ticket: name == "backend" }
    }

    #[test]
    fn a_card_in_suggest_asks_and_one_in_auto_tells_with_an_undo() {
        assert_eq!(headline(&shift(false), NOW), "Claude 5시간 한도 86% · 리셋 2시간 뒤 — backend·frontend를 Codex로 옮길까요?");
        assert_eq!(buttons(&shift(false)), ("옮기기", "이번엔 그대로"));
        assert_eq!(headline(&shift(true), NOW), "backend·frontend: Claude 5시간 한도 86% → Codex로 옮겨요");
        assert_eq!(buttons(&shift(true)), ("되돌리기", "확인"));
        let rows = rows(&shift(false), stands);
        assert_eq!((rows[0].now.as_str(), rows[0].after.as_str(), rows[0].moves_engine), ("Claude · opus", "Codex · 기본 모델", true));
        let mut half = shift(true);
        half.moves[0].done = true;
        assert_eq!(progress(&half).as_deref(), Some("1/2 옮김"));
        assert_eq!(progress(&shift(false)), None);
    }

    #[test]
    fn the_setup_card_reads_as_a_recommendation() {
        let setup = Proposal {
            why: Why::Setup,
            engine: None,
            window: None,
            moves: vec![Move { member: "qa".into(), role: "qa".into(), from: Engine::Claude, to: Engine::Codex, model: Some("gpt-6.1-sol".into()), effort: Some("medium".into()), was_model: Some("sonnet".into()), done: false, now: false }],
            ..shift(false)
        };
        assert_eq!(kind(&setup), "추천 구성");
        assert_eq!(buttons(&setup), ("적용", "나중에"));
        assert_eq!(rows(&setup, |_| Stands::default())[0].after, "Codex · gpt-6.1-sol · medium");
        assert!(!offers_now(&setup, |_| Stands { working: false, on_ticket: true }), "the setup moves between tickets");
    }

    #[test]
    fn each_row_says_when_its_move_happens_and_a_member_mid_ticket_is_offered_move_now() {
        let card = shift(false);
        let when: Vec<String> = rows(&card, stands).into_iter().map(|r| r.when).collect();
        assert_eq!(when, ["지금 티켓을 마친 뒤", "손이 비는 대로"]);
        assert!(offers_now(&card, stands), "backend is mid-ticket");
        assert!(!offers_now(&card, |_| Stands::default()), "nobody on a ticket: nothing to hurry");
        // Made now: its row says so, and nothing is left to hurry.
        let mut now = shift(true);
        now.moves[0].now = true;
        assert_eq!(rows(&now, stands)[0].when, "이번 턴이 끝날 때 · 티켓 넘김");
        assert!(!offers_now(&now, stands));
        now.moves[0].done = true;
        assert_eq!(rows(&now, stands)[0].when, "옮김");
        // Working off a ticket: at its turn's end.
        assert_eq!(rows(&card, |_| Stands { working: true, on_ticket: false })[1].when, "이번 턴이 끝나면");
        // The card's buttons: "Move now" answers with now; on moves made, it hurries them and the card stays.
        let mut e = Engines::default();
        e.propose(shift(false));
        assert_eq!(e.decide("acme", 7, Answer::Now), Some(Command::ApplyEngineProposal { squad: "acme".into(), id: 7, now: true }));
        assert_eq!(e.first(), None);
        let mut made = Engines::default();
        made.propose(shift(true));
        assert_eq!(made.decide("acme", 7, Answer::Now), Some(Command::ApplyEngineProposal { squad: "acme".into(), id: 7, now: true }));
        assert!(made.first().is_some(), "moves on their way: the card still tells of them, with its undo");
    }

    #[test]
    fn the_card_says_the_forecast_and_in_suggest_offers_to_do_it_on_its_own() {
        use equipe_core::protocol::engines::Forecast;
        const DAY: i64 = 86_400;
        let mut card = shift(false);
        card.window = Some(Window::Weekly);
        card.resets_at = NOW + 3 * DAY + 6 * 3600;
        card.forecast = Some(Forecast { runs_out_at: Some(NOW + DAY + 6 * 3600), resets_at: NOW + 3 * DAY + 6 * 3600, load_bp: Some(6_400) });
        assert_eq!(forecast_words(&card, NOW).as_deref(), Some("지금 속도면 Claude는 1일 6시간 뒤에 다 써요. 리셋보다 2일 빨라요 · backend·frontend: 이번 주 사용량의 64%"));
        card.forecast = Some(Forecast { runs_out_at: None, resets_at: NOW + 2 * DAY, load_bp: None });
        assert_eq!(forecast_words(&card, NOW).as_deref(), Some("지금 속도면 Claude는 2일 뒤 리셋까지 버텨요"));
        assert_eq!(forecast_words(&shift(false), NOW), None, "no forecast known: no line");
        assert!(offers_auto(&shift(false), Switching::Suggest));
        assert!(!offers_auto(&shift(false), Switching::Auto) && !offers_auto(&shift(true), Switching::Suggest), "already on its own");
        // Every language words the new lines.
        for lang in equipe_core::i18n::Lang::ALL {
            for key in ["engines.when_now", "engines.when_ticket", "engines.move_now", "engines.move_after", "engines.auto_offer", "engines.forecast_out", "engines.forecast_load", "plan_use.auto_hint", "models.move_now", "models.hurry"] {
                assert!(crate::i18n::of(lang).own(key).is_some(), "{lang:?} {key}");
            }
        }
    }

    #[test]
    fn a_click_answers_once_and_only_as_the_card_allows() {
        let mut e = Engines::default();
        assert!(e.propose(shift(false)));
        assert_eq!(e.decide("acme", 7, Answer::Undo), None, "nothing made yet to undo");
        assert_eq!(e.decide("acme", 7, Answer::Apply), Some(Command::ApplyEngineProposal { squad: "acme".into(), id: 7, now: false }));
        assert_eq!(e.first(), None, "hidden at once");
        assert_eq!(e.decide("acme", 7, Answer::Apply), None, "not twice");
        e.forget_sent();
        assert!(e.first().is_some(), "a dead link: shown again");
        // Shown again as its members move: replaced, not added.
        assert!(!e.propose(shift(true)));
        assert_eq!(e.decide("acme", 7, Answer::Undo), Some(Command::UndoEngineProposal { squad: "acme".into(), id: 7 }));
        e.close(7);
        assert_eq!(e.open().count(), 0);
    }

    #[test]
    fn the_meters_show_both_engines_by_how_used_they_are_and_how_sure() {
        let mut e = Engines::default();
        let w = |used, left| Some(PlanWindow { used_percent: used, resets_at: NOW + left });
        e.read(Engine::Claude, PlanUsage { five_hour: w(86, 7200), seven_day: w(41, 86400) }, NOW);
        let meters = e.meters(NOW);
        assert_eq!(meters.iter().map(|m| m.engine).collect::<Vec<_>>(), [Engine::Claude, Engine::Codex]);
        assert_eq!(meters[0].gauges.iter().map(|g| (g.percent, g.tone)).collect::<Vec<_>>(), [(86, Tone::Near), (41, Tone::Calm)]);
        assert_eq!(meters[0].tone(), Tone::Near);
        assert!(meters[1].gauges.is_empty() && meters[1].read_at.is_none());
        assert_eq!(read_words(&meters[1], NOW), "Codex 사용량은 아직 읽지 못했어요");
        // A window past its reset starts over; an old reading says so.
        e.read(Engine::Codex, PlanUsage { five_hour: w(99, -60), seven_day: w(72, 86400) }, NOW - 3600);
        let codex = &e.meters(NOW)[1];
        assert_eq!(codex.gauges.iter().map(|g| (g.percent, g.tone)).collect::<Vec<_>>(), [(0, Tone::Calm), (72, Tone::Calm)]);
        assert!(codex.stale);
        assert_eq!(read_words(codex, NOW), "1시간 전에 읽음");
        let g = e.meters(NOW)[0].gauges[0];
        assert_eq!(used_words(Engine::Claude, &g, NOW, |_| "17:40".into()), "5시간 86% 사용 · 2시간 뒤 초기화 (17:40)");
    }

    #[test]
    fn an_older_daemons_plan_is_claudes() {
        // As an older daemon sends it: no `plans`, and no `engine_switching` (it left out suggest).
        let snapshot: Snapshot = serde_json::from_str(r#"{"squad":"acme","sessions":[],"plan":{}}"#).unwrap();
        assert_eq!(snapshot.plan, Some(PlanUsage::default()));
        let mut e = Engines::default();
        e.load(&snapshot, NOW);
        assert_eq!(e.plans.iter().map(|p| (p.engine, p.at)).collect::<Vec<_>>(), [(Engine::Claude, NOW)]);
        assert_eq!(e.mode, Switching::Suggest);
    }

    #[test]
    fn parallel_work_says_its_mode_and_on_max_its_caps() {
        // The tests' screens speak Korean.
        assert_eq!((parallel_word(ParallelMode::Auto), parallel_word(ParallelMode::Max)), ("균형", "전속력"));
        assert_eq!(parallel_short(&Parallel::default()), "병렬: 균형");
        assert_eq!(parallel_short(&Parallel::max(6, 16)), "병렬: 전속력 · 멤버당 6 · 엔진당 16");
        assert!(parallel_about(&Parallel::max(8, 20)).contains("멤버당 최대 8개, 엔진당 최대 20개"));
        assert_eq!(parallel_about(&Parallel::default()), "플랜 사용량이 늘수록 한 번에 진행하는 티켓을 줄여요");
    }
}
