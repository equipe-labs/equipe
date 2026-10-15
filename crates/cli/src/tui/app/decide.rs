//! A member's questions (`ask_decision`) in the terminal: the set the
//! conversation on screen shows (the member's own; in the PM's, everyone's),
//! answered with single keys while nothing is typed — a number picks (or,
//! where several may be picked, toggles), `o` puts the person's own words in
//! the composer, Enter goes on (sending after the last), `s` skips one, ←
//! goes back.

use std::collections::BTreeSet;

use crossterm::event::KeyCode;
use equipe_core::protocol::decide::{Answer, QuestionSet};
use equipe_core::protocol::{Command, Event};
use equipe_core::t;

use super::{App, Pane};

/// Where the person is in one set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picking {
    pub step: usize,
    pub picks: Vec<BTreeSet<usize>>,
    /// Their own words, by question.
    pub others: Vec<String>,
    pub skipped: BTreeSet<usize>,
    /// `o` was pressed: the composer holds their words for the question on show.
    pub typing: bool,
}

impl Picking {
    pub fn new(set: &QuestionSet) -> Self {
        let n = set.questions.len();
        Self { picks: vec![BTreeSet::new(); n], others: vec![String::new(); n], ..Self::default() }
    }

    fn answered(&self) -> bool {
        self.picks.get(self.step).is_some_and(|p| !p.is_empty()) || self.others.get(self.step).is_some_and(|o| !o.trim().is_empty())
    }

    fn answers(&self, set: &QuestionSet) -> Vec<Answer> {
        (0..set.questions.len())
            .map(|n| {
                if self.skipped.contains(&n) {
                    return Answer::skipped();
                }
                let other = self.others.get(n).map(|o| o.trim().to_owned()).filter(|o| !o.is_empty());
                Answer { picked: self.picks.get(n).map(|p| p.iter().copied().collect()).unwrap_or_default(), other }
            })
            .collect()
    }

    /// On to the next question, or the answers after the last.
    fn on(&mut self, set: &QuestionSet) -> Option<Vec<Answer>> {
        if self.step + 1 >= set.questions.len() {
            return Some(self.answers(set));
        }
        self.step += 1;
        None
    }
}

impl App {
    /// The set the conversation on screen shows, if any.
    pub fn shown_questions(&self) -> Option<&QuestionSet> {
        let agent = self.viewed()?;
        let member = agent.member.as_ref()?;
        let is_pm = agent.is_pm();
        self.questions.iter().find(|s| is_pm || s.member == member.name)
    }

    /// How many sets wait besides the one on show, in this conversation.
    pub fn more_questions(&self) -> usize {
        let Some(agent) = self.viewed() else { return 0 };
        let (is_pm, name) = (agent.is_pm(), agent.member.as_ref().map(|m| m.name.clone()).unwrap_or_default());
        self.questions.iter().filter(|s| is_pm || s.member == name).count().saturating_sub(1)
    }

    /// Where the person is in set `id`.
    pub fn picking_of(&self, id: u64) -> Option<&Picking> {
        self.picking.get(&id)
    }

    /// A set asked, or closed (answered anywhere, closed, its member gone).
    pub(super) fn on_questions(&mut self, event: Event) {
        match event {
            Event::QuestionsAsked { questions, .. } => {
                if !self.questions.iter().any(|s| s.id == questions.id) {
                    self.flash = Some(t!("app.decide_asked", name = questions.member, count = questions.questions.len()));
                    self.questions.push(questions);
                }
            }
            Event::QuestionsClosed { id, .. } => {
                self.questions.retain(|s| s.id != id);
                self.picking.remove(&id);
            }
            _ => {}
        }
    }

