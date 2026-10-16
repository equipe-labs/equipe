//! Both engines' plans and members moving between them, as the window shows
//! them — the meters at the sidebar's foot, `/usage`, the card above the
//! composer (a proposal in `suggest`, what was done in `auto` with an undo,
//! the recommended setup once), the badge by each member — and the squad's
//! choice in settings. No UI types here, so the rules are tested without a
//! window. Only the person's click answers a card.

use equipe_core::protocol::engines::{Move, Proposal, Switching, Why, Window};
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
    /// 옮기기, 적용.
    Apply,
    /// 이번엔 그대로, 나중에 — or 확인 on moves already made.
    Leave,
    /// 되돌리기.
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
            (Answer::Apply, false) => Command::ApplyEngineProposal { squad, id },
            (Answer::Undo, true) => Command::UndoEngineProposal { squad, id },
            (Answer::Leave, _) => Command::DismissEngineProposal { squad, id },
            _ => return None,
        };
        self.sent.push(id);
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

/// One window of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauge {
    pub window: Window,
    /// Used, 0–100; a window past its reset starts over at 0.
    pub percent: u8,
    pub resets_at: i64,
    pub tone: Tone,
}

/// How a window's use reads: calm, watched (70%), low (85%), out (100%) — the switching rules' marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Calm,
    Watch,
    Low,
    Out,
}

pub fn tone(percent: u8) -> Tone {
    match percent {
        100.. => Tone::Out,
        85.. => Tone::Low,
        70.. => Tone::Watch,
        _ => Tone::Calm,
    }
}

fn gauge(window: Window, w: PlanWindow, now: i64) -> Gauge {
    let percent = if w.resets_at <= now { 0 } else { w.used_percent.min(100) };
    Gauge { window, percent, resets_at: w.resets_at, tone: tone(percent) }
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

/// "5시간", "주간".
pub fn window_word(window: Option<Window>) -> &'static str {
    match window {
        Some(Window::Weekly) => t!("engines.window_week"),
        _ => t!("engines.window_5h"),
    }
}

/// The meter's short label: "5h", "주".
pub fn window_short(window: Window) -> &'static str {
    match window {
        Window::FiveHour => t!("engines.short_5h"),
        Window::Weekly => t!("engines.short_week"),
    }
}

/// "35분", "2시간", "3일": how long until `at`.
pub fn span(at: i64, now: i64) -> String {
    let secs = (at - now).max(60) as u64;
    match secs {
        ..3_600 => t!("engines.span_minutes", count = secs / 60),
        3_600..172_800 => t!("engines.span_hours", count = (secs + 1_800) / 3_600),
        _ => t!("engines.span_days", count = (secs + 43_200) / 86_400),
    }
}

/// A window on the pointer: "5시간 · 62% 사용 · 2시간 뒤 리셋 (17:40)".
pub fn gauge_words(g: &Gauge, now: i64, when: impl Fn(i64) -> String) -> String {
    t!("engines.gauge", window = window_word(Some(g.window)), percent = g.percent, span = span(g.resets_at, now), at = when(g.resets_at))
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

// ── the card's words ──

/// The card's small heading: what kind of card it is.
pub fn kind(p: &Proposal) -> &'static str {
    match (p.why, p.applied) {
        (Why::Setup, _) => t!("engines.kind_setup"),
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
        Why::StepDown => t!("engines.step_down_why"),
        Why::Return => t!("engines.return_why"),
        Why::Exhausted => t!("engines.hand_over_why"),
        Why::Shift | Why::Drain => t!("engines.move_why"),
    }
}

/// One move as the card's table shows it: who, now, after, whether done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub member: String,
    pub role: String,
    pub now: String,
    pub after: String,
    pub moves_engine: bool,
    pub done: bool,
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

pub fn rows(p: &Proposal) -> Vec<Row> {
    p.moves
        .iter()
        .map(|m: &Move| Row {
            member: m.member.clone(),
            role: m.role.clone(),
            now: seat(m.from, m.was_model.as_deref(), None),
            after: seat(m.to, m.model.as_deref(), m.effort.as_deref()),
            moves_engine: !m.in_place(),
            done: m.done,
        })
        .collect()
}

/// The buttons' words: the yes (or undo), the no.
pub fn buttons(p: &Proposal) -> (&'static str, &'static str) {
    match (p.why, p.applied) {
        (_, true) => (t!("engines.undo"), t!("engines.ok")),
        (Why::Setup, false) => (t!("engines.apply"), t!("engines.later")),
        (Why::StepDown, false) => (t!("engines.lighten"), t!("engines.keep")),
        (Why::Exhausted, false) => (t!("engines.hand_over_now"), t!("engines.wait")),
        (_, false) => (t!("engines.move"), t!("engines.keep")),
    }
}

