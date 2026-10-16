//! A member's questions (`ask_decision`), drawn: the card docked above the
//! composer in the asking member's conversation (and the PM's) — "1/2" and
//! the question, fold ⌄ and close ✕; each option a row with its label, a
//! line on what it means and its number key; a last row for words of the
//! person's own; Skip, Back and Next ⌘↵ (Submit on the last). The same card
//! answers a set picked in 나에게's peek. Sent, the answers' message folds in
//! the conversation to one line, "결정함 · 2개", that opens to each question
//! and what was decided. The rules are `crate::decide`'s.

use std::collections::HashMap;

use equipe_core::protocol::Command;
use equipe_core::protocol::decide::{Answer, QuestionSet};
use equipe_core::t;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::decide::{self as rules, Decided, Draft, Effect, Key};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::render;
use crate::state::Need;
use crate::theme::{MONO, Palette, R_LG, R_MD, R_SM, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The disclosure of a decided row.
pub const PART_DECIDED: u32 = (1 << 20) + 9;

/// Where a card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Above the composer: it folds (⌄) and closes (✕).
    Docked,
    /// 나에게's peek, under its own heading.
    Peek,
}

/// The card's state that lives with the window: where the person is in each
/// set, and the boxes for their own words, by set and question.
#[derive(Default)]
pub struct DecideUi {
    pub drafts: HashMap<u64, Draft>,
    pub fields: HashMap<(u64, usize), Entity<TextareaState>>,
    subscriptions: HashMap<(u64, usize), Subscription>,
}

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
        for set in sets {
            self.decide.drafts.entry(set.id).or_insert_with(|| Draft::new(&set));
            for n in 0..set.questions.len() {
                let k = (set.id, n);
                if self.decide.fields.contains_key(&k) {
                    continue;
                }
                // A question without options is answered in this box alone.
                let placeholder = if set.questions[n].options.is_empty() { t!("decide.free_placeholder") } else { t!("decide.other_placeholder") };
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
    }

    /// The set the conversation on screen shows: the oldest of the member's
    /// own — in the PM's, of everyone's.
    pub fn decide_shown(&self) -> Option<u64> {
        let member = self.model.selected_member()?;
        if self.model.opened.is_some() {
            return None;
        }
        let is_pm = crate::state::Model::is_pm(member);
        self.model.asked.in_conversation(&member.name, is_pm).first().map(|s| s.id)
    }

    /// The words typed in each of a set's boxes, by question.
    fn decide_others(&self, id: u64, cx: &App) -> Vec<String> {
        let count = self.model.asked.get(id).map_or(0, |s| s.questions.len());
        (0..count).map(|n| self.decide.fields.get(&(id, n)).map(|f| f.read(cx).value().to_string()).unwrap_or_default()).collect()
    }

    /// Whether the box for their own words of set `id`'s question on show has the typing.
    fn decide_in_other(&self, id: u64, window: &Window, cx: &App) -> bool {
        let step = self.decide.drafts.get(&id).map_or(0, |d| d.step);
        self.decide.fields.get(&(id, step)).is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }

    /// A keystroke, before anyone else sees it: the card's keys while it is
    /// open above an empty composer (or its own box has the typing); in
    /// 나에게's peek, the digits, Space, Tab and ⌘↵ only — the list keeps ↑↓,
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
                _ => return Took::Pass,
            },
            Screen::Settings => return Took::Pass,
        };
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
        let Some(k) = rules::key_of(key, m.platform, m.shift) else {
            return if in_other { Took::Typing } else { Took::Pass };
        };
        if m.platform && k != Key::Next {
            return Took::Pass;
        }
        if peek && matches!(k, Key::Up | Key::Down | Key::Enter | Key::Esc | Key::Back) && !in_other {
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
        let (Some(id), Some(k)) = (id, rules::key_of(key, cmd, false)) else { return false };
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
    let id = shell.decide_shown()?;
    let member = shell.model.selected_member()?;
    let more = shell.model.asked.in_conversation(&member.name, crate::state::Model::is_pm(member)).len().saturating_sub(1);
    let body = card(shell, id, Place::Docked, more, cx)?;
    Some(motion::rise(div().w_full().mb_3().child(body), ("decide-in", id)).into_any_element())
}

