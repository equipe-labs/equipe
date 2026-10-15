//! A member's questions (`ask_decision`) as the card the person answers them
//! on — above the composer in the conversation, and in 나에게's peek: which
//! set shows where, where the person is in it (the step, the row the keys are
//! on, what is picked, what was skipped), what each key does, and the
//! answers it sends. Then, in the conversation, the message those answers
//! went in, read back as "decided". No UI types here, so the rules are
//! tested without a window.

use std::collections::BTreeSet;

use equipe_core::protocol::decide::{self, Answer, Question, QuestionSet};

// ── the sets waiting ──

/// Question sets waiting for the person, oldest first. One answered (or
/// closed) here hides at once, so a second key press can't send it twice;
/// the daemon's `QuestionsClosed` takes it for good.
#[derive(Debug, Clone, Default)]
pub struct Asked {
    sets: Vec<QuestionSet>,
    sent: Vec<u64>,
}

impl Asked {
    /// The snapshot's sets replace everything.
    pub fn load(&mut self, sets: Vec<QuestionSet>) {
        self.sets = sets;
        self.sent.clear();
    }

    pub fn push(&mut self, set: QuestionSet) {
        if !self.sets.iter().any(|s| s.id == set.id) {
            self.sets.push(set);
        }
    }

    /// No longer waiting, whoever answered. Whether it was here.
    pub fn close(&mut self, id: u64) -> bool {
        let had = self.sets.iter().any(|s| s.id == id);
        self.sets.retain(|s| s.id != id);
        self.sent.retain(|s| *s != id);
        had
    }

    /// Answered (or closed) from here: out of sight until the daemon says so.
    pub fn sent(&mut self, id: u64) {
        if !self.sent.contains(&id) {
            self.sent.push(id);
        }
    }

    /// What was sent into a dead link never came back: show it again.
    pub fn forget_sent(&mut self) {
        self.sent.clear();
    }

    /// Every set still waiting, oldest first.
    pub fn open(&self) -> impl Iterator<Item = &QuestionSet> {
        self.sets.iter().filter(|s| !self.sent.contains(&s.id))
    }

    pub fn get(&self, id: u64) -> Option<&QuestionSet> {
        self.open().find(|s| s.id == id)
    }

    pub fn ids(&self) -> Vec<u64> {
        self.open().map(|s| s.id).collect()
    }

    /// Whether `member` waits on the person with a set of its own.
    pub fn from(&self, member: &str) -> Option<&QuestionSet> {
        self.open().find(|s| s.member == member)
    }

    /// The sets a conversation shows, oldest first: the member's own — and,
    /// in the PM's, everyone's (the person lives there).
    pub fn in_conversation(&self, member: &str, is_pm: bool) -> Vec<&QuestionSet> {
        self.open().filter(|s| is_pm || s.member == member).collect()
    }
}

// ── answering one set ──

/// Where the person is in one set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    /// The question on show.
    pub step: usize,
    /// The row the keys are on: an option, or — one past them — the box for
    /// their own words.
    pub cursor: usize,
    /// What is picked, question by question.
    pub picks: Vec<BTreeSet<usize>>,
    /// The questions skipped.
    pub skipped: BTreeSet<usize>,
    /// Which way the last step went (1 on, -1 back), for its transition.
    pub went: i8,
    /// Folded to its one line (Esc); the composer has the keys again.
    pub collapsed: bool,
}

/// A key the card takes while it has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// 1–9: the option of that number (0-based here).
    Pick(usize),
    Up,
    Down,
    /// Picks the row the keys are on (or, in several-pick, toggles it).
    Space,
    /// Picks the row the keys are on when nothing is picked yet, then on.
    Enter,
    /// ⌘↵: on to the next question, or send.
    Next,
    /// Into the box for their own words, and out of it.
    Tab,
    /// ←: the question before.
    Back,
    Esc,
}

/// What a key did, for the window to follow up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Not the card's key.
    Nothing,
    /// The card changed: draw it again.
    Changed,
    /// The box for their own words takes the typing.
    FocusOther,
    /// …and gives it back.
    LeaveOther,
    Collapse,
    /// Every question answered or skipped: send these.
    Send(Vec<Answer>),
}

impl Draft {
    pub fn new(set: &QuestionSet) -> Self {
        Self { picks: vec![BTreeSet::new(); set.questions.len()], ..Self::default() }
    }

