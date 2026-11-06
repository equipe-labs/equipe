//! Members' helpers, as the daemon runs them ([`crate::helpers`]), and the
//! watch on subagents gone quiet ([`crate::subagent_watch`]).
//!
//! `spawn_helper` starts one now — or queues it behind the member's
//! [`helpers::AT_ONCE`] at work — as a turn of its own on the member's
//! Codex, provider and model, in its working folder, read-only, with no MCP
//! server. Its steps show as the member's subagent's; its answer waits for
//! `helper_result`, which holds the member's call until one comes (or
//! [`helpers::Limits::wait`] passes, or the person writes). A helper is
//! stopped at its time or token limit, or by the person, and hands in what
//! it had; the member's turn ending, or the member leaving, stops them all.
//! What they used goes into the member's turn's usage.

use super::*;
use crate::helpers::{self, Answer, Ended, Pool, Shown, Start};
use crate::subagent_watch::{self, Change};

/// One conversation's helpers.
#[derive(Default)]
pub(super) struct Helping {
    pool: Pool,
    runs: HashMap<String, Run>,
    /// Those waiting their turn: what each was called and asked.
    waiting: HashMap<String, (String, String)>,
    /// `helper_result` calls held for an answer.
    waiters: Vec<Waiter>,
    /// What the helpers that ended this turn used.
    used: Usage,
}

/// A helper at work.
struct Run {
    what: String,
    began: std::time::Instant,
    shown: Shown,
    stop: Option<oneshot::Sender<()>>,
    /// Why it is being stopped, once it is.
    why: Option<Ended>,
    task: tokio::task::AbortHandle,
}

/// A `helper_result` held: for `id`'s answer, or any.
struct Waiter {
    key: u64,
    id: Option<String>,
    reply: oneshot::Sender<ToolResult>,
}

/// The board's server for a member with helpers: its calls may run as long
/// as `helper_result` waits.
pub(super) fn offer(server: &mut Value) {
    if let Some(args) = server["args"].as_array_mut() {
        args.push(json!("--helpers"));
    }
    server["tool_timeout_sec"] = json!(helpers::tool_timeout_secs());
}

fn said(text: String) -> ToolResult {
    ToolResult { text, is_error: false, image: None }
}

fn refused(text: impl Into<String>) -> ToolResult {
    ToolResult { text: text.into(), is_error: true, image: None }
}

