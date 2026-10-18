//! Handed-in work the person said in words to accept, drawn: the card docked
//! above the composer in the conversation it was said in — "T-1, T-3 수락할까요?",
//! each ticket with its size (rest on its number for the ticket's own card),
//! a box to leave one out when there are several, what starts after, how code
//! lands, and [모두 수락하기 ⌘↵] [아니요] — and the same card in 나에게's peek,
//! under "결정 필요". Confirmed, it leaves everywhere; the daemon's notices in
//! the PM's conversation ("T-42을(를) 수락했습니다") are what remains. The
//! rules are `crate::accept`'s.

use equipe_core::protocol::accept::{AcceptProposal, Verdict};
use equipe_core::protocol::{Ticket, TicketId};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::accept::{self as rules};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::state::Need;
use crate::theme::{MONO, Palette, R_LG, R_MD, SMALL, look};
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
    /// The card the conversation on screen shows: the oldest said in it.
    pub fn accept_shown(&self) -> Option<u64> {
        if self.model.opened.is_some() {
            return None;
        }
        let here = self.model.selected?;
        self.model.accepts.shown_in(here).map(|c| c.id)
    }

    /// How ticket `id` lands once confirmed, by its project's way here.
    fn accept_landing(&self, card: &AcceptProposal, id: TicketId) -> Option<equipe_core::protocol::accept::Land> {
        let t = self.model.tickets.iter().find(|t| t.id == id)?;
        rules::landing(t, card.verdict, self.merge_way_of(&self.project_of(t)))
    }

    /// The person's answer to card `id`, as their command.
    pub fn accept_decide(&mut self, id: u64, confirm: bool, cx: &mut Context<Self>) {
        let Some(card) = self.model.accepts.get(id).cloned() else { return };
        let squad = self.model.squad.clone();
        let lands: Vec<(TicketId, Option<equipe_core::protocol::accept::Land>)> = card.tickets.iter().map(|t| (*t, self.accept_landing(&card, *t))).collect();
        let kept = self.model.accepts.kept(id);
        let land = |t: TicketId| lands.iter().find(|(id, _)| *id == t).and_then(|(_, l)| *l);
        if let Some(command) = self.model.accepts.decide(&squad, id, confirm, land) {
            self.send_command(command);
            if confirm {
                let shown = AcceptProposal { tickets: kept, ..card };
                self.say(t!("accept.sent", what = what(&shown)));
            }
            cx.notify();
        }
    }

    /// Tick or untick a ticket on card `id`.
    pub fn accept_toggle(&mut self, id: u64, ticket: TicketId, cx: &mut Context<Self>) {
        self.model.accepts.toggle(id, ticket);
        cx.notify();
    }

    /// ⌘⏎ confirms the card on show: above an empty composer when no other
    /// card has the keys, or picked in 나에게's peek. True when taken.
    pub fn on_accept_key(&mut self, key: &str, m: &Modifiers, cx: &mut Context<Self>) -> bool {
        if key != "enter" || !m.platform || m.shift || m.control || m.alt || !self.can_act() {
            return false;
        }
        let id = match self.screen {
            Screen::Chat if self.composer.text(cx).is_empty() && self.model.approvals.first().is_none() && self.decide_shown().is_none() => self.accept_shown(),
            Screen::Board if self.board.view.peek => match self.picked_need() {
                Some(Need::Accept(id)) => Some(id),
                _ => None,
            },
            _ => None,
        };
        let Some(id) = id.filter(|id| !self.model.accepts.kept(*id).is_empty()) else { return false };
        self.accept_decide(id, true, cx);
        true
    }

    /// The size of each ticket on a card, asked of git once per look of it.
    pub fn accept_stats(&mut self) {
        let wanted: Vec<Ticket> = self
            .model
            .accepts
            .open()
            .flat_map(|c| c.tickets.clone())
            .filter_map(|id| self.model.tickets.iter().find(|t| t.id == id && t.branch.is_some()).cloned())
            .filter(|t| self.board.stat(t).is_none())
            .collect();
        for t in wanted {
            let seen = crate::diffs::fingerprint(&t);
            if self.accept_asked.get(&t.id) == Some(&seen) {
                continue;
            }
            if let Some(source) = crate::diffs::source(&self.model, &t) {
                self.accept_asked.insert(t.id, seen);
                self.fetch(t.id, source, false);
            }
        }
    }
}

