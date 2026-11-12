//! Decisions a member puts to the person (`ask_decision`): questions with
//! options to pick. Asking doesn't hold the member up — the set waits for
//! the person (kept in the squad's journal, so a restart doesn't lose it),
//! and their answers go to the member as their next message, starting a
//! turn the way anything they say does. Closed unanswered, the member is told
//! to go on without them; a member that leaves takes its questions with it.
//! A member on a ticket that ends its turn with a question still open puts the
//! ticket down as waiting ([`Runtime::asked_waiting_note`]); the answer takes
//! it back to the member ([`Runtime::resume_asked`]) rather than leaving it to
//! run out its lease as stalled.
//! Never a way to permission: that stays [`Runtime::ask_permission`]'s.

use equipe_protocol::decide::{self, Answer, QuestionSet, Reaction};

use super::*;
use crate::visuals;

impl Runtime {
    /// `squad`'s question sets waiting for the person, oldest first — read
    /// from its journal the first time, a decision asked more than once
    /// there (before asking again replaced the card) kept once, the latest.
    pub(super) fn question_sets(&mut self, squad: &str) -> &mut Vec<QuestionSet> {
        if !self.questions.contains_key(squad) {
            let kept = self.journal(squad).map(Journal::questions).unwrap_or_default();
            let count = kept.len();
            let once = decide::collapse(kept);
            let collapsed = once.len() != count;
            self.questions.insert(squad.to_owned(), once);
            if collapsed {
                self.save_questions(squad);
            }
        }
        self.questions.entry(squad.to_owned()).or_default()
    }

    fn save_questions(&mut self, squad: &str) {
        let sets = self.questions.get(squad).cloned().unwrap_or_default();
        if let Some(journal) = self.journal(squad) {
            let _ = journal.set_questions(&sets);
        }
    }

    /// Where conversation `id` works, and where its pictures may be: its
    /// ticket's worktree (or its own), and the squad's folder.
    pub(super) fn picture_places(&self, squad: &str, id: SessionId) -> (PathBuf, Vec<PathBuf>) {
        let mut roots = vec![squad_home::path(&self.config.home, squad)];
        let Some(session) = self.sessions.get(&id) else { return (roots[0].clone(), roots) };
        let cwd = session.work.as_ref().map_or_else(|| session.cwd.clone(), |w| w.cwd.clone());
        roots.extend(session.work.as_ref().and_then(|w| w.sandbox.as_ref()).map(|s| s.path.clone()));
        roots.extend(session.sandbox.as_ref().map(|s| s.path.clone()));
        (cwd, roots)
    }

