//! A member's questions (`ask_decision`), drawn: the card docked above the
//! composer in the asking member's conversation (and the PM's) — "1/2" and
//! the question, fold ⌄ and close ✕; each option a row with its label, a
//! line on what it means and its number key; a last row for words of the
//! person's own; Skip, Back and Next ⌘↵ (Submit on the last). A visual
//! question's options are a grid of tiles instead — balanced rows of one
//! size, fewer columns when narrow, nothing moving when one is picked — each
//! picture opening big. A design choice's tiles each have "This one", sent at
//! once, and "Mix", a menu of what the other options can give ("A with B's
//! type"); the card has "Again", another round, the box's words as why. The
//! same card answers a set picked in the Inbox's peek.
//! Sent, the answers' message folds in the conversation to one line, "Decided ·
//! 2 questions", that opens to each question and what was decided. The rules are `crate::decide`'s.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use equipe_core::protocol::decide::{self as asked, Answer, Question, QuestionSet};
use equipe_core::protocol::{Command, SessionId};
use equipe_core::t;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::decide::{self as rules, Decided, Draft, Effect, Key, MixItem, Rank, Rest};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::render;
use crate::state::Need;
use crate::theme::{MONO, Palette, R_LG, R_MD, R_SM, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::visuals::{self as visual, Size, Viewer};
use crate::{Screen, Shell};

use super::visuals::Shots;

/// The disclosure of a decided row.
pub const PART_DECIDED: u32 = (1 << 20) + 9;

/// Where a card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Above the composer: it folds (⌄) and closes (✕).
    Docked,
    /// The Inbox's peek, under its own heading.
    Peek,
}

/// The card's state that lives with the window: where the person is in each
/// set, and the boxes for their own words, by set and question.
#[derive(Default)]
pub struct DecideUi {
    pub drafts: HashMap<u64, Draft>,
    pub fields: HashMap<(u64, usize), Entity<TextareaState>>,
    /// What is attached to each answer in their own words, by set and question.
    pub strips: HashMap<(u64, usize), crate::attach::Strip>,
    subscriptions: HashMap<(u64, usize), Subscription>,
    /// How tall the window is, as last drawn: a card takes only part of it.
    pub room: f32,
    /// Each set's options, scrolled — and the step and key position last
    /// brought into sight, so a key moving to a tile below the fold shows it.
    scrolls: HashMap<u64, (ScrollHandle, Seen)>,
    /// The mix menu open: set, question, and the option it mixes into.
    pub mixing: Option<Mixing>,
    /// The menu last closed by a press outside it, and when: that press on
    /// its own button closes it rather than opening it again.
    mix_closed: Option<(Mixing, std::time::Instant)>,
    /// The card the person went to in each member's conversation (← →, the
    /// list), by the member's session: it stays in front until answered.
    pub browsing: HashMap<SessionId, (u64, usize)>,
    /// The list of every question the conversation offers, open over the
    /// card — the row the keys are on.
    pub listing: Option<usize>,
    /// A question just asked in the conversation on screen, and when: lit
    /// for a moment, so it is noticed.
    pub fresh: Option<(u64, std::time::Instant)>,
    /// Bumped with each fresh question: its arrival plays again.
    fresh_serial: u64,
}

/// How long a question just asked stays lit.
const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(6);

/// Where the card in front stands among the conversation's questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Nav {
    /// Its place, from 0.
    at: usize,
    count: usize,
    rest: Rest,
}

/// Which mix menu: set, question, option.
pub type Mixing = (u64, usize, usize);

/// The step and key position last brought into sight.
type Seen = Rc<Cell<Option<(usize, usize)>>>;

/// What a keystroke came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Took {
    /// The card's: stop it there.
    Taken,
    /// Typing in the box for their own words: the box's, and nobody else's.
    Typing,
    /// Not the card's.
    Pass,
}

