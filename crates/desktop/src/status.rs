//! The Status screen's figures and words (`views/status.rs` draws them): is
//! the squad's work going well, what slows or stops it, what the person can
//! do. The daemon works it out ([`equipe_core::protocol::status`], the
//! `squad_status` request) and sends facts — a cause, how many, which
//! engine, until when — and this puts them in the person's language.
//!
//! The answer is asked for every [`POLL_OPEN`] while the screen is open and
//! every [`POLL_CLOSED`] otherwise (the sidebar's dot reads it); the daemon
//! keeps one for 15 seconds, so asking is cheap. A daemon that predates the
//! request answers "unknown method": the sidebar's Status row hides.

use std::time::Duration;

use equipe_core::protocol::status::{Action, Block, Cause, EngineBudget, Handling, Mood, Note, Severity, SquadStatus, WasteKey};
use equipe_core::protocol::{Engine, PauseWhy, TicketId};
use equipe_core::t;

use crate::switching::{engine_name, span};

/// How often the daemon is asked while the Status screen is open.
pub const POLL_OPEN: Duration = Duration::from_secs(30);
/// … and while it is not, for the sidebar's dot.
pub const POLL_CLOSED: Duration = Duration::from_secs(120);
/// How long an answer may take.
pub const ASK_WAIT: Duration = Duration::from_secs(12);

/// Whether the daemon answers `squad_status`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Support {
    #[default]
    Unknown,
    Yes,
    /// An older daemon: "unknown method".
    No,
}

/// The screen's state.
#[derive(Debug, Default)]
pub struct StatusUi {
    pub support: Support,
    pub data: Option<SquadStatus>,
    /// A look is under way.
    pub loading: bool,
    /// The last look failed, and why.
    pub failed: Option<String>,
    /// Pins the person just flipped, before the daemon's next answer: ticket and whether pinned.
    pub flipped: Vec<(TicketId, bool)>,
    /// Shown from a sample, as the picture script has it: the daemon is not asked.
    pub demo: bool,
}

/// Whether an answer's error says the daemon doesn't know the request.
pub fn unsupported(why: &str) -> bool {
    why.contains("unknown method")
}

impl StatusUi {
    /// An answer for `squad` came in, or an error.
    pub fn answered(&mut self, answer: Result<SquadStatus, String>) {
        self.loading = false;
        match answer {
            Ok(status) => {
                self.support = Support::Yes;
                self.failed = None;
                self.flipped.clear();
                self.data = Some(status);
            }
            Err(why) if unsupported(&why) => {
                self.support = Support::No;
                self.data = None;
            }
            Err(why) => self.failed = Some(why),
        }
    }

    /// The status of `squad`, when the daemon has told it.
    pub fn of(&self, squad: &str) -> Option<&SquadStatus> {
        self.data.as_ref().filter(|s| s.squad == squad)
    }

    /// Whether the sidebar shows its Status row: the daemon knows the request.
    pub fn offered(&self) -> bool {
        self.support == Support::Yes
    }

    /// The color of the squad's dot, when it is known.
    pub fn mood(&self, squad: &str) -> Option<Mood> {
        self.of(squad).map(|s| s.mood)
    }

    /// Whether the person watches `ticket` (pinned), counting a flip not yet answered.
    pub fn pinned(&self, ticket: TicketId) -> bool {
        self.flipped.iter().rev().find(|(t, _)| *t == ticket).map_or_else(|| self.data.as_ref().is_some_and(|s| s.watched.pins.contains(&ticket)), |(_, on)| *on)
    }

    /// The person pinned or let go of `ticket`; kept until the daemon answers.
    pub fn flip(&mut self, ticket: TicketId, on: bool) {
        self.flipped.retain(|(t, _)| *t != ticket);
        self.flipped.push((ticket, on));
    }
}

/// The offset of the person's clock from UTC at unix time `now`, in seconds.
pub fn utc_offset_secs(now: u64) -> i32 {
    let local = crate::state::local_time(now);
    let local_s = i64::from(local.hour) * 3600 + i64::from(local.minute) * 60;
    let utc_s = (now % 86_400) as i64 / 60 * 60;
    let mut offset = local_s - utc_s;
    if offset > 14 * 3600 {
        offset -= 86_400;
    } else if offset < -12 * 3600 {
        offset += 86_400;
    }
    offset as i32
}

