//! Decisions a member puts to the person (`ask_decision`): questions with
//! options to pick. Asking doesn't hold the member up — the set waits for
//! the person (kept in the squad's journal, so a restart doesn't lose it),
//! and their answers go to the member as their next message, starting a
//! turn the way anything they say does. Closed unanswered, the member is told
//! to go on without them; a member that leaves takes its questions with it.
//! Never a way to permission: that stays [`Runtime::ask_permission`]'s.

use equipe_protocol::decide::{self, Answer, QuestionSet};

use super::*;

impl Runtime {
    /// `squad`'s question sets waiting for the person, oldest first — read
    /// from its journal the first time.
    pub(super) fn question_sets(&mut self, squad: &str) -> &mut Vec<QuestionSet> {
        if !self.questions.contains_key(squad) {
            let kept = self.journal(squad).map(Journal::questions).unwrap_or_default();
            self.questions.insert(squad.to_owned(), kept);
        }
        self.questions.entry(squad.to_owned()).or_default()
    }

    fn save_questions(&mut self, squad: &str) {
        let sets = self.questions.get(squad).cloned().unwrap_or_default();
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_questions(&sets);
        }
    }

    /// `ask_decision`: the questions checked, kept and shown; the member is
    /// told the answers come as the person's next message.
    pub(super) fn ask_decision(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let questions = board::questions_arg(args)?;
        // Asked in a conversation beside the member's own: answered there.
        let session = if self.sessions.get(&id).is_some_and(Session::is_working) {
            id
        } else {
            self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && s.is_working()).map_or(id, |(&lane, _)| lane)
        };
        let is_pm = self.sessions.get(&id).is_some_and(Session::is_pm);
        let ticket = self.sessions.get(&session).or(self.sessions.get(&id)).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session, member: member.to_owned(), ticket, questions, asked_at: now_secs() };
        let (count, first) = (set.questions.len(), set.questions[0].question.clone());
        self.question_sets(squad).push(set.clone());
        self.save_questions(squad);
        if let (Some(ticket), Some(journal)) = (ticket, self.journal(squad)) {
            let asked: Vec<&str> = set.questions.iter().map(|q| q.question.as_str()).collect();
            let _ = journal.ticket_event(ticket, member, "asked", &asked.join("\n"));
        }
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: set });
        if !is_pm {
            let line = format!("{member} put a decision to the person: {first}");
            let shown = if count == 1 {
                say!(self, "news.decision.one", name = member, question = clip(&first, 120))
            } else {
                say!(self, "news.decision.other", name = member, count = count, question = clip(&first, 120))
            };
            self.brief_pm(squad, line, Some(shown));
        }
        Ok("Asked. The person sees your questions with their options and answers when they can; the answers will \
arrive as their next message (\"Answers to your question: 1) … → …\"). Don't wait for them in this turn: end it now \
with a short line, or carry on with work that doesn't depend on the answer."
            .into())
    }

    /// Take set `id` out of what waits, if it still does.
    fn take_questions(&mut self, squad: &str, id: u64) -> Option<QuestionSet> {
        let sets = self.question_sets(squad);
        let at = sets.iter().position(|s| s.id == id)?;
        let set = sets.remove(at);
        self.save_questions(squad);
        Some(set)
    }

    /// Where a set's answers go: the conversation that asked, or — gone
    /// since (a lane folded back) — its member's.
    fn asker(&mut self, squad: &str, set: &QuestionSet) -> Option<SessionId> {
        if self.sessions.contains_key(&set.session) {
            return Some(set.session);
        }
        let member = self.member_id(squad, &set.member)?;
        Some(self.route(member))
    }

    /// The person's answers: to the member in one message, which starts its
    /// turn (or waits as a memo while one runs). Answered already, on another
    /// screen: nothing more.
    pub(super) async fn answer_questions(&mut self, squad: &str, id: u64, answers: Vec<Answer>) {
        let Some(set) = self.take_questions(squad, id) else { return };
        let answers = fit(&set, answers);
        let message = decide::answer_message(&set, &answers);
        if let (Some(ticket), Some(journal)) = (set.ticket, self.journal(squad)) {
            let _ = journal.ticket_event(ticket, "you", "decided", &message);
        }
        self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id, answers: Some(answers) });
        if let Some(to) = self.asker(squad, &set) {
            Box::pin(self.submit(to, message)).await;
        }
    }

    /// Closed without an answer: the member goes on without one.
    pub(super) async fn dismiss_questions(&mut self, squad: &str, id: u64) {
        let Some(set) = self.take_questions(squad, id) else { return };
        self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id, answers: None });
        if let Some(to) = self.asker(squad, &set) {
            Box::pin(self.submit(to, decide::dismissed_message(&set))).await;
        }
    }

    /// The sets `which` picks no longer wait — their member left, their
    /// ticket was cancelled. Nobody is told: there is nobody to tell.
    pub(super) fn drop_questions(&mut self, squad: &str, which: impl Fn(&QuestionSet) -> bool) {
        let (gone, kept): (Vec<QuestionSet>, Vec<QuestionSet>) = std::mem::take(self.question_sets(squad)).into_iter().partition(|s| which(s));
        *self.question_sets(squad) = kept;
        if gone.is_empty() {
            return;
        }
        self.save_questions(squad);
        for set in gone {
            self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id: set.id, answers: None });
        }
    }
}

/// Answers made to fit their questions: one each (missing ones skipped),
/// picks that exist, once each, in order — one at most where only one may
/// be picked — and words of their own trimmed, none when blank.
pub(super) fn fit(set: &QuestionSet, answers: Vec<Answer>) -> Vec<Answer> {
    let mut answers = answers.into_iter();
    set.questions
        .iter()
        .map(|q| {
            let Answer { picked, other } = answers.next().unwrap_or_default();
            let mut picked: Vec<usize> = picked.into_iter().filter(|&i| i < q.options.len()).collect();
            if !q.multi_select {
                picked.truncate(1);
            }
            picked.sort_unstable();
            picked.dedup();
            let other = other.map(|o| o.trim().to_owned()).filter(|o| !o.is_empty());
            Answer { picked, other }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_protocol::decide::{Choice, Question};

    #[test]
    fn answers_fit_their_questions() {
        let q = |multi_select| Question {
            question: "?".into(),
            header: String::new(),
            options: vec![Choice { label: "a".into(), description: String::new() }, Choice { label: "b".into(), description: String::new() }],
            multi_select,
        };
        let set = QuestionSet { id: 1, session: 1, member: "pm".into(), ticket: None, questions: vec![q(false), q(true), q(false)], asked_at: 0 };
        let fitted = fit(&set, vec![Answer { picked: vec![1, 0], other: Some("  ".into()) }, Answer { picked: vec![1, 9, 0, 1], other: Some(" x ".into()) }]);
        assert_eq!(
            fitted,
            [Answer { picked: vec![1], other: None }, Answer { picked: vec![0, 1], other: Some("x".into()) }, Answer::skipped()],
            "one pick where one may be picked, none out of range, the missing one skipped"
        );
    }
}