impl Shell {
    /// A draft and a box per question for every set still waiting; a set no
    /// longer waiting lets its own go.
    pub fn decide_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sets: Vec<QuestionSet> = self.model.asked.open().cloned().collect();
        let live: Vec<u64> = sets.iter().map(|s| s.id).collect();
        self.decide.drafts.retain(|id, _| live.contains(id));
        self.decide.fields.retain(|(id, _), _| live.contains(id));
        self.decide.subscriptions.retain(|(id, _), _| live.contains(id));
        self.decide.scrolls.retain(|id, _| live.contains(id));
        for set in &sets {
            self.decide.drafts.entry(set.id).or_insert_with(|| Draft::new(set));
            self.decide.scrolls.entry(set.id).or_default();
            for n in 0..set.questions.len() {
                let k = (set.id, n);
                if self.decide.fields.contains_key(&k) {
                    continue;
                }
                // A question without options is answered in this box alone.
                let placeholder = if set.questions[n].options.is_empty() {
                    t!("decide.free_placeholder")
                } else if rules::reacts(&set.questions[n]) {
                    t!("decide.design_why_placeholder")
                } else {
                    t!("decide.other_placeholder")
                };
                let field = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 4).submit_on_enter(false).placeholder(placeholder));
                let id = set.id;
                let subscription = cx.subscribe_in(&field, window, move |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { secondary: true, .. } => {
                        this.decide_key(id, Key::Next, true, window, cx);
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                });
                self.decide.fields.insert(k, field);
                self.decide.subscriptions.insert(k, subscription);
            }
        }
        self.decide_take_fresh(cx);
    }

    /// A question just asked (or asked again) in the conversation on screen:
    /// it comes to the front — unless the person is in the middle of another
    /// card, which stays — lights up for a moment, and its card comes before
    /// the less pressing ones over the composer.
    fn decide_take_fresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.model.asked.take_fresh() else { return };
        let Some(member) = self.model.selected_member().map(|m| m.session) else { return };
        let order = self.decide_order();
        if !order.contains(&id) {
            return;
        }
        let front = self.decide_front().map(|(f, _)| f).filter(|f| *f != id);
        let busy = front.is_some_and(|f| self.decide.drafts.get(&f).is_some_and(Draft::begun) || self.decide.fields.iter().any(|((set, _), field)| *set == f && !field.read(cx).value().trim().is_empty()));
        if !busy && let Some(to) = rules::jump(&order, id) {
            self.decide.browsing.insert(member, to);
        }
        self.decide.fresh = Some((id, std::time::Instant::now()));
        self.decide.fresh_serial += 1;
        self.chat.dock_bring.set(true);
        // Unlit again after a while.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FRESH_FOR).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    /// The questions the conversation on screen offers, by id, in order.
    pub fn decide_order(&self) -> Vec<u64> {
        self.model.questions_here().iter().map(|(s, _)| s.id).collect()
    }

    /// The card in front in the conversation on screen, and its place.
    fn decide_front(&self) -> Option<(u64, usize)> {
        let member = self.model.selected_member()?;
        if self.model.opened.is_some() {
            return None;
        }
        rules::in_front(&self.decide_order(), self.decide.browsing.get(&member.session).copied())
    }

    /// ← / → or ‹ ›: the card before or after, nothing answered.
    pub fn decide_browse(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(member) = self.model.selected_member().map(|m| m.session) else { return };
        let order = self.decide_order();
        if let Some(to) = rules::browse(&order, self.decide.browsing.get(&member).copied(), by) {
            self.decide.browsing.insert(member, to);
        }
        self.decide.listing = None;
        self.decide.mixing = None;
        cx.notify();
    }

    /// Straight to set `id`, picked from the list.
    pub fn decide_jump(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(member) = self.model.selected_member().map(|m| m.session) else { return };
        if let Some(to) = rules::jump(&self.decide_order(), id) {
            self.decide.browsing.insert(member, to);
            if let Some(draft) = self.decide.drafts.get_mut(&id) {
                draft.collapsed = false;
            }
        }
        self.decide.listing = None;
        cx.notify();
    }

    /// The list of every question here, opened (on the card in front) or closed.
    pub fn decide_list(&mut self, open: bool, cx: &mut Context<Self>) {
        self.decide.listing = if open { Some(self.decide_front().map_or(0, |(_, at)| at)) } else { None };
        cx.notify();
    }

    /// "Ask PM to tidy these": the older questions, listed in one plain
    /// message to the PM — the person's own, in their words. Nothing closes.
    pub fn decide_ask_tidy(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = match self.screen {
            Screen::Chat => self.model.questions_here().into_iter().filter(|(_, r)| *r == Rank::Older).map(|(s, _)| s.id).collect(),
            _ => self.model.questions_waiting().into_iter().filter(|(_, r)| *r == Rank::Older).map(|(s, _)| s.id).collect(),
        };
        let Some(pm) = self.model.pm().filter(|_| !ids.is_empty() && self.can_act()) else { return };
        let sets: Vec<&QuestionSet> = ids.iter().filter_map(|id| self.model.asked.get(*id)).collect();
        let text = tidy_message(&sets, self.model.unix);
        self.send_command(Command::Submit { session: pm, text, about: Some(self.model.focus.clone()) });
        self.say(t!("decide.tidy_sent"));
        self.decide.listing = None;
        cx.notify();
    }

    /// The set the conversation on screen shows: the one the person went
    /// to, else the first in order — what the member asked since the person
    /// last spoke here ([`rules::order`]).
    pub fn decide_shown(&self) -> Option<u64> {
        self.decide_front().map(|(id, _)| id)
    }

    /// The decision waiting about ticket `ticket`, shown in its peek — not
    /// while it waits on the person, whose number keys tick its asks off.
    pub fn ticket_question(&self, ticket: u32) -> Option<u64> {
        let waiting = self.model.tickets.iter().any(|t| t.id == ticket && t.status == equipe_core::protocol::TicketStatus::Waiting);
        (!waiting).then(|| self.model.asked.open().find(|s| s.ticket == Some(ticket)).map(|s| s.id)).flatten()
    }

    /// The words typed in each of a set's boxes, by question.
    fn decide_others(&self, id: u64, cx: &App) -> Vec<String> {
        let count = self.model.asked.get(id).map_or(0, |s| s.questions.len());
        (0..count)
            .map(|n| {
                let typed = self.decide.fields.get(&(id, n)).map(|f| f.read(cx).value().to_string()).unwrap_or_default();
                // What is attached goes with the words, as tags the member opens.
                crate::attach::message(&typed, &self.answer_tags(super::composer::AttachTo::Answer(id, n)))
            })
            .collect()
    }

    /// Whether an answer of set `id` still has an attachment on its way.
    fn decide_uploading(&self, id: u64) -> bool {
        self.decide.strips.iter().any(|((set, _), strip)| *set == id && strip.busy())
    }

    /// Whether the box for their own words of set `id`'s question on show has the typing.
    fn decide_in_other(&self, id: u64, window: &Window, cx: &App) -> bool {
        let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
        self.decide.fields.get(&(id, step)).is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }

    /// A keystroke, before anyone else sees it: the card's keys while it is
    /// open above an empty composer (or its own box has the typing); in
    /// The Inbox's peek, the digits, Space, Tab and ⌘↵ only — the list keeps ↑↓,
    /// ⏎ and Esc.
    pub fn on_decide_key(&mut self, key: &str, m: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> Took {
        if m.control || m.alt || m.function {
            return Took::Pass;
        }
        let (id, peek) = match self.screen {
            Screen::Chat => match self.decide_shown() {
                Some(id) => (id, false),
                None => return Took::Pass,
            },
            Screen::Board => match self.picked_need() {
                Some(Need::Questions(id)) if self.board.view.peek => (id, true),
                // A ticket's peek showing the decision its member waits on.
                Some(Need::Ticket(t)) if self.board.view.peek && self.board.flow.is_none() => match self.ticket_question(t) {
                    Some(id) => (id, true),
                    None => return Took::Pass,
                },
                _ => return Took::Pass,
            },
            Screen::Settings | Screen::Design | Screen::Roadmap | Screen::Metrics | Screen::Project | Screen::Progress | Screen::Orchestration | Screen::Usage | Screen::Docs | Screen::Studio => return Took::Pass,
        };
        // The list of them open: ↑↓ through it, ⏎ to the one picked, Esc away.
        if !peek && let Some(at) = self.decide.listing && self.composer.text(cx).is_empty() {
            let order = self.decide_order();
            match key {
                "up" => self.decide.listing = Some(at.saturating_sub(1)),
                "down" => self.decide.listing = Some((at + 1).min(order.len().saturating_sub(1))),
                "enter" => match order.get(at) {
                    Some(id) => self.decide_jump(*id, cx),
                    None => self.decide_list(false, cx),
                },
                "escape" => self.decide_list(false, cx),
                _ => return Took::Pass,
            }
            cx.notify();
            return Took::Taken;
        }
        if self.decide.drafts.get(&id).is_none_or(|d| d.collapsed) {
            return Took::Pass;
        }
        let in_other = self.decide_in_other(id, window, cx);
        if !in_other {
            let free = if peek { !self.board.typing(window, cx) } else { self.composer.is_focused(window, cx) && self.composer.text(cx).is_empty() };
            if !free {
                return Took::Pass;
            }
        }
        let Some(k) = rules::key_of(key, m.secondary(), m.shift) else {
            return if in_other { Took::Typing } else { Took::Pass };
        };
        if m.secondary() && k != Key::Next {
            return Took::Pass;
        }
        if peek && matches!(k, Key::Up | Key::Down | Key::Enter | Key::Esc | Key::Back | Key::Card(_)) && !in_other {
            return Took::Pass;
        }
        // ← → over attachments in the composer are theirs.
        if matches!(k, Key::Card(_)) && !in_other && !self.composer.strip.is_empty() {
            return Took::Pass;
        }
        if in_other && !matches!(k, Key::Tab | Key::Esc | Key::Next | Key::Up) {
            return Took::Typing;
        }
        if self.decide_key(id, k, in_other, window, cx) { Took::Taken } else if in_other { Took::Typing } else { Took::Pass }
    }

    /// Key `k` on set `id`'s card: what it does, done. Whether it was the card's.
    pub fn decide_key(&mut self, id: u64, k: Key, in_other: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(set) = self.model.asked.get(id).cloned() else { return false };
        let others = self.decide_others(id, cx);
        let Some(draft) = self.decide.drafts.get_mut(&id) else { return false };
        let effect = draft.key(&set, k, &others, in_other);
        let step = draft.step;
        let changed = effect == Effect::Changed;
        match effect {
            Effect::Nothing => return false,
            Effect::Changed => {}
            Effect::FocusOther => {
                if let Some(field) = self.decide.fields.get(&(id, step)).cloned() {
                    field.update(cx, |s, cx| s.focus(window, cx));
                }
            }
            Effect::LeaveOther | Effect::Collapse => self.decide_give_back(window, cx),
            Effect::Send(answers) => self.decide_send(id, answers, window, cx),
            Effect::Browse(by) => self.decide_browse(by, cx),
        }
        // A step taken in the box: the next question's box, if it had the typing.
        if in_other && changed {
            self.decide_give_back(window, cx);
        }
        cx.notify();
        true
    }

    /// The keys back where the person was: the composer, or the list.
    fn decide_give_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Chat => self.composer.focus(window, cx),
            _ => window.blur(cx),
        }
    }

    /// The answers to the member, in one message; the card goes at once.
    pub fn decide_send(&mut self, id: u64, answers: Vec<Answer>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() || self.model.asked.get(id).is_none() {
            return;
        }
        if self.decide_uploading(id) {
            self.composer.flash = Some(t!("composer.answer_uploading").into());
            return cx.notify();
        }
        self.decide.strips.retain(|(set, _), _| *set != id);
        self.send_command(Command::AnswerQuestions { squad: self.model.squad.clone(), id, answers, about: Some(self.model.focus.clone()) });
        self.model.asked.sent(id);
        self.model.revision += 1;
        self.decide_give_back(window, cx);
        self.after_decide(Need::Questions(id), cx);
    }

    /// ✕: closed without an answer; the member goes on without one.
    pub fn decide_dismiss(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() || self.model.asked.get(id).is_none() {
            return;
        }
        self.send_command(Command::DismissQuestions { squad: self.model.squad.clone(), id });
        self.model.asked.sent(id);
        self.model.revision += 1;
        self.decide_give_back(window, cx);
        self.after_decide(Need::Questions(id), cx);
    }

    fn after_decide(&mut self, need: Need, cx: &mut Context<Self>) {
        if self.board.view.selected == Some(need) {
            self.board.view.selected = None;
        }
        self.chat_sync(cx);
        self.board_sync(cx);
        cx.notify();
    }

    /// A click on option `n`.
    pub fn decide_pick(&mut self, id: u64, n: usize, cx: &mut Context<Self>) {
        let Some(set) = self.model.asked.get(id).cloned() else { return };
        if let Some(draft) = self.decide.drafts.get_mut(&id) {
            draft.pick(&set, n);
            cx.notify();
        }
    }

    /// A design choice reacted to in one click — this one, a mix, another
    /// round: on at once, sent from the last question.
    fn decide_react(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>, how: impl FnOnce(&mut Draft, &QuestionSet, &[String]) -> Option<Vec<Answer>>) {
        self.decide.mixing = None;
        if !self.can_act() {
            return;
        }
        let Some(set) = self.model.asked.get(id).cloned() else { return };
        let others = self.decide_others(id, cx);
        let Some(draft) = self.decide.drafts.get_mut(&id) else { return };
        match how(draft, &set, &others) {
            Some(answers) => self.decide_send(id, answers, window, cx),
            None => {
                self.decide_give_back(window, cx);
                cx.notify();
            }
        }
    }

    /// "This one" on option `n`.
    pub fn decide_take(&mut self, id: u64, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.decide_react(id, window, cx, |draft, set, others| draft.take(set, n, others));
    }

    /// A line of option `base`'s mix menu: it, with `from`'s `aspect`.
    pub fn decide_mix(&mut self, id: u64, base: usize, from: usize, aspect: String, window: &mut Window, cx: &mut Context<Self>) {
        self.decide_react(id, window, cx, |draft, set, others| draft.mix(set, base, from, &aspect, others));
    }

    /// "Again": another round, the box's words as why.
    pub fn decide_again(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.decide_react(id, window, cx, |draft, set, others| draft.again(set, others));
    }

    /// "Mix": option `base`'s mix menu, opened or closed.
    pub fn decide_mix_menu(&mut self, id: u64, base: usize, cx: &mut Context<Self>) {
        let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
        let which = (id, step, base);
        let just_closed = self.decide.mix_closed.take().is_some_and(|(m, at)| m == which && at.elapsed() < std::time::Duration::from_millis(400));
        self.decide.mixing = if self.decide.mixing == Some(which) || just_closed { None } else { Some(which) };
        cx.notify();
    }

    /// A press outside the mix menu closes it.
    pub fn decide_mix_outside(&mut self, cx: &mut Context<Self>) {
        if let Some(which) = self.decide.mixing.take() {
            self.decide.mix_closed = Some((which, std::time::Instant::now()));
            cx.notify();
        }
    }

    /// Dev tool (`snapshot.rs`): a design choice's one-click actions on the
    /// card on show — `dtake:<n>` (option n's This one), `dmix:<n>` (its mix
    /// menu opened) or `dmix:<n>,<k>` (that menu's kth line), `dagain`.
    #[cfg(feature = "snapshot")]
    pub fn decide_script_react(&mut self, verb: &str, arg: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = match self.screen {
            Screen::Board => match self.picked_need() {
                Some(Need::Questions(id)) => Some(id),
                _ => None,
            },
            _ => self.decide_shown(),
        };
        let Some(id) = id else { return };
        let mut numbers = arg.split(',').filter_map(|n| n.trim().parse::<usize>().ok()).map(|n| n.saturating_sub(1));
        match (verb, numbers.next(), numbers.next()) {
            ("dtake", Some(n), _) => self.decide_take(id, n, window, cx),
            ("dmix", Some(base), None) => self.decide_mix_menu(id, base, cx),
            ("dmix", Some(base), Some(k)) => {
                let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
                let item = self.model.asked.get(id).and_then(|s| s.questions.get(step)).and_then(|q| rules::mix_menu(q, base).into_iter().nth(k));
                if let Some(item) = item {
                    self.decide_mix(id, base, item.from, item.aspect, window, cx);
                }
            }
            ("dagain", ..) => self.decide_again(id, window, cx),
            _ => {}
        }
    }

    /// Next, Skip or Back, clicked.
    pub fn decide_step(&mut self, id: u64, how: Step, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = self.model.asked.get(id).cloned() else { return };
        let others = self.decide_others(id, cx);
        let Some(draft) = self.decide.drafts.get_mut(&id) else { return };
        let sent = match how {
            Step::Next => draft.next(&set, &others),
            Step::Skip => draft.skip(&set, &others),
            Step::Back => {
                draft.back();
                None
            }
        };
        match sent {
            Some(answers) => self.decide_send(id, answers, window, cx),
            None => cx.notify(),
        }
    }

    /// ⌄ folds the card to one line; a click on it opens it again.
    pub fn decide_fold(&mut self, id: u64, folded: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.decide.drafts.get_mut(&id) {
            draft.collapsed = folded;
        }
        self.decide_give_back(window, cx);
        cx.notify();
    }

    /// Dev tool (`snapshot.rs`): a key on the card on show, wherever the
    /// window's focus is (`cmd-enter` with ⌘).
    #[cfg(feature = "snapshot")]
    pub fn decide_script_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let id = match self.screen {
            Screen::Board => match self.picked_need() {
                Some(Need::Questions(id)) => Some(id),
                _ => None,
            },
            _ => self.decide_shown(),
        };
        let (cmd, key) = match key.strip_prefix("cmd-") {
            Some(rest) => (true, rest),
            None => (false, key),
        };
        let (shift, key) = match key.strip_prefix("shift-") {
            Some(rest) => (true, rest),
            None => (false, key),
        };
        let (Some(id), Some(k)) = (id, rules::key_of(key, cmd, shift)) else { return false };
        let in_other = self.decide_in_other(id, window, cx);
        self.decide_key(id, k, in_other, window, cx)
    }

    /// Dev tool (`snapshot.rs`): words in the box of the question on show.
    #[cfg(feature = "snapshot")]
    pub fn decide_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = match self.screen {
            Screen::Board => match self.picked_need() {
                Some(Need::Questions(id)) => Some(id),
                _ => None,
            },
            _ => self.decide_shown(),
        };
        let Some(id) = id else { return };
        let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
        if let Some(field) = self.decide.fields.get(&(id, step)).cloned() {
            field.update(cx, |s, cx| {
                s.set_value(text.to_owned(), window, cx);
                s.focus(window, cx);
            });
            if let Some(draft) = self.decide.drafts.get_mut(&id) {
                draft.cursor = self.model.asked.get(id).map_or(0, |s| s.questions[step].options.len());
            }
        }
        cx.notify();
    }
}