    pub fn question<'a>(&self, set: &'a QuestionSet) -> Option<&'a Question> {
        set.questions.get(self.step)
    }

    pub fn is_last(&self, set: &QuestionSet) -> bool {
        self.step + 1 >= set.questions.len()
    }

    /// The row of the box for their own words.
    pub fn other_row(&self, set: &QuestionSet) -> usize {
        self.question(set).map_or(0, |q| q.options.len())
    }

    pub fn picked(&self, n: usize) -> bool {
        self.picks.get(self.step).is_some_and(|p| p.contains(&n))
    }

    /// Option `n` of the question on show: the one picked (several-pick:
    /// toggled), the keys on it. False when there is no such option.
    pub fn pick(&mut self, set: &QuestionSet, n: usize) -> bool {
        let Some(q) = self.question(set) else { return false };
        if n >= q.options.len() {
            return false;
        }
        let multi = q.multi_select;
        if self.picks.len() < set.questions.len() {
            self.picks.resize(set.questions.len(), BTreeSet::new());
        }
        let picks = &mut self.picks[self.step];
        if multi {
            if !picks.remove(&n) {
                picks.insert(n);
            }
        } else {
            picks.clear();
            picks.insert(n);
        }
        self.cursor = n;
        self.skipped.remove(&self.step);
        true
    }

    /// Whether the question on show has an answer: a pick, or words of
    /// their own (`others`, by question).
    pub fn answered(&self, others: &[String]) -> bool {
        self.picks.get(self.step).is_some_and(|p| !p.is_empty()) || others.get(self.step).is_some_and(|o| !o.trim().is_empty())
    }

    /// On to the next question — or, from the last, the answers to send.
    fn on(&mut self, set: &QuestionSet, others: &[String]) -> Option<Vec<Answer>> {
        if self.is_last(set) {
            return Some(self.answers(set, others));
        }
        self.step += 1;
        self.went = 1;
        self.cursor = self.picks.get(self.step).and_then(|p| p.iter().next().copied()).unwrap_or(0);
        None
    }

    /// Next / Submit: on, when the question on show is answered.
    pub fn next(&mut self, set: &QuestionSet, others: &[String]) -> Option<Vec<Answer>> {
        if !self.answered(others) {
            return None;
        }
        self.skipped.remove(&self.step);
        self.on(set, others)
    }

    /// Skip: no answer to this one — the member's call — and on.
    pub fn skip(&mut self, set: &QuestionSet, others: &[String]) -> Option<Vec<Answer>> {
        self.skipped.insert(self.step);
        self.on(set, others)
    }

    /// The question before.
    pub fn back(&mut self) -> bool {
        if self.step == 0 {
            return false;
        }
        self.step -= 1;
        self.went = -1;
        self.cursor = self.picks.get(self.step).and_then(|p| p.iter().next().copied()).unwrap_or(0);
        true
    }

    /// What goes to the member: each question's picks and words, or nothing
    /// for one skipped.
    pub fn answers(&self, set: &QuestionSet, others: &[String]) -> Vec<Answer> {
        (0..set.questions.len())
            .map(|n| {
                if self.skipped.contains(&n) {
                    return Answer::skipped();
                }
                let picked = self.picks.get(n).map(|p| p.iter().copied().collect()).unwrap_or_default();
                let other = others.get(n).map(|o| o.trim().to_owned()).filter(|o| !o.is_empty());
                Answer { picked, other }
            })
            .collect()
    }

    /// A key, while the card has them: `in_other` when the box for their own
    /// words has the typing (then only Tab, Esc, ⌘↵ and ↑ are the card's).
    pub fn key(&mut self, set: &QuestionSet, key: Key, others: &[String], in_other: bool) -> Effect {
        let Some(q) = self.question(set) else { return Effect::Nothing };
        let rows = q.options.len();
        if self.collapsed {
            return Effect::Nothing;
        }
        if in_other {
            return match key {
                Key::Tab | Key::Esc | Key::Up => {
                    self.cursor = rows.saturating_sub(1);
                    Effect::LeaveOther
                }
                Key::Next => self.next(set, others).map_or(Effect::Changed, Effect::Send),
                _ => Effect::Nothing,
            };
        }
        match key {
            Key::Pick(n) => {
                if self.pick(set, n) {
                    Effect::Changed
                } else {
                    Effect::Nothing
                }
            }
            Key::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Effect::Changed
            }
            Key::Down if self.cursor + 1 >= rows => {
                self.cursor = rows;
                Effect::FocusOther
            }
            Key::Down => {
                self.cursor += 1;
                Effect::Changed
            }
            Key::Space if self.cursor >= rows => Effect::FocusOther,
            Key::Space => {
                self.pick(set, self.cursor);
                Effect::Changed
            }
            Key::Enter => {
                if !self.answered(others) && self.cursor < rows {
                    self.pick(set, self.cursor);
                }
                match self.next(set, others) {
                    Some(answers) => Effect::Send(answers),
                    None => Effect::Changed,
                }
            }
            Key::Next => match self.next(set, others) {
                Some(answers) => Effect::Send(answers),
                None => Effect::Changed,
            },
            Key::Tab => {
                self.cursor = rows;
                Effect::FocusOther
            }
            Key::Back => {
                if self.back() {
                    Effect::Changed
                } else {
                    Effect::Nothing
                }
            }
            Key::Esc => {
                self.collapsed = true;
                Effect::Collapse
            }
        }
    }
}

