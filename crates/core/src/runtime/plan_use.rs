//! Where the plans went, as the daemon follows it (`crate::plan_use`): every
//! minute, off the loop, the engines' transcripts on this machine are read on
//! from where they were left, and a ticket — or the use outside equipe — that
//! burns a large share of a week in a few hours is told to every screen once
//! (again to a screen that attaches while it is recent). A screen's look and
//! the PM's `plan_use` (answered here; not offered to the PM until the quality
//! gate passes it, `crate::plan_use::TOOL`) read the same, off the loop too.

use super::*;
use crate::plan_use::{self, transcripts::Roots};
use crate::protocol::PlanWindow;
use crate::protocol::plan_use::{Burn, Span};

/// A burn told is told again only after this long.
const TELL_AGAIN: i64 = 6 * 60 * 60;
/// A burn still shown to a screen that attaches this long after it.
const RECENT: i64 = 3 * 60 * 60;

#[derive(Default)]
pub(super) struct PlanUseWatch {
    /// A pass under way, so they never pile up.
    pub passing: bool,
    /// Burns told, by engine and who, with when.
    pub told: HashMap<String, i64>,
    /// The burns told lately, for a screen that attaches.
    pub recent: Vec<Burn>,
}

impl Runtime {
    /// Whether this daemon reads the machine's transcripts: the person's own
    /// equipe home only — not a test's or a stand-in's — unless
    /// `EQUIPE_PLAN_USE=on` says to.
    fn reads_transcripts(&self) -> bool {
        if !self.config.persist {
            return false;
        }
        match std::env::var("EQUIPE_PLAN_USE").ok().as_deref() {
            Some("on") => true,
            Some("off") => false,
            _ => std::env::var_os("HOME").is_some_and(|h| Path::new(&h).join(".equipe") == self.config.home),
        }
    }

    /// The weekly windows of the plans read.
    fn weeks(&self) -> Vec<(Engine, PlanWindow)> {
        self.plans.iter().filter_map(|p| Some((p.engine, p.usage.seven_day?))).collect()
    }

    /// The minute's pass: read on, then look for burns. Off the loop.
    pub(super) fn plan_use_pass(&mut self) {
        if self.plan_use.passing || !self.reads_transcripts() {
            return;
        }
        self.plan_use.passing = true;
        let (home, weeks, internal) = (self.config.home.clone(), self.weeks(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let now = now_secs() as i64;
            if let Err(e) = plan_use::refresh(&home, &Roots::of_this_machine(), now, None) {
                crate::daemon_log!("equipe: couldn't keep the plans' use: {e}");
            }
            let burns = plan_use::burns(&home, &weeks, now);
            let _ = internal.send(Internal::PlanUsePassed(burns));
        });
    }

    pub(super) fn on_plan_use_passed(&mut self, burns: Vec<Burn>) {
        self.plan_use.passing = false;
        let now = now_secs() as i64;
        self.plan_use.told.retain(|_, at| now - *at < TELL_AGAIN);
        self.plan_use.recent.retain(|b| now - b.at < RECENT);
        for burn in burns {
            let key = format!("{}:{}", plan_use::report::engine_word(burn.engine), plan_use::report::who_key(&burn.who));
            if self.plan_use.told.contains_key(&key) {
                continue;
            }
            self.plan_use.told.insert(key.clone(), now);
            crate::daemon_log!("equipe: {key} burned {}bp of the week in {}h", burn.week_bp, burn.hours);
            self.plan_use.recent.push(burn.clone());
            self.publish(Event::PlanBurn { burn });
        }
    }

    /// The burns a screen that attaches is told.
    pub(super) fn recent_burns(&self) -> Vec<Burn> {
        let now = now_secs() as i64;
        self.plan_use.recent.iter().filter(|b| now - b.at < RECENT).cloned().collect()
    }

    /// A screen's look at `engine`'s plan: read on a little first (the rest
    /// waits for the minute's pass), then tell every screen. Off the loop.
    pub(super) fn plan_use_look(&mut self, engine: Engine, span: Span) {
        let week = self.plans.iter().find(|p| p.engine == engine).and_then(|p| p.usage.seven_day);
        let (home, reads, internal) = (self.config.home.clone(), self.reads_transcripts(), self.internal.clone());
        tokio::task::spawn_blocking(move || {
            let now = now_secs() as i64;
            let read_once = if reads { plan_use::refresh(&home, &Roots::of_this_machine(), now, Some(plan_use::ASKED_BUDGET)).unwrap_or(false) } else { true };
            let report = plan_use::look(&home, engine, span, week, now, !read_once);
            let _ = internal.send(Internal::Event(Box::new(Event::PlanUse { report: Box::new(report) })));
        });
    }

    /// The PM's `plan_use`: each engine's report, in words. Off the loop;
    /// `reply` gets it.
    pub(super) fn plan_use_answer(&mut self, call: &ToolCall, reply: oneshot::Sender<ToolResult>) {
        let is_pm = self.member_id(&call.squad, &call.member).and_then(|id| self.sessions.get(&id)).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role == crate::roles::PM);
        if !is_pm {
            let _ = reply.send(ToolResult::error("Only the PM reads where the plans went; ask the PM."));
            return;
        }
        let span = if call.arguments.get("span").and_then(Value::as_str) == Some("day") { Span::Day } else { Span::Week };
        let mut engines: Vec<(Engine, Option<PlanWindow>)> = self.plans.iter().map(|p| (p.engine, p.usage.seven_day)).collect();
        if !engines.iter().any(|(e, _)| *e == Engine::Claude) {
            engines.insert(0, (Engine::Claude, None));
        }
        let (home, reads) = (self.config.home.clone(), self.reads_transcripts());
        tokio::task::spawn_blocking(move || {
            let now = now_secs() as i64;
            let read_once = if reads { plan_use::refresh(&home, &Roots::of_this_machine(), now, Some(plan_use::ASKED_BUDGET)).unwrap_or(false) } else { true };
            let reports: Vec<_> = engines.into_iter().map(|(engine, week)| plan_use::look(&home, engine, span, week, now, !read_once)).collect();
            let _ = reply.send(ToolResult { text: plan_use::answer(&reports), is_error: false, image: None });
        });
    }
}
