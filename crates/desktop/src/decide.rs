//! A member's questions (`ask_decision`) as the card the person answers them
//! on — above the composer in the conversation, and in 나에게's peek: which
//! set shows where, where the person is in it (the step, the row the keys are
//! on, what is picked, what was skipped), what each key does, and the
//! answers it sends. Then, in the conversation, the message those answers
//! went in, read back as "decided". A design choice's options are reacted to
//! in one click — this one, this one with an aspect of another (its mix
//! menu), another round — each sent at once. No UI types here, so the rules
//! are tested without a window.

use std::collections::{BTreeMap, BTreeSet};

use equipe_core::protocol::decide::{self, Answer, Question, QuestionSet, Reaction};
use equipe_core::t;

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

/// What a set is called where it waits — 나에게's row, the card's line —
/// when it is the designer's weekly design review: "주간 디자인 리뷰 · 3가지",
/// the things the review brings. `None`: any other set, called by its question.
pub fn review_title(set: &QuestionSet) -> Option<String> {
    set.review.then(|| equipe_core::t!("decide.design_review", n = equipe_core::design::review::PARTS).to_string())
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
    /// A design choice reacted to beyond a pick — a mix, another round — by question.
    pub reactions: BTreeMap<usize, Reaction>,
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
        self.reactions.remove(&self.step);
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
        self.reactions.contains_key(&self.step) || self.picks.get(self.step).is_some_and(|p| !p.is_empty()) || others.get(self.step).is_some_and(|o| !o.trim().is_empty())
    }

    /// "This one": option `n` of a design choice, and on at once — the
    /// answers to send, from the last question. `None` too when there is
    /// no such option (then nothing moved).
    pub fn take(&mut self, set: &QuestionSet, n: usize, others: &[String]) -> Option<Vec<Answer>> {
        if !self.question(set).is_some_and(reacts) || !self.pick(set, n) {
            return None;
        }
        self.skipped.remove(&self.step);
        self.on(set, others)
    }

    /// Option `base` with option `from`'s `aspect` — one its mix menu
    /// offers — and on at once.
    pub fn mix(&mut self, set: &QuestionSet, base: usize, from: usize, aspect: &str, others: &[String]) -> Option<Vec<Answer>> {
        let q = self.question(set)?;
        if !reacts(q) || !mix_menu(q, base).iter().any(|m| m.from == from && m.aspect == aspect) || !self.pick(set, base) {
            return None;
        }
        self.reactions.insert(self.step, Reaction::Mix { base, from, aspect: aspect.to_owned() });
        self.skipped.remove(&self.step);
        self.on(set, others)
    }

    /// "Again": none of these, another round — their words in the box as
    /// why — and on at once.
    pub fn again(&mut self, set: &QuestionSet, others: &[String]) -> Option<Vec<Answer>> {
        if !self.question(set).is_some_and(reacts) {
            return None;
        }
        if let Some(picks) = self.picks.get_mut(self.step) {
            picks.clear();
        }
        self.reactions.insert(self.step, Reaction::Again);
        self.skipped.remove(&self.step);
        self.on(set, others)
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
                match self.reactions.get(&n) {
                    Some(Reaction::Mix { base, from, aspect }) => Answer { other, ..Answer::mix(*base, *from, aspect.clone()) },
                    Some(Reaction::Again) => Answer::again(other),
                    None => Answer { picked, other, reaction: None },
                }
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
        // A question answered in their own words only: ⏎ and Space go to its box.
        if rows == 0 && matches!(key, Key::Enter | Key::Space) && !self.answered(others) {
            self.cursor = 0;
            return Effect::FocusOther;
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

// ── a design choice, reacted to ──

/// Whether a question's options are reacted to in one click: a choice made
/// by looking (its options pictures), one pick of several.
pub fn reacts(q: &Question) -> bool {
    q.is_visual() && !q.multi_select && q.options.len() > 1
}

/// One line of an option's mix menu: what goes into it, from which option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixItem {
    pub from: usize,
    pub aspect: String,
    /// "A에 B의 글꼴".
    pub label: String,
}

/// Option `base`'s mix menu: every other option's aspects, in order — none
/// when nothing names any.
pub fn mix_menu(q: &Question, base: usize) -> Vec<MixItem> {
    let Some(taker) = q.options.get(base) else { return Vec::new() };
    q.options
        .iter()
        .enumerate()
        .filter(|(from, _)| *from != base)
        .flat_map(|(from, giver)| {
            giver.aspects.iter().map(move |aspect| MixItem { from, aspect: aspect.clone(), label: mix_label(&taker.label, &giver.label, aspect) })
        })
        .collect()
}

/// "A에 B의 글꼴": the labels without their "(추천)".
pub fn mix_label(base: &str, from: &str, aspect: &str) -> String {
    let (base, _) = crate::visuals::recommended(base);
    let (from, _) = crate::visuals::recommended(from);
    t!("decide.design_mix_item", base = base, from = from, aspect = aspect)
}

/// What a question was answered with, as the conversation shows it: a mix
/// as "A에 B의 글꼴", another round as such (why beside it, as a note).
pub fn answer_text(read: &decide::Read) -> String {
    match &read.mix {
        Some(mix) => mix_label(&mix.base, &mix.from, &mix.aspect),
        None if read.again => t!("decide.design_again_said").to_owned(),
        None => read.answer.clone(),
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
        let options = |labels: &[&str]| labels.iter().map(|l| Choice { label: (*l).into(), description: String::new(), ..Choice::default() }).collect();
        QuestionSet {
            id: 7,
            session: 1,
            member: "pm".into(),
            ticket: None,
            questions: vec![
                Question { question: "Which database?".into(), header: "Database".into(), options: options(&["Postgres (Recommended)", "SQLite", "MySQL"]), multi_select: false, ..Question::default() },
                Question { question: "Which features first?".into(), header: "Scope".into(), options: options(&["Auth", "Payments", "Search"]), multi_select: true, ..Question::default() },
            ],
            asked_at: 0,
            review: false,
        }
    }

    #[test]
    fn the_designers_weekly_review_is_called_so_where_it_waits() {
        assert_eq!(review_title(&QuestionSet { review: true, member: "designer".into(), ..set() }).as_deref(), Some("주간 디자인 리뷰 · 3가지"));
        assert_eq!(review_title(&set()), None, "any other set: by its question");
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
        assert_eq!(sent, Effect::Send(vec![Answer { picked: vec![0], ..Default::default() }, Answer { picked: vec![1, 2], ..Default::default() }]));
    }

    #[test]
    fn a_question_without_options_is_answered_in_its_box() {
        let mut s = set();
        s.questions = vec![Question { question: "EU 스토어에 공개할 전화번호는?".into(), header: String::new(), options: Vec::new(), multi_select: false, ..Question::default() }];
        let mut d = Draft::new(&s);
        assert_eq!(d.other_row(&s), 0);
        assert_eq!(d.key(&s, Key::Enter, &others(1), false), Effect::FocusOther, "⏎ goes to the box");
        assert_eq!(d.key(&s, Key::Pick(0), &others(1), false), Effect::Nothing, "nothing to pick");
        let typed = vec!["+82 10-1234-5678".to_owned()];
        assert_eq!(d.key(&s, Key::Next, &typed, true), Effect::Send(vec![Answer { picked: vec![], other: Some("+82 10-1234-5678".into()), ..Default::default() }]));
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
            Effect::Send(vec![Answer { picked: vec![], other: Some("MariaDB, we run one".into()), ..Default::default() }, Answer { picked: vec![0], other: Some("SSO later".into()), ..Default::default() }])
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

    /// The designer's question: three screens, each naming what it can give.
    fn design() -> QuestionSet {
        let option = |label: &str, aspects: &[&str]| Choice { label: label.into(), images: vec![format!("/w/{label}.png")], aspects: aspects.iter().map(|a| (*a).into()).collect(), ..Choice::default() };
        let mut s = set();
        s.member = "designer".into();
        s.questions = vec![
            Question { question: "Which direction?".into(), options: vec![option("Calm (Recommended)", &["type", "colour"]), option("Bold", &["type", "colour"]), option("Warm", &["shape"])], ..Question::default() },
            Question { question: "Which home?".into(), options: vec![option("Cards", &[]), option("List", &[])], ..Question::default() },
        ];
        s
    }

    #[test]
    fn a_design_choice_reacts_in_one_click_and_a_plain_one_does_not() {
        let s = design();
        assert!(reacts(&s.questions[0]) && reacts(&s.questions[1]));
        assert!(!reacts(&set().questions[0]), "words alone: the keys and Next as before");
        let mut several = s.questions[0].clone();
        several.multi_select = true;
        assert!(!reacts(&several), "several picks are ticked, then sent");
        // A plain question takes none of it.
        let plain = set();
        let mut d = Draft::new(&plain);
        assert_eq!(d.take(&plain, 0, &others(2)), None);
        assert_eq!((d.step, d.picked(0)), (0, false));
    }

    #[test]
    fn this_one_picks_and_goes_on_at_once_and_sends_from_the_last() {
        let s = design();
        let mut d = Draft::new(&s);
        let none = others(2);
        assert_eq!(d.take(&s, 7, &none), None, "no such option");
        assert_eq!(d.step, 0);
        assert_eq!(d.take(&s, 1, &none), None, "on to the next question");
        assert_eq!(d.step, 1);
        let sent = d.take(&s, 0, &["".into(), " cards, but airy ".into()]);
        assert_eq!(sent, Some(vec![Answer::pick(1), Answer { picked: vec![0], other: Some("cards, but airy".into()), ..Answer::default() }]), "their words go with it");
    }

    #[test]
    fn the_mix_menu_offers_every_other_options_aspects_and_a_mix_is_sent() {
        let s = design();
        let menu = mix_menu(&s.questions[0], 0);
        assert_eq!(menu.iter().map(|m| (m.from, m.aspect.as_str())).collect::<Vec<_>>(), [(1, "type"), (1, "colour"), (2, "shape")]);
        for m in &menu {
            assert!(m.label.contains("Calm") && !m.label.contains("Recommended") && m.label.contains(&m.aspect), "{}", m.label);
        }
        assert!(menu[0].label.contains("Bold") && menu[2].label.contains("Warm"));
        assert!(mix_menu(&s.questions[1], 0).is_empty(), "nothing named: no menu");
        assert!(mix_menu(&s.questions[0], 9).is_empty());
        let mut d = Draft::new(&s);
        let none = others(2);
        assert_eq!(d.mix(&s, 0, 2, "type", &none), None, "Warm has no type to give");
        assert_eq!(d.step, 0);
        assert_eq!(d.mix(&s, 0, 0, "type", &none), None, "not with itself");
        assert_eq!(d.mix(&s, 0, 2, "shape", &none), None);
        assert_eq!(d.step, 1, "mixed: on at once");
        let sent = d.take(&s, 1, &none).unwrap();
        assert_eq!(sent, [Answer::mix(0, 2, "shape"), Answer::pick(1)]);
        // Back, and a plain pick: the mix is gone.
        let mut d = Draft::new(&s);
        d.mix(&s, 1, 0, "colour", &none);
        d.back();
        d.pick(&s, 2);
        assert_eq!(d.answers(&s, &none)[0], Answer::pick(2));
    }

    #[test]
    fn again_asks_for_another_round_with_the_box_as_why() {
        let s = design();
        let mut d = Draft::new(&s);
        d.pick(&s, 0);
        assert_eq!(d.again(&s, &others(2)), None);
        assert_eq!(d.step, 1);
        let why = vec!["  너무 차가워요 ".to_owned(), String::new()];
        let sent = d.again(&s, &why).unwrap();
        assert_eq!(sent, [Answer::again(Some("너무 차가워요".into())), Answer::again(None)], "nothing picked; why when they said");
        // The answers read back as what they were.
        let message = decide::answer_message(&s, &sent);
        let Some(Decided::Answered(read)) = decided(&message) else { panic!("{message}") };
        assert!(read[0].again && read[0].note.as_deref() == Some("너무 차가워요"));
        assert_eq!(answer_text(&read[1]), t!("decide.design_again_said"));
        let message = decide::answer_message(&s, &[Answer::mix(0, 1, "type"), Answer::pick(0)]);
        let Some(Decided::Answered(read)) = decided(&message) else { panic!("{message}") };
        assert_eq!(answer_text(&read[0]), mix_label("Calm (Recommended)", "Bold", "type"));
        assert_eq!(answer_text(&read[1]), "Cards");
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
        let message = decide::answer_message(&s, &[Answer { picked: vec![0], ..Default::default() }, Answer::skipped()]);
        let Some(Decided::Answered(read)) = decided(&message) else { panic!("{message}") };
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].answer, "Postgres (Recommended)");
        assert!(read[1].skipped);
        assert_eq!(decided(&decide::dismissed_message(&s)), Some(Decided::Closed));
        assert_eq!(decided("고마워요"), None);
    }
}