/// How far along the moves are: "1/2 옮김".
pub fn progress(p: &Proposal) -> Option<String> {
    let done = p.moves.iter().filter(|m| m.done).count();
    (p.applied && !p.moves.is_empty()).then(|| t!("engines.progress", done = done, all = p.moves.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000;

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
        }
    }

    #[test]
    fn a_card_in_suggest_asks_and_one_in_auto_tells_with_an_undo() {
        assert_eq!(headline(&shift(false), NOW), "Claude 5시간 한도 86% · 리셋 2시간 뒤 — backend·frontend를 다음 티켓부터 Codex로 옮길까요?");
        assert_eq!(buttons(&shift(false)), ("옮기기", "이번엔 그대로"));
        assert_eq!(headline(&shift(true), NOW), "backend·frontend: Claude 5시간 한도 86% → 다음 티켓부터 Codex로");
        assert_eq!(buttons(&shift(true)), ("되돌리기", "확인"));
        let rows = rows(&shift(false));
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
            moves: vec![Move { member: "qa".into(), role: "qa".into(), from: Engine::Claude, to: Engine::Codex, model: Some("gpt-6.1-sol".into()), effort: Some("medium".into()), was_model: Some("sonnet".into()), done: false }],
            ..shift(false)
        };
        assert_eq!(kind(&setup), "추천 구성");
        assert_eq!(buttons(&setup), ("적용", "나중에"));
        assert_eq!(rows(&setup)[0].after, "Codex · gpt-6.1-sol · medium");
    }

    #[test]
    fn a_click_answers_once_and_only_as_the_card_allows() {
        let mut e = Engines::default();
        assert!(e.propose(shift(false)));
        assert_eq!(e.decide("acme", 7, Answer::Undo), None, "nothing made yet to undo");
        assert_eq!(e.decide("acme", 7, Answer::Apply), Some(Command::ApplyEngineProposal { squad: "acme".into(), id: 7 }));
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
    fn the_meters_show_both_engines_by_how_low_they_run_and_how_sure() {
        let mut e = Engines::default();
        let w = |used, left| Some(PlanWindow { used_percent: used, resets_at: NOW + left });
        e.read(Engine::Claude, PlanUsage { five_hour: w(86, 7200), seven_day: w(41, 86400) }, NOW);
        let meters = e.meters(NOW);
        assert_eq!(meters.iter().map(|m| m.engine).collect::<Vec<_>>(), [Engine::Claude, Engine::Codex]);
        assert_eq!(meters[0].gauges.iter().map(|g| (g.percent, g.tone)).collect::<Vec<_>>(), [(86, Tone::Low), (41, Tone::Calm)]);
        assert_eq!(meters[0].tone(), Tone::Low);
        assert!(meters[1].gauges.is_empty() && meters[1].read_at.is_none());
        assert_eq!(read_words(&meters[1], NOW), "Codex 사용량은 아직 읽지 못했어요");
        // A window past its reset starts over; an old reading says so.
        e.read(Engine::Codex, PlanUsage { five_hour: w(99, -60), seven_day: w(72, 86400) }, NOW - 3600);
        let codex = &e.meters(NOW)[1];
        assert_eq!(codex.gauges.iter().map(|g| (g.percent, g.tone)).collect::<Vec<_>>(), [(0, Tone::Calm), (72, Tone::Watch)]);
        assert!(codex.stale);
        assert_eq!(read_words(codex, NOW), "1시간 전에 읽음");
        assert_eq!([tone(69), tone(70), tone(85), tone(100)], [Tone::Calm, Tone::Watch, Tone::Low, Tone::Out]);
        let g = e.meters(NOW)[0].gauges[0];
        assert_eq!(gauge_words(&g, NOW, |_| "17:40".into()), "5시간 · 86% 사용 · 2시간 뒤 리셋 (17:40)");
    }

    #[test]
    fn an_older_daemons_plan_is_claudes() {
        let snapshot = Snapshot { squad: "acme".into(), plan: Some(PlanUsage::default()), ..Default::default() };
        let mut e = Engines::default();
        e.load(&snapshot, NOW);
        assert_eq!(e.plans.iter().map(|p| (p.engine, p.at)).collect::<Vec<_>>(), [(Engine::Claude, NOW)]);
        assert_eq!(e.mode, Switching::Suggest);
    }
}