// ── words ──

/// `16:34`, on the person's clock.
pub fn clock(at: u64) -> String {
    let local = crate::state::local_time(at);
    equipe_core::i18n::fill(t!("time.clock_24"), &[("hh", format!("{:02}", local.hour)), ("mm", format!("{:02}", local.minute))])
}

/// When something goes on: the time today, with the weekday when it is further off (`Mon 16:34`).
pub fn when(at: u64, now: u64) -> String {
    if at.saturating_sub(now) < 20 * 3600 {
        return clock(at);
    }
    let local = crate::state::local_time(at);
    let day = t!("status.weekdays_short").split('|').nth(local.weekday as usize).unwrap_or_default().to_owned();
    t!("status.day_time", day = day, time = clock(at))
}

/// `updated 16:34`.
pub fn updated(at: u64) -> String {
    t!("status.updated", time = clock(at))
}

fn engine_of(engine: Option<Engine>) -> &'static str {
    engine.map(engine_name).unwrap_or_default()
}

/// The one sentence on the status line: what slows or stops the work, or that nothing does.
pub fn headline(s: &SquadStatus) -> String {
    let top = s.blocked.first();
    let count = top.map_or(0, |b| u64::from(b.count));
    let engine = engine_of(top.and_then(|b| b.engine));
    match s.mood {
        Mood::Good if s.empty => t!("status.good_empty").to_owned(),
        Mood::Good => t!("status.good").to_owned(),
        Mood::Stopped => match top.map(|b| b.cause) {
            Some(Cause::EngineOut) => t!("status.stop_engines").to_owned(),
            _ => t!("status.stop_breaker").to_owned(),
        },
        Mood::Slowed => match top.map(|b| (b.cause, b.why)) {
            None => {
                let pacing = s.notes.iter().find_map(|n| if let Note::Pacing { engine, .. } = n { Some(*engine) } else { None });
                t!("status.slow_pacing", engine = engine_of(pacing))
            }
            Some((Cause::EngineOut, _)) => t!("status.slow_engine_out", engine = engine),
            Some((Cause::SignIn, _)) | Some((Cause::RunsPaused, Some(PauseWhy::SignIn))) => t!("status.slow_sign_in", engine = engine),
            Some((Cause::RunsPaused, _)) => t!("status.slow_paused", count = count),
            Some((Cause::HandedBack, _)) => t!("status.slow_handed_back", count = count),
            Some((Cause::Quiet, _)) => t!("status.slow_quiet", count = count),
            Some((Cause::Failing, _)) => t!("status.slow_failing").to_owned(),
            Some((Cause::BreakerOpen, _)) => t!("status.stop_breaker").to_owned(),
            Some((Cause::Decisions | Cause::Approvals, _)) => t!("status.slow_waiting").to_owned(),
            Some((Cause::MacLocked, _)) => t!("status.slow_mac_locked").to_owned(),
        },
    }
}

/// The muted second line: the engines' notes (`Codex limit · back in 3d · Claude pacing (6 at once)`), or how work goes.
pub fn detail(s: &SquadStatus, now: u64) -> String {
    let notes: Vec<String> = s
        .notes
        .iter()
        .map(|n| match *n {
            Note::Out { engine, back_at: Some(at) } => t!("status.note_out_back", engine = engine_name(engine), span = span(at as i64, now as i64)),
            Note::Out { engine, back_at: None } => t!("status.note_out", engine = engine_name(engine)),
            Note::Pacing { engine, at_once } => t!("status.note_pacing", engine = engine_name(engine), count = at_once),
        })
        .collect();
    if !notes.is_empty() {
        return notes.join(" · ");
    }
    match (s.mood, s.blocked.first()) {
        (Mood::Good, _) => t!("status.detail_good", runs = s.tiles.runs_active, accepted = s.tiles.accepted_today),
        (_, Some(b)) => cause_text(b, now),
        _ => String::new(),
    }
}

