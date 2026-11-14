//! How a member's turn on a ticket ends when it doesn't end the work — and
//! what equipe does then, so that no ticket sits until its lease runs out
//! and someone finds it stalled:
//!
//! - **The engine failed the turn.** The run waits and goes again by itself,
//!   longer each time ([`BACKOFF`]: 30 seconds, 2 minutes, 10 minutes, then
//!   every 30) — never at once, never in a loop. A failure within seconds with
//!   nothing done (no tool call, no token) is a *fast failure*. When several
//!   of a squad's tickets fail fast within minutes the cause is likely one for
//!   all — a full disk, a lost sign-in, a missing tool, a quota — so the
//!   squad's breaker opens: nothing new starts and nothing is retried, the
//!   person is told once with what the engine said, and one test turn now and
//!   then looks whether the engine works again. The squad goes on by itself
//!   when it does, or as soon as the person says anything to a member.
//! - **The engine reached its plan's limit.** The turn counts as limited, not
//!   completed. The run is paused for that engine, its ticket kept exactly as
//!   it is, and goes on by itself: on the other engine when its member moves
//!   there (the switching rules in `runtime/engines.rs`), or on this one once
//!   it has room again.
//! - **The turn ended with no end to the work** — no hand-in, wait, question,
//!   hand-back or design QA verdict, nothing waiting on the person. equipe
//!   resumes the same conversation once with a short reminder of the ways to
//!   finish (cheap: its context is cached). Ended again the same way, the
//!   ticket goes to the PM with the member's last words as the reason.
//!
//! A turn the person stopped is left as it was: the lease stays the last net.
//! Each of these endings is counted in the squad's journal
//! ([`turn_ending`]), so a change that brings them back shows.

use std::collections::HashSet;

use equipe_protocol::{PauseWhy, RunPause};

use super::*;

/// The codes the squad's journal counts turn endings under.
pub mod turn_ending {
    /// A turn on a ticket ended with no end to the work.
    pub const NO_END: &str = "no_end";
    /// …and equipe resumed it once with the ways to finish.
    pub const NUDGED: &str = "nudged";
    /// …and, ended again so, equipe handed the ticket to the PM.
    pub const TO_PM: &str = "handed_to_pm";
    /// The member handed its ticket back itself (`hand_back`).
    pub const HANDED_BACK: &str = "handed_back";
    /// The engine failed the turn after it had begun.
    pub const FAILED: &str = "failed";
    /// The engine failed the turn within seconds, with nothing done.
    pub const FAILED_FAST: &str = "failed_fast";
    /// The engine reached its plan's limit: the run paused for it.
    pub const LIMITED: &str = "limited";
    /// Several fast failures at once: the squad's starts paused.
    pub const SQUAD_PAUSED: &str = "squad_paused";
}

/// A failure within this many seconds, with nothing done, is a fast one.
pub(super) const FAST_SECS: u64 = 20;
/// How long a run waits before going again after its engine failed, by the
/// failures in a row; the last one repeats.
pub(super) const BACKOFF: [u64; 4] = [30, 2 * 60, 10 * 60, 30 * 60];
/// This many of a squad's tickets failing fast within [`BREAKER_WINDOW`]
/// seconds open its breaker.
pub(super) const BREAKER_TICKETS: usize = 3;
pub(super) const BREAKER_WINDOW: u64 = 5 * 60;
/// The first test turn of an open breaker comes this long after it opened;
/// each that fails doubles the wait, up to [`PROBE_MOST`].
pub(super) const PROBE_FIRST: u64 = 2 * 60;
pub(super) const PROBE_MOST: u64 = 30 * 60;
/// A limit with no known reset is looked at again this long after.
pub(super) const LIMIT_RECHECK: u64 = 30 * 60;
/// After this many failures in a row on one ticket, the person is told (once).
pub(super) const TELL_AFTER: u32 = 3;
/// A limit message an engine ends its turn on is a short line: no longer.
const LIMIT_LINE: usize = 160;
/// What a turn said that is kept to read it: its last characters.
const TEXT_KEPT: usize = 2_000;
/// The member's last words handed to the PM, at most.
const LAST_WORDS: usize = 600;

/// What a session's turn running now has done and said, as its events
/// told it: what equipe weighs when the turn ends.
#[derive(Default)]
pub(super) struct TurnWatch {
    tools: u32,
    error: Option<(String, Option<String>)>,
    text: String,
    /// What the turn was asked — kept to ask it again when its engine
    /// failed before it did anything.
    pub(super) prompt: Option<String>,
    /// The person (or equipe) stopped it.
    pub(super) stopped: bool,
    /// How it ended, until the turn's end is acted on.
    pub(super) ended: Option<Ending>,
}

impl TurnWatch {
    /// A turn starts on `prompt`: nothing of the one before carries over.
    pub(super) fn begin(&mut self, prompt: &str) {
        *self = TurnWatch { prompt: Some(prompt.to_owned()), ..TurnWatch::default() };
    }
}

/// How a turn ended.
pub(super) struct Ending {
    pub(super) outcome: TurnOutcome,
    secs: u64,
    tools: u32,
    tokens: u64,
    error: Option<(String, Option<String>)>,
    text: String,
    prompt: Option<String>,
    /// Its engine said it reached its plan's limit.
    pub(super) limit: bool,
}

impl Ending {
    /// Nothing done: no tool call, no token.
    fn did_nothing(&self) -> bool {
        self.tools == 0 && self.tokens == 0
    }

    /// Failed within seconds with nothing done: the engine never got going.
    pub(super) fn fast(&self) -> bool {
        self.outcome == TurnOutcome::Failed && self.secs <= FAST_SECS && self.did_nothing()
    }

    /// What the engine said, in a line, anything like a key blanked.
    fn cause(&self) -> String {
        match &self.error {
            Some((message, _)) if !message.trim().is_empty() => crate::signals::one_line(message),
            _ => "the engine stopped without saying why".to_owned(),
        }
    }

    /// The end of what it said: its last paragraph.
    fn last_words(&self) -> Option<String> {
        let text = self.text.trim();
        let last = text.rsplit("\n\n").find(|p| !p.trim().is_empty())?.trim();
        Some(clip_chars(last, LAST_WORDS))
    }
}