/// A step of the card, clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Skip,
    Back,
}

// ── drawing ──

/// The card above the composer, for the set the conversation shows.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let (id, at) = shell.decide_front()?;
    let order = shell.model.questions_here();
    let nav = Nav { at, count: order.len(), rest: rules::rest(&order, id) };
    if shell.decide.listing.is_some() {
        return Some(div().w_full().min_h_0().mb_3().child(list(shell, &order, id, cx)).into_any_element());
    }
    let body = card(shell, id, Place::Docked, Some(nav), cx)?;
    let serial = shell.decide.fresh.filter(|(f, _)| *f == id).map_or(0, |_| shell.decide.fresh_serial);
    Some(motion::rise(v_flex().w_full().min_h_0().mb_3().child(body), ("decide-in", render::hash_of((id, serial)))).into_any_element())
}

/// Whether set `id` was just asked here, and is lit.
fn is_fresh(shell: &Shell, id: u64) -> bool {
    shell.decide.fresh.is_some_and(|(f, at)| f == id && at.elapsed() < FRESH_FOR)
}

/// The older questions, in the message that asks the PM to tidy them: each
/// one's member, ticket, question and how long it has waited.
pub fn tidy_message(sets: &[&QuestionSet], now: u64) -> String {
    let lines: Vec<String> = sets
        .iter()
        .map(|s| {
            let ticket = s.ticket.map(|t| format!(" · T-{t}")).unwrap_or_default();
            let first = s.questions.first().map(|q| q.question.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
            let first: String = if first.chars().count() > 140 { format!("{}…", first.chars().take(140).collect::<String>()) } else { first };
            let more = if s.questions.len() > 1 { format!(" {}", t!("decide.and_more", count = s.questions.len() - 1)) } else { String::new() };
            format!("- {}{ticket}: {first}{more} ({})", s.member, crate::activity::ago(now.saturating_sub(s.asked_at)))
        })
        .collect();
    t!("decide.tidy_message", days = asked::STALE_DAYS, list = lines.join("\n")).to_string()
}

/// Every question the conversation offers, as one list over the card: what
/// was asked here, the team's, and the older ones — each a row that goes
/// to it; the older ones with "Ask PM to tidy these".
fn list(shell: &Shell, order: &[(&QuestionSet, Rank)], front: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let handle = cx.entity().downgrade();
    let cursor = shell.decide.listing.unwrap_or(0);
    let now = shell.model.unix;
    let group_of = |r: Rank| match r {
        Rank::Reply | Rank::Own => 0,
        Rank::Others => 1,
        Rank::Older => 2,
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut last_group = None;
    for (n, (set, rank)) in order.iter().enumerate() {
        let group = group_of(*rank);
        if last_group != Some(group) {
            last_group = Some(group);
            let label = match group {
                0 => t!("decide.list_here").to_string(),
                1 => t!("decide.list_others").to_string(),
                _ => t!("decide.list_older", days = asked::STALE_DAYS).to_string(),
            };
            let tidy = (group == 2).then(|| {
                let handle = handle.clone();
                ui::button(p, "decide-tidy", Kind::Secondary, t!("decide.tidy")).disabled(!shell.can_act()).on_click(move |_, _, cx| {
                    handle.update(cx, |shell, cx| shell.decide_ask_tidy(cx)).ok();
                })
            });
            rows.push(
                h_flex()
                    .w_full()
                    .pt(px(if rows.is_empty() { 0. } else { 8. }))
                    .pb_1()
                    .gap_2()
                    .items_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted)
                    .child(div().flex_1().child(label))
                    .children(tidy)
                    .into_any_element(),
            );
        }
        let (name, _) = who(shell, set);
        let first = set.questions.first().map(|q| if q.header.trim().is_empty() { q.question.clone() } else { format!("{} · {}", q.header, q.question) }).unwrap_or_default();
        let title = crate::decide::review_title(set).unwrap_or(first);
        let id = set.id;
        let open = handle.clone();
        let on = n == cursor;
        rows.push(
            h_flex()
                .id(("decide-list-row", id))
                .w_full()
                .min_h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(R_SM))
                .cursor_pointer()
                .when(on, |r| r.bg(p.wash))
                .hover(move |s| s.bg(p.wash))
                .text_size(px(SMALL))
                .child(div().flex_none().w(px(22.)).font_family(MONO).text_size(px(11.)).text_color(p.muted).child(format!("{}", n + 1)))
                .child(crate::theme::member_mark(p, &shell.model, &name, 14.))
                .child(div().flex_none().text_color(p.text2).child(name.clone()))
                .children(set.ticket.map(|t| div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(format!("T-{t}"))))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(if id == front { p.accent } else { p.text }).child(title))
                .child(div().flex_none().text_size(px(11.5)).text_color(p.muted).child(crate::activity::ago(now.saturating_sub(set.asked_at))))
                .on_click(move |_, _, cx| {
                    open.update(cx, |shell, cx| shell.decide_jump(id, cx)).ok();
                })
                .into_any_element(),
        );
    }
    let close = {
        let handle = handle.clone();
        ui::icon_button(p, "decide-list-close", Icon::X, t!("decide.collapse"), Some("Esc")).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.decide_list(false, cx)).ok();
        })
    };
    let head = h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("decide.list_tip")))
        .child(div().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(order.len().to_string()))
        .child(close);
    v_flex()
        .id("decide-list")
        .w_full()
        .max_h(px(card_tallest(shell, Place::Docked)))
        .p(px(14.))
        .gap_2()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .shadow_sm()
        .child(head)
        .child(v_flex().id("decide-list-rows").w_full().min_h_0().overflow_y_scroll().gap_0p5().children(rows))
        .child(div().pt_1().text_size(px(12.)).text_color(p.muted).child(t!("decide.list_keys")))
        .into_any_element()
}