    /// A key, while a set shows in the conversation: the card's when nothing is
    /// typed (or the person's own words are, after `o`). `None`: not its key.
    pub(super) fn on_decide_key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Option<Vec<Command>> {
        if self.pane != Pane::Conversation || ctrl || alt {
            return None;
        }
        let set = self.shown_questions()?.clone();
        let typing = self.picking.get(&set.id).is_some_and(|p| p.typing);
        if typing {
            let words = self.composer.text().trim().to_owned();
            let picking = self.picking.get_mut(&set.id)?;
            return match code {
                KeyCode::Enter => {
                    picking.others[picking.step] = words;
                    picking.typing = false;
                    self.composer.clear();
                    Some(self.decide_next(&set, false))
                }
                KeyCode::Esc => {
                    picking.typing = false;
                    self.composer.clear();
                    Some(Vec::new())
                }
                _ => None,
            };
        }
        if !self.composer.is_empty() {
            return None;
        }
        let picking = self.picking.entry(set.id).or_insert_with(|| Picking::new(&set));
        let question = set.questions.get(picking.step)?;
        match code {
            KeyCode::Char(c @ '1'..='9') => {
                let n = c as usize - '1' as usize;
                if n >= question.options.len() {
                    return Some(Vec::new());
                }
                let picks = &mut picking.picks[picking.step];
                if question.multi_select {
                    if !picks.remove(&n) {
                        picks.insert(n);
                    }
                } else {
                    picks.clear();
                    picks.insert(n);
                }
                picking.skipped.remove(&picking.step);
                Some(Vec::new())
            }
            KeyCode::Char('o') => {
                picking.typing = true;
                let kept = picking.others[picking.step].clone();
                self.composer.insert_str(&kept);
                Some(Vec::new())
            }
            KeyCode::Char('s') => Some(self.decide_next(&set, true)),
            KeyCode::Enter => Some(self.decide_next(&set, false)),
            KeyCode::Left if picking.step > 0 => {
                picking.step -= 1;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    /// Enter (or `s`, skipping): the next question — or, after the last, the
    /// answers to the member, the set out of sight at once.
    fn decide_next(&mut self, set: &QuestionSet, skip: bool) -> Vec<Command> {
        let Some(picking) = self.picking.get_mut(&set.id) else { return Vec::new() };
        if skip {
            picking.skipped.insert(picking.step);
        } else if !picking.answered() {
            self.flash = Some(t!("app.decide_pick_first").into());
            return Vec::new();
        } else {
            picking.skipped.remove(&picking.step);
        }
        let Some(answers) = picking.on(set) else { return Vec::new() };
        self.questions.retain(|s| s.id != set.id);
        self.picking.remove(&set.id);
        self.flash = Some(t!("app.decide_sent", name = set.member));
        vec![Command::AnswerQuestions { squad: self.squad.clone(), id: set.id, answers }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use equipe_core::protocol::decide::{Choice, Question};
    use equipe_core::protocol::{Access, Engine, Member, SessionInfo, SessionState, Snapshot};

    /// A squad of its PM, on screen.
    fn pm_app() -> (App, u64) {
        let info = SessionInfo {
            session: 1,
            project: "/repo".into(),
            member: Some(Member { role: "pm".into(), name: "pm".into() }),
            engine: Engine::Claude,
            model: None,
            access: Access::ReadOnly,
            workspace: None,
            state: SessionState::Ready,
            title: None,
            tokens: 0,
            context: None,
            recent: Vec::new(),
            queued: Vec::new(),
            queued_project: None,
            queued_in: Vec::new(),
            steering: Vec::new(),
            estimate: None,
            lane: None,
            work: None,
            mode: None,
            effort: None,
        };
        let snapshot = Snapshot { squad: "acme".into(), projects: vec!["/repo".into()], sessions: vec![info], ..Snapshot::default() };
        (App::from_snapshot("/repo".into(), snapshot, &[]).0, 1)
    }

    fn set(member: &str, session: u64) -> QuestionSet {
        let options = |labels: &[&str]| labels.iter().map(|l| Choice { label: (*l).into(), description: String::new() }).collect();
        QuestionSet {
            id: 7,
            session,
            member: member.into(),
            ticket: None,
            questions: vec![
                Question { question: "Which database?".into(), header: "DB".into(), options: options(&["Postgres (Recommended)", "SQLite"]), multi_select: false },
                Question { question: "Which features first?".into(), header: String::new(), options: options(&["Auth", "Payments", "Search"]), multi_select: true },
            ],
            asked_at: 0,
        }
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<Command> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE), std::time::Instant::now())
    }

    #[test]
    fn numbers_pick_o_takes_their_words_enter_goes_on_and_sends_after_the_last() {
        let (mut app, id) = pm_app();
        app.on_event(Event::QuestionsAsked { squad: "acme".into(), questions: set("pm", id) });
        assert!(app.flash.as_deref().unwrap_or_default().contains("pm"), "{:?}", app.flash);
        assert_eq!(app.shown_questions().map(|s| s.id), Some(7));
        // Enter before a pick says what to do, and stays.
        assert!(press(&mut app, KeyCode::Enter).is_empty());
        assert_eq!(app.picking_of(7).unwrap().step, 0);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.picking_of(7).unwrap().picks[0].iter().copied().collect::<Vec<_>>(), [0], "one pick where one may be picked");
        press(&mut app, KeyCode::Char('9'));
        assert!(press(&mut app, KeyCode::Enter).is_empty());
        assert_eq!(app.picking_of(7).unwrap().step, 1);
        // Several: toggled; then their own words, typed in the composer.
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('o'));
        assert!(app.picking_of(7).unwrap().typing);
        for c in "SSO later".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let sent = press(&mut app, KeyCode::Enter);
        assert_eq!(
            sent,
            vec![Command::AnswerQuestions {
                squad: "acme".into(),
                id: 7,
                answers: vec![Answer { picked: vec![0], other: None }, Answer { picked: vec![2], other: Some("SSO later".into()) }],
            }]
        );
        assert!(app.composer.is_empty() && app.shown_questions().is_none(), "sent: out of sight at once");
    }

    #[test]
    fn s_skips_and_typing_wins_over_the_cards_keys() {
        let (mut app, id) = pm_app();
        app.on_event(Event::QuestionsAsked { squad: "acme".into(), questions: set("pm", id) });
        // Something typed: the keys are the composer's.
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.composer.text(), "h");
        assert!(app.picking_of(7).is_none_or(|p| p.picks[0].is_empty()));
        app.composer.clear();
        assert!(press(&mut app, KeyCode::Char('s')).is_empty());
        let sent = press(&mut app, KeyCode::Char('s'));
        assert_eq!(sent, vec![Command::AnswerQuestions { squad: "acme".into(), id: 7, answers: vec![Answer::skipped(), Answer::skipped()] }]);
    }

    #[test]
    fn a_members_set_shows_in_its_conversation_and_the_pms_and_goes_when_closed_anywhere() {
        let (mut app, id) = pm_app();
        app.on_event(Event::QuestionsAsked { squad: "acme".into(), questions: set("backend", id + 1) });
        assert_eq!(app.shown_questions().map(|s| s.member.as_str()), Some("backend"), "the PM's conversation shows everyone's");
        app.on_event(Event::QuestionsClosed { squad: "acme".into(), id: 7, answers: None });
        assert!(app.shown_questions().is_none());
    }
}