/// What a run holds about how its turns end, beside its work.
#[derive(Default)]
pub(super) struct RunEnd {
    /// Paused by equipe, to go on by itself.
    pub(super) parked: Option<Park>,
    /// Resumed once already with the ways to finish.
    pub(super) nudged: bool,
    /// Set by `hand_back` (or by equipe): the ticket goes back when the turn ends.
    pub(super) handed_back: Option<HandBack>,
    /// Its browser is in the person's hands, or a sign-in was asked of them,
    /// this turn: its end waits on them.
    pub(super) with_person: bool,
    /// What it last wrote on its ticket's record.
    pub(super) last_comment: Option<String>,
}

/// A run paused by equipe.
pub(super) struct Park {
    why: ParkWhy,
    /// When it was paused.
    at: u64,
    /// When it goes on by itself, if it does on a clock.
    due: Option<u64>,
    /// What it is asked when it goes on.
    prompt: String,
}

/// Whether a turn's end is the engine's limit: it said so, or the engine is out
/// as last read — however the turn failed, except by its sign-in.
fn limit_ending(said: bool, signed_out: bool, engine_out: bool) -> bool {
    said || (engine_out && !signed_out)
}

enum ParkWhy {
    Failed { attempt: u32, cause: String },
    Limit { engine: Engine },
    SignIn { engine: Engine },
}

/// A ticket handed back: why, and what goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HandBack {
    pub(super) reason: Reason,
    /// The tickets it waits on, or duplicates.
    pub(super) tickets: Vec<TicketId>,
    /// Who should take it instead, as the member suggests.
    pub(super) suggest: Option<String>,
    pub(super) note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason {
    /// It can't go on until other tickets are done.
    WaitingOn,
    /// Another ticket does the same, or did it already.
    Duplicate,
    /// Another role's work.
    NotMyRole,
    /// It can't be done as asked.
    Cannot,
    /// equipe's: its turns kept ending with no end to the work.
    NoEnd,
}

impl Reason {
    /// As `hand_back`'s `reason` names it.
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "waiting_on" => Some(Reason::WaitingOn),
            "duplicate" => Some(Reason::Duplicate),
            "not_my_role" => Some(Reason::NotMyRole),
            "cannot" => Some(Reason::Cannot),
            _ => None,
        }
    }

    /// In the PM's words.
    fn said(self) -> &'static str {
        match self {
            Reason::WaitingOn => "waiting on other tickets",
            Reason::Duplicate => "a duplicate",
            Reason::NotMyRole => "not its role's work",
            Reason::Cannot => "can't be done as asked",
            Reason::NoEnd => "its turns kept ending without a hand-in",
        }
    }
}

/// Squad-wide: how failing engines stand.
#[derive(Default)]
pub(super) struct TurnEnds {
    /// Failures in a row, per squad and ticket.
    failures: HashMap<(String, TicketId), u32>,
    /// Each squad's recent fast failures: when, on which ticket, why.
    fast: HashMap<String, Vec<(u64, TicketId, String)>>,
    breakers: HashMap<String, Breaker>,
    /// Tickets whose repeated failures the person was told of.
    told: HashSet<(String, TicketId)>,
    /// Engines out of their limit the person was told of, per squad: until when.
    told_limit: HashMap<(String, Engine), u64>,
    /// What a conversation without a ticket was asked while its engine was out
    /// of its limit: kept, not sent to fail again, and started when the engine
    /// has room or its member moved.
    pub(super) desk_held: HashMap<SessionId, Vec<String>>,
}

impl TurnEnds {
    /// Whether `squad`'s starts are paused: its breaker is open.
    pub(super) fn paused(&self, squad: &str) -> bool {
        self.breakers.contains_key(squad)
    }

    /// `squad`'s open breaker, for the status: what the engine said, and when it opened.
    pub(super) fn breaker(&self, squad: &str) -> Option<crate::status::Breaker> {
        self.breakers.get(squad).map(|b| crate::status::Breaker { cause: b.cause.clone(), since: b.since })
    }
}

struct Breaker {
    cause: String,
    /// When it opened (unix seconds).
    since: u64,
    probe_at: u64,
    gap: u64,
    /// The run whose turn is the test now.
    probing: Option<SessionId>,
}

/// How long to wait after `attempt` failures in a row.
pub(super) fn backoff(attempt: u32) -> u64 {
    BACKOFF[(attempt.max(1) as usize - 1).min(BACKOFF.len() - 1)]
}

/// Whether what an engine said means its plan's limit is reached: Codex's
/// "You've hit your usage limit", Claude Code's "Claude AI usage limit
/// reached" or "You've hit your limit · resets …".
pub(super) fn says_limit(text: &str) -> bool {
    let lower = text.to_lowercase().replace('’', "'");
    [
        "usage limit reached",
        "hit your usage limit",
        "hit your limit",
        "usage_limit_reached",
        "usagelimitexceeded",
        "weekly limit reached",
        "5-hour limit reached",
        "out of extra usage",
        "rate_limit_exceeded",
        "insufficient_quota",
    ]
    .iter()
    .any(|w| lower.contains(w))
}

/// Whether what an engine said means the conversation it was asked to go on
/// with is gone (Claude Code: "No conversation found with session ID: …").
pub(super) fn lost_conversation(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("no conversation found") || lower.contains("conversation not found")
}

fn clip_chars(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(most).collect();
    cut.push('…');
    cut
}