/// A row's plain cause: what is the matter, with its count, engine and time.
pub fn cause_text(b: &Block, now: u64) -> String {
    let engine = engine_of(b.engine);
    let count = u64::from(b.count);
    let back = b.at.map(|at| t!("status.back", when = when(at, now)));
    let ago = |at: Option<u64>| at.filter(|at| *at > 0 && *at <= now).map(|at| t!("status.ago", span = crate::state::short_span(now - at)));
    let join = |main: String, extra: Option<String>| match extra {
        Some(extra) => format!("{main} · {extra}"),
        None => main,
    };
    match b.cause {
        Cause::BreakerOpen => {
            let main = match b.detail.as_deref() {
                Some(cause) => t!("status.cause_breaker", cause = cause),
                None => t!("status.cause_breaker_plain").to_owned(),
            };
            let held = (b.count > 0).then(|| t!("status.held", count = count));
            join(join(main, held), ago(b.at))
        }
        Cause::EngineOut if b.count > 0 => join(t!("status.cause_limit", count = count, engine = engine), back),
        Cause::EngineOut => join(t!("status.cause_limit_none", engine = engine), back),
        Cause::SignIn if b.count > 0 => t!("status.cause_sign_in_runs", count = count, engine = engine),
        Cause::SignIn => t!("status.cause_sign_in", engine = engine),
        Cause::RunsPaused => match b.why {
            Some(PauseWhy::EngineLimit) => join(t!("status.cause_limit", count = count, engine = engine), back),
            Some(PauseWhy::Retrying) => join(t!("status.cause_retrying", count = count), back),
            Some(PauseWhy::SignIn) => t!("status.cause_sign_in_runs", count = count, engine = engine),
            Some(PauseWhy::SquadPaused) | None => t!("status.cause_squad_paused", count = count),
        },
        Cause::HandedBack => join(t!("status.cause_handed_back", count = count), ago(b.at)),
        Cause::Decisions => t!("status.cause_decisions", count = count),
        Cause::Approvals => t!("status.cause_approvals", count = count),
        Cause::Quiet => join(t!("status.cause_quiet", count = count), ago(b.at)),
        Cause::Failing => t!("status.cause_failing", count = count),
        Cause::MacLocked => t!("status.cause_mac_locked", count = count),
    }
}

/// What a row's button says, or who handles it.
pub fn handling_word(h: Handling) -> &'static str {
    match h {
        Handling::Fixed => t!("status.act_fixed"),
        Handling::Resolving => t!("status.act_resolving"),
        Handling::You { action: Action::Open } => t!("status.act_open"),
        Handling::You { action: Action::Answer } => t!("status.act_answer"),
        Handling::You { action: Action::Look } => t!("status.act_look"),
        Handling::You { action: Action::Unlock } => t!("status.act_unlock"),
    }
}

/// How a row's dot reads to a screen reader.
pub fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Red => t!("status.sev_red"),
        Severity::Amber => t!("status.sev_amber"),
        Severity::Info => t!("status.sev_info"),
    }
}

pub fn mood_word(mood: Mood) -> &'static str {
    match mood {
        Mood::Good => t!("status.mood_good"),
        Mood::Slowed => t!("status.mood_slowed"),
        Mood::Stopped => t!("status.mood_stopped"),
    }
}