/// ‹ 3 / 23 ›: the cards before and after, and where this one stands — a
/// click on the place opens the list of them all.
fn nav_control(p: Palette, nav: Nav, handle: &WeakEntity<Shell>) -> AnyElement {
    let step = |id: &'static str, glyph: Icon, by: isize, name: &'static str, key: &'static str| {
        let handle = handle.clone();
        ui::icon_button(p, id, glyph, name, Some(key)).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.decide_browse(by, cx)).ok();
        })
    };
    let open = handle.clone();
    let place = div()
        .id("decide-place")
        .px_1()
        .rounded(px(R_SM))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .font_family(MONO)
        .text_size(px(11.5))
        .text_color(p.text2)
        .child(format!("{} / {}", nav.at + 1, nav.count))
        .on_click(move |_, _, cx| {
            open.update(cx, |shell, cx| shell.decide_list(true, cx)).ok();
        });
    h_flex()
        .flex_none()
        .gap_0p5()
        .items_center()
        .child(step("decide-prev", Icon::ArrowLeft, -1, t!("decide.prev_card"), "←"))
        .child(tip(place, p, t!("decide.list_tip"), None))
        .child(step("decide-next-card", Icon::ArrowRight, 1, t!("decide.next_card"), "→"))
        .into_any_element()
}

/// "+22 waiting · 9 older than 3 days": where the rest are.
fn rest_words(rest: Rest) -> Option<String> {
    let mut words = Vec::new();
    if rest.more > 0 {
        words.push(t!("decide.more", count = rest.more).to_string());
    }
    if rest.older > 0 {
        words.push(t!("decide.older", count = rest.older, days = asked::STALE_DAYS).to_string());
    }
    (!words.is_empty()).then(|| words.join(" · "))
}

/// The card in the Inbox's peek.
pub fn peek(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("peek")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("state.section_decide")))
        .children(card(shell, id, Place::Peek, None, cx))
        .into_any_element()
}

/// A decision its member waits on about ticket `ticket`, in the ticket's
/// peek: the same card, under its own heading.
pub fn for_ticket(shell: &Shell, ticket: u32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let id = shell.ticket_question(ticket)?;
    let p = shell.palette;
    let body = card(shell, id, Place::Peek, None, cx)?;
    Some(v_flex().gap_2().child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("state.section_decide"))).child(body).into_any_element())
}

/// Who asks: their dot and name.
fn who(shell: &Shell, set: &QuestionSet) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.name == set.member).map_or(set.member.as_str(), |m| m.role.as_str());
    let (_, hue) = look(role, &set.member);
    (set.member.clone(), shell.palette.member(hue))
}