impl Runtime {
    /// Follow a session's turn through its events: what it did and said,
    /// and how it ended. Whether a turn that just ended reached its
    /// engine's limit — the turn is kept as limited, not completed.
    pub(super) fn watch_turn(&mut self, event: &Event) -> bool {
        let Some(id) = event.session() else { return false };
        let Some(session) = self.sessions.get_mut(&id) else { return false };
        let watch = &mut session.watch;
        match event {
            Event::ToolUse { .. } => watch.tools += 1,
            Event::TextDelta { text, .. } => {
                watch.text.push_str(text);
                if watch.text.len() > TEXT_KEPT * 2 {
                    let mut cut = watch.text.len() - TEXT_KEPT;
                    while !watch.text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    watch.text.drain(..cut);
                }
            }
            Event::Error { message, next, .. } if session.turn.is_some() => watch.error = Some((message.clone(), next.clone())),
            Event::TurnEnded { outcome, usage, .. } => {
                let outcome = if watch.stopped { TurnOutcome::Interrupted } else { *outcome };
                let error = watch.error.take();
                let text = std::mem::take(&mut watch.text);
                // The engine's own words on its limit: its error, or a short
                // last line it ended on (Claude Code's limit message).
                let short_end = text.trim().rsplit("\n\n").next().is_some_and(|last| last.chars().count() <= LIMIT_LINE && says_limit(last));
                let limit = outcome != TurnOutcome::Interrupted && (error.as_ref().is_some_and(|(m, _)| says_limit(m)) || short_end);
                let ending = Ending {
                    outcome,
                    secs: session.turn_began.map_or(0, |began| now_secs().saturating_sub(began)),
                    tools: std::mem::take(&mut watch.tools),
                    tokens: usage.as_ref().map_or(0, Usage::total),
                    error,
                    text,
                    prompt: watch.prompt.clone(),
                    limit,
                };
                watch.ended = Some(ending);
                return limit;
            }
            _ => {}
        }
        false
    }

    /// Count a turn ending in `squad`'s journal, and in the daemon's log.
    fn count_ending(&mut self, squad: &str, kind: &str, ticket: Option<TicketId>, member: &str) {
        crate::daemon_log!("equipe: {squad}: turn ending {kind}{} ({member})", ticket.map(|t| format!(" on T-{t}")).unwrap_or_default());
        if let Some(journal) = self.journal(squad) {
            let _ = journal.count_turn_ending(kind, ticket, member);
        }
    }

    /// A turn of run `id` ended well (it didn't fail, and its engine had
    /// room): its ticket's failures in a row are over, and a test turn of
    /// its squad's breaker passed.
    pub(super) fn ended_well(&mut self, id: SessionId) {
        let Some(s) = self.sessions.get(&id) else { return };
        let (Some(squad), Some(ticket)) = (s.squad.clone(), s.work.as_ref().map(|w| w.ticket)) else { return };
        self.turn_ends.failures.remove(&(squad.clone(), ticket));
        self.turn_ends.told.remove(&(squad.clone(), ticket));
        if self.turn_ends.breakers.get(&squad).is_some_and(|b| b.probing == Some(id)) {
            self.close_breaker(&squad);
        }
    }

