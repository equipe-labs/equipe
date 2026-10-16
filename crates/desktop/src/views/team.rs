//! A change to the team the PM proposed, drawn: the card docked above the
//! composer in the PM's conversation — "팀 변경 제안", what it does and what
//! waits for it, the PM's reason, the details (role, model, what it picks up
//! first), that it runs on the person's plan alongside the team, and
//! [추가하기 ⌘↵] [아니요] — and the same card in 나에게's peek, under "결정
//! 필요". Decided, it leaves everywhere; the daemon's notice in the PM's
//! conversation ("팀에 mobile-2을(를) 추가했어요") is what remains. The rules
//! are `crate::team`'s.

use equipe_core::protocol::team::{Change, TeamChange};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::state::Need;
use crate::team::{self as rules, Holder};
use crate::theme::{Palette, R_LG, R_MD, SMALL, look};
use crate::tips::tip;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// Where a card is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Docked,
    Peek,
}

impl Shell {
    /// The change the conversation on screen shows: the oldest — in the PM's.
    pub fn team_shown(&self) -> Option<u64> {
        let member = self.model.selected_member()?;
        if self.model.opened.is_some() || !crate::state::Model::is_pm(member) {
            return None;
        }
        self.model.team.first().map(|c| c.id)
    }

    /// The person's answer to change `id`, as their command.
    pub fn team_decide(&mut self, id: u64, approve: bool, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        if let Some(command) = self.model.team.decide(&squad, id, approve) {
            self.send_command(command);
            cx.notify();
        }
    }

    /// ⌘⏎ approves the change on show: above an empty composer when no other
    /// card has the keys, or picked in 나에게's peek. True when taken.
    pub fn on_team_key(&mut self, key: &str, m: &Modifiers, cx: &mut Context<Self>) -> bool {
        if key != "enter" || !m.platform || m.shift || m.control || m.alt || !self.can_act() {
            return false;
        }
        let id = match self.screen {
            Screen::Chat if self.composer.text(cx).is_empty() && self.model.approvals.first().is_none() && self.decide_shown().is_none() => self.team_shown(),
            Screen::Board if self.board.view.peek => match self.picked_need() {
                Some(Need::Team(id)) => Some(id),
                _ => None,
            },
            _ => None,
        };
        let Some(id) = id else { return false };
        self.team_decide(id, true, cx);
        true
    }
}

/// The card above the composer, for the change the PM's conversation shows.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let id = shell.team_shown()?;
    let body = card(shell, id, Place::Docked, cx)?;
    Some(motion::rise(div().w_full().mb_3().child(body), ("team-in", id)).into_any_element())
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

/// 나에게's row for a change: its headline, who proposed it.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String, Rgba)> {
    let change = shell.model.team.get(id)?;
    let (name, color) = who(shell, change);
    Some((headline(shell, change), name, color))
}

/// The mark 나에게 shows a change to the team with.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("team-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Users, 13., p.accent)), p, t!("team.title"), None).into_any_element()
}

/// Who proposed it: their dot and name.
fn who(shell: &Shell, change: &TeamChange) -> (String, Rgba) {
    let role = shell.model.members.iter().find(|m| m.name == change.by).map_or(change.by.as_str(), |m| m.role.as_str());
    let (_, hue) = look(role, &change.by);
    (change.by.clone(), shell.palette.member(hue))
}

/// The card's first line, from the team and the board as they are now.
fn headline(shell: &Shell, change: &TeamChange) -> String {
    let model = &shell.model;
    let team: Vec<Holder<'_>> = model.members.iter().map(|m| Holder { name: &m.name, role: &m.role, ticket: m.ticket }).collect();
    let waiting = change.unblocks.first().and_then(|id| model.since.get(id)).map(|at| model.unix.saturating_sub(*at));
    rules::headline(change, &team, waiting)
}

/// One detail: a quiet label and its value.
fn detail(p: Palette, label: &'static str, value: String) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_baseline()
        .text_size(px(12.5))
        .line_height(px(19.))
        .child(div().w(px(92.)).flex_none().text_color(p.muted).child(label))
        .child(div().flex_1().min_w_0().text_color(p.text).child(value))
        .into_any_element()
}

/// One change: what it does, why, the details, and the answers.
fn card(shell: &Shell, id: u64, place: Place, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let change = model.team.get(id)?.clone();
    let handle = cx.entity().downgrade();
    let (name, color) = who(shell, &change);

    let head = h_flex()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(Icon::Users, 13., p.accent))
        .child(div().text_color(p.accent).child(t!("team.title")));
    let title = div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(headline(shell, &change));
    let reason = (!change.reason.trim().is_empty()).then(|| div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(change.reason.clone()));

    let mut details = vec![detail(p, t!("team.role"), change.role.clone())];
    match change.change {
        Change::Add => {
            details.push(detail(p, t!("team.model"), rules::model(&change)));
            if let Some(first) = rules::first_work(&change, &model.tickets) {
                details.push(detail(p, t!("team.first"), first));
            }
        }
        Change::Remove => {
            if let Some(ticket) = change.busy_on {
                details.push(detail(p, t!("team.now_on"), t!("team.back_to_board", id = ticket)));
            }
        }
    }
    let details = v_flex().w_full().px_3().py_2().gap_0p5().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).children(details);
    let quota = (change.change == Change::Add).then(|| {
        h_flex()
            .gap_1p5()
            .items_start()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(p.muted)
            .child(div().pt(px(2.)).child(icon(Icon::Info, 12., p.muted)))
            .child(div().flex_1().min_w_0().child(t!("team.quota")))
    });

    let more = model.team.ids().len().saturating_sub(1);
    let mut said = vec![h_flex().gap(px(5.)).items_center().child(div().size(px(6.)).rounded_full().bg(color)).child(t!("team.by", name = name)).into_any_element()];
    if more > 0 && place == Place::Docked {
        said.push(div().child(format!("· {}", t!("team.more", n = more))).into_any_element());
    }
    let yes = {
        let handle = handle.clone();
        ui::button(p, ("team-yes", id), Kind::Primary, rules::yes_label(&change)).key("⌘↵").disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.team_decide(id, true, cx)).ok();
        })
    };
    let no = {
        let handle = handle.clone();
        ui::button(p, ("team-no", id), Kind::Secondary, t!("team.decline")).disabled(!shell.can_act()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.team_decide(id, false, cx)).ok();
        })
    };
    let foot = h_flex()
        .w_full()
        .pt_3()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .child(h_flex().flex_1().min_w_0().gap(px(5.)).items_center().flex_wrap().text_size(px(SMALL)).text_color(p.muted).children(said))
        .child(yes)
        .child(no);
    let inner = v_flex().w_full().gap_2p5().child(head).child(v_flex().gap_1().child(title).children(reason)).child(details).children(quota).child(foot);
    Some(match place {
        Place::Docked => div()
            .id(("team-card", id))
            .w_full()
            .p(px(14.))
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_sm()
            .child(inner)
            .into_any_element(),
        Place::Peek => div().id(("team-card", id)).w_full().child(inner).into_any_element(),
    })
}