/// One set, at the step the person is on.
fn card(shell: &Shell, id: u64, place: Place, nav: Option<Nav>, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let set = shell.model.asked.get(id)?.clone();
    let draft = shell.decide.drafts.get(&id).cloned().unwrap_or_else(|| Draft::new(&set));
    let handle = cx.entity().downgrade();
    let (name, _) = who(shell, &set);
    if draft.collapsed && place == Place::Docked {
        let mark = crate::theme::member_mark(p, &shell.model, &name, 15.).into_any_element();
        return Some(folded(p, id, &set, &name, mark, nav.and_then(|n| rest_words(n.rest)), is_fresh(shell, id), &handle));
    }
    let q = draft.question(&set)?.clone();
    let total = set.questions.len();
    let others = shell.decide_others(id, cx);
    let answered = draft.answered(&others);
    let last = draft.is_last(&set);
    let field = shell.decide.fields.get(&(id, draft.step)).cloned();
    let typing = field.as_ref().is_some_and(|f| !f.read(cx).value().trim().is_empty());

    // The head: 1/2, the question, fold and close.
    let step_pill = div()
        .flex_none()
        .h(px(20.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(p.wash)
        .border_1()
        .border_color(p.hair)
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(p.text2)
        // One question: its word ("Database"), if it has one; several: where the person is.
        .child(if total == 1 && !q.header.is_empty() { q.header.clone() } else { format!("{}/{}", draft.step + 1, total) });
    let fold = (place == Place::Docked).then(|| {
        let handle = handle.clone();
        ui::icon_button(p, ("decide-fold", id), Icon::ChevronDown, t!("decide.collapse"), Some("Esc")).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_fold(id, true, window, cx)).ok();
        })
    });
    let close = {
        let handle = handle.clone();
        ui::icon_button(p, ("decide-close", id), Icon::X, t!("decide.close"), None).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_dismiss(id, window, cx)).ok();
        })
    };
    let head = h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(div().pt(px(2.)).child(step_pill))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                // The weekly design review's: said over its question.
                // Just asked here: said so, quietly, for a moment.
                .when(place == Place::Docked && is_fresh(shell, id), |c| c.child(div().text_size(px(11.)).line_height(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(t!("decide.new"))))
                .children(crate::decide::review_title(&set).map(|title| div().text_size(px(12.)).line_height(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(p.accent).child(title)))
                .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(q.question.clone())),
        )
        .child(h_flex().flex_none().mt(px(-3.)).mr(px(-6.)).gap_0p5().children(fold).child(close));

    // The question's own pictures; then the options — a visual question's
    // as a grid of tiles, balanced rows of one size, those without a picture
    // a quiet tile of their label among them; else rows — then the box for
    // their own words.
    let shots = Shots::of(shell, handle.clone(), cx);
    let key = render::hash_of(("decide-grid", id, place == Place::Docked));
    let guess = match place {
        Place::Docked => GRID_GUESS,
        Place::Peek => shell.board.peek_width - 34.,
    };
    let width = shots.width(key, guess);
    let described = q.options.iter().any(|o| !o.description.trim().is_empty());
    // A design choice: each tile has its one-click actions under its caption.
    let reacts = rules::reacts(&q);
    let actions_h = if reacts { ACTIONS_H } else { 0. };
    // A whole row of tiles fits what the card has left for its options: the
    // card's height less its question, its box, its attachments and its foot.
    let tiles_room = (card_tallest(shell, place) - 280. - actions_h).max(0.);
    let layout = visual::grid_within(width, q.options.len(), described, tiles_room);
    let own = (!q.images.is_empty()).then(|| question_pictures(&shots, id, draft.step, &q, width, &layout, cx));
    let tile_h = visual::tile_height(layout.tile, described) + actions_h;
    let grid = q.is_visual().then(|| {
        // Each tile where the layout puts it, in whatever state: nothing reflows.
        let tiles: Vec<AnyElement> = (0..q.options.len())
            .filter_map(|n| {
                let state = visual::TileState { picked: draft.picked(n), cursor: draft.cursor == n, hover: 0. };
                let (x, y, _, _) = layout.rect(n, tile_h, state)?;
                let actions = reacts.then(|| Actions { menu: rules::mix_menu(&q, n), open: shell.decide.mixing == Some((id, draft.step, n)), can_act: shell.can_act() });
                let tile = option_tile(&shots, id, draft.step, n, &q, state.picked, state.cursor, layout.tile, described, actions, &handle, cx);
                Some(div().absolute().left(px(x)).top(px(y)).child(tile).into_any_element())
            })
            .collect();
        div().relative().w_full().h(px(layout.height(tile_h))).children(tiles)
    });
    let rows: Vec<AnyElement> = q
        .options
        .iter()
        .enumerate()
        .filter(|_| !q.is_visual())
        .map(|(n, option)| {
            let (label, recommended) = option.shown();
            option_row(p, id, n, &label, recommended, &option.description, draft.picked(n), draft.cursor == n, q.multi_select, &handle)
        })
        .collect();
    let other_on = draft.cursor >= q.options.len() || typing;
    let other = field.map(|field| {
        h_flex()
            .id(("decide-other", id))
            .w_full()
            .px_3()
            .py(px(5.))
            .gap_2()
            .items_center()
            .rounded(px(R_MD))
            .border_1()
            .border_color(if other_on { p.accent } else { p.hair })
            .when(typing, |r| r.bg(p.accent_bg))
            .child(icon(Icon::Pencil, 13., if other_on { p.accent } else { p.muted }))
            .child(div().flex_1().min_w_0().text_size(px(14.)).child(super::composer::answer_box(&field, super::composer::AttachTo::Answer(id, draft.step), handle.clone())))
            .child(key_badge(p, "Tab", false))
            .on_drop(super::composer::on_answer_drop(super::composer::AttachTo::Answer(id, draft.step), handle.clone()))
            .into_any_element()
    });
    // What is attached to this answer, and "Attach" to add more.
    let attached = other.is_some().then(|| {
        let to = super::composer::AttachTo::Answer(id, draft.step);
        let items = shell.strip_of(to).map(|s| s.items().to_vec()).unwrap_or_default();
        super::composer::answer_attachments(p, to, id << 8 | draft.step as u64, items, handle.clone())
    });
    let multi = q.multi_select.then(|| div().text_size(px(12.)).text_color(p.muted).child(t!("decide.multi_hint")));
    let reduce = cx.reduce_motion();
    let went = f32::from(draft.went);
    let visual = q.is_visual();
    let anything = own.is_some() || multi.is_some() || visual || !rows.is_empty();
    // Where the grid is among what scrolls, to bring the keys' tile into sight.
    let grid_at = usize::from(own.is_some()) + usize::from(multi.is_some());
    // What scrolls when the card has more than it has room for: the head and
    // the foot (Skip, Submit) stay, whole.
    let options = v_flex()
        .id(("decide-scroll", id))
        .w_full()
        .min_h_0()
        .overflow_y_scroll()
        .gap_2()
        .children(own)
        .children(multi)
        .children(grid.map(|g| div().w_full().when(visual, |d| d.pt_1().pb_0p5()).child(g)))
        .when(!rows.is_empty(), |c| c.child(v_flex().w_full().gap_1p5().children(rows)));
    let options = match shell.decide.scrolls.get(&id) {
        Some((scroll, seen)) => {
            follow(scroll, seen, &draft, visual.then_some((grid_at, &layout, tile_h)));
            options.track_scroll(scroll)
        }
        None => options,
    };
    // Measured whether or not it is a grid: the question's own pictures size by it too.
    let options = shots.measured(options, key);
    // The box for their own words stays under the options, never scrolled away: Tab reaches it.
    let body = v_flex().w_full().min_h_0().gap_2().child(div().flex_none().child(head)).when(anything, |b| b.child(options)).children(other.map(|o| div().flex_none().w_full().child(o))).children(attached.map(|a| div().flex_none().w_full().child(a)));
    // Each step slides in from the side it came from.
    let body: AnyElement = if reduce || draft.went == 0 {
        body.into_any_element()
    } else {
        body.with_animation(("decide-step", render::hash_of((id, draft.step))), Animation::new(motion::SLIDE).with_easing(motion::ease_out), move |el, t| el.left(px(went * 18. * (1. - t))).opacity(0.25 + 0.75 * t))
            .into_any_element()
    };

    // The foot: who asks, the keys, and Skip / Back / Next.
    let send_label = if last { t!("decide.submit") } else { t!("decide.next") };
    // A design choice's tiles send at once: Next is the quieter way.
    let next = {
        let handle = handle.clone();
        ui::button(p, ("decide-next", id), if reacts { Kind::Secondary } else { Kind::Primary }, send_label).key("⌘↵").disabled(!answered || !shell.can_act()).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Next, window, cx)).ok();
        })
    };
    let skip = {
        let handle = handle.clone();
        ui::button(p, ("decide-skip", id), Kind::Ghost, t!("decide.skip")).disabled(!shell.can_act()).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Skip, window, cx)).ok();
        })
    };
    let again = reacts.then(|| {
        let handle = handle.clone();
        ui::button(p, ("decide-again", id), Kind::Secondary, t!("decide.design_again")).icon(Icon::Retry).tip(t!("decide.design_again_tip"), None).disabled(!shell.can_act()).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_again(id, window, cx)).ok();
        })
    });
    let back = (draft.step > 0).then(|| {
        let handle = handle.clone();
        ui::button(p, ("decide-back", id), Kind::Ghost, t!("decide.back")).key("⇧←").on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Back, window, cx)).ok();
        })
    });
    let mut said = Vec::new();
    // More than one here: ‹ 3 / 23 › to go through them.
    if let Some(nav) = nav.filter(|n| n.count > 1) {
        said.push(nav_control(p, nav, &handle));
    }
    said.push(h_flex().gap(px(5.)).items_center().child(crate::theme::member_mark(shell.palette, &shell.model, &name, 15.)).child(t!("decide.asks", name = name)).into_any_element());
    if !q.header.is_empty() && total > 1 {
        said.push(div().child(format!("· {}", q.header)).into_any_element());
    }
    if let Some(t) = set.ticket {
        said.push(h_flex().gap(px(5.)).child("·").child(div().font_family(MONO).child(format!("T-{t}"))).into_any_element());
    }
    if let Some(words) = nav.and_then(|n| rest_words(n.rest)) {
        let open = handle.clone();
        said.push(
            div()
                .id(("decide-rest", id))
                .cursor_pointer()
                .hover(move |s| s.text_color(p.text2))
                .child(format!("· {words}"))
                .on_click(move |_, _, cx| {
                    open.update(cx, |shell, cx| shell.decide_list(true, cx)).ok();
                })
                .into_any_element(),
        );
    }
    let keys = match place {
        _ if q.options.is_empty() => t!("decide.keys_free"),
        Place::Docked if reacts => t!("decide.design_keys"),
        Place::Docked if q.is_visual() => t!("decide.keys_visual"),
        Place::Docked if q.multi_select => t!("decide.keys_multi"),
        Place::Docked => t!("decide.keys"),
        Place::Peek => t!("decide.keys_peek"),
    };
    let keys = match nav {
        Some(nav) if nav.count > 1 => format!("{} · {keys}", t!("decide.keys_cards")),
        _ => keys.to_string(),
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .flex_wrap()
        .child(
            v_flex()
                .flex_1()
                .min_w(px(crate::ui::WORDS_AT_LEAST))
                .gap_0p5()
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(p.muted)
                .child(h_flex().gap(px(5.)).items_center().flex_wrap().children(said))
                .child(div().opacity(0.8).overflow_hidden().whitespace_nowrap().text_ellipsis().child(keys)),
        )
        .child(h_flex().flex_none().ml_auto().gap_2().children(back).children(again).child(skip).child(next));
    let inner = v_flex().w_full().min_h_0().gap_3().child(body).child(foot.flex_none());
    // Never taller than the window has room for: past it, the options scroll.
    let tallest = card_tallest(shell, place);
    Some(match place {
        Place::Docked => v_flex()
            .id(("decide-card", id))
            .w_full()
            .max_h(px(tallest))
            .p(px(14.))
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(if is_fresh(shell, id) { p.accent } else { p.hair })
            .shadow_sm()
            .overflow_hidden()
            .child(inner)
            .into_any_element(),
        Place::Peek => v_flex().id(("decide-card", id)).w_full().max_h(px(tallest)).child(inner).into_any_element(),
    })
}