static NEXT_WAITER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Runtime {
    /// Whether `session`'s engine has no subagents of its own, so equipe's
    /// helpers stand in: Codex's CLI on another provider.
    pub(super) fn helpers_offered(&self, session: &Session) -> bool {
        let other_provider = self.config.nemotron.is_some() && session.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
        helpers::offered(session.engine, session.codex.is_none(), other_provider)
    }

    /// The conversation a member's call is about: its own when that works
    /// now, else a lane of its at work.
    fn working_for(&self, id: SessionId) -> SessionId {
        if self.sessions.get(&id).is_some_and(Session::is_working) {
            return id;
        }
        self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && s.is_working()).map_or(id, |(&lane, _)| lane)
    }

    pub(super) fn spawn_helper(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let Some(member) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(refused("You're not on the squad."));
            return;
        };
        let id = self.working_for(member);
        let Some(session) = self.sessions.get(&id) else { return };
        if !self.helpers_offered(session) {
            let _ = reply.send(refused("Your engine has subagents of its own: hand the work to those."));
            return;
        }
        if !session.is_working() {
            let _ = reply.send(refused("Helpers work during your turn: start them as you work."));
            return;
        }
        let Some(brief) = board::arg_str(&call.arguments, "brief").map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned) else {
            let _ = reply.send(refused("Say what the helper should find out (brief)."));
            return;
        };
        let what = helpers::what_of(board::arg_str(&call.arguments, "what"), &brief);
        let helping = self.helping.entry(id).or_default();
        let (helper, start) = helping.pool.spawn();
        match start {
            Start::Now => self.start_helper(id, &helper, what, brief),
            Start::Queued { .. } => {
                helping.waiting.insert(helper.clone(), (what, brief));
            }
        }
        let _ = reply.send(said(helpers::spawned_text(&helper, start)));
    }

    /// Helper `helper` of conversation `id` sets to work.
    fn start_helper(&mut self, id: SessionId, helper: &str, what: String, brief: String) {
        let Some(session) = self.sessions.get(&id) else { return };
        let limits = helpers::limits();
        let name = session.name().unwrap_or("the member").to_owned();
        let cwd = session.work.as_ref().map_or_else(|| session.cwd.clone(), |w| w.cwd.clone());
        let model = session.ran.clone().or_else(|| session.model.clone());
        let effort = crate::nemotron::agent_effort(effort_of(session).as_deref()).map(str::to_owned);
        let Some(cfg) = self.config.nemotron.as_ref() else { return };
        let provider = cfg.codex_provider();
        let mut env = vec![cfg.codex_env()];
        env.push((guard::MEMBER_ENV.to_owned(), name.clone()));
        let binary = self.config.codex_binary.clone();
        let instructions = helpers::instructions(&name, &cwd, &limits);
        let internal = self.internal.clone();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (session_id, helper_id) = (id, helper.to_owned());
        let asked = brief.clone();
        let task = tokio::spawn(async move {
            let request = helpers::request(&brief, model.as_deref(), effort.as_deref(), &instructions, &env, &provider);
            let forward = internal.clone();
            let (s, h) = (session_id, helper_id.clone());
            let outcome = codex_app::run_turn(&binary, &cwd, request, None, None, Some(stop_rx), move |activity| {
                if matches!(activity, Activity::Text(_) | Activity::Tool { .. } | Activity::ToolDone(_) | Activity::Spent(_)) {
                    let _ = forward.send(Internal::Helper { session: s, id: h.clone(), activity });
                }
            })
            .await;
            let outcome = outcome.map(|turn| turn.result).map_err(|err| err.to_string());
            let _ = internal.send(Internal::HelperEnded { session: session_id, id: helper_id, outcome });
        });
        let internal = self.internal.clone();
        let (session_id, helper_id) = (id, helper.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(limits.time).await;
            let _ = internal.send(Internal::HelperTimeUp { session: session_id, id: helper_id });
        });
        let run = Run { what: what.clone(), began: std::time::Instant::now(), shown: Shown::default(), stop: Some(stop_tx), why: None, task: task.abort_handle() };
        self.helping.entry(id).or_default().runs.insert(helper.to_owned(), run);
        self.publish(Event::SubagentStarted { session: id, id: helper.to_owned(), what, brief: asked });
    }

    /// A helper's step, word or spending: shown as its member's subagent's,
    /// and stopped at its token limit.
    pub(super) fn helper_did(&mut self, id: SessionId, helper: &str, activity: Activity) {
        let Some(run) = self.helping.get_mut(&id).and_then(|h| h.runs.get_mut(helper)) else { return };
        let shown = run.shown.on(helper, activity);
        let over = run.shown.spent >= helpers::limits().tokens;
        for activity in shown {
            if let Some(event) = activity_event(id, activity) {
                self.publish(event);
            }
        }
        if over {
            self.stop_helper(id, helper, Ended::TokenLimit);
        }
    }

    /// Stop a helper at work, for `why`: its engine winds it down, and it
    /// hands in what it had.
    pub(super) fn stop_helper(&mut self, id: SessionId, helper: &str, why: Ended) {
        let Some(run) = self.helping.get_mut(&id).and_then(|h| h.runs.get_mut(helper)) else { return };
        if run.why.is_some() {
            return;
        }
        run.why = Some(why);
        if let Some(stop) = run.stop.take() {
            let _ = stop.send(());
        }
        let task = run.task.clone();
        tokio::spawn(async move {
            tokio::time::sleep(STOP_GRACE).await;
            task.abort();
        });
    }

    /// A helper's turn ended: its answer kept for the member, the next
    /// waiting one started, and anyone waiting on it answered.
    pub(super) fn helper_ended(&mut self, id: SessionId, helper: &str, outcome: Result<TurnResult, String>) {
        let Some(helping) = self.helping.get_mut(&id) else { return };
        let Some(run) = helping.runs.remove(helper) else { return };
        let (text, usage, failed) = match &outcome {
            Ok(result) => (result.text.clone(), result.usage.clone(), None),
            Err(why) => (String::new(), None, Some(why.clone())),
        };
        match &usage {
            Some(u) => {
                helping.used.input_tokens = Some(helping.used.input_tokens.unwrap_or(0) + u.input_tokens.unwrap_or(0));
                helping.used.cache_tokens = Some(helping.used.cache_tokens.unwrap_or(0) + u.cache_tokens.unwrap_or(0));
                helping.used.output_tokens = Some(helping.used.output_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0));
            }
            None => helping.used.input_tokens = Some(helping.used.input_tokens.unwrap_or(0) + run.shown.spent),
        }
        let ended = match (run.why, failed) {
            (Some(why), _) => why,
            (None, Some(why)) => Ended::Failed(why),
            (None, None) => Ended::Answered,
        };
        // Stopped before its answer: all it had said, not only its last words.
        let text = if ended == Ended::Answered { text } else { run.shown.so_far() };
        let answer = Answer { id: helper.to_owned(), what: run.what, ended: ended.clone(), text: text.clone(), took: run.began.elapsed(), steps: run.shown.steps };
        let shown = if ended == Ended::Answered { text } else { helpers::answer_text(&answer, &helpers::limits()) };
        let next = helping.pool.ended(answer);
        let next = next.and_then(|n| helping.waiting.remove(&n).map(|(what, brief)| (n, what, brief)));
        self.publish(Event::SubagentEnded { session: id, id: helper.to_owned(), result: Some(shown), failed: matches!(ended, Ended::Failed(_)) });
        if let Some((next, what, brief)) = next {
            self.start_helper(id, &next, what, brief);
        }
        self.answer_waiters(id);
    }

    pub(super) fn helper_result(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let Some(member) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(refused("You're not on the squad."));
            return;
        };
        let id = self.working_for(member);
        let wanted = board::arg_str(&call.arguments, "id").map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
        let helping = self.helping.entry(id).or_default();
        if let Some(wanted) = &wanted
            && !helping.pool.knows(wanted)
        {
            let _ = reply.send(refused(format!("No helper {wanted} of yours is out: its answer was collected already, or it never was.")));
            return;
        }
        if helping.pool.outstanding().next().is_none() && !helping.pool.has_answers() {
            let _ = reply.send(said("You have no helpers out.".to_owned()));
            return;
        }
        let key = NEXT_WAITER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        helping.waiters.push(Waiter { key, id: wanted, reply });
        self.answer_waiters(id);
        if self.helping.get(&id).is_some_and(|h| h.waiters.iter().any(|w| w.key == key)) {
            let internal = self.internal.clone();
            let wait = helpers::limits().wait;
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let _ = internal.send(Internal::HelperWaitOver(key));
            });
        }
    }

    /// Each waiting `helper_result` whose answer is in, answered.
    fn answer_waiters(&mut self, id: SessionId) {
        let Some(helping) = self.helping.get_mut(&id) else { return };
        let limits = helpers::limits();
        let mut kept = Vec::new();
        for waiter in std::mem::take(&mut helping.waiters) {
            let answers = helping.pool.take(waiter.id.as_deref());
            if answers.is_empty() {
                kept.push(waiter);
                continue;
            }
            let mut text = answers.iter().map(|a| helpers::answer_text(a, &limits)).collect::<Vec<_>>().join("\n\n---\n\n");
            let out = still_out(helping);
            if !out.is_empty() {
                text.push_str(&format!("\n\n---\n\n{out}"));
            }
            let _ = waiter.reply.send(said(text));
        }
        helping.waiters = kept;
    }

    /// A `helper_result` waited its time: which still work.
    pub(super) fn helper_wait_over(&mut self, key: u64) {
        for helping in self.helping.values_mut() {
            if let Some(at) = helping.waiters.iter().position(|w| w.key == key) {
                let waiter = helping.waiters.remove(at);
                let _ = waiter.reply.send(said(format!("No answer yet. {} Call `{}` again to wait on.", still_out(helping), helpers::HELPER_RESULT)));
                return;
            }
        }
    }

    /// What happens to helpers as events pass: the person writing releases
    /// a held `helper_result`; a turn's end or a member leaving stops them,
    /// what they used going into the turn's usage.
    pub(super) fn follow_helpers(&mut self, event: &mut Event) {
        match event {
            Event::SteerQueued { session, .. } => {
                let Some(helping) = self.helping.get_mut(session) else { return };
                let out = still_out(helping);
                for waiter in std::mem::take(&mut helping.waiters) {
                    let _ = waiter.reply.send(said(format!(
                        "The person sent you a message: read it now. Your helpers keep working ({out}); call `{}` again to collect them.",
                        helpers::HELPER_RESULT
                    )));
                }
            }
            Event::TurnEnded { session, usage, .. } => {
                let used = self.stop_helpers(*session);
                if used.total() > 0 {
                    let u = usage.get_or_insert_with(Usage::default);
                    u.input_tokens = Some(u.input_tokens.unwrap_or(0) + used.input_tokens.unwrap_or(0));
                    u.cache_tokens = Some(u.cache_tokens.unwrap_or(0) + used.cache_tokens.unwrap_or(0));
                    u.output_tokens = Some(u.output_tokens.unwrap_or(0) + used.output_tokens.unwrap_or(0));
                }
            }
            Event::SessionClosed { session } => {
                self.stop_helpers(*session);
            }
            _ => {}
        }
    }

    /// Every helper of `id` stopped, none waiting started: what they used.
    fn stop_helpers(&mut self, id: SessionId) -> Usage {
        let Some(mut helping) = self.helping.remove(&id) else { return Usage::default() };
        let mut used = helping.used;
        for helper in helping.pool.stop_all() {
            let Some(mut run) = helping.runs.remove(&helper) else { continue };
            used.input_tokens = Some(used.input_tokens.unwrap_or(0) + run.shown.spent);
            if let Some(stop) = run.stop.take() {
                let _ = stop.send(());
            }
            let task = run.task;
            tokio::spawn(async move {
                tokio::time::sleep(STOP_GRACE).await;
                task.abort();
            });
        }
        for waiter in helping.waiters {
            let _ = waiter.reply.send(said("Your turn ended, and your helpers with it.".to_owned()));
        }
        used
    }

    /// The person stops one subagent of `id`'s: a helper hands in what it
    /// had; one of Codex's own has its turn interrupted.
    pub(super) fn stop_subagent(&mut self, id: SessionId, sub: &str) {
        if self.helping.get(&id).is_some_and(|h| h.runs.contains_key(sub)) {
            return self.stop_helper(id, sub, Ended::Stopped);
        }
        if let Some(stop) = self.sessions.get(&id).and_then(|s| s.stop_subagent.as_ref()) {
            let _ = stop.send(sub.to_owned());
        }
    }

    /// A subagent of Codex's own was heard from.
    pub(super) fn subagent_heard(&mut self, id: SessionId, sub: &str) {
        if let Some(run) = self.sessions.get_mut(&id).and_then(|s| s.subagents.iter_mut().find(|s| s.id == sub)) {
            run.heard = std::time::Instant::now();
        }
    }

    /// The watched subagents gone quiet are told as looking stuck, and those
    /// heard from again as moving. None is stopped for it.
    pub(super) fn watch_subagents(&mut self) {
        let mut told = Vec::new();
        let after = subagent_watch::stuck_after();
        for (&id, session) in &mut self.sessions {
            for sub in session.subagents.iter_mut().filter(|s| s.watched) {
                match subagent_watch::look(sub.heard.elapsed(), sub.stuck, after) {
                    Change::Stuck(quiet) => {
                        sub.stuck = true;
                        told.push(Event::SubagentStuck { session: id, id: sub.id.clone(), quiet_secs: Some(quiet) });
                    }
                    Change::Moving => {
                        sub.stuck = false;
                        told.push(Event::SubagentStuck { session: id, id: sub.id.clone(), quiet_secs: None });
                    }
                    Change::Same => {}
                }
            }
        }
        for event in told {
            self.publish(event);
        }
    }
}

/// Which of a conversation's helpers are still out.
fn still_out(helping: &Helping) -> String {
    let running: Vec<(String, String, Duration)> = helping.runs.iter().map(|(id, r)| (id.clone(), r.what.clone(), r.began.elapsed())).collect();
    let mut running = running;
    running.sort_by(|a, b| a.0.cmp(&b.0));
    let mut waiting: Vec<(String, String)> = helping.waiting.iter().map(|(id, (what, _))| (id.clone(), what.clone())).collect();
    waiting.sort_by(|a, b| a.0.cmp(&b.0));
    helpers::still_out(&running, &waiting)
}