    /// After a turn: when it ended other than as it should on a ticket — the
    /// engine failed or ran out, or the work has no end — pause the run to go
    /// on by itself, remind it how to finish, or hand its ticket to the PM.
    /// Whether it did any of that (what waits for the run then waits too).
    pub(super) fn unfinished_turn(&mut self, id: SessionId, ending: Option<Ending>) -> bool {
        let Some(ending) = ending else { return false };
        let Some(s) = self.sessions.get(&id) else { return false };
        // A conversation off any ticket (the person's with the PM, a member's
        // own) whose engine can't resume it — gone, or one kept from another
        // engine before a move: asked again at once in a fresh one, opened
        // with the end of the last, instead of failing every message after.
        if s.work.is_none()
            // Only a conversation it was asked to resume: a fresh one can't be lost again.
            && s.claude_session.is_some()
            && ending.outcome == TurnOutcome::Failed
            && ending.error.as_ref().is_some_and(|(m, _)| lost_conversation(m))
            && let Some(prompt) = ending.prompt.clone()
        {
            if let Some(s) = self.sessions.get_mut(&id) {
                s.refresh = Some(true);
            }
            self.start_turn(id, prompt);
            return true;
        }
        let Some(work) = s.work.as_ref().filter(|w| !w.preparing) else { return false };
        let (Some(squad), Some(name)) = (s.squad.clone(), s.name().map(str::to_owned)) else { return false };
        if ending.outcome == TurnOutcome::Interrupted {
            // A test turn stopped tells nothing: the next one comes later.
            self.probe_over(&squad, id);
            return false;
        }
        let (ticket, engine) = (work.ticket, s.engine);
        let failed = ending.outcome == TurnOutcome::Failed;
        // An engine out of its limit fails every turn: that is the limit, not a
        // failure to retry — unless the failure is its sign-in.
        let signed_out = failed && ending.error.as_ref().is_some_and(|(_, next)| crate::engine_auth::lost_by(next.as_deref()).is_some());
        let out = self.engine_out(&squad, engine).is_some();
        if limit_ending(ending.limit, signed_out, out) {
            self.park_for_limit(id, &squad, &name, ticket, engine, &ending);
            return true;
        }
        if failed {
            if let Some(engine) = ending.error.as_ref().and_then(|(_, next)| crate::engine_auth::lost_by(next.as_deref())) {
                // Its sign-in: carried on by `engine_auth` once it works again.
                self.park(id, Park { why: ParkWhy::SignIn { engine }, at: now_secs(), due: None, prompt: String::new() });
                return true;
            }
            self.park_after_failure(id, &squad, &name, ticket, &ending);
            return true;
        }
        // No end to the work. Only a member with the board's tools can end
        // it; without them it says so in words, and the person accepts.
        let Some(s) = self.sessions.get(&id) else { return false };
        let board_tools = has_tools(s) && self.config.equipe_binary.is_some();
        let waits_on_person = self.run_waits_on_person(id, s);
        // What the person said meanwhile goes to it first: that is its next turn.
        let said_meanwhile = !s.queued.is_empty() || !s.steering.is_empty();
        if !board_tools || waits_on_person || said_meanwhile {
            return false;
        }
        let nudged = s.work.as_ref().is_some_and(|w| w.end.nudged);
        let last = s.work.as_ref().and_then(|w| w.end.last_comment.clone()).or_else(|| ending.last_words()).unwrap_or_else(|| "(it said nothing)".to_owned());
        self.count_ending(&squad, turn_ending::NO_END, Some(ticket), &name);
        if !nudged {
            self.nudge(id, &squad, &name, ticket);
            return true;
        }
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
            work.end.handed_back = Some(HandBack { reason: Reason::NoEnd, tickets: Vec::new(), suggest: None, note: last });
        }
        self.end_work(id, WorkEnd::HandedBack);
        true
    }

    /// Resume run `id`'s conversation once with the ways to finish its ticket.
    fn nudge(&mut self, id: SessionId, squad: &str, name: &str, ticket: TicketId) {
        self.count_ending(squad, turn_ending::NUDGED, Some(ticket), name);
        let designer = self.sessions.get(&id).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role == design_qa::DESIGNER);
        let qa = if designer { "\n- a design QA ticket checked: `design_qa` with your verdict;" } else { "" };
        let prompt = format!(
            "[equipe] Your turn on T-{ticket} ended without ending the work, so the ticket would sit unfinished. Every turn on \
a ticket ends in exactly one of these ways:\n- done: `submit_for_review` with a short summary;\n- you need the person: \
`block_on_person` with what they must do (or `ask_decision`, then end your turn, when only their answer is missing);\n- you \
shouldn't or can't do it: `hand_back` — `waiting_on` the tickets it waits for (named in `tickets`), `duplicate`, \
`not_my_role` (with `suggest`), or `cannot` — with a note;{qa}\n- otherwise, keep working on it now, in this turn.\nA command \
you started in the background doesn't wake you once your turn has ended: wait for it within the turn. Don't repeat work \
already done — look at where you are, then end the turn in one of the ways above."
        );
        let text = say!(self, "notice.nudged", id = ticket);
        self.publish(Event::Notice { undo: None, session: id, text });
        self.start_turn(id, prompt);
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
            work.end.nudged = true;
        }
    }

    /// Run `id`'s engine failed its turn: it goes again after a wait, longer
    /// each failure in a row; a fast one counts toward its squad's breaker.
    fn park_after_failure(&mut self, id: SessionId, squad: &str, name: &str, ticket: TicketId, ending: &Ending) {
        // Its conversation can't be resumed (gone, or one from another engine
        // after a move): the next try starts a fresh one at once. That is
        // equipe's own bookkeeping, not an engine failing, so it never counts
        // toward the squad's breaker nor waits out a backoff.
        let lost = ending.error.as_ref().is_some_and(|(m, _)| lost_conversation(m));
        let fast = ending.fast() && !lost;
        let cause = ending.cause();
        let key = (squad.to_owned(), ticket);
        let attempt = {
            let n = self.turn_ends.failures.entry(key.clone()).or_default();
            *n += 1;
            *n
        };
        self.count_ending(squad, if fast { turn_ending::FAILED_FAST } else { turn_ending::FAILED }, Some(ticket), name);
        let now = now_secs();
        let fresh = lost.then(|| self.fresh_conversation(id, squad, ticket)).flatten();
        let started_fresh = fresh.is_some();
        let prompt = match (fresh, ending.prompt.as_deref().filter(|_| ending.did_nothing())) {
            (Some(fresh), _) => fresh,
            (None, Some(asked)) => format!("{asked}\n\n[equipe: your engine failed this turn before it began ({cause}); this is try {}. Go on as asked above.]", attempt + 1),
            (None, None) => format!(
                "[equipe] Your last turn on T-{ticket} stopped when your engine failed ({cause}). Carry on where you left off — \
look at what is already done in your worktree first, and don't redo it."
            ),
        };
        let due = if started_fresh { now } else { now + backoff(attempt) };
        self.park(id, Park { why: ParkWhy::Failed { attempt, cause: cause.clone() }, at: now, due: Some(due), prompt });
        if fast {
            let recent = self.turn_ends.fast.entry(squad.to_owned()).or_default();
            recent.retain(|(at, _, _)| now.saturating_sub(*at) <= BREAKER_WINDOW);
            recent.retain(|(_, t, _)| *t != ticket);
            recent.push((now, ticket, cause.clone()));
            if recent.len() >= BREAKER_TICKETS && !self.turn_ends.paused(squad) {
                self.open_breaker(squad, &cause);
            }
        }
        // A probe that failed again: the breaker waits longer for the next.
        let probe = self.turn_ends.breakers.get_mut(squad).filter(|b| b.probing == Some(id));
        if let Some(b) = probe {
            b.probing = None;
            b.gap = (b.gap * 2).min(PROBE_MOST);
            b.probe_at = now + b.gap;
            let at = b.probe_at;
            self.wake_at(at);
        }
        if attempt >= TELL_AFTER && !self.turn_ends.paused(squad) && self.turn_ends.told.insert(key) {
            let title = self.find_ticket(squad, ticket).map(|t| t.title).unwrap_or_default();
            let shown = say!(self, "news.ticket_failing", id = ticket, cause = cause, title = title);
            let line = format!(
                "T-{ticket}: {name}'s engine failed {attempt} turns in a row ({cause}). equipe keeps trying on its own, waiting longer \
each time; nothing to requeue. If the cause is something the person can fix, tell them."
            );
            self.brief_pm(squad, line, Some(shown));
        }
    }

    /// Run `id`'s engine can't resume the ticket's conversation (it is gone
    /// from where the engine keeps them): it is forgotten, and the next turn
    /// starts a fresh one with the ticket's brief. What that turn is asked.
    fn fresh_conversation(&mut self, id: SessionId, squad: &str, ticket: TicketId) -> Option<String> {
        let found = self.find_ticket(squad, ticket)?;
        let language = self.journals.get(squad).and_then(Journal::language);
        let s = self.sessions.get_mut(&id)?;
        let board_tools = has_tools(s) && self.config.equipe_binary.is_some();
        let work = s.work.as_mut()?;
        work.claude_session = None;
        work.history.clear();
        let mut prompt = work_prompt(&found, &work.project, work.sandbox.as_ref(), None, false, board_tools, language.as_deref());
        prompt.push_str(
            "\n[equipe] Your earlier conversation on this ticket couldn't be resumed, so this is a fresh one on the same branch \
and worktree, nothing reset: look at `git status`, its log and what is already done before you go on, and don't redo it.",
        );
        if let Some(journal) = self.journal(squad) {
            let mut saved = journal.ticket_work(ticket).unwrap_or_default();
            saved.engine_session = None;
            let _ = journal.set_ticket_work(ticket, &saved);
        }
        Some(prompt)
    }

    /// Several of `squad`'s tickets failed fast at once: nothing new starts
    /// and nothing retries until a test turn works; the person is told once.
    fn open_breaker(&mut self, squad: &str, cause: &str) {
        let now = now_secs();
        self.turn_ends.breakers.insert(squad.to_owned(), Breaker { cause: cause.to_owned(), since: now, probe_at: now + PROBE_FIRST, gap: PROBE_FIRST, probing: None });
        self.count_ending(squad, turn_ending::SQUAD_PAUSED, None, "equipe");
        self.status_changed(squad);
        self.wake_at(now + PROBE_FIRST);
        let shown = say!(self, "news.squad_paused", cause = cause);
        let line = format!(
            "equipe: several members' engines failed at once ({cause}) — likely one cause for all, such as a full disk, an expired \
sign-in, a missing tool or a quota. equipe starts nothing new and retries nothing until a test turn works; it tries one by \
itself now and then, and at once when the person says anything to a member. Tell the person plainly what the engine said; \
don't requeue or reassign the paused tickets."
        );
        self.brief_pm(squad, line, Some(shown));
        // Every paused run of the squad says so on its card.
        let parked: Vec<SessionId> = self.parked_runs(squad, |w| matches!(w, ParkWhy::Failed { .. }));
        for run in parked {
            self.show_pause(run);
        }
    }

    /// `squad`'s engines work again: it goes on, every paused run first.
    fn close_breaker(&mut self, squad: &str) {
        if self.turn_ends.breakers.remove(squad).is_none() {
            return;
        }
        self.turn_ends.fast.remove(squad);
        self.status_changed(squad);
        let now = now_secs();
        // The others go again now, a few seconds apart.
        let parked: Vec<SessionId> = self.parked_runs(squad, |w| matches!(w, ParkWhy::Failed { .. }));
        for (n, run) in parked.into_iter().enumerate() {
            if let Some(park) = self.sessions.get_mut(&run).and_then(|s| s.work.as_mut()).and_then(|w| w.end.parked.as_mut()) {
                park.due = Some(now + 5 * n as u64);
            }
            self.show_pause(run);
        }
        self.wake_at(now);
        let shown = say!(self, "news.squad_resumed");
        self.brief_pm(squad, "equipe: the engines work again — a test turn passed; the paused tickets go on by themselves.".into(), Some(shown));
        let _ = self.internal.send(Internal::Dispatch);
    }

    /// `squad`'s test turn on run `id` is over without saying whether the
    /// engines work: the next comes after the breaker's wait.
    fn probe_over(&mut self, squad: &str, id: SessionId) {
        let now = now_secs();
        if let Some(b) = self.turn_ends.breakers.get_mut(squad).filter(|b| b.probing == Some(id)) {
            b.probing = None;
            b.probe_at = now + b.gap;
            let at = b.probe_at;
            self.wake_at(at);
        }
    }

    /// The person acted in `squad` — said something to a member: an open
    /// breaker tries a test turn now, rather than at its next time.
    pub(super) fn person_acted(&mut self, squad: &str) {
        if let Some(b) = self.turn_ends.breakers.get_mut(squad).filter(|b| b.probing.is_none()) {
            b.probe_at = now_secs();
            let _ = self.internal.send(Internal::TurnEndsDue);
        }
    }

    /// Something every squad shares changed for the better (the disk has
    /// room, an engine is signed in again): open breakers test now.
    pub(super) fn try_breakers_now(&mut self) {
        let squads: Vec<String> = self.turn_ends.breakers.keys().cloned().collect();
        for squad in squads {
            self.person_acted(&squad);
        }
    }

    /// Run `id`'s engine reached its plan's limit: the run pauses for it,
    /// to go on on the other engine if its member moves there, or here
    /// once there is room.
    fn park_for_limit(&mut self, id: SessionId, squad: &str, name: &str, ticket: TicketId, engine: Engine, ending: &Ending) {
        self.count_ending(squad, turn_ending::LIMITED, Some(ticket), name);
        let now = now_secs();
        let resets = self.engine_out(squad, engine).map(|at| at.max(0) as u64).filter(|at| *at > now);
        let due = resets.unwrap_or(now + LIMIT_RECHECK);
        let prompt = match ending.prompt.as_deref().filter(|_| ending.did_nothing()) {
            Some(asked) => format!("{asked}\n\n[equipe: your engine was out of its usage limit when this was first asked; it has room again. Go on as asked above.]"),
            None => format!(
                "[equipe] Your engine reached its usage limit during your last turn on T-{ticket}, and it has room again. Carry on \
where you left off — look at what is already done in your worktree first, and don't redo it."
            ),
        };
        self.park(id, Park { why: ParkWhy::Limit { engine }, at: now, due: Some(due), prompt });
        let line = format!(
            "T-{ticket} paused: {name}'s engine ({}) reached its usage limit during the turn. Its branch, worktree and conversation are \
kept; it goes on by itself — on the other engine if {name} moves there, or once this one has room. Don't requeue or reassign it.",
            engine_name(engine)
        );
        // The person hears of an engine's limit once, not of every ticket it cut short.
        let told = self.turn_ends.told_limit.get(&(squad.to_owned(), engine)).is_some_and(|until| *until > now);
        let shown = (!told).then(|| say!(self, "news.limit_paused", engine = engine_name(engine), id = ticket));
        self.turn_ends.told_limit.insert((squad.to_owned(), engine), due);
        self.brief_pm(squad, line, shown);
        // The rules weigh it now: a member may move to the other engine, its
        // ticket handed over; the plan is read again for when it resets.
        match engine {
            Engine::Claude => {
                self.read_claude_usage();
            }
            Engine::Codex => {
                self.read_codex_usage(true);
            }
        }
        self.consider_engines(squad);
        self.show_pause(id);
    }

    /// Pause run `id` as `park` says, its card saying why.
    fn park(&mut self, id: SessionId, park: Park) {
        let due = park.due;
        let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) else { return };
        work.end.parked = Some(park);
        if let Some(at) = due {
            self.wake_at(at);
        }
        self.show_pause(id);
    }

    /// Wake the loop at unix time `at` to see what paused run goes on.
    fn wake_at(&self, at: u64) {
        let internal = self.internal.clone();
        let wait = at.saturating_sub(now_secs());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let _ = internal.send(Internal::TurnEndsDue);
        });
    }

    /// What run `id`'s card says of its pause, as it stands now — kept on
    /// the ticket's run and told to the screens.
    fn show_pause(&mut self, id: SessionId) {
        let Some(s) = self.sessions.get(&id) else { return };
        let (Some(squad), Some(work)) = (s.squad.clone(), s.work.as_ref()) else { return };
        let (ticket, generation) = (work.ticket, work.execution_generation);
        let name = s.name().map(str::to_owned).unwrap_or_default();
        let pause = work.end.parked.as_ref().map(|park| match &park.why {
            _ if self.turn_ends.paused(&squad) && matches!(park.why, ParkWhy::Failed { .. }) => RunPause {
                why: PauseWhy::SquadPaused,
                engine: Some(s.engine),
                resumes_at: None,
                moves_to: None,
                cause: self.turn_ends.breakers.get(&squad).map(|b| b.cause.clone()),
                attempt: 0,
            },
            ParkWhy::Failed { attempt, cause, .. } => {
                RunPause { why: PauseWhy::Retrying, engine: Some(s.engine), resumes_at: park.due, moves_to: None, cause: Some(cause.clone()), attempt: *attempt }
            }
            ParkWhy::Limit { engine } => RunPause {
                why: PauseWhy::EngineLimit,
                engine: Some(*engine),
                resumes_at: park.due,
                moves_to: self.moving_to(&squad, &name),
                cause: None,
                attempt: 0,
            },
            ParkWhy::SignIn { engine } => RunPause { why: PauseWhy::SignIn, engine: Some(*engine), resumes_at: None, moves_to: None, cause: None, attempt: 0 },
        });
        let Some(journal) = self.journal(&squad) else { return };
        // Told again only when it says something new.
        let shown = journal.execution(ticket).ok().flatten().filter(|e| e.generation == generation).map(|e| e.pause);
        if shown.as_ref() == Some(&pause) {
            return;
        }
        if journal.set_execution_pause(ticket, generation, pause.as_ref()).unwrap_or(false)
            && let Some(ticket) = journal.ticket(ticket).ok().flatten()
        {
            self.publish(Event::TicketChanged { squad: squad.clone(), ticket });
            self.status_changed(&squad);
        }
    }

    /// `squad`'s runs paused for a reason `which` picks, oldest pause first.
    fn parked_runs(&self, squad: &str, which: impl Fn(&ParkWhy) -> bool) -> Vec<SessionId> {
        let mut runs: Vec<(u64, SessionId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .filter_map(|(&id, s)| {
                let park = s.work.as_ref()?.end.parked.as_ref()?;
                which(&park.why).then_some((park.due.unwrap_or(u64::MAX), id))
            })
            .collect();
        runs.sort();
        runs.into_iter().map(|(_, id)| id).collect()
    }

    /// Whether run `id` is paused by equipe: it holds its ticket and its
    /// lease meanwhile.
    pub(super) fn is_parked(session: &Session) -> bool {
        session.work.as_ref().is_some_and(|w| w.end.parked.is_some())
    }

    /// As of unix time `now`: every paused run whose time has come goes on —
    /// a retry, unless its squad's breaker holds it; an engine's limit, once
    /// that engine has room and its member isn't moving off it — and each
    /// open breaker's test turn, when due.
    pub(super) fn turn_ends_due(&mut self, now: u64) {
        self.release_desks();
        let parked: Vec<(SessionId, String)> = self
            .sessions
            .iter()
            .filter(|(_, s)| Self::is_parked(s) && !s.is_working())
            .filter_map(|(&id, s)| Some((id, s.squad.clone()?)))
            .collect();
        for (id, squad) in parked {
            let Some(s) = self.sessions.get(&id) else { continue };
            let name = s.name().map(str::to_owned).unwrap_or_default();
            let Some(park) = s.work.as_ref().and_then(|w| w.end.parked.as_ref()) else { continue };
            let due = park.due.is_some_and(|at| at <= now);
            let at = park.at;
            match park.why {
                ParkWhy::Failed { .. } if due && !self.turn_ends.paused(&squad) => self.go_on(id),
                ParkWhy::Limit { engine } => {
                    // Moving to the other engine: the move takes the ticket there.
                    if self.moving_to(&squad, &name).is_some() {
                        self.show_pause(id);
                        continue;
                    }
                    match self.engine_out(&squad, engine).map(|at| at.max(0) as u64) {
                        // Still out, as last read: until it starts over.
                        Some(resets) if resets > now => {
                            if let Some(park) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).and_then(|w| w.end.parked.as_mut())
                                && park.due != Some(resets)
                            {
                                park.due = Some(resets);
                                self.wake_at(resets);
                            }
                            self.show_pause(id);
                        }
                        // Room again — read so, or its time has come and nothing says otherwise.
                        Some(_) => self.go_on(id),
                        None if due || self.engine_room_since(&squad, engine, at) => self.go_on(id),
                        None => {}
                    }
                }
                _ => {}
            }
        }
        // A test turn whose run is gone, or idle and not paused again, is over.
        let lost: Vec<(String, SessionId)> = self
            .turn_ends
            .breakers
            .iter()
            .filter_map(|(squad, b)| b.probing.map(|p| (squad.clone(), p)))
            .filter(|(_, p)| self.sessions.get(p).is_none_or(|s| !s.is_working() && !Self::is_parked(s)))
            .collect();
        for (squad, probe) in lost {
            self.probe_over(&squad, probe);
        }
        // Each open breaker's test turn: the run paused longest goes first.
        let due: Vec<String> = self.turn_ends.breakers.iter().filter(|(_, b)| b.probing.is_none() && b.probe_at <= now).map(|(s, _)| s.clone()).collect();
        for squad in due {
            let probe = self.parked_runs(&squad, |w| matches!(w, ParkWhy::Failed { .. })).into_iter().find(|id| self.sessions.get(id).is_some_and(|s| !s.is_working()));
            match probe {
                Some(id) => {
                    if let Some(b) = self.turn_ends.breakers.get_mut(&squad) {
                        b.probing = Some(id);
                    }
                    self.go_on(id);
                }
                // Nothing left to test with: the squad goes on, and the next failure tells.
                None => self.close_breaker(&squad),
            }
        }
    }

    /// A turn about to start on an engine that is out of its limit isn't
    /// started: it would only fail again. A run pauses for the limit, to go on
    /// where its member moves or once there is room; a conversation without a
    /// ticket keeps what it was asked ([`TurnEnds::desk_held`]). Whether the
    /// turn was held.
    pub(super) fn hold_for_limit(&mut self, id: SessionId, prompt: &str) -> bool {
        let Some(s) = self.sessions.get(&id) else { return false };
        let (Some(squad), true) = (s.squad.clone(), s.member.is_some()) else { return false };
        let engine = s.engine;
        let Some(resets) = self.engine_out(&squad, engine) else { return false };
        let now = now_secs();
        let due = (resets.max(0) as u64).max(now + 1);
        if s.work.is_some() {
            let prompt = prompt.to_owned();
            self.park(id, Park { why: ParkWhy::Limit { engine }, at: now, due: Some(due), prompt });
            self.show_pause(id);
            self.consider_engines(&squad);
            return true;
        }
        let name = s.name().unwrap_or_default().to_owned();
        self.turn_ends.desk_held.entry(id).or_default().push(prompt.to_owned());
        self.wake_at(due);
        self.log_engines(&squad, format!("{name}: a turn waits — {} is out of its usage limit", engine_name(engine)));
        true
    }

    /// Conversations that waited for an engine's limit start now where it has
    /// room (or their member moved to another engine), with all they were asked.
    fn release_desks(&mut self) {
        let ids: Vec<SessionId> = self.turn_ends.desk_held.keys().copied().collect();
        for id in ids {
            let Some(s) = self.sessions.get(&id) else {
                self.turn_ends.desk_held.remove(&id);
                continue;
            };
            let squad = s.squad.clone().unwrap_or_default();
            if s.is_working() || self.engine_out(&squad, s.engine).is_some() {
                continue;
            }
            if let Some(asked) = self.turn_ends.desk_held.remove(&id) {
                self.start_turn(id, asked.join("\n\n"));
            }
        }
    }

    /// Run `id`, paused, goes on now: its pause off its card, and its turn.
    fn go_on(&mut self, id: SessionId) {
        let Some(park) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()).and_then(|w| w.end.parked.take()) else { return };
        self.show_pause(id);
        let asked = self.sessions.get(&id).and_then(|s| s.watch.prompt.clone());
        self.start_turn(id, park.prompt.clone());
        // What it was asked first stays what a retry asks again.
        if let (Some(asked), Some(s)) = (asked, self.sessions.get_mut(&id))
            && matches!(park.why, ParkWhy::Failed { .. } | ParkWhy::Limit { .. })
            && park.prompt.starts_with(asked.as_str())
        {
            s.watch.prompt = Some(asked);
        }
    }

    /// A paused run whose engine's sign-in works again goes on through
    /// `engine_auth`'s carry-on, which starts its turn: its pause is over.
    pub(super) fn unpark_on_turn(&mut self, id: SessionId) {
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut())
            && work.end.parked.take().is_some()
        {
            self.show_pause(id);
        }
    }

    // ── handing a ticket back ──

    /// `hand_back`: the member on a ticket says it can't or shouldn't carry
    /// it — it waits on other tickets, duplicates one, is another role's, or
    /// can't be done. The ticket goes back when the turn ends.
    pub(super) fn hand_back_tool(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let reason = board::arg_str(args, "reason").and_then(Reason::parse).ok_or("Say `reason`: waiting_on, duplicate, not_my_role or cannot.")?;
        let note = board::arg_str(args, "note").map(str::trim).filter(|n| !n.is_empty()).ok_or("Say why in `note`, in a line or two — the PM reads it.")?.to_owned();
        let tickets = board::arg_ids(args, "tickets").unwrap_or_default();
        let suggest = board::arg_str(args, "suggest").map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let Some(work) = self.sessions.get(&id).and_then(|s| s.work.as_ref()) else {
            return Err("You're not on a ticket — there is nothing to hand back.".into());
        };
        let on = work.ticket;
        if work.handed_in.is_some() || work.waiting.is_some() {
            return Err(format!("T-{on} is already handed in or waiting for the person."));
        }
        let board = self.tickets(squad);
        if let Some(missing) = tickets.iter().find(|t| !board.iter().any(|b| b.id == **t)) {
            return Err(format!("There is no T-{missing}."));
        }
        if tickets.contains(&on) {
            return Err(format!("T-{on} is the ticket you are on — name the other tickets."));
        }
        if reason == Reason::WaitingOn {
            if tickets.is_empty() {
                return Err("Name the tickets it waits on in `tickets`.".into());
            }
            let open: Vec<TicketId> = tickets.iter().copied().filter(|t| board.iter().any(|b| b.id == *t && !b.status.is_closed())).collect();
            if open.is_empty() {
                let done: Vec<String> = tickets.iter().map(|t| format!("T-{t}")).collect();
                return Err(format!("{} {} done already — nothing to wait for: carry on.", done.join(", "), if done.len() == 1 { "is" } else { "are" }));
            }
            if let Some(cycle) = board::dependency_loop(&board, on, &open) {
                let path: Vec<String> = cycle.iter().map(|t| format!("T-{t}")).collect();
                return Err(format!("That would wait in a loop: {}. Hand it back as `cannot` and say why instead.", path.join(" → ")));
            }
        }
        let hand = HandBack { reason, tickets: tickets.clone(), suggest, note };
        if let Some(work) = self.sessions.get_mut(&id).and_then(|s| s.work.as_mut()) {
            work.end.handed_back = Some(hand);
        }
        let named = tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        Ok(match reason {
            Reason::WaitingOn => format!(
                "T-{on} goes back on the board when this turn ends, waiting on {named}; your changes are kept on its branch, and it comes \
back to you, in the same worktree, once those are done. End your turn now with a short line."
            ),
            _ => format!(
                "T-{on} goes to the PM when this turn ends, with your note, to cancel, give to someone else or reshape; your changes are kept \
on its branch. End your turn now with a short line."
            ),
        })
    }

    /// What a ticket handed back becomes, as its run puts it down: waiting on
    /// other tickets, back to its member once they are done; otherwise among
    /// the proposals, for the PM. Returns the record's word, the line for
    /// the PM and the news key.
    pub(super) fn handed_back(&mut self, squad: &str, ticket: &mut Ticket, name: &str, hand: &HandBack) -> (&'static str, String, &'static str) {
        let named = hand.tickets.iter().map(|t| format!("T-{t}")).collect::<Vec<_>>().join(", ");
        let id = ticket.id;
        self.count_ending(squad, if hand.reason == Reason::NoEnd { turn_ending::TO_PM } else { turn_ending::HANDED_BACK }, Some(id), name);
        if hand.reason == Reason::WaitingOn {
            for t in &hand.tickets {
                if !ticket.depends_on.contains(t) {
                    ticket.depends_on.push(*t);
                }
            }
            ticket.status = TicketStatus::Todo;
            ticket.assignee = None;
            ticket.owner = Some(name.to_owned());
            ticket.note = Some(format!("{}{named}, which {} closed now. {}", board::WAITED_ON, if hand.tickets.len() == 1 { "is" } else { "are" }, hand.note));
            let line = format!("{name} put T-{id} down to wait on {named}; it goes back to {name} once those are done: {}", hand.note);
            return ("waiting on", line, "news.handed_back_waiting");
        }
        ticket.status = TicketStatus::Proposed;
        ticket.assignee = None;
        let mut why = format!("{} — {}", hand.reason.said(), hand.note);
        if !named.is_empty() {
            why.push_str(&format!(" ({named})"));
        }
        if let Some(who) = &hand.suggest {
            why.push_str(&format!(" Suggests: {who}."));
        }
        ticket.note = Some(format!("{}{why}", crate::protocol::HANDED_BACK));
        let next = match hand.reason {
            Reason::Duplicate => "If it truly duplicates finished or ongoing work, cancel it (`update_ticket`, status cancelled); \
if its branch holds work worth keeping, ask the person first.",
            Reason::NotMyRole => "Give it to the right role or member (`update_ticket` with `owner` and status todo), or propose \
one with `propose_team_change` if nobody on the team fits.",
            Reason::Cannot => "Reshape it so it can be done (`update_ticket`, then status todo), split it, or put it to the person if \
only they can unblock it.",
            Reason::NoEnd => "Look at its record and branch: put it back (`update_ticket`, status todo — the same owner or another) \
with what to do next in `body`, cancel it, or ask the person.",
            Reason::WaitingOn => "",
        };
        let line = format!("{name} handed T-{id} back — {why}. It is a proposal again, its branch kept. {next}");
        let news = match hand.reason {
            Reason::Duplicate => "news.handed_back_duplicate",
            Reason::NotMyRole => "news.handed_back_role",
            Reason::NoEnd => "news.handed_back_unfinished",
            _ => "news.handed_back_cannot",
        };
        ("handed back", line, news)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_turn_on_an_engine_out_of_its_limit_is_limited_not_a_fast_failure() {
        assert!(limit_ending(false, false, true), "failed on an exhausted engine: the limit");
        assert!(limit_ending(true, false, false), "it said so");
        assert!(!limit_ending(false, false, false), "failed with room: a failure to retry");
        assert!(!limit_ending(false, true, true), "its sign-in lost: carried on by the sign-in");
    }

    #[test]
    fn a_failing_engine_waits_longer_each_time_up_to_half_an_hour() {
        let waits: Vec<u64> = (1..=6).map(backoff).collect();
        assert_eq!(waits, [30, 120, 600, 1800, 1800, 1800]);
        assert_eq!(backoff(0), 30, "never at once");
    }

    #[test]
    fn an_engines_own_words_on_its_limit_are_known() {
        for said in [
            "You’ve hit your usage limit. Visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 16th, 2026 8:18 AM.",
            "Claude AI usage limit reached|1900000000",
            "You've hit your limit · resets 3pm (Europe/Paris)",
            "Weekly limit reached ∙ resets Nov 15",
        ] {
            assert!(says_limit(said), "{said}");
        }
        for said in ["Claude Code reported error_during_execution: ENOSPC: no space left on device", "The rate limiter now shows a banner.", "ok"] {
            assert!(!says_limit(said), "{said}");
        }
    }

    #[test]
    fn hand_back_takes_four_reasons_and_nothing_else() {
        assert_eq!(Reason::parse("waiting_on"), Some(Reason::WaitingOn));
        assert_eq!(Reason::parse(" duplicate "), Some(Reason::Duplicate));
        assert_eq!(Reason::parse("not_my_role"), Some(Reason::NotMyRole));
        assert_eq!(Reason::parse("cannot"), Some(Reason::Cannot));
        assert_eq!(Reason::parse("no_end"), None, "equipe's own, not a member's");
    }

    #[test]
    fn a_fast_failure_did_nothing_within_seconds() {
        let ending = |outcome, secs, tools, tokens| Ending { outcome, secs, tools, tokens, error: None, text: String::new(), prompt: None, limit: false };
        assert!(ending(TurnOutcome::Failed, 2, 0, 0).fast());
        assert!(!ending(TurnOutcome::Failed, 2, 1, 0).fast(), "it used a tool");
        assert!(!ending(TurnOutcome::Failed, 300, 0, 0).fast(), "it ran for minutes");
        assert!(!ending(TurnOutcome::Completed, 2, 0, 0).fast());
        assert_eq!(ending(TurnOutcome::Failed, 2, 0, 0).cause(), "the engine stopped without saying why");
        let said = Ending { error: Some(("token ghp_abcdefghijklmnopqrstuvwxyz0123 refused".into(), None)), ..ending(TurnOutcome::Failed, 2, 0, 0) };
        assert!(!said.cause().contains("ghp_abc"), "a key is never told: {}", said.cause());
    }
}
