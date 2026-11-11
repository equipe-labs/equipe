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
        let facts = rules::gather(&Week { from, to: now, tickets: &tickets, events: &events, turns: &turns, waits: &waits, briefs: rules::briefs(&crate::squad_home::path(&self.config.home, squad), &projects) });
        crate::daemon_log!("equipe: {squad}: weekly retro {} handed to the PM ({} shipped, {} troubles)", routine.id, facts.shipped.len(), facts.troubles.len());
        let mut facts = rules::facts_text(&facts, &routines::Local);
        // Decisions made in the person's words this week, and where they went wrong — only when there were any.
        if let Some(line) = self.words_week(squad, from) {
            facts.push_str(&format!("\n\n{line}"));
        }
        // How turns on tickets ended other than as they should — only when any did.
        if let Some(line) = self.journal(squad).and_then(|j| j.turn_endings_since(from).ok()).and_then(|counts| endings_line(&counts)) {
            facts.push_str(&format!("\n\n{line}"));
        }
        rules::message(routine, &facts)
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

/// The week's turn endings that weren't as they should be, in a line:
/// `None` when there were none.
fn endings_line(counts: &[(String, u64)]) -> Option<String> {
    if counts.is_empty() {
        return None;
    }
    let listed: Vec<String> = counts.iter().map(|(kind, n)| format!("{} {n}", kind.replace('_', " "))).collect();
    Some(format!(
        "Turns on tickets that didn't end as they should (counted by equipe): {}. Each was carried on by equipe; many of one \
kind say something in how the squad works needs a change.",
        listed.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::endings_line;

    #[test]
    fn the_weeks_turn_endings_are_one_line_when_there_were_any() {
        assert_eq!(endings_line(&[]), None);
        let line = endings_line(&[("no_end".into(), 4), ("failed_fast".into(), 2)]).unwrap();
        assert!(line.contains("no end 4, failed fast 2"), "{line}");
    }
}
