//! Context hygiene: a conversation whose context has grown long goes on in a
//! fresh engine session, opened with a packet — so the squad stops paying for
//! (and the model stops wading through) hundreds of thousands of tokens it
//! mostly doesn't need.
//!
//! Every model call re-reads the whole conversation: from the prompt cache, so
//! cheaply per token, but a conversation at a million tokens makes each call
//! cost many times what one at a quarter of that does, and models attend
//! worse to very long contexts. The engines' own compaction is a model call
//! over the full context, mid-turn, and lossy. equipe already knows each
//! conversation's context ([`Session::context`], [`Session::desk_context`]) and
//! already writes a packet that carries a conversation onto a new session when
//! a member changes engine (`engines::rotation_packet`, built from the same
//! parts as the switch's), so at a clean turn boundary it does the same on the
//! same engine: no model call, nothing summarized by a model, the ticket, its
//! criteria and the last exchanges verbatim.
//!
//! Rotation never happens mid-turn (it is decided as a turn starts), while an
//! approval or a question waits on the person, while a hand-in or a wait is
//! open, or while another move is under way. The squad turns it off with
//! `equipe context-config --rotation off`.

use super::*;

/// A conversation at the desk (no ticket: the PM's, and the conversations the
/// person opened with it) goes on in a fresh session once its context reaches
/// this many tokens. Long contexts cost tokens on every call and models attend
/// worse to them; a desk conversation's memory and the board carry what lasts.
pub const ROTATE_DESK_TOKENS: u64 = 250_000;

/// A conversation on a ticket goes on in a fresh session at this many tokens:
/// higher than the desk's, as the work in progress (files read, commands run)
/// is worth keeping a little longer, and the packet has more to carry.
pub const ROTATE_TICKET_TOKENS: u64 = 300_000;

/// Where the engines' own compaction is asked to start for a desk
/// conversation, as a backstop to rotation: rotation works at turn
/// boundaries, but a single turn can run for hours and never reach one —
/// the conversations measured at a million tokens. Above the rotation
/// budget, so that rotation (free, deterministic) goes first when a turn
/// ends and the engine's (a summarizing model call over the whole context)
/// only catches what runs past it.
pub const NATIVE_DESK_TOKENS: u64 = 300_000;

/// The same for a conversation on a ticket.
pub const NATIVE_TICKET_TOKENS: u64 = 360_000;

/// The environment variable Claude Code reads to leave out its 1M-token
/// window variants, so the standard window — and with it the engine's own
/// compaction — comes early.
pub const DISABLE_1M_ENV: &str = "CLAUDE_CODE_DISABLE_1M_CONTEXT";

/// What a turn is run with for its context: Claude Code's 1M window left out
/// unless the squad or the ticket asks for it, and the engine's own
/// compaction asked to start at the budget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Window {
    pub env: Option<(String, String)>,
    pub compact_at: Option<u64>,
}

/// The [`Window`] of a turn of `engine`: `native` is the squad's compaction
/// setting, `large` whether a 1M window is allowed.
pub(super) fn window_for(engine: Engine, on_ticket: bool, native: bool, large: bool) -> Window {
    let budget = native.then_some(if on_ticket { NATIVE_TICKET_TOKENS } else { NATIVE_DESK_TOKENS });
    match engine {
        // On the standard window the engine compacts on its own, early; its
        // window flag matters only where a larger one is allowed.
        Engine::Claude if !large => Window { env: Some((DISABLE_1M_ENV.to_owned(), "1".to_owned())), compact_at: None },
        Engine::Claude => Window { env: None, compact_at: budget },
        Engine::Codex => Window { env: None, compact_at: budget },
    }
}

/// Whether a ticket asks for the large (1M) window: a line of its body that
/// says `context: large`, for work that must hold a big repository in mind.
pub fn wants_large_window(body: &str) -> bool {
    body.lines().any(|l| l.trim().eq_ignore_ascii_case("context: large"))
}

/// Whether a conversation of `used` tokens of context is due to rotate.
pub(super) fn due(on_ticket: bool, used: u64) -> bool {
    used >= if on_ticket { ROTATE_TICKET_TOKENS } else { ROTATE_DESK_TOKENS }
}