/// What a card does, in a few words: "T-42, T-71 수락".
fn what(card: &AcceptProposal) -> String {
    let tickets = rules::list(&card.tickets);
    match card.verdict {
        Verdict::Accept => t!("accept.what", tickets = tickets),
        Verdict::Merge => t!("accept.what_merge", tickets = tickets),
        Verdict::Changes => t!("accept.what_changes", tickets = tickets),
    }
}

/// The card above the composer, for the conversation on screen.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let id = shell.accept_shown()?;
    let body = card(shell, id, Place::Docked, cx)?;
    Some(motion::rise(div().w_full().mb_3().child(body), ("accept-in", id)).into_any_element())
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

/// 나에게's row for a card: its question, who raised it.
pub fn row_text(shell: &Shell, id: u64) -> Option<(String, String, Rgba)> {
    let card = shell.model.accepts.get(id)?;
    let (name, color) = who(shell, card);
    Some((rules::title(card), name, color))
}

/// The mark 나에게 shows a card to confirm with.
pub fn mark(p: Palette) -> AnyElement {
    tip(div().id("accept-mark").size(px(14.)).flex_none().flex().items_center().justify_center().child(icon(Icon::Check, 13., p.accent)), p, t!("accept.title"), None).into_any_element()
}

/// Who raised it: the member's dot and words, or the person's own words.
fn who(shell: &Shell, card: &AcceptProposal) -> (String, Rgba) {
    if card.from_words() {
        return (t!("accept.by_words").to_owned(), shell.palette.muted);
    }
    let role = shell.model.members.iter().find(|m| m.name == card.by).map_or(card.by.as_str(), |m| m.role.as_str());
    let (_, hue) = look(role, &card.by);
    (t!("accept.by", name = card.by), shell.palette.member(hue))
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

/// A ticket on the card: its box (when there are several), its number — the
/// ticket's own card on the pointer — its title, and its size.
fn ticket_row(shell: &Shell, card: &AcceptProposal, ticket: TicketId, several: bool, place: Place, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = card.id;
    let kept = shell.model.accepts.keeps(id, ticket);
    let found = shell.model.tickets.iter().find(|t| t.id == ticket);
    let title = found.map(|t| t.title.clone()).unwrap_or_default();
    let size = found.and_then(|t| shell.board.stat(t)).map(|s| {
        h_flex()
            .flex_none()
            .gap_1p5()
            .font_family(MONO)
            .text_size(px(11.5))
            .child(div().text_color(p.accent).child(format!("+{}", s.added)))
            .child(div().text_color(p.muted).child(format!("−{}", s.removed)))
    });
    let check = several.then(|| {
        div()
            .flex_none()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if kept { p.accent } else { p.muted })
            .when(kept, |c| c.bg(p.accent).child(icon(Icon::Check, 11., p.on_accent)))
    });
    let number = super::ticket_card::hover_link(
        div().id(SharedString::from(format!("accept-link-{}-{id}-{ticket}", place == Place::Peek))).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.text2).child(format!("T-{ticket}")),
        ticket,
        super::ticket_card::Side::Above,
    );
    let handle = cx.entity().downgrade();
    let row = h_flex()
        .id(SharedString::from(format!("accept-row-{}-{id}-{ticket}", place == Place::Peek)))
        .w_full()
        .h(px(30.))
        .px_2()
        .gap_2p5()
        .items_center()
        .rounded(px(6.))
        .children(check)
        .child(number)
        .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(if kept { p.text } else { p.muted }).when(!kept, |d| d.line_through()).child(title))
        .children(size);
    if several {
        tip(
            row.cursor_pointer().hover(|s| s.bg(p.wash)).on_click(move |_, _, cx| {
                handle.update(cx, |shell, cx| shell.accept_toggle(id, ticket, cx)).ok();
            }),
            p,
            t!("accept.keep", id = ticket),
            None,
        )
        .into_any_element()
    } else {
        row.into_any_element()
    }
}