    /// `ask_decision`: the questions checked, kept and shown; the member is
    /// told the answers come as the person's next message.
    pub(super) fn ask_decision(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let mut questions = board::questions_arg(args)?;
        // Each picture found where the member works, or the call is refused with why.
        let (cwd, roots) = self.picture_places(squad, id);
        let fix = "Save the pictures in your worktree or the squad's folder and give their paths, then ask again.";
        for (n, q) in questions.iter_mut().enumerate() {
            q.images = visuals::resolve_all(&q.images, &cwd, &roots, visuals::PER_QUESTION).map_err(|e| format!("Question {}: {e}. {fix}", n + 1))?;
            for (m, option) in q.options.iter_mut().enumerate() {
                option.images = visuals::resolve_all(&option.images, &cwd, &roots, visuals::PER_OPTION).map_err(|e| format!("Question {}, option {}: {e}. {fix}", n + 1, m + 1))?;
            }
        }
        // Their thumbnails made now, off the loop: the card shows them the moment it opens.
        visuals::thumbs::warm(&self.config.home, questions.iter().flat_map(|q| q.images().map(str::to_owned)).collect());
        let nudge = visuals::nudge(&questions);
        // Asked in a conversation beside the member's own: answered there.
        let session = if self.sessions.get(&id).is_some_and(Session::is_working) {
            id
        } else {
            self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && s.is_working()).map_or(id, |(&lane, _)| lane)
        };
        let is_pm = self.sessions.get(&id).is_some_and(Session::is_pm);
        let ticket = self.sessions.get(&session).or(self.sessions.get(&id)).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let review = self.review_asked(id, now_secs());
        let retro = self.retro_asked(id, now_secs(), args, questions.len());
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session, member: member.to_owned(), ticket, questions, asked_at: now_secs(), review, retro };
        let (count, first) = (set.questions.len(), set.questions[0].question.clone());
        // The same decision asked again replaces its card — every screen's —
        // rather than adding a second one.
        let added = decide::add(self.question_sets(squad), set);
        let set = self.question_sets(squad).last().cloned().expect("just added");
        self.save_questions(squad);
        if let decide::Added::Replaced { old } = added {
            self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id: old, answers: None });
        }
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
        let asked = "Asked. The person sees your questions with their options and answers when they can; the answers will \
arrive as their next message (\"Answers to your question: 1) … → …\"). Don't wait for them in this turn: end it now \
with a short line, or carry on with work that doesn't depend on the answer.";
        Ok(match nudge {
            Some(nudge) => format!("{asked}\n\n{nudge}"),
            None => asked.into(),
        })
    }

    /// What `ticket` still asks of the person through open questions and
    /// secret cards, one line each: what it waits on when its member ended its
    /// turn after asking.
    pub(super) fn asked_of_person(&mut self, squad: &str, ticket: TicketId) -> Vec<String> {
        let mut needs: Vec<String> = Vec::new();
        for set in self.question_sets(squad).iter().filter(|s| s.ticket == Some(ticket)) {
            needs.extend(set.questions.iter().map(|q| one_line(&q.question)));
        }
        needs.extend(self.secret_asks(squad).iter().filter(|a| a.ticket == Some(ticket)).map(|a| one_line(&a.why)));
        needs.retain(|n| !n.is_empty());
        needs
    }

    /// The note of a waiting ticket whose member, on session `id`, asked the
    /// person something that is still open and ended its turn without a
    /// hand-in or a wait of its own; `None` when it asked nothing, or the
    /// answers came while its turn ran.
    pub(super) fn asked_waiting_note(&mut self, id: SessionId) -> Option<String> {
        let session = self.sessions.get(&id)?;
        let (squad, work) = (session.squad.clone()?, session.work.as_ref()?);
        if work.handed_in.is_some() || work.preparing {
            return None;
        }
        let ticket = work.ticket;
        let needs = self.asked_of_person(&squad, ticket);
        (!needs.is_empty()).then(|| board::waiting_note(&needs, None))
    }

    /// The person settled `set` (answered or dismissed): when its ticket
    /// waited on exactly what it asked and nothing else is open, the ticket
    /// goes back to its member with `message` — the same conversation,
    /// worktree and engine session — and starts again. Whether it did.
    pub(super) async fn resume_asked(&mut self, squad: &str, set: &QuestionSet, message: &str) -> bool {
        let settled: Vec<String> = set.questions.iter().map(|q| one_line(&q.question)).collect();
        match set.ticket {
            Some(ticket) => self.resume_waiting_on(squad, ticket, &settled, message).await,
            None => false,
        }
    }

    /// `ticket` is waiting on `settled` alone — each item of its note is one
    /// of them — and nothing else it asked is open: resumed with `message`.
    pub(super) async fn resume_waiting_on(&mut self, squad: &str, ticket: TicketId, settled: &[String], message: &str) -> bool {
        let Some(found) = self.journal(squad).and_then(|j| j.ticket(ticket).ok().flatten()) else { return false };
        if found.status != TicketStatus::Waiting {
            return false;
        }
        let needs: Vec<String> = found.note.as_deref().map(board::waiting_needs).unwrap_or_default().iter().map(|n| one_line(n)).collect();
        if needs.is_empty() || !needs.iter().all(|n| settled.contains(n)) || !self.asked_of_person(squad, ticket).is_empty() {
            return false;
        }
        let found = self.resumed(squad, found, "you", message);
        let line = format!("The person answered what T-{} · {} waited on; it is back with {}", found.id, found.title, found.owner.as_deref().unwrap_or("its member"));
        let shown = say!(self, "news.resumed", id = found.id, title = found.title);
        self.brief_pm(squad, line, Some(shown));
        self.dispatch(squad).await;
        true
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
        // A designer's question answered: the person's taste, kept for every design after.
        let designer = self.sessions.get(&set.session).and_then(|s| s.member.as_ref()).is_some_and(|m| m.role == "designer");
        if let Some(entry) = designer.then(|| crate::taste::decided(&set, &answers, &crate::squad_memory::date(now_secs()))).flatten() {
            self.learn_taste(squad, &entry);
        }
        // Its visual picks: the brand's decision record, the chosen picture kept beside each.
        if designer {
            let project = set.ticket.and_then(|t| self.find_ticket(squad, t)).and_then(|t| t.project).filter(|p| crate::design::folder::is_design_dir(&p.join(crate::design::DIR))).map(|p| crate::board::project_name(&p));
            for (decision, picture) in crate::taste::decisions::from_answers(&set, &answers, &crate::squad_memory::date(now_secs())) {
                self.record_decision(squad, project.as_deref(), &decision, Some(std::path::Path::new(&picture)), crate::squad_home::PERSON);
            }
        }
        if let (Some(ticket), Some(journal)) = (set.ticket, self.journal(squad)) {
            let _ = journal.ticket_event(ticket, "you", "decided", &message);
        }
        self.decision_settled(squad, &set, Some(&answers));
        self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id, answers: Some(answers) });
        // Its ticket waited for this answer: it goes back to its member with the answers.
        if self.resume_asked(squad, &set, &message).await {
            return;
        }
        if let Some(to) = self.asker(squad, &set) {
            self.note_screen(to);
            Box::pin(self.submit(to, message)).await;
        }
    }

    /// Add `entry` to the person's design taste in the squad's memory, as
    /// equipe: what they chose and said, not what a member recalls of it.
    pub(super) fn learn_taste(&mut self, squad: &str, entry: &str) {
        let folder = crate::squad_home::path(&self.config.home, squad);
        let before = crate::squad_memory::facts(&folder).into_iter().find(|f| f.name == crate::taste::NAME).map(|f| f.body);
        let body = crate::taste::with(before.as_deref(), entry);
        let fact = crate::squad_memory::Remember { name: crate::taste::NAME, description: crate::taste::DESCRIPTION, kind: "decision", body: &body, confirmed: true };
        let language = self.journals.get(squad).and_then(Journal::language);
        if let Err(why) = crate::squad_memory::remember(&folder, "equipe", &fact, &crate::squad_memory::date(now_secs()), language.as_deref()) {
            crate::daemon_log!("equipe: {squad}'s design taste wasn't kept: {why}");
        }
    }

    /// Closed without an answer: the member goes on without one.
    pub(super) async fn dismiss_questions(&mut self, squad: &str, id: u64) {
        let Some(set) = self.take_questions(squad, id) else { return };
        self.decision_settled(squad, &set, None);
        self.publish(Event::QuestionsClosed { squad: squad.to_owned(), id, answers: None });
        let message = decide::dismissed_message(&set);
        if self.resume_asked(squad, &set, &message).await {
            return;
        }
        if let Some(to) = self.asker(squad, &set) {
            Box::pin(self.submit(to, message)).await;
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

/// `text` on one line, its runs of white space as single spaces.
pub(super) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Answers made to fit their questions: one each (missing ones skipped),
/// picks that exist, once each, in order — one at most where only one may
/// be picked — and words of their own trimmed, none when blank. A mix holds
/// only between two options of one-pick, the aspect one the other has (its
/// base then what is picked); another round, only where there were options
/// (nothing then picked).
pub(super) fn fit(set: &QuestionSet, answers: Vec<Answer>) -> Vec<Answer> {
    let mut answers = answers.into_iter();
    set.questions
        .iter()
        .map(|q| {
            let Answer { picked, other, reaction } = answers.next().unwrap_or_default();
            let mut picked: Vec<usize> = picked.into_iter().filter(|&i| i < q.options.len()).collect();
            if !q.multi_select {
                picked.truncate(1);
            }
            picked.sort_unstable();
            picked.dedup();
            let other = other.map(|o| o.trim().to_owned()).filter(|o| !o.is_empty());
            let reaction = reaction.filter(|r| match r {
                Reaction::Mix { base, from, aspect } => !q.multi_select && base != from && *base < q.options.len() && q.options.get(*from).is_some_and(|f| f.aspects.contains(aspect)),
                Reaction::Again => !q.options.is_empty(),
            });
            match reaction {
                Some(Reaction::Mix { base, .. }) => picked = vec![base],
                Some(Reaction::Again) => picked.clear(),
                None => {}
            }
            Answer { picked, other, reaction }
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
            options: vec![Choice { label: "a".into(), ..Choice::default() }, Choice { label: "b".into(), ..Choice::default() }],
            multi_select,
            ..Question::default()
        };
        let set = QuestionSet { id: 1, session: 1, member: "pm".into(), ticket: None, questions: vec![q(false), q(true), q(false)], asked_at: 0, review: false, retro: None };
        let fitted = fit(&set, vec![Answer { picked: vec![1, 0], other: Some("  ".into()), ..Answer::default() }, Answer { picked: vec![1, 9, 0, 1], other: Some(" x ".into()), ..Answer::default() }]);
        assert_eq!(
            fitted,
            [Answer::pick(1), Answer { picked: vec![0, 1], other: Some("x".into()), ..Answer::default() }, Answer::skipped()],
            "one pick where one may be picked, none out of range, the missing one skipped"
        );
    }

    #[test]
    fn a_mix_holds_between_two_options_on_an_aspect_one_has_and_another_round_picks_nothing() {
        let option = |label: &str| Choice { label: label.into(), aspects: vec!["type".into(), "colour".into()], ..Choice::default() };
        let design = Question { question: "Which?".into(), options: vec![option("A"), option("B"), Choice { label: "C".into(), ..Choice::default() }], ..Question::default() };
        let set = QuestionSet { id: 1, session: 1, member: "designer".into(), ticket: None, questions: vec![design; 5], asked_at: 0, review: false, retro: None };
        let mut stray = Answer::mix(0, 1, "type");
        stray.picked = vec![2];
        let fitted = fit(
            &set,
            vec![stray, Answer::mix(1, 1, "type"), Answer::mix(0, 2, "type"), Answer::mix(0, 1, "shape"), Answer { picked: vec![1], other: Some(" too cold ".into()), reaction: Some(Reaction::Again) }],
        );
        assert_eq!(fitted[0], Answer::mix(0, 1, "type"), "its base is what is picked");
        assert_eq!(fitted[1], Answer::pick(1), "not with itself");
        assert_eq!(fitted[2], Answer::pick(0), "C names nothing to take");
        assert_eq!(fitted[3], Answer::pick(0), "B has no shape to give");
        assert_eq!(fitted[4], Answer::again(Some("too cold".into())), "another round: nothing picked, why kept");
    }
}
