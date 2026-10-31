//! A routine the PM proposed, drawn: the card docked above the composer in
//! the PM's conversation — "정기 업무 제안", its name, when it would run
//! ("매주 월 09:00"), what the PM would be asked each time, the PM's reason,
//! who proposed it, and [켜기 ⌘↵] [아니요] — and the same card in 나에게's
//! peek, under "결정 필요". Turned on, it joins 정기 업무 (`views::routines`);
//! decided, the card leaves everywhere. The rules are `crate::routine`'s.

use equipe_core::protocol::routines::RoutineProposal;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::state::Need;
use crate::theme::{Palette, R_LG, R_MD, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

use super::routines::schedule_words;

/// Where a card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Docked,
    Peek,
}

impl Shell {
    /// The proposal the conversation on screen shows: the oldest — in the PM's.
    pub fn routine_shown(&self) -> Option<u64> {
        let member = self.model.selected_member()?;
        if self.model.opened.is_some() || !crate::state::Model::is_pm(member) {
            return None;
        }
        self.model.routine_asks.first().map(|r| r.id)
    }

    /// The person's answer to proposal `id`, as their command.
    pub fn routine_decide(&mut self, id: u64, approve: bool, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        if let Some(command) = self.model.routine_asks.decide(&squad, id, approve) {
            self.send_command(command);
            cx.notify();
        }
    }

    /// ⌘⏎ turns on the proposal on show: above an empty composer when no
    /// other card has the keys, or picked in 나에게's peek. True when taken.
    pub fn on_routine_key(&mut self, key: &str, m: &Modifiers, cx: &mut Context<Self>) -> bool {
        if key != "enter" || !m.platform || m.shift || m.control || m.alt || !self.can_act() {
            return false;
        }
        let id = match self.screen {
            Screen::Chat if self.composer.text(cx).is_empty() && self.model.approvals.first().is_none() && self.decide_shown().is_none() && self.team_shown().is_none() => self.routine_shown(),
            Screen::Board if self.board.view.peek => match self.picked_need() {
                Some(Need::Routine(id)) => Some(id),
                _ => None,
            },
            _ => None,
        };
        let Some(id) = id else { return false };
        self.routine_decide(id, true, cx);
        true
    }
}

/// The card above the composer, for the proposal the PM's conversation shows.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let id = shell.routine_shown()?;
    let body = card(shell, id, Place::Docked, cx)?;
    Some(motion::rise(div().w_full().mb_3().child(body), ("routine-in", id)).into_any_element())
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
        .children(card(shell, id, Place::Peek, cx))
        .into_any_element()
}

/// 나에게's row for a proposal: its name and when, who proposed it.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String, Rgba)> {
    let proposal = shell.model.routine_asks.get(id)?;
    let (name, color) = who(shell, proposal);
    Some((t!("routine_ask.row", title = proposal.routine.title, when = schedule_words(proposal.routine.when)), name, color))
}

/// The mark 나에게 shows a proposed routine with.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("routine-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Clock, 13., p.accent)), p, t!("routine_ask.title"), None).into_any_element()
}

/// Who proposed it: their dot and name.
fn who(shell: &Shell, proposal: &RoutineProposal) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.name == proposal.by).map_or(proposal.by.as_str(), |m| m.role.as_str());
    let (_, hue) = look(role, &proposal.by);
    (proposal.by.clone(), shell.palette.member(hue))
}

/// One detail: a quiet label and its value.
fn detail(p: Palette, label: &'static str, value: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .text_size(px(12.5))
        .line_height(px(19.))
        .child(div().w(px(92.)).flex_none().text_color(p.muted).child(label))
        .child(div().flex_1().min_w_0().text_color(p.text).child(value))
        .into_any_element()
}

/// One proposal: its name, when it runs, what it asks, why, and the answers.
fn card(shell: &Shell, id: u64, place: Place, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let proposal = shell.model.routine_asks.get(id)?.clone();
    let handle = cx.entity().downgrade();
    let (name, color) = who(shell, &proposal);

    let head = h_flex()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(Icon::Clock, 13., p.accent))
        .child(div().text_color(p.accent).child(t!("routine_ask.title")));
    let title = div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(proposal.routine.title.clone());
    let reason = (!proposal.reason.trim().is_empty()).then(|| div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(proposal.reason.clone()));

    let clamp = if place == Place::Docked { 4 } else { 12 };
    let details = v_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_0p5()
        .rounded(px(R_MD))
        .bg(p.wash)
        .border_1()
        .border_color(p.hair)
        .child(detail(p, t!("routine_ask.when"), div().child(schedule_words(proposal.routine.when)).into_any_element()))
        .child(detail(p, t!("routine_ask.what"), div().line_clamp(clamp).text_ellipsis().child(proposal.routine.prompt.clone()).into_any_element()));
    let note = h_flex()
        .gap_1p5()
        .items_start()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(p.muted)
        .child(div().pt(px(2.)).child(icon(Icon::Info, 12., p.muted)))
        .child(div().flex_1().min_w_0().child(t!("routine_ask.note")));

    let more = shell.model.routine_asks.ids().len().saturating_sub(1);
    let mut said = vec![h_flex().gap(px(5.)).items_center().child(div().size(px(6.)).rounded_full().bg(color)).child(t!("routine_ask.by", name = name)).into_any_element()];
    if more > 0 && place == Place::Docked {
        said.push(div().child(format!("· {}", t!("routine_ask.more", n = more))).into_any_element());
    }
    let yes = {
        let handle = handle.clone();
        ui::button(p, ("routine-yes", id), Kind::Primary, t!("routine_ask.approve")).key("⌘↵").disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.routine_decide(id, true, cx)).ok();
        })
    };
    let no = {
        let handle = handle.clone();
        ui::button(p, ("routine-no", id), Kind::Secondary, t!("routine_ask.decline")).disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.routine_decide(id, false, cx)).ok();
        })
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .flex_wrap()
        .child(h_flex().flex_1().min_w(px(crate::ui::WORDS_AT_LEAST)).gap(px(5.)).items_center().flex_wrap().text_size(px(SMALL)).text_color(p.muted).children(said))
        .child(h_flex().flex_none().ml_auto().gap_2().child(yes).child(no));
    let inner = v_flex().w_full().gap_2p5().child(head).child(v_flex().gap_1().child(title).children(reason)).child(details).child(note).child(foot);
    Some(match place {
        Place::Docked => div()
            .id(("routine-card", id))
            .w_full()
            .p(px(14.))
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_sm()
            .child(inner)
            .into_any_element(),
        Place::Peek => div().id(("routine-card", id)).w_full().child(inner).into_any_element(),
    })
}