/// One card: what it asks, the tickets, what follows, and the answers.
fn card(shell: &Shell, id: u64, place: Place, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let card = model.accepts.get(id)?.clone();
    let handle = cx.entity().downgrade();
    let (name, color) = who(shell, &card);
    let kept = model.accepts.kept(id);
    let several = card.tickets.len() > 1;

    let head = h_flex()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(Icon::Check, 13., p.accent))
        .child(div().text_color(p.accent).child(t!("accept.title")));
    let title = div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(rules::title(&card));
    let summary = (!card.summary.trim().is_empty()).then(|| div().text_size(px(13.)).line_height(px(19.)).text_color(p.text2).child(card.summary.clone()));
    let rows: Vec<AnyElement> = card.tickets.iter().map(|t| ticket_row(shell, &card, *t, several, place, cx)).collect();
    let list = v_flex().w_full().p_1().gap_0p5().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).children(rows);

    let mut details = Vec::new();
    if card.verdict == Verdict::Changes && !card.note.trim().is_empty() {
        details.push(detail(p, t!("accept.note"), card.note.clone()));
    }
    if let Some(next) = rules::starts(&card, &kept, &model.tickets) {
        details.push(detail(p, t!("accept.next"), next));
    }
    let project = |t: &Ticket| shell.project_of(t);
    let way = |name: &str| shell.merge_way_of(name);
    if let Some(lands) = rules::lands(&card, &kept, &model.tickets, project, way) {
        details.push(detail(p, t!("accept.lands"), lands));
    }
    let details = (!details.is_empty()).then(|| v_flex().w_full().px_1().gap_0p5().children(details));
    let only = h_flex()
        .gap_1p5()
        .items_start()
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(p.muted)
        .child(div().pt(px(2.)).child(icon(Icon::Shield, 12., p.muted)))
        .child(div().flex_1().min_w_0().child(t!("accept.only_click")));

    let more = model.accepts.ids().len().saturating_sub(1);
    let mut said = vec![h_flex().gap(px(5.)).items_center().child(div().size(px(6.)).rounded_full().bg(color)).child(name).into_any_element()];
    if more > 0 && place == Place::Docked {
        said.push(div().child(format!("· {}", t!("accept.more", n = more))).into_any_element());
    }
    let can = shell.can_act();
    let yes = {
        let handle = handle.clone();
        ui::button(p, ("accept-yes", id), Kind::Primary, rules::yes_label(&card, kept.len())).key("⌘↵").disabled(!can || kept.is_empty()).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.accept_decide(id, true, cx)).ok();
        })
    };
    let no = {
        let handle = handle.clone();
        ui::button(p, ("accept-no", id), Kind::Secondary, t!("accept.no")).disabled(!can).on_click(move |_, _, cx| {
            handle.update(cx, |shell, cx| shell.accept_decide(id, false, cx)).ok();
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
    let inner = v_flex().w_full().gap_2p5().child(head).child(v_flex().gap_1().child(title).children(summary)).child(list).children(details).child(only).child(foot);
    Some(match place {
        Place::Docked => div()
            .id(("accept-card", id))
            .w_full()
            .p(px(14.))
            .rounded(px(R_LG))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_sm()
            .child(inner)
            .into_any_element(),
        Place::Peek => div().id(("accept-card", id)).w_full().child(inner).into_any_element(),
    })
}
