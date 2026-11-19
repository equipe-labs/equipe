//! The blocks in front of a member's message, said once per conversation and
//! then only as they change ([`turn_context`]), and what each of a PM's
//! turns carried, kept block by block for `equipe usage --pm-context`.

use super::*;

impl Runtime {
    /// `parts` in front of `body` for session `id`'s turn, as its
    /// conversation has been told them so far — whole in a fresh one — and
    /// what each weighed.
    pub(super) fn compose_turn(&mut self, id: SessionId, parts: &[turn_context::Part], body: &str) -> (String, Vec<turn_context::Measure>) {
        let Some(s) = self.sessions.get_mut(&id) else {
            return (turn_context::full(parts, body), Vec::new());
        };
        // The conversation the turn goes on in: its engine session, or the history it carries.
        let (handle, carried) = match &s.work {
            Some(w) => (w.claude_session.clone(), !w.history.is_empty()),
            None => (s.claude_session.clone(), !s.history.is_empty()),
        };
        s.told.begin(handle.as_deref(), handle.is_none() && !carried);
        turn_context::compose(&mut s.told, parts, body)
    }

    /// A PM's desk turn's blocks, its charter's parts beside them, kept in
    /// the squad's journal and said in the daemon's log.
    pub(super) fn keep_turn_context(&self, id: SessionId, parts: &[turn_context::Measure], charter: Option<&str>, memory: Option<&str>, machine: Option<&str>) {
        let Some(s) = self.sessions.get(&id).filter(|s| s.is_pm() && s.work.is_none()) else { return };
        let Some(squad) = s.squad.as_deref() else { return };
        let mut rows: Vec<(&str, usize, usize)> = parts.iter().map(|m| (m.key, m.full, m.sent)).collect();
        if let Some(charter) = charter {
            // The charter is sent with every call; its memory and the machine's credentials apart.
            let (memory, machine) = (memory.map_or(0, str::len), machine.map_or(0, str::len));
            let rest = charter.len().saturating_sub(memory + machine);
            rows.extend([("charter", rest, rest), ("charter.memory", memory, memory), ("charter.machine", machine, machine)].into_iter().filter(|(_, n, _)| *n > 0));
        }
        let (full, sent): (usize, usize) = rows.iter().filter(|(k, ..)| !k.starts_with("charter")).fold((0, 0), |(f, s), (_, a, b)| (f + a, s + b));
        crate::daemon_log!(
            "equipe: {squad}: PM turn context: {} of {} chars in front of the message sent ({})",
            sent,
            full,
            rows.iter().map(|(k, f, s)| if f == s { format!("{k} {s}") } else { format!("{k} {s}/{f}") }).collect::<Vec<_>>().join(", ")
        );
        if let Some(journal) = self.journals.get(squad) {
            let _ = journal.record_turn_context(id, &rows);
        }
    }
}