/// The key a keystroke is, while the card has the keys: digits pick, arrows
/// move, Space, ⏎, ⌘⏎, Tab, ←, Esc. `None` for any other.
pub fn key_of(key: &str, cmd: bool, shift: bool) -> Option<Key> {
    match key {
        "enter" if cmd => Some(Key::Next),
        "enter" if !shift => Some(Key::Enter),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Back),
        "space" => Some(Key::Space),
        "tab" => Some(Key::Tab),
        "escape" => Some(Key::Esc),
        digit if !cmd && digit.len() == 1 => digit.parse::<usize>().ok().filter(|n| (1..=9).contains(n)).map(|n| Key::Pick(n - 1)),
        _ => None,
    }
}

// ── read back in the conversation ──

/// The person's answers, as their message to the member shows in the
/// conversation: one line, "결정함 · 2개", that opens to each question and
/// what was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    Answered(Vec<decide::Read>),
    /// Closed without an answer.
    Closed,
}

/// The decision a message of the person's carries, if it is one.
pub fn decided(message: &str) -> Option<Decided> {
    if decide::is_dismissed(message) {
        return Some(Decided::Closed);
    }
    decide::read_answers(message).map(Decided::Answered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use equipe_core::protocol::decide::Choice;

    fn set() -> QuestionSet {
        let options = |labels: &[&str]| labels.iter().map(|l| Choice { label: (*l).into(), description: String::new() }).collect();
        QuestionSet {
            id: 7,
            session: 1,
            member: "pm".into(),
            ticket: None,
            questions: vec![
                Question { question: "Which database?".into(), header: "Database".into(), options: options(&["Postgres (Recommended)", "SQLite", "MySQL"]), multi_select: false },
                Question { question: "Which features first?".into(), header: "Scope".into(), options: options(&["Auth", "Payments", "Search"]), multi_select: true },
            ],
            asked_at: 0,
        }
    }

    fn others(n: usize) -> Vec<String> {
        vec![String::new(); n]
    }

    #[test]
    fn a_number_picks_one_and_steps_on_then_several_are_toggled_and_sent() {
        let s = set();
        let mut d = Draft::new(&s);
        let none = others(2);
        assert_eq!(d.key(&s, Key::Next, &none, false), Effect::Changed, "nothing picked: Next stays");
        assert_eq!(d.step, 0);
        assert_eq!(d.key(&s, Key::Pick(1), &none, false), Effect::Changed);
        assert_eq!(d.key(&s, Key::Pick(0), &none, false), Effect::Changed);
        assert!(d.picked(0) && !d.picked(1), "one pick only where one may be picked");
        assert_eq!(d.key(&s, Key::Pick(5), &none, false), Effect::Nothing, "no such option");
        assert_eq!(d.key(&s, Key::Next, &none, false), Effect::Changed);
        assert_eq!((d.step, d.went, d.cursor), (1, 1, 0));
        // Several: toggled; Space toggles the row the keys are on.
        d.key(&s, Key::Pick(0), &none, false);
        d.key(&s, Key::Pick(2), &none, false);
        d.key(&s, Key::Pick(0), &none, false);
        assert_eq!(d.cursor, 0, "the keys follow the pick");
        d.key(&s, Key::Down, &none, false);
        d.key(&s, Key::Down, &none, false);
        assert_eq!(d.key(&s, Key::Down, &none, false), Effect::FocusOther);
        assert_eq!(d.cursor, 3, "past the options: the box for their own words");
        d.key(&s, Key::Up, &none, false);
        d.key(&s, Key::Up, &none, false);
        assert_eq!(d.cursor, 1);
        d.key(&s, Key::Space, &none, false);
        assert_eq!(d.picks[1].iter().copied().collect::<Vec<_>>(), [1, 2]);
        let sent = d.key(&s, Key::Next, &none, false);
        assert_eq!(sent, Effect::Send(vec![Answer { picked: vec![0], other: None }, Answer { picked: vec![1, 2], other: None }]));
    }

    #[test]
    fn enter_picks_the_row_the_keys_are_on_when_nothing_is_picked() {
        let s = set();
        let mut d = Draft::new(&s);
        let none = others(2);
        d.key(&s, Key::Down, &none, false);
        assert_eq!(d.key(&s, Key::Enter, &none, false), Effect::Changed);
        assert_eq!((d.step, d.picks[0].iter().copied().collect::<Vec<_>>()), (1, vec![1]));
        // Back keeps what was picked, and the keys go to it.
        assert_eq!(d.key(&s, Key::Back, &none, false), Effect::Changed);
        assert_eq!((d.step, d.cursor, d.went), (0, 1, -1));
        assert_eq!(d.key(&s, Key::Back, &none, false), Effect::Nothing, "nothing before the first");
    }

    #[test]
    fn their_own_words_answer_alone_or_beside_a_pick_and_tab_and_esc_go_in_and_out() {
        let s = set();
        let mut d = Draft::new(&s);
        assert_eq!(d.key(&s, Key::Tab, &others(2), false), Effect::FocusOther);
        assert_eq!(d.cursor, 3);
        let typed = vec!["MariaDB, we run one".to_owned(), String::new()];
        // In the box, digits are its own; ⌘↵ goes on.
        assert_eq!(d.key(&s, Key::Pick(1), &typed, true), Effect::Nothing);
        assert_eq!(d.key(&s, Key::Next, &typed, true), Effect::Changed);
        assert_eq!(d.step, 1);
        d.key(&s, Key::Pick(0), &typed, false);
        let mut both = typed.clone();
        both[1] = " SSO later ".into();
        assert_eq!(d.key(&s, Key::Esc, &both, true), Effect::LeaveOther, "Esc in the box gives the keys back");
        assert_eq!(
            d.key(&s, Key::Next, &both, false),
            Effect::Send(vec![Answer { picked: vec![], other: Some("MariaDB, we run one".into()) }, Answer { picked: vec![0], other: Some("SSO later".into()) }])
        );
    }

    #[test]
    fn a_skipped_question_sends_nothing_for_it_and_esc_folds_the_card() {
        let s = set();
        let mut d = Draft::new(&s);
        let none = others(2);
        assert_eq!(d.skip(&s, &none), None);
        assert_eq!(d.step, 1);
        d.pick(&s, 2);
        assert_eq!(d.skip(&s, &none), Some(vec![Answer::skipped(), Answer::skipped()]), "skipped, though picked");
        // Picking again after a skip answers it.
        let mut d = Draft::new(&s);
        d.skip(&s, &none);
        d.back();
        d.pick(&s, 1);
        d.next(&s, &none);
        assert!(!d.skipped.contains(&0));
        assert_eq!(d.key(&s, Key::Esc, &none, false), Effect::Collapse);
        assert!(d.collapsed);
        assert_eq!(d.key(&s, Key::Pick(0), &none, false), Effect::Nothing, "folded: the keys are the composer's");
    }

    #[test]
    fn keystrokes_read_as_the_cards_keys() {
        assert_eq!(key_of("1", false, false), Some(Key::Pick(0)));
        assert_eq!(key_of("9", false, false), Some(Key::Pick(8)));
        assert_eq!(key_of("0", false, false), None);
        assert_eq!(key_of("enter", true, false), Some(Key::Next));
        assert_eq!(key_of("enter", false, false), Some(Key::Enter));
        assert_eq!(key_of("enter", false, true), None, "⇧⏎ is the composer's");
        assert_eq!((key_of("space", false, false), key_of("tab", false, false), key_of("escape", false, false)), (Some(Key::Space), Some(Key::Tab), Some(Key::Esc)));
        assert_eq!(key_of("a", false, false), None);
    }

    #[test]
    fn a_set_shows_in_its_members_conversation_and_the_pms_and_hides_once_sent() {
        let mut asked = Asked::default();
        let mut a = set();
        a.member = "backend".into();
        let mut b = set();
        b.id = 8;
        asked.load(vec![a.clone(), b.clone()]);
        asked.push(a.clone());
        assert_eq!(asked.ids(), [7, 8], "not twice");
        assert_eq!(asked.in_conversation("backend", false).iter().map(|s| s.id).collect::<Vec<_>>(), [7]);
        assert_eq!(asked.in_conversation("pm", true).iter().map(|s| s.id).collect::<Vec<_>>(), [7, 8], "the PM's shows everyone's");
        assert!(asked.from("backend").is_some() && asked.from("qa").is_none());
        asked.sent(7);
        assert_eq!(asked.ids(), [8], "sent from here: hidden at once");
        asked.forget_sent();
        assert_eq!(asked.ids(), [7, 8], "a dead link: shown again");
        assert!(asked.close(7) && !asked.close(7));
        assert_eq!(asked.ids(), [8]);
    }

    #[test]
    fn the_answers_message_reads_back_as_decided() {
        let s = set();
        let message = decide::answer_message(&s, &[Answer { picked: vec![0], other: None }, Answer::skipped()]);
        let Some(Decided::Answered(read)) = decided(&message) else { panic!("{message}") };
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].answer, "Postgres (Recommended)");
        assert!(read[1].skipped);
        assert_eq!(decided(&decide::dismissed_message(&s)), Some(Decided::Closed));
        assert_eq!(decided("고마워요"), None);
    }
}