/// The options scrolled to what the keys are on: a new question from its
/// top; a tile the keys moved to, into sight — from where the grid (child
/// `grid` of what scrolls) was drawn last, the tile where the layout puts it.
fn follow(scroll: &ScrollHandle, seen: &Cell<Option<(usize, usize)>>, draft: &Draft, grid: Option<(usize, &visual::Grid, f32)>) {
    let now = (draft.step, draft.cursor);
    let before = seen.replace(Some(now));
    if before == Some(now) {
        return;
    }
    let offset = scroll.offset();
    if before.is_none_or(|(step, _)| step != draft.step) {
        scroll.set_offset(point(offset.x, px(0.)));
        return;
    }
    let Some((at, layout, tile_h)) = grid else { return };
    let (Some(bounds), Some((_, y, _, h))) = (scroll.bounds_for_item(at), layout.rect(draft.cursor, tile_h, visual::TileState::default())) else { return };
    let view = scroll.bounds();
    let scrolled = -f32::from(offset.y);
    // The grid's top within what scrolls (a child's bounds are laid out
    // unscrolled; 4 points of padding over it), then the tile's.
    let top = f32::from(bounds.top() - view.top()) + 4. + y;
    let (room, margin) = (f32::from(view.size.height), 8.);
    let to = if top - margin < scrolled {
        top - margin
    } else if top + h + margin > scrolled + room {
        top + h + margin - room
    } else {
        return;
    };
    scroll.set_offset(point(offset.x, px(-to.max(0.))));
}

/// A number key's badge, or a key's name: filled when its row is picked.
fn key_badge(p: Palette, key: &str, on: bool) -> impl IntoElement {
    div()
        .flex_none()
        .min_w(px(20.))
        .h(px(20.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(if on { p.accent } else { p.hair })
        .when(on, |b| b.bg(p.accent))
        .font_family(MONO)
        .font_features(crate::theme::key_face())
        .text_size(px(11.))
        .text_color(if on { p.bg } else { p.muted })
        .child(key.to_owned())
}

/// One option: its label (strong) — "Recommended" beside it, in the
/// screen's language, when it is the one recommended — what it means
/// (quiet), its number key; picked, filled and outlined in the accent; the
/// keys on it, outlined.
#[allow(clippy::too_many_arguments)]
fn option_row(p: Palette, id: u64, n: usize, label: &str, recommended: bool, description: &str, picked: bool, cursor: bool, multi: bool, handle: &WeakEntity<Shell>) -> AnyElement {
    let handle = handle.clone();
    let check = multi.then(|| {
        div()
            .flex_none()
            .mt(px(2.))
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if picked { p.accent } else { p.muted })
            .when(picked, |c| c.bg(p.accent).child(icon(Icon::Check, 11., p.bg)))
    });
    let border = if picked {
        p.accent
    } else if cursor {
        Rgba { a: (p.accent.a * 0.5).max(0.35), ..p.accent }
    } else {
        p.hair
    };
    let row = h_flex()
        .id(("decide-option", render::hash_of((id, n))))
        .w_full()
        .px_3()
        .py(px(8.))
        .gap_2p5()
        .items_start()
        .rounded(px(R_MD))
        .border_1()
        .border_color(border)
        .when(picked, |r| r.bg(p.accent_bg))
        .when(!picked, |r| r.hover(move |s| s.bg(p.wash)))
        .cursor_pointer()
        .children(check)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().min_w_0().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label.to_owned()))
                        .when(recommended, |r| r.child(super::visuals::badge(p, false))),
                )
                .when(!description.is_empty(), |c| c.child(div().text_size(px(12.5)).line_height(px(18.)).text_color(if picked { p.text2 } else { p.muted }).child(description.to_owned()))),
        )
        .child(div().pt(px(1.)).child(key_badge(p, &(n + 1).to_string(), picked)))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.decide_pick(id, n, cx)).ok();
        });
    row.into_any_element()
}

/// How wide the grid of a card above the composer is, before it was drawn.
const GRID_GUESS: f32 = 640.;

/// How tall a decision's card may be where it is: past it, its options scroll.
fn card_tallest(shell: &Shell, place: Place) -> f32 {
    let room = if shell.decide.room > 0. { shell.decide.room } else { 760. };
    match place {
        Place::Docked => (room * 0.72).max(300.),
        Place::Peek => (room - 140.).max(320.),
    }
}

