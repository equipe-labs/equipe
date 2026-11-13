//! Tidying the person's conversations with the PM by itself ([`crate::tidy`]):
//! once an hour, and at start, a conversation nobody is waiting on that was
//! left alone for days is put away, as if the person had — kept, listed as
//! put away by equipe, to be taken up again. And one still without a title
//! whose first exchange never finished takes it from the person's first words.

use super::*;
use crate::tidy;

impl Runtime {
    /// One pass over every squad whose journal is open.
    pub(super) async fn tidy_conversations(&mut self) {
        self.tidied_at = Some(std::time::Instant::now());
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            self.title_from_first_words(&squad, now);
            let gone = self.due_conversations(&squad, now);
            if gone.is_empty() {
                continue;
            }
            for id in &gone {
                self.put_one_away(*id, true).await;
            }
            self.publish_put_away(&squad);
        }
    }

    /// The conversations of `squad` that are put away now.
    pub(super) fn due_conversations(&mut self, squad: &str, now: u64) -> Vec<SessionId> {
        let Some(journal) = self.journals.get(squad).filter(|j| j.auto_put_away()) else { return Vec::new() };
        let marks = journal.conversation_marks().unwrap_or_default();
        let mut due: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.kept && s.squad.as_deref() == Some(squad))
            .filter_map(|(&id, s)| {
                let &(last_active, pinned) = marks.get(&id)?;
                let said = s.transcript.entries().iter().any(|e| matches!(e, Entry::User { text, .. } if !text.trim().is_empty() && !harness::only_harness(text)));
                let busy = s.is_working() || s.is_preparing() || !s.queued.is_empty() || !s.steering.is_empty() || self.helping.contains_key(&id);
                let asked = self.waits_on_person(squad, id);
                tidy::due(&tidy::Facts { last_active, pinned, said, busy, asked }, now).then_some(id)
            })
            .collect();
        due.sort_unstable();
        due
    }

    /// Something the conversation put to the person is still waiting for them.
    fn waits_on_person(&self, squad: &str, id: SessionId) -> bool {
        self.questions.get(squad).is_some_and(|sets| sets.iter().any(|q| q.session == id))
            || self.pending.values().any(|p| p.approval.session == id)
            || self.routine_proposals.get(squad).is_some_and(|all| all.iter().any(|r| r.session == id))
            || self.words_live(squad).iter().any(|l| l.session == id)
    }

    /// Put a conversation away — by the person, or (`auto`) by equipe — without
    /// telling the screens the list: the caller does, once.
    pub(super) async fn put_one_away(&mut self, id: SessionId, auto: bool) -> Option<String> {
        let squad = self.sessions.get(&id).filter(|s| s.kept).map(|s| s.squad.clone())?;
        if self.config.persist
            && let Some(journal) = squad.as_deref().and_then(|q| journal_for(&mut self.journals, &self.config.home, q))
        {
            let _ = if auto { journal.close_conversation_auto(id) } else { journal.close_conversation(id) };
        }
        self.close(id).await;
        squad
    }

    /// A conversation with no title and no finished first exchange, quiet for
    /// a while, is named after the person's first words (the engine's own
    /// title, when one comes, is kept: this is only for the one that never will).
    fn title_from_first_words(&mut self, squad: &str, now: u64) {
        let Some(journal) = self.journals.get(squad) else { return };
        let marks = journal.conversation_marks().unwrap_or_default();
        let named: Vec<(SessionId, String)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.kept && s.squad.as_deref() == Some(squad) && s.title.is_none() && !s.title_started && !s.is_working() && s.queued.is_empty())
            .filter(|(id, _)| marks.get(id).is_some_and(|(at, _)| now.saturating_sub(*at) >= tidy::TITLE_AFTER))
            .filter_map(|(&id, s)| {
                let words = s.transcript.entries().iter().find_map(|e| match e {
                    Entry::User { text, .. } if !text.trim().is_empty() && !harness::only_harness(text) => harness::words(text).map(|w| w.into_owned()),
                    _ => None,
                })?;
                Some((id, crate::conversation_title::fallback(&words))).filter(|(_, title)| !title.is_empty())
            })
            .collect();
        for (id, title) in named {
            if self.config.persist
                && let Some(journal) = self.journals.get(squad)
            {
                let _ = journal.set_title_if_empty(id, &title);
            }
            if let Some(s) = self.sessions.get_mut(&id) {
                s.title = Some(title.clone());
            }
            self.publish(Event::ConversationTitle { session: id, title, named: false });
        }
    }

    /// The squad's automatic tidying, on or off, as the person set it.
    pub(super) fn set_auto_put_away(&mut self, squad: String, on: bool) {
        let kept = self.journal(&squad).map(|j| j.set_auto_put_away(on));
        match kept {
            Some(Ok(())) => {
                self.publish(Event::AutoPutAwayChanged { squad, on });
                if on {
                    self.tidied_at = None;
                }
            }
            Some(Err(why)) => crate::daemon_log!("equipe: {squad}'s tidying of conversations wasn't kept: {why}"),
            None => {}
        }
    }

    /// The person pins a conversation, or lets it go.
    pub(super) fn pin_conversation(&mut self, id: SessionId, pinned: bool) {
        let Some(squad) = self.sessions.get(&id).filter(|s| s.kept).and_then(|s| s.squad.clone()) else { return };
        let kept = self.journal(&squad).map(|j| j.set_conversation_pinned(id, pinned));
        if matches!(kept, Some(Ok(true))) {
            self.publish(Event::ConversationPinned { session: id, pinned });
        }
    }
}