/// A tile's line under its number.
pub fn tile_sub(s: &SquadStatus, tile: Tile) -> String {
    let t = &s.tiles;
    match tile {
        Tile::Accepted => t!("status.tile_accepted_sub", count = t.accepted_2h),
        Tile::InProgress => t!("status.tile_progress_sub", active = t.runs_active, max = t.runs_max),
        Tile::NeedsYou => t!("status.tile_needs_sub", decisions = t.decisions, approvals = t.approvals),
        Tile::Waiting => t!("status.tile_waiting_sub", count = t.waiting_urgent),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Accepted,
    InProgress,
    NeedsYou,
    Waiting,
}

/// An engine's weekly line on the budget card: how far it lasts, or when it is back.
pub fn budget_line(e: &EngineBudget, now: u64) -> String {
    if let Some(at) = e.out_until {
        return if at > 0 { t!("status.budget_out", when = when(at, now)) } else { t!("status.budget_out_unknown").to_owned() };
    }
    let Some(week) = e.weekly else {
        return if e.five_hour.is_some() { t!("status.budget_no_week").to_owned() } else { t!("status.budget_unread").to_owned() };
    };
    match (e.runs_out_at, e.lasts) {
        (Some(at), _) => t!("status.budget_runs_out", when = when(at, now)),
        (None, true) => t!("status.budget_lasts", when = when(week.resets_at.max(0) as u64, now)),
        (None, false) => t!("status.budget_resets", when = when(week.resets_at.max(0) as u64, now)),
    }
}

/// A waste measure's label.
pub fn waste_label(key: WasteKey) -> &'static str {
    match key {
        WasteKey::PmDeskContext => t!("status.waste_pm_desk"),
        WasteKey::HeavyShare => t!("status.waste_heavy"),
        WasteKey::CancelledShare => t!("status.waste_cancelled"),
        WasteKey::ReworkShare => t!("status.waste_rework"),
        WasteKey::FirstHandInAccept => t!("status.waste_accept"),
    }
}