/// An option shown as its picture: the picture whole in the grid's 4:3 box
/// (a click opens it big, among the others) — or, without one, a quiet box
/// with its label — "Recommended" on it when it is the one recommended; under it
/// its label on one line, two of what it means (both whole under the
/// pointer) and its number key. Every tile of a grid is one size in every
/// state (`visuals::frame`): picked, a ring inside its border, a check on its
/// picture, its number filled and a faint tint, the words as legible as
/// ever; under the pointer, a shadow. Nothing moves.
#[allow(clippy::too_many_arguments)]
fn option_tile(shots: &Shots, id: u64, step: usize, n: usize, q: &Question, picked: bool, cursor: bool, w: f32, described: bool, actions: Option<Actions>, handle: &WeakEntity<Shell>, cx: &mut App) -> AnyElement {
    let p = shots.p;
    let option = &q.options[n];
    let (label, recommended) = option.shown();
    let key = render::hash_of(("decide-tile", id, step, n));
    let (fw, h) = (w - 2., (w * visual::RATIO).floor());
    let frame = visual::frame(visual::TileState { picked, cursor, hover: 0. });
    let face: AnyElement = match option.images.first() {
        Some(path) => {
            let more = option.images.len() - 1;
            let look = shots.ask(path, Size::Thumb, cx);
            let face = super::visuals::picture(shots, key, path, look, fw, h, R_LG - 1., true, 12.)
                .when(more > 0, |f| f.child(div().absolute().bottom(px(8.)).right(px(8.)).h(px(18.)).px(px(6.)).flex().items_center().rounded(px(9.)).bg(rgba(0x0000008C)).text_size(px(10.5)).text_color(rgb(0xFFFFFF)).child(format!("+{more}"))));
            let open = {
                let (handle, viewer) = (handle.clone(), Viewer::of_question(id, step, q, path));
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                    if let Some(viewer) = viewer.clone() {
                        handle.update(cx, |shell, cx| shell.open_viewer(viewer, cx)).ok();
                    }
                }
            };
            tip(div().id(("decide-shot", key)).cursor_pointer().child(face).on_click(open), p, t!("visuals.open"), None).into_any_element()
        }
        // No picture: its label, quietly, in the same box — a pick like the others.
        None => div()
            .size_full()
            .px_4()
            .flex()
            .items_center()
            .justify_center()
            .rounded_t(px(R_LG - 1.))
            .bg(p.wash)
            .child(div().max_w_full().text_center().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::MEDIUM).text_color(p.text2).line_clamp(3).text_ellipsis().child(label.clone()))
            .into_any_element(),
    };
    let face = div()
        .relative()
        .flex_none()
        .w(px(fw))
        .h(px(h))
        .child(face)
        .when(recommended, |f| f.child(div().absolute().top(px(8.)).left(px(8.)).child(super::visuals::badge(p, false))))
        // One pick: a check on the picture (several: the box by the label says it).
        .when(picked && !q.multi_select, |f| {
            f.child(div().absolute().top(px(8.)).right(px(8.)).size(px(22.)).flex().items_center().justify_center().rounded_full().bg(p.accent).shadow_sm().child(icon(Icon::Check, 13., p.on_accent)))
        });
    let check = q.multi_select.then(|| {
        div()
            .flex_none()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if picked { p.accent } else { p.muted })
            .when(picked, |c| c.bg(p.accent).child(icon(Icon::Check, 11., p.bg)))
    });
    let caption_h = visual::caption_height(described);
    let description = option.description.trim();
    let whole = if description.is_empty() { label.clone() } else { format!("{label}\n{description}") };
    let caption = v_flex()
        .id(("decide-caption", key))
        .flex_none()
        .w_full()
        .h(px(caption_h + 1.))
        .px(px(10.))
        .pt(px(8.))
        .pb(px(9.))
        .gap_0p5()
        .overflow_hidden()
        .border_t_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .h(px(visual::CAPTION_TITLE))
                .gap_2()
                .children(check)
                .child(div().flex_1().min_w_0().text_size(px(13.5)).line_height(px(19.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).truncate().child(label))
                .child(key_badge(p, &(n + 1).to_string(), picked)),
        )
        .when(!description.is_empty(), |c| c.child(div().w_full().text_size(px(12.)).line_height(px(17.)).text_color(p.text2).line_clamp(2).text_ellipsis().child(description.to_owned())));
    let caption = tip(caption, p, whole, None);
    let border = if picked {
        p.accent
    } else if cursor {
        Rgba { a: (p.accent.a * 0.5).max(0.35), ..p.accent }
    } else {
        p.hair
    };
    let actions = actions.map(|a| tile_actions(p, id, n, key, w, a, picked || cursor, handle));
    let handle = handle.clone();
    let tile = v_flex()
        .id(("decide-option-tile", key))
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(visual::tile_height(w, described) + if actions.is_some() { ACTIONS_H } else { 0. }))
        .rounded(px(R_LG))
        .border(px(frame.border))
        .border_color(border)
        .bg(if picked { p.accent_bg } else { p.bg })
        .overflow_hidden()
        .cursor_pointer()
        .child(face)
        .child(caption)
        .children(actions)
        // The ring, inside the border: the tile is the size it was.
        .when(frame.ring > 0., |t| t.child(super::visuals::ring(p, R_LG - frame.border, frame.ring)))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.decide_pick(id, n, cx)).ok();
        });
    super::visuals::lift(tile, ("decide-lift", key), p).into_any_element()
}

/// How tall a design choice's tile's actions are, under its caption.
const ACTIONS_H: f32 = 38.;

/// A design choice's tile's actions: what its mix menu offers, whether it is
/// open, and whether anything can be sent.
struct Actions {
    menu: Vec<MixItem>,
    open: bool,
    can_act: bool,
}

/// About how wide words are on a small button, to know whether two fit.
fn words_width(words: &str) -> f32 {
    words.chars().map(|c| if c.is_ascii() { 7.4 } else { 13.5 }).sum()
}

/// "This one" — filled on the tile picked or under the keys, else outlined —
/// and "Mix" with its menu ("A with B's type"), only when another option has
/// something to give; on a tile too narrow for both words, "Mix" is its
/// chevron alone, named in a tip.
#[allow(clippy::too_many_arguments)]
fn tile_actions(p: Palette, id: u64, n: usize, key: u64, w: f32, actions: Actions, on: bool, handle: &WeakEntity<Shell>) -> AnyElement {
    let take = {
        let handle = handle.clone();
        ui::button(p, ("decide-take", key), if on { Kind::Primary } else { Kind::Secondary }, t!("decide.design_take")).tip(t!("decide.design_take_tip"), None).disabled(!actions.can_act).on_click(move |_, window, cx| {
            cx.stop_propagation();
            handle.update(cx, |shell, cx| shell.decide_take(id, n, window, cx)).ok();
        })
    };
    let mix = (!actions.menu.is_empty()).then(|| {
        let mix_words = t!("decide.design_mix");
        let both = words_width(t!("decide.design_take")) + words_width(mix_words) + 20. + 34. + 6. + 20.;
        let open = {
            let handle = handle.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                handle.update(cx, |shell, cx| shell.decide_mix_menu(id, n, cx)).ok();
            }
        };
        let button = if both <= w {
            ui::button(p, ("decide-mix", key), Kind::Ghost, mix_words).icon(Icon::ChevronDown).tip(t!("decide.design_mix_tip"), None)
        } else {
            ui::icon_button(p, ("decide-mix", key), Icon::ChevronDown, t!("decide.design_mix_tip"), None)
        };
        let button = button.selected(actions.open).disabled(!actions.can_act).on_click(open);
        let menu = actions.open.then(|| mix_menu(p, id, n, key, &actions.menu, handle));
        div().relative().flex_none().child(button).children(menu)
    });
    h_flex()
        .id(("decide-actions", key))
        .flex_none()
        .w_full()
        .h(px(ACTIONS_H))
        .px(px(10.))
        .pb(px(10.))
        .gap(px(6.))
        .items_center()
        .child(take)
        .children(mix)
        .into_any_element()
}

/// Option `base`'s mix menu, under its button: a line for each aspect
/// another option can give, a click sending the mix.
fn mix_menu(p: Palette, id: u64, base: usize, key: u64, items: &[MixItem], handle: &WeakEntity<Shell>) -> AnyElement {
    let lines = items.iter().enumerate().map(|(k, item)| {
        let (handle, from, aspect) = (handle.clone(), item.from, item.aspect.clone());
        h_flex()
            .id(("decide-mix-item", render::hash_of((key, k))))
            .w_full()
            .min_h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(R_SM))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .text_size(px(13.))
            .text_color(p.text)
            .child(div().flex_1().min_w_0().child(item.label.clone()))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                handle.update(cx, |shell, cx| shell.decide_mix(id, base, from, aspect.clone(), window, cx)).ok();
            })
    });
    let close = handle.clone();
    let surface = v_flex()
        .id(("decide-mix-menu", key))
        .min_w(px(220.))
        .max_w(px(340.))
        .p_1()
        .rounded(px(R_MD + 2.))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |shell, cx| shell.decide_mix_outside(cx)).ok();
        })
        .children(lines);
    div()
        .absolute()
        .top(px(crate::theme::CONTROL + 4.))
        .left_0()
        .child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(motion::pop(surface, ("decide-mix-in", key), true))).with_priority(2))
        .into_any_element()
}