impl Runtime {
    /// As a turn of session `id` starts: when its conversation reached its
    /// budget and it is at a clean point, the conversation goes on in a fresh
    /// engine session — its resume handle dropped, the old one kept in the
    /// journal — and the packet it opens with comes back, to open the turn's
    /// `prompt`. `None`: nothing rotates.
    pub(super) fn rotate_context(&mut self, id: SessionId, prompt: &str) -> Option<String> {
        let s = self.sessions.get(&id)?;
        let squad = s.squad.clone()?;
        if !has_tools(s) || s.compacting.is_some() || !s.steering.is_empty() {
            return None;
        }
        let on_ticket = s.work.is_some();
        // The conversation's context as its engine last said, and its engine session.
        let (used, resumable) = if on_ticket {
            (s.context.map_or(0, |(used, _)| used), s.work.as_ref().is_some_and(|w| w.claude_session.is_some() || !w.history.is_empty()))
        } else {
            (s.desk_context.map_or(0, |(used, _)| used), s.claude_session.is_some())
        };
        if !resumable || !due(on_ticket, used) {
            return None;
        }
        if !self.journal(&squad).is_none_or(Journal::context_rotation) {
            return None;
        }
        if self.rotation_blocked(id, &squad) {
            return None;
        }
        let s = self.sessions.get(&id)?;
        let name = s.name().unwrap_or("you").to_owned();
        let mut entries = s.transcript.entries().to_vec();
        // The message this turn answers is already in the transcript: it follows the packet, not twice.
        if matches!(entries.last(), Some(Entry::User { text, .. }) if !text.trim().is_empty() && prompt.contains(text.trim())) {
            entries.pop();
        }
        let estimate = s.estimate.as_ref().and_then(|e| e.live_what.clone().or_else(|| e.what.clone()));
        let ticket_id = s.work.as_ref().map(|w| w.ticket);
        let worktree = s.work.as_ref().map(|w| w.sandbox.as_ref().map_or_else(|| w.cwd.clone(), |sb| sb.path.clone())).map(|p| p.display().to_string());
        let ticket = ticket_id.and_then(|t| self.journal(&squad).and_then(|j| j.ticket(t).ok().flatten()));
        let notes: Vec<String> = match (&ticket, self.journal(&squad)) {
            (Some(ticket), Some(journal)) => journal
                .ticket_history(ticket.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|(by, _, _)| *by == name)
                .map(|(_, kind, text)| format!("{kind}: {}", clip(&text, 400)))
                .rev()
                .take(8)
                .collect(),
            _ => Vec::new(),
        };
        let packet = engines::rotation_packet(&name, used, ticket.as_ref().map(|t| (t, notes.as_slice(), worktree.as_deref())), &entries, estimate, Vec::new());
        // The same notice, spinner and all, as the desk's own fresh conversations.
        self.compaction_began(id, compact::By::Equipe, CompactTrigger::Auto);
        let session = self.sessions.get_mut(&id)?;
        session.context = None;
        if let Some(work) = session.work.as_mut() {
            work.claude_session = None;
            work.history.clear();
        } else {
            session.claude_session = None;
            session.history.clear();
            session.desk_context = None;
        }
        if let Some(journal) = self.journal(&squad) {
            match ticket_id {
                Some(t) => {
                    let mut kept = journal.ticket_work(t).unwrap_or_default();
                    kept.engine_session = None;
                    let _ = journal.set_ticket_work(t, &kept);
                    let _ = journal.ticket_event(t, "equipe", "context refreshed", &format!("{name}'s conversation reached {}k tokens: it goes on in a fresh session, opened with a packet", used / 1000));
                }
                None => {
                    let _ = journal.clear_engine_session(id);
                }
            }
            let _ = journal.record_rotation(id, Some(&name), ticket_id, if on_ticket { "ticket" } else { "desk" }, used);
        }
        let shown = say!(self, "notice.context_rotated");
        self.publish(Event::Notice { undo: None, session: id, text: shown });
        self.compaction_ended(id, None);
        Some(packet)
    }

    /// Whether something open keeps conversation `id` from starting afresh:
    /// an approval or a question waiting on the person, a hand-in or a wait
    /// on its ticket, a move to the other engine that hasn't opened yet.
    fn rotation_blocked(&mut self, id: SessionId, squad: &str) -> bool {
        let Some(s) = self.sessions.get(&id) else { return true };
        let name = s.name().unwrap_or_default().to_owned();
        if s.work.as_ref().is_some_and(|w| w.handed_in.is_some() || w.waiting.is_some() || w.preparing) {
            return true;
        }
        if self.pending.values().any(|p| p.approval.session == id) || self.engines.has_brief(id) {
            return true;
        }
        self.question_sets(squad).iter().any(|q| q.session == id || (!name.is_empty() && q.member == name))
    }
}

// ── obsolete tickets ──