/// The card in 나에게's peek.
pub fn peek(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("peek")
        .size_full()
        .overflow_y_scroll()
        .p_4()
        .gap_3()
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(t!("state.section_decide")))
        .children(card(shell, id, Place::Peek, 0, cx))
        .into_any_element()
}

/// Who asks: their dot and name.
fn who(shell: &Shell, set: &QuestionSet) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.name == set.member).map_or(set.member.as_str(), |m| m.role.as_str());
    let (_, hue) = look(role, &set.member);
    (set.member.clone(), shell.palette.member(hue))
}

/// One set, at the step the person is on.
fn card(shell: &Shell, id: u64, place: Place, more: usize, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let set = shell.model.asked.get(id)?.clone();
    let draft = shell.decide.drafts.get(&id).cloned().unwrap_or_else(|| Draft::new(&set));
    let handle = cx.entity().downgrade();
    let (name, color) = who(shell, &set);
    if draft.collapsed && place == Place::Docked {
        return Some(folded(p, id, &set, &name, color, more, &handle));
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
        .child(div().flex_1().min_w_0().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(q.question.clone()))
        .child(h_flex().flex_none().mt(px(-3.)).mr(px(-6.)).gap_0p5().children(fold).child(close));

    // The options, then the box for their own words.
    let rows: Vec<AnyElement> = q.options.iter().enumerate().map(|(n, option)| option_row(p, id, n, &option.label, &option.description, draft.picked(n), draft.cursor == n, q.multi_select, &handle)).collect();
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
            .child(div().flex_1().min_w_0().text_size(px(14.)).child(Textarea::new(&field).appearance(false).bordered(false)))
            .child(key_badge(p, "Tab", false))
            .into_any_element()
    });
    let multi = q.multi_select.then(|| div().text_size(px(12.)).text_color(p.muted).child(t!("decide.multi_hint")));
    let reduce = cx.reduce_motion();
    let went = f32::from(draft.went);
    let body = v_flex().w_full().gap_2().child(head).children(multi).child(v_flex().w_full().gap_1p5().children(rows).children(other));
    // Each step slides in from the side it came from.
    let body: AnyElement = if reduce || draft.went == 0 {
        body.into_any_element()
    } else {
        body.with_animation(("decide-step", render::hash_of((id, draft.step))), Animation::new(motion::SLIDE).with_easing(motion::ease_out), move |el, t| el.left(px(went * 18. * (1. - t))).opacity(0.25 + 0.75 * t))
            .into_any_element()
    };

    // The foot: who asks, the keys, and Skip / Back / Next.
    let send_label = if last { t!("decide.submit") } else { t!("decide.next") };
    let next = {
        let handle = handle.clone();
        ui::button(p, ("decide-next", id), Kind::Primary, send_label).key("⌘↵").disabled(!answered || !shell.can_act()).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Next, window, cx)).ok();
        })
    };
    let skip = {
        let handle = handle.clone();
        ui::button(p, ("decide-skip", id), Kind::Ghost, t!("decide.skip")).disabled(!shell.can_act()).on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Skip, window, cx)).ok();
        })
    };
    let back = (draft.step > 0).then(|| {
        let handle = handle.clone();
        ui::button(p, ("decide-back", id), Kind::Ghost, t!("decide.back")).key("←").on_click(move |_, window, cx| {
            handle.update(cx, |shell, cx| shell.decide_step(id, Step::Back, window, cx)).ok();
        })
    });
    let mut said = vec![h_flex().gap(px(5.)).items_center().child(div().size(px(6.)).rounded_full().bg(color)).child(t!("decide.asks", name = name)).into_any_element()];
    if !q.header.is_empty() && total > 1 {
        said.push(div().child(format!("· {}", q.header)).into_any_element());
    }
    if let Some(t) = set.ticket {
        said.push(h_flex().gap(px(5.)).child("·").child(div().font_family(MONO).child(format!("T-{t}"))).into_any_element());
    }
    if more > 0 {
        said.push(div().child(format!("· {}", t!("decide.more", count = more))).into_any_element());
    }
    let keys = match place {
        _ if q.options.is_empty() => t!("decide.keys_free"),
        Place::Docked if q.multi_select => t!("decide.keys_multi"),
        Place::Docked => t!("decide.keys"),
        Place::Peek => t!("decide.keys_peek"),
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(p.muted)
                .child(h_flex().gap(px(5.)).items_center().flex_wrap().children(said))
                .child(div().opacity(0.8).overflow_hidden().whitespace_nowrap().text_ellipsis().child(keys)),
        )
        .children(back)
        .child(skip)
        .child(next);
    let inner = v_flex().w_full().gap_3().child(body).child(foot);
    Some(match place {
        Place::Docked => div()
            .id(("decide-card", id))
            .w_full()
            .p(px(14.))
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_sm()
            .overflow_hidden()
            .child(inner)
            .into_any_element(),
        Place::Peek => div().id(("decide-card", id)).w_full().child(inner).into_any_element(),
    })
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
        .text_size(px(11.))
        .text_color(if on { p.bg } else { p.muted })
        .child(key.to_owned())
}

