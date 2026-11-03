//! The PM's weekly retro (`crate::retro`) handed over: the routine with the
//! week's facts gathered from the journal, the decision the PM puts next
//! marked as the retro's changes (with the lesson behind each), and the
//! lessons of the changes the person takes kept in the squad's memory —
//! by equipe, not left to the PM to remember.

use equipe_protocol::decide::{Answer, QuestionSet, Retro};
use equipe_protocol::routines::Routine;

use super::*;
use crate::retro::{self as rules, Wait, Week};

impl Runtime {
    /// What the PM is handed for the retro: the routine, then the facts of
    /// the week since the last one.
    pub(super) fn retro_message(&mut self, squad: &str, routine: &Routine, now: u64) -> String {
        let from = rules::since(routine.last_run, now);
        let projects = self.projects(squad);
        // Still waiting on the person: the decisions put to them and not answered.
        let open: Vec<Wait> = self
            .question_sets(squad)
            .iter()
            .map(|s| Wait {
                kind: "decision".into(),
                member: s.member.clone(),
                ticket: s.ticket,
                what: s.questions.first().map(|q| q.question.clone()).unwrap_or_default(),
                asked: s.asked_at,
                answered: None,
                outcome: String::new(),
            })
            .collect();
        let Some(journal) = self.journal(squad) else { return crate::retro::message(routine, "") };
        let tickets = journal.tickets().unwrap_or_default();
        // A week before too: when a member said work would be done may have been said then.
        let events = journal.ticket_events_since(from.saturating_sub(rules::WEEK)).unwrap_or_default();
        let turns = journal.turns_since(from).unwrap_or_default();
        let mut waits = journal.waits_since(from).unwrap_or_default();
        waits.extend(open);
        let facts = rules::gather(&Week { from, to: now, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs: rules::briefs(&projects) });
        crate::daemon_log!("equipe: {squad}: weekly retro {} handed to the PM ({} shipped, {} troubles)", routine.id, facts.shipped.len(), facts.troubles.len());
        rules::message(routine, &rules::facts_text(&facts, &routines::Local))
    }

    /// The retro's mark for the decision `id` puts now — the lesson behind
    /// each change — when it is the one the retro asked for; once.
    pub(super) fn retro_asked(&mut self, id: SessionId, now: u64, args: &Value, questions: usize) -> Option<Retro> {
        rules::is_its_proposal(self.retros.remove(&id), now).then(|| rules::lessons_of(args, questions))
    }

    /// A set answered or closed: how long it waited on the person, for the
    /// next retro; and, the retro's own, the lessons the person confirmed,
    /// into the squad's memory.
    pub(super) fn decision_settled(&mut self, squad: &str, set: &QuestionSet, answers: Option<&[Answer]>) {
        let now = now_secs();
        let wait = Wait {
            kind: "decision".into(),
            member: set.member.clone(),
            ticket: set.ticket,
            what: set.questions.first().map(|q| q.question.clone()).unwrap_or_default(),
            asked: set.asked_at,
            answered: Some(now),
            outcome: if answers.is_some() { "answered" } else { "closed" }.into(),
        };
        if let Some(journal) = self.journal(squad) {
            let _ = journal.record_wait(&wait);
        }
        let lessons = answers.map(|a| rules::confirmed(set, a)).unwrap_or_default();
        if let Some(entry) = rules::entry(&crate::squad_memory::date(now), &lessons) {
            self.learn_lessons(squad, &entry);
        }
    }

    /// Add `entry` to the squad's lessons, as equipe: what the person confirmed.
    fn learn_lessons(&mut self, squad: &str, entry: &str) {
        let folder = crate::squad_home::path(&self.config.home, squad);
        let before = crate::squad_memory::facts(&folder).into_iter().find(|f| f.name == rules::LESSONS).map(|f| f.body);
        let body = rules::with(before.as_deref(), entry);
        let fact = crate::squad_memory::Remember { name: rules::LESSONS, description: rules::LESSONS_DESCRIPTION, kind: "decision", body: &body, confirmed: true };
        let language = self.journals.get(squad).and_then(Journal::language);
        // Saved as equipe's, it isn't listed as a change: "nothing changed" is no failure.
        match crate::squad_memory::remember(&folder, "equipe", &fact, &crate::squad_memory::date(now_secs()), language.as_deref()) {
            Err(why) if !why.starts_with("Nothing changed") => crate::daemon_log!("equipe: {squad}'s lessons weren't kept: {why}"),
            _ => {}
        }
    }
}