impl Runtime {
    /// Runs still working a ticket that was cancelled meanwhile — however it
    /// was (the person's cancel stops its run at once; this catches the rest):
    /// each is stopped, its session closed and its ticket's record told. Work
    /// on a ticket nobody needs any more is the costliest waste a lane makes.
    pub(super) fn stop_obsolete_runs(&mut self, squad: &str) {
        if !self.journal(squad).is_none_or(Journal::context_obsolete_check) {
            return;
        }
        let working: Vec<(SessionId, TicketId)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.squad.as_deref() == Some(squad))
            .filter_map(|(&id, s)| s.work.as_ref().map(|w| (id, w.ticket)))
            .collect();
        for (id, ticket) in working {
            let cancelled = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()).is_some_and(|t| t.status == TicketStatus::Cancelled);
            if !cancelled {
                continue;
            }
            self.stop_turn(id);
            self.end_work(id, WorkEnd::Cancelled);
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket, "equipe", "stopped early", "its ticket was cancelled while a run worked on it: the run was stopped and its session closed");
            }
        }
    }

    /// Before members start tickets: those that may already be obsolete
    /// ([`crate::obsolete`]) leave `tickets` — held, not started — and the PM
    /// is told once, on its brief. The PM confirms by touching the ticket
    /// (`update_ticket`), or cancels it; a ticket the person started by hand
    /// (`first`) is theirs.
    pub(super) fn hold_obsolete(&mut self, squad: &str, tickets: &mut Vec<Ticket>, first: &[TicketId]) {
        if !self.journal(squad).is_none_or(Journal::context_obsolete_check) {
            return;
        }
        let all = tickets.clone();
        let mut held: Vec<TicketId> = Vec::new();
        for ticket in &all {
            let Some(flag) = crate::obsolete::flag(ticket, &all).filter(|_| !first.contains(&ticket.id)) else { continue };
            let history = self.journal(squad).and_then(|j| j.ticket_history(ticket.id).ok()).unwrap_or_default();
            if crate::obsolete::confirmed(&history) {
                continue;
            }
            held.push(ticket.id);
            if crate::obsolete::held_before(&history) {
                continue;
            }
            let (other, key, why) = match flag {
                crate::obsolete::Flag::CancelledBlocker(other) => (other, "news.obsolete_blocker", format!("T-{other}, which it depends on, was cancelled")),
                crate::obsolete::Flag::Twin(other) => (other, "news.obsolete_duplicate", format!("T-{other} is open with the same title")),
            };
            if let Some(journal) = self.journal(squad) {
                let _ = journal.ticket_event(ticket.id, "equipe", crate::obsolete::HELD, &why);
            }
            let line = format!(
                "T-{} \"{}\" was held, not started: {why}. It starts only after you act on it: update_ticket with just its id lets it start as it is, \
a fix to its dependencies or title lets it start as changed, status cancelled drops it.",
                ticket.id, ticket.title
            );
            let shown = self.said(key, &[("id", ticket.id.to_string()), ("title", ticket.title.clone()), ("other", other.to_string())]);
            self.brief_pm(squad, line, Some(shown));
        }
        tickets.retain(|t| !held.contains(&t.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_leaves_its_1m_window_out_unless_a_large_one_is_allowed_and_codex_only_gets_its_limit() {
        let env = Some((DISABLE_1M_ENV.to_owned(), "1".to_owned()));
        assert_eq!(window_for(Engine::Claude, false, true, false), Window { env: env.clone(), compact_at: None });
        assert_eq!(window_for(Engine::Claude, true, true, true), Window { env: None, compact_at: Some(NATIVE_TICKET_TOKENS) });
        assert_eq!(window_for(Engine::Claude, false, true, true), Window { env: None, compact_at: Some(NATIVE_DESK_TOKENS) });
        assert_eq!(window_for(Engine::Claude, false, false, true), Window { env: None, compact_at: None });
        assert_eq!(window_for(Engine::Codex, true, true, false), Window { env: None, compact_at: Some(NATIVE_TICKET_TOKENS) });
        assert_eq!(window_for(Engine::Codex, true, false, false), Window::default());
        // The engine's backstop sits above rotation, so rotation goes first at a turn boundary.
        const { assert!(NATIVE_DESK_TOKENS > ROTATE_DESK_TOKENS && NATIVE_TICKET_TOKENS > ROTATE_TICKET_TOKENS) };
    }

    #[test]
    fn a_ticket_asks_for_the_large_window_with_a_line_of_its_own() {
        assert!(wants_large_window("Integrate it.\n\ncontext: large\n"));
        assert!(wants_large_window("  Context: Large  "));
        assert!(!wants_large_window("Keep the context large enough to read the plan."));
        assert!(!wants_large_window("context: standard"));
    }
}