/// The question's own pictures (what is there now, say), a click opens one
/// big: beside a grid of options, each a tile's size, from the grid's left
/// edge; over options without pictures, one big — several, tiles.
#[allow(clippy::too_many_arguments)]
fn question_pictures(shots: &Shots, id: u64, step: usize, q: &Question, width: f32, layout: &visual::Grid, cx: &mut App) -> AnyElement {
    let p = shots.p;
    let big = q.images.len() == 1 && !q.is_visual();
    // Big, it is asked for at the column's size, not a thumbnail's.
    let size = if big { Size::Inline } else { Size::Thumb };
    let (w, h) = if big {
        visual::lead_box(width)
    } else if q.is_visual() {
        (layout.tile, (layout.tile * visual::RATIO).floor())
    } else {
        let own = visual::grid(width, q.images.len());
        (own.tile, (own.tile * visual::RATIO).floor())
    };
    let tiles: Vec<AnyElement> = q
        .images
        .iter()
        .map(|path| {
            let key = render::hash_of(("decide-own", id, step, path));
            let viewer = Viewer::of_question(id, step, q, path);
            let handle = shots.handle();
            let face = super::visuals::picture(shots, key, path, shots.ask(path, size, cx), w - 2., h - 2., R_LG - 1., false, 10.);
            let tile = div()
                .id(("decide-own", key))
                .flex_none()
                .rounded(px(R_LG))
                .border_1()
                .border_color(p.hair)
                .overflow_hidden()
                .cursor_pointer()
                .child(face)
                .on_click(move |_, _, cx| {
                    if let Some(viewer) = viewer.clone() {
                        handle.update(cx, |shell, cx| shell.open_viewer(viewer, cx)).ok();
                    }
                });
            super::visuals::lift(tip(tile, p, visual::name_of(path).to_owned(), None), ("decide-own-lift", key), p).into_any_element()
        })
        .collect();
    h_flex().w_full().flex_wrap().gap(px(visual::GAP)).children(tiles).into_any_element()
}

/// The card folded (Esc, ⌄): one line, who asks and how many, and where the
/// rest are; a click opens it.
#[allow(clippy::too_many_arguments)]
fn folded(p: Palette, id: u64, set: &QuestionSet, name: &str, mark: AnyElement, rest: Option<String>, fresh: bool, handle: &WeakEntity<Shell>) -> AnyElement {
    let open = handle.clone();
    let close = handle.clone();
    let mut words = t!("decide.folded", name = name, count = set.questions.len());
    if let Some(review) = crate::decide::review_title(set) {
        words = format!("{review} · {words}");
    }
    if let Some(rest) = rest {
        words = format!("{words} · {rest}");
    }
    h_flex()
        .id(("decide-folded", id))
        .w_full()
        .h(px(40.))
        .pl_3()
        .pr_1()
        .gap_2()
        .items_center()
        .rounded(px(R_LG))
        .bg(p.raised)
        .border_1()
        .border_color(if fresh { p.accent } else { p.hair })
        .shadow_sm()
        .cursor_pointer()
        .text_size(px(SMALL))
        .child(div().size(px(7.)).rounded_full().bg(p.accent))
        .child(mark)
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(words))
        .child(icon(Icon::ChevronUp, 14., p.muted))
        .child(ui::icon_button(p, ("decide-folded-close", id), Icon::X, t!("decide.close"), None).on_click(move |_, window, cx| {
            cx.stop_propagation();
            close.update(cx, |shell, cx| shell.decide_dismiss(id, window, cx)).ok();
        }))
        .on_click(move |_, window, cx| {
            open.update(cx, |shell, cx| shell.decide_fold(id, false, window, cx)).ok();
        })
        .into_any_element()
}

/// The person's answers in the conversation: "Decided · 2 questions" and what was
/// decided in short; opened, each question and its answer.
#[allow(clippy::too_many_arguments)]
pub fn decided_row(p: Palette, tones: render::Tones, handle: WeakEntity<Shell>, key: u64, decided: &Decided, open: bool, window: &mut Window, cx: &mut App) -> AnyElement {
    let Decided::Answered(read) = decided else {
        return h_flex()
            .h(px(28.))
            .gap(px(8.))
            .items_center()
            .text_size(px(SMALL))
            .text_color(p.muted)
            .child(icon(Icon::XCircle, 13., p.muted))
            .child(t!("decide.closed"))
            .into_any_element();
    };
    let progress = motion::openness(("decided-open", key), open, window, cx);
    let summary = read.iter().map(|r| if r.skipped { t!("decide.skipped_short").to_owned() } else { rules::answer_text(r) }).collect::<Vec<_>>().join(" · ");
    let line = h_flex()
        .id(("decided", key))
        .min_h(px(28.))
        .gap(px(8.))
        .items_center()
        .text_size(px(SMALL))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| s.text_color(p.text2))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.chat_toggle(key, PART_DECIDED, cx)).ok();
        })
        .child(icon(Icon::Check, 13., tones.added))
        .child(div().flex_none().child(format!("{} · {}", t!("decide.decided"), t!("decide.count", count = read.len()))))
        .child(div().min_w_0().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(summary))
        .child(icon(Icon::ChevronRight, 12., p.muted).with_transformation(Transformation::rotate(radians(progress * std::f32::consts::FRAC_PI_2))));
    let body = (progress > 0.).then(|| {
        let items = read.iter().enumerate().map(|(n, r)| {
            let answer = if r.skipped { t!("decide.skipped").to_owned() } else { rules::answer_text(r) };
            v_flex()
                .gap_0p5()
                .child(h_flex().gap_1p5().items_baseline().child(div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(p.muted).child(format!("{}.", n + 1))).child(div().text_size(px(12.5)).line_height(px(18.)).text_color(p.text2).child(r.question.clone())))
                .child(
                    h_flex()
                        .pl(px(18.))
                        .gap_1p5()
                        .items_start()
                        .child(div().flex_none().text_color(if r.skipped { p.muted } else { tones.added }).child("→"))
                        .child(div().text_size(px(13.)).line_height(px(19.)).font_weight(if r.skipped { FontWeight::NORMAL } else { FontWeight::SEMIBOLD }).text_color(if r.skipped { p.muted } else { p.text }).child(answer)),
                )
                .children(r.note.clone().map(|note| h_flex().pl(px(18.)).gap_1p5().items_start().child(div().pt(px(3.)).child(icon(Icon::Pencil, 11., p.muted))).child(div().text_size(px(12.5)).line_height(px(18.)).text_color(p.text2).child(note))))
        });
        motion::Reveal::new(("decided-body", key), progress, div().opacity(progress).ml(px(21.)).mt_1().px_3().py_2().rounded(px(R_SM)).bg(p.code_bg).child(v_flex().gap_2().children(items)).into_any_element())
    });
    v_flex().w_full().child(line).children(body).into_any_element()
}

/// The Inbox's row for a set: a mark, its ticket, the first question (and how
/// many more), who asks.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, Option<u32>, String, Rgba)> {
    let set = shell.model.asked.get(id)?;
    let first = set.questions.first().map(|q| q.question.clone()).unwrap_or_default();
    let title = match set.questions.len() {
        // The weekly design review, the weekly retro: called so, whatever they ask.
        _ if set.review || set.retro.is_some() => crate::decide::review_title(set).unwrap_or(first),
        0 | 1 => first,
        n => format!("{first} {}", t!("decide.and_more", count = n - 1)),
    };
    let (name, color) = who(shell, set);
    Some((title, set.ticket, name, color))
}

/// The mark the Inbox shows a decision with.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("decide-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Message, 13., p.accent)), p, t!("state.section_decide"), None).into_any_element()
}

#[cfg(test)]
mod tests {
    use equipe_core::protocol::decide::{Question, QuestionSet};

    #[test]
    fn the_ask_to_tidy_lists_each_older_question_with_who_asked_its_ticket_and_how_long_it_waited() {
        let day = 86_400;
        let set = |id: u64, member: &str, ticket: Option<u32>, questions: &[&str], ago: u64| QuestionSet {
            id,
            session: 1,
            member: member.into(),
            ticket,
            questions: questions.iter().map(|q| Question { question: (*q).into(), ..Question::default() }).collect(),
            asked_at: 20 * day - ago,
            review: false,
            retro: None,
        };
        let a = set(1, "backend", Some(318), &["How did the\nAWS sign-up go?"], 5 * day);
        let b = set(2, "pm", None, &["Where first?", "For whom?"], 4 * day);
        let text = super::tidy_message(&[&a, &b], 20 * day);
        assert!(text.contains("3일 넘게 기다린"), "{text}");
        assert!(text.contains("- backend · T-318: How did the AWS sign-up go? (5일 전)"), "{text}");
        assert!(text.contains("- pm: Where first? 외 1개 (4일 전)"), "{text}");
        assert!(text.contains("닫을 수 있게"), "asks which can be closed — the person closes them: {text}");
    }
}