/// A waste measure's value: tokens per call as `640k`, a share as `12%`; `—` when not measured.
pub fn waste_value(key: WasteKey, value: Option<f64>) -> String {
    let Some(v) = value else { return "—".to_owned() };
    match key {
        WasteKey::PmDeskContext => crate::usage::short_count(v.round() as u64),
        _ if v > 0.0 && v < 1.0 => "<1%".to_owned(),
        _ => format!("{:.0}%", v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::status::{Tiles, WatchedList};

    fn block(cause: Cause, count: u32) -> Block {
        Block { cause, severity: Severity::Amber, count, why: None, engine: None, at: None, detail: None, tickets: vec![], handling: Handling::Fixed, goto: None }
    }

    fn status(mood: Mood, blocked: Vec<Block>) -> SquadStatus {
        SquadStatus { squad: "acme".into(), at: 1_800_000_000, mood, blocked, tiles: Tiles::default(), watched: WatchedList::default(), ..SquadStatus::default() }
    }

    #[test]
    fn an_older_daemons_answer_hides_the_row_and_a_failure_keeps_what_was_known() {
        let mut ui = StatusUi::default();
        assert!(!ui.offered(), "before the first answer the row isn't offered");
        ui.answered(Err("unknown method".into()));
        assert_eq!((ui.support, ui.offered()), (Support::No, false));
        ui.answered(Ok(status(Mood::Good, vec![])));
        assert!(ui.offered() && ui.mood("acme") == Some(Mood::Good) && ui.mood("other").is_none());
        ui.answered(Err("the daemon didn't answer".into()));
        assert!(ui.offered(), "a failed look doesn't take the row away");
        assert!(ui.of("acme").is_some() && ui.failed.is_some());
    }

    #[test]
    fn a_pin_reads_as_flipped_until_the_daemon_says_so() {
        let mut ui = StatusUi::default();
        let mut s = status(Mood::Good, vec![]);
        s.watched.pins = vec![4];
        ui.answered(Ok(s.clone()));
        assert!(ui.pinned(4) && !ui.pinned(5));
        ui.flip(5, true);
        ui.flip(4, false);
        assert!(ui.pinned(5) && !ui.pinned(4));
        s.watched.pins = vec![5];
        ui.answered(Ok(s));
        assert!(ui.pinned(5) && !ui.pinned(4) && ui.flipped.is_empty());
    }

    #[test]
    fn the_status_line_says_what_slows_or_stops_the_work() {
        let now = 1_800_000_000;
        let good = status(Mood::Good, vec![]);
        assert_eq!(headline(&good), t!("status.good"));
        let mut stopped = status(Mood::Stopped, vec![Block { severity: Severity::Red, detail: Some("No space left".into()), at: Some(now - 600), count: 3, handling: Handling::Resolving, ..block(Cause::BreakerOpen, 3) }]);
        assert_eq!(headline(&stopped), t!("status.stop_breaker"));
        let row = cause_text(&stopped.blocked[0], now);
        assert!(row.contains("No space left") && row.contains("10"), "{row}");
        stopped.blocked = vec![Block { engine: Some(Engine::Claude), ..block(Cause::EngineOut, 0) }];
        assert_eq!(headline(&stopped), t!("status.stop_engines"));
        let slowed = status(Mood::Slowed, vec![Block { engine: Some(Engine::Codex), at: Some(now + 3 * 86_400), ..block(Cause::EngineOut, 12) }]);
        assert!(headline(&slowed).contains("Codex"), "{}", headline(&slowed));
        let row = cause_text(&slowed.blocked[0], now);
        assert!(row.contains("12") && row.contains("Codex"), "{row}");
    }

    #[test]
    fn the_second_line_names_the_engines_with_their_limit_and_pacing() {
        let now = 1_800_000_000;
        let mut s = status(Mood::Slowed, vec![]);
        s.notes = vec![Note::Out { engine: Engine::Codex, back_at: Some(now + 3 * 86_400) }, Note::Pacing { engine: Engine::Claude, at_once: 6 }];
        let line = detail(&s, now);
        assert!(line.contains("Codex") && line.contains("3") && line.contains("Claude") && line.contains('6') && line.matches(" · ").count() == 2, "{line}");
        assert!(headline(&s).contains("Claude"), "pacing alone slows the line: {}", headline(&s));
        let good = detail(&status(Mood::Good, vec![]), now);
        assert!(!good.is_empty());
    }

    #[test]
    fn every_cause_has_words_in_every_language_the_app_ships() {
        // Every key the words reach exists in every language (the same test the rest of the app's words go through).
        let problems = equipe_core::i18n::problems(crate::i18n::FILES);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        for cause in [Cause::BreakerOpen, Cause::RunsPaused, Cause::HandedBack, Cause::Decisions, Cause::Approvals, Cause::Quiet, Cause::EngineOut, Cause::Failing, Cause::SignIn, Cause::MacLocked] {
            for why in [None, Some(PauseWhy::EngineLimit), Some(PauseWhy::Retrying), Some(PauseWhy::SignIn), Some(PauseWhy::SquadPaused)] {
                let b = Block { why, engine: Some(Engine::Claude), at: Some(5), detail: Some("x".into()), ..block(cause, 2) };
                let text = cause_text(&b, 1_800_000_000);
                assert!(!text.is_empty() && !text.starts_with("status."), "{cause:?} {why:?}: {text}");
            }
        }
    }

    #[test]
    fn the_budget_line_says_how_far_the_week_lasts_or_when_the_engine_is_back() {
        let now = 1_800_000_000;
        let week = equipe_core::protocol::PlanWindow { used_percent: 60, resets_at: now as i64 + 4 * 86_400 };
        let base = EngineBudget { engine: Engine::Claude, plan_type: None, weekly: Some(week), five_hour: None, out_until: None, runs_out_at: None, lasts: false, pacing_at_once: None, stale: false };
        let out = budget_line(&EngineBudget { out_until: Some(now + 3 * 3600), ..base.clone() }, now);
        let runs_out = budget_line(&EngineBudget { runs_out_at: Some(now + 2 * 86_400), ..base.clone() }, now);
        let lasts = budget_line(&EngineBudget { lasts: true, ..base.clone() }, now);
        let unread = budget_line(&EngineBudget { weekly: None, ..base.clone() }, now);
        let all = [out, runs_out, lasts, unread];
        assert!(all.iter().all(|l| !l.is_empty()) && all.iter().collect::<std::collections::HashSet<_>>().len() == 4, "{all:?}");
    }

    #[test]
    fn waste_values_read_as_tokens_per_call_or_a_share() {
        assert_eq!(waste_value(WasteKey::PmDeskContext, Some(640_000.0)), "640k");
        assert_eq!(waste_value(WasteKey::HeavyShare, Some(12.4)), "12%");
        assert_eq!(waste_value(WasteKey::CancelledShare, Some(0.2)), "<1%");
        assert_eq!(waste_value(WasteKey::CancelledShare, Some(0.0)), "0%");
        assert_eq!(waste_value(WasteKey::FirstHandInAccept, None), "—");
    }
}