/// One option: its label (strong), what it means (quiet), its number key;
/// picked, filled and outlined in the accent; the keys on it, outlined.
#[allow(clippy::too_many_arguments)]
fn option_row(p: Palette, id: u64, n: usize, label: &str, description: &str, picked: bool, cursor: bool, multi: bool, handle: &WeakEntity<Shell>) -> AnyElement {
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
                .child(div().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(label.to_owned()))
                .when(!description.is_empty(), |c| c.child(div().text_size(px(12.5)).line_height(px(18.)).text_color(p.muted).child(description.to_owned()))),
        )
        .child(div().pt(px(1.)).child(key_badge(p, &(n + 1).to_string(), picked)))
        .on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.decide_pick(id, n, cx)).ok();
        });
    row.into_any_element()
}

/// The card folded (Esc, ⌄): one line, who asks and how many; a click opens it.
fn folded(p: Palette, id: u64, set: &QuestionSet, name: &str, color: Rgba, more: usize, handle: &WeakEntity<Shell>) -> AnyElement {
    let open = handle.clone();
    let close = handle.clone();
    let mut words = t!("decide.folded", name = name, count = set.questions.len());
    if more > 0 {
        words = format!("{words} · {}", t!("decide.more", count = more));
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
        .border_color(p.hair)
        .shadow_sm()
        .cursor_pointer()
        .text_size(px(SMALL))
        .child(div().size(px(7.)).rounded_full().bg(p.accent))
        .child(div().size(px(6.)).rounded_full().bg(color))
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

/// The person's answers in the conversation: "결정함 · 2개" and what was
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
    let summary = read.iter().map(|r| if r.skipped { t!("decide.skipped_short").to_owned() } else { r.answer.clone() }).collect::<Vec<_>>().join(" · ");
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
            let answer = if r.skipped { t!("decide.skipped").to_owned() } else { r.answer.clone() };
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

/// 나에게's row for a set: a mark, its ticket, the first question (and how
/// many more), who asks.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, Option<u32>, String, Rgba)> {
    let set = shell.model.asked.get(id)?;
    let first = set.questions.first().map(|q| q.question.clone()).unwrap_or_default();
    let title = match set.questions.len() {
        0 | 1 => first,
        n => format!("{first} {}", t!("decide.and_more", count = n - 1)),
    };
    let (name, color) = who(shell, set);
    Some((title, set.ticket, name, color))
}

/// The mark 나에게 shows a decision with.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("decide-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Message, 13., p.accent)), p, t!("state.section_decide"), None).into_any_element()
}
