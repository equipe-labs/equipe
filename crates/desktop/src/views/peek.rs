//! The picked thing, at the right of the list (Space) or as a page (⏎): what
//! it is and what the person is asked. A review shows the hand-in, what to
//! check, the files changed, what it unblocks, the last words; a wait, what
//! to do (1–9 ticks lines off); a proposal, why; a request for permission,
//! its card. The answers sit at the foot, the words box (R) with them.

use equipe_core::t;

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Flow, btn, dot, hue_of, kbd, live_activity, member_color};
use super::icons;
use crate::board::{self as rules, MergeWay, Tab};
use crate::convo::Block;
use crate::state::{Need, local_hm, short_span, status_word};
use crate::theme::{MONO, Palette, SMALL, look};
use crate::Shell;

/// The peek: a scrolling body and the answers at its foot.
pub fn panel(shell: &Shell, need: Need, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    match need {
        Need::Permission(id) => v_flex()
            .id("peek")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(kicker(p, t!("state.section_permission")))
            .children(super::approvals::card(shell, id, false, cx))
            .into_any_element(),
        Need::Ticket(id) => {
            let Some(t) = shell.model.tickets.iter().find(|t| t.id == id).cloned() else { return div().into_any_element() };
            v_flex()
                .size_full()
                .child(
                    v_flex()
                        .id("peek")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&shell.board.peek_scroll)
                        .p_4()
                        .gap_4()
                        .child(head(shell, &t, false, cx))
                        .children(body(shell, &t, false, cx)),
                )
                .child(foot(shell, &t, cx))
                .into_any_element()
        }
    }
}

/// A ticket beside the conversation (a `T-n` in a row was clicked): what the
/// peek shows of it, to read; answering it is the board's (its ⤢).
pub fn side(shell: &Shell, id: TicketId, cx: &mut Context<Shell>) -> AnyElement {
    let Some(t) = shell.model.tickets.iter().find(|t| t.id == id).cloned() else { return div().into_any_element() };
    v_flex().id(("side-peek", id as u64)).size_full().overflow_y_scroll().p_4().gap_4().child(head(shell, &t, false, cx)).children(body(shell, &t, false, cx)).into_any_element()
}

/// The ticket as a page: its head, the tabs, and the tab's content.
pub fn page(shell: &Shell, t: &Ticket, tab: Tab, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let tab_button = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Shell>| {
        let on = tab == which;
        div()
            .id(id)
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .text_size(px(SMALL))
            .cursor_pointer()
            .border_b_2()
            .border_color(if on { p.text } else { transparent_black().into() })
            .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(p.text2).hover(move |s| s.text_color(p.text)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.view.page = Some(which);
                cx.notify();
            }))
            .child(label)
    };
    let content: Vec<AnyElement> = match tab {
        Tab::Overview => body(shell, t, true, cx),
        Tab::Changes => changes(shell, t, true, cx).into_iter().collect(),
        Tab::Activity => vec![activity(shell, t.id, 40)],
    };
    v_flex()
        .flex_1()
        .min_h_0()
        .child(
            v_flex()
                .flex_none()
                .px_6()
                .pt_4()
                .gap_3()
                .border_b_1()
                .border_color(p.hair)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(btn(p, "page-back", "←", Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                            this.board.view.page = None;
                            cx.notify();
                        })))
                        .child(div().flex_1()),
                )
                .child(head(shell, t, true, cx))
                .child(
                    h_flex()
                        .gap_1()
                        .child(tab_button("tab-overview", t!("peek.tab_overview"), Tab::Overview, cx))
                        .child(tab_button("tab-changes", t!("peek.tab_changes"), Tab::Changes, cx))
                        .child(tab_button("tab-activity", t!("peek.tab_activity"), Tab::Activity, cx)),
                ),
        )
        .child(h_flex().flex_1().min_h_0().justify_center().child(v_flex().id("page").w_full().max_w(px(820.)).h_full().overflow_y_scroll().px_6().py_5().gap_5().children(content)))
        .child(foot(shell, t, cx))
}


fn kicker(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text.into())
}

/// Icon, number, status; the title; where, who, the branch, how big, how long.
fn head(shell: &Shell, t: &Ticket, wide: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let _ = cx;
    let who = rules::who(t).map(str::to_owned);
    let mut facts: Vec<AnyElement> = Vec::new();
    if let Some(project) = model.ticket_project_name(t) {
        facts.push(h_flex().gap(px(6.)).items_center().child(dot(hue_of(p, model, t).unwrap_or(p.muted), 6.)).child(project).into_any_element());
    }
    if let Some(w) = &who {
        let member = model.members.iter().find(|m| m.name == *w || m.role == *w);
        let (mark, _) = member.map_or_else(|| look(w, w), |m| look(&m.role, &m.name));
        facts.push(h_flex().gap_1().text_color(member_color(p, model, w)).child(mark).child(w.clone()).into_any_element());
    }
    if let Some(branch) = t.branch.clone().filter(|_| !t.kind.in_home()) {
        facts.push(div().font_family(MONO).text_size(px(12.)).child(format!("{branch} → main")).into_any_element());
    }
    if let Some(stat) = shell.board.stat(t) {
        facts.push(div().child(stat.short()).into_any_element());
    }
    if let Some(since) = model.in_state_for(t) {
        facts.push(div().child(t!("peek.since", span = short_span(since))).into_any_element());
    }
    if t.kind.in_home() {
        facts.push(div().child(t!("board.kind_label", kind = equipe_core::i18n::label::kind(crate::i18n::lang(), t.kind))).into_any_element());
    }
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(icons::status(p, t.status, 14.))
                .child(div().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{}", t.id)))
                .child(div().text_size(px(12.)).text_color(icons::color(p, t.status)).child(if t.not_on_main() { t!("state.not_on_main") } else { status_word(t.status) })),
        )
        .child(div().text_size(px(if wide { 20. } else { 16. })).line_height(px(if wide { 28. } else { 24. })).font_weight(FontWeight::SEMIBOLD).child(t.title.clone()))
        .child(h_flex().flex_wrap().gap_x_3().gap_y_1().text_size(px(12.)).text_color(p.muted).children(facts))
        .into_any_element()
}

/// The body, by what the ticket asks.
fn body(shell: &Shell, t: &Ticket, wide: bool, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let handle = cx.entity().downgrade();
    let mut out: Vec<AnyElement> = Vec::new();
    let md = |key: &'static str, text: &str| super::chat::markdown(p, (key, t.id as u64), text, 14., 22., p.text2, &handle).into_any_element();
    match t.status {
        TicketStatus::Waiting => {
            let items = t.note.as_deref().map(rules::checklist).unwrap_or_default();
            let ticked = shell.board.ticked.get(&t.id).cloned().unwrap_or_default();
            if !items.is_empty() {
                out.push(
                    v_flex()
                        .gap_1()
                        .child(kicker(p, t!("peek.your_part")))
                        .children(items.iter().enumerate().map(|(i, item)| {
                            let on = ticked.contains(&i);
                            let id = t.id;
                            h_flex()
                                .id(("todo", i))
                                .min_h(px(30.))
                                .px_2()
                                .gap_3()
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(p.wash))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let set = this.board.ticked.entry(id).or_default();
                                    if !set.remove(&i) {
                                        set.insert(i);
                                    }
                                    cx.notify();
                                }))
                                .child(div().size(px(15.)).flex_none().flex().items_center().justify_center().rounded(px(4.)).border_1().border_color(if on { p.accent } else { p.muted }).when(on, |d| d.bg(p.accent).text_color(p.on_primary).text_size(px(10.)).child("✓")))
                                .child(div().flex_1().min_w_0().text_size(px(14.)).text_color(if on { p.muted } else { p.text }).when(on, |d| d.line_through()).child(item.clone()))
                                .children((i < 9).then(|| kbd(p, ["1", "2", "3", "4", "5", "6", "7", "8", "9"][i])))
                        }))
                        .into_any_element(),
                );
            } else if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
                out.push(md("peek-note", note));
            }
            if !t.body.trim().is_empty() {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.the_work"))).child(md("peek-body", &t.body)).into_any_element());
            }
        }
        TicketStatus::Review | TicketStatus::Done => {
            if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty()) {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.hand_in"))).child(md("peek-note", note)).into_any_element());
            }
            let checks = rules::checklist(&t.body);
            if !checks.is_empty() {
                out.push(
                    v_flex()
                        .gap_1()
                        .child(kicker(p, t!("peek.to_check")))
                        .children(checks.into_iter().map(|c| h_flex().gap_2().items_start().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(div().flex_none().text_color(p.muted).child("□")).child(div().flex_1().min_w_0().child(c))))
                        .into_any_element(),
                );
            } else if t.note.is_none() && !t.body.trim().is_empty() {
                out.push(md("peek-body", &t.body));
            }
            out.extend(changes(shell, t, wide, cx));
        }
        TicketStatus::Proposed => {
            if !t.body.trim().is_empty() {
                out.push(v_flex().gap_1().child(kicker(p, t!("peek.why"))).child(md("peek-body", &t.body)).into_any_element());
            }
            let facts = [
                (t!("peek.owner"), t.owner.clone().unwrap_or_else(|| "—".into())),
                (t!("peek.project"), model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into())),
                (t!("peek.proposed_by"), t.created_by.clone()),
            ];
            out.push(
                v_flex()
                    .gap_1()
                    .children(facts.into_iter().map(|(k, v)| h_flex().gap_3().text_size(px(13.)).child(div().w(px(64.)).flex_none().text_color(p.muted).child(k)).child(div().text_color(p.text2).child(v))))
                    .into_any_element(),
            );
        }
        _ => {
            if t.status == TicketStatus::InProgress {
                let mut live: Vec<String> = Vec::new();
                if let Some(a) = live_activity(model, t) {
                    live.push(a);
                }
                if let Some(eta) = model.eta(t) {
                    live.push(if eta.at <= model.unix { t!("board.eta_passed", span = short_span(model.unix - eta.at)) } else { t!("peek.eta", at = if eta.guessed { format!("~{}", local_hm(eta.at)) } else { local_hm(eta.at) }) });
                }
                if let Some(q) = model.quiet_for(t) {
                    live.push(format!("{} {}", t!("board.flag_quiet"), short_span(q)));
                }
                if !live.is_empty() {
                    out.push(h_flex().gap_2().items_center().text_size(px(13.)).text_color(p.text2).when(model.is_live(t), |d| d.child(icons::live_dot(p, shell.board.pulse))).child(live.join(" · ")).into_any_element());
                }
            }
            if let Some(note) = t.note.as_deref().filter(|n| !n.trim().is_empty() && t.status.is_closed()) {
                out.push(md("peek-note", note));
            }
            if !t.body.trim().is_empty() {
                out.push(md("peek-body", &t.body));
            }
            if t.branch.is_some() {
                out.extend(changes(shell, t, wide, cx));
            }
        }
    }
    out.extend(links(shell, t, cx));
    if !wide {
        out.push(activity(shell, t.id, 3));
    }
    out
}

/// The files a branch changed, each with its lines; D for the whole diff.
fn changes(shell: &Shell, t: &Ticket, wide: bool, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    t.branch.as_ref()?;
    let shown = if wide { 200 } else { 10 };
    let body: AnyElement = match shell.board.stat(t) {
        None => div().text_size(px(12.)).text_color(p.muted).child("…").into_any_element(),
        Some(stat) if stat.files.is_empty() => div().text_size(px(12.)).text_color(p.muted).child(t!("diff.none")).into_any_element(),
        Some(stat) => v_flex()
            .children(stat.files.iter().take(shown).map(|f| {
                h_flex()
                    .h(px(24.))
                    .gap_3()
                    .items_center()
                    .text_size(px(12.))
                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_color(p.text2).child(f.path.clone()))
                    .child(div().flex_none().text_color(p.accent).child(format!("+{}", f.added)))
                    .child(div().w(px(36.)).flex_none().text_align(TextAlign::Right).text_color(p.muted).child(format!("−{}", f.removed)))
            }))
            .when(stat.files.len() > shown, |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("peek.more_files", n = stat.files.len() - shown))))
            .into_any_element(),
    };
    let n = shell.board.stat(t).map_or(0, |s| s.files.len());
    Some(
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(kicker(p, t!("peek.files_changed", n = n)))
                    .child(div().flex_1())
                    .child(btn(p, "peek-diff", t!("peek.whole_diff"), Some("D"), false, true).h(px(24.)).on_click(cx.listener(|this, _, w, cx| this.board_open_diff(w, cx)))),
            )
            .child(body)
            .into_any_element(),
    )
}

/// What it waits for and what waits for it, each a click to it.
fn links(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let named = |dep: TicketId, cx: &mut Context<Shell>| {
        let found = model.tickets.iter().find(|x| x.id == dep);
        h_flex()
            .id(("link", dep as usize))
            .h(px(26.))
            .px_1()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board_reveal(dep);
                this.board_changed(cx);
            }))
            .children(found.map(|d| icons::status(p, d.status, 12.)))
            .child(div().w(px(40.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{dep}")))
            .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(found.map(|d| d.title.clone()).unwrap_or_default()))
            .into_any_element()
    };
    let mut out = Vec::new();
    let waits: Vec<AnyElement> = t.depends_on.iter().map(|d| named(*d, cx)).collect();
    if !waits.is_empty() {
        out.push(v_flex().child(kicker(p, t!("peek.waits_for"))).children(waits).into_any_element());
    }
    let unblocks: Vec<AnyElement> = model.unblocks(t.id).into_iter().map(|d| named(d, cx)).collect();
    if !unblocks.is_empty() {
        out.push(v_flex().child(kicker(p, t!("peek.unblocks"))).children(unblocks).into_any_element());
    }
    out
}

/// The last words of the work on a ticket.
fn activity(shell: &Shell, id: TicketId, turns: usize) -> AnyElement {
    let p = shell.palette;
    let convo = shell.model.convos.values().find(|c| c.has_work_on(id));
    let blocks: Vec<Block> = convo
        .map(|c| {
            let mut c = c.clone();
            c.showing = Some(id);
            c.blocks().into_iter().filter(|b| matches!(b, Block::User(_) | Block::Reply { .. } | Block::Error { .. })).collect()
        })
        .unwrap_or_default();
    let start = blocks.len().saturating_sub(turns);
    let who = shell.model.member_on(id).map(|m| (m.name.clone(), m.role.clone())).or_else(|| {
        let t = shell.model.tickets.iter().find(|t| t.id == id)?;
        let w = rules::who(t)?.to_owned();
        Some((w.clone(), shell.model.members.iter().find(|m| m.name == w).map_or(w, |m| m.role.clone())))
    });
    let (mark, hue) = who.as_ref().map_or(("·", 0), |(n, r)| look(r, n));
    let short = |s: &str| {
        let s = s.trim();
        if s.chars().count() > 280 { format!("{}…", s.chars().take(280).collect::<String>()) } else { s.to_owned() }
    };
    v_flex()
        .gap_2()
        .child(kicker(p, t!("peek.last_words")))
        .when(blocks.is_empty(), |c| c.child(div().text_size(px(12.)).text_color(p.muted).child(t!("peek.no_words"))))
        .children(blocks[start..].iter().map(|b| {
            let (lead, color, text) = match b {
                Block::User(text) => ("›".to_owned(), p.accent, short(text)),
                Block::Reply { text, .. } => (mark.to_owned(), p.member(hue), short(text)),
                Block::Error { message, .. } => ("✗".to_owned(), p.red, short(message)),
                _ => (String::new(), p.muted, String::new()),
            };
            h_flex().gap_2().items_start().text_size(px(13.)).line_height(px(20.)).child(div().w(px(14.)).flex_none().text_color(color).child(lead)).child(div().flex_1().min_w_0().text_color(p.text2).child(text))
        }))
        .into_any_element()
}

/// The foot: the question open, the words box, and the answers.
fn foot(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let can = shell.can_act();
    let id = t.id;
    let project = model.ticket_project_name(t).unwrap_or_else(|| t!("board.squad_folder").into());
    let way = shell.merge_way_of(&project);
    let mut buttons: Vec<AnyElement> = Vec::new();
    let mut add = |b: Stateful<Div>| buttons.push(b.into_any_element());
    match t.status {
        TicketStatus::Review => {
            let label = match (rules::needs_way(t), way) {
                (true, Some(w)) => t!("peek.accept_way", way = w.label()),
                _ => t!("peek.accept").to_owned(),
            };
            add(btn(p, "p-yes", label, Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))));
            add(btn(p, "p-reply", t!("peek.request_changes"), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))));
        }
        TicketStatus::Waiting => {
            add(btn(p, "p-yes", t!("peek.did_it"), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))));
            add(btn(p, "p-reply", t!("peek.memo_and_go"), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))));
        }
        TicketStatus::Proposed => {
            add(btn(p, "p-yes", t!("peek.take_proposal"), Some("A"), true, can).on_click(cx.listener(|this, _, _, cx| this.board_yes(cx))));
            add(btn(p, "p-no", t!("peek.drop"), Some("N"), false, can).on_click(cx.listener(|this, _, _, cx| this.board_no(cx))));
            add(btn(p, "p-reply", t!("peek.reply_pm"), Some("R"), false, can).on_click(cx.listener(|this, _, w, cx| this.board_open_reply(w, cx))));
        }
        TicketStatus::Done if t.not_on_main() => {
            add(btn(p, "p-merge", t!("peek.merge"), Some("M"), true, can).on_click(cx.listener(move |this, _, _, cx| {
                this.board.flow = Some(Flow::Method(id));
                cx.notify();
            })));
        }
        _ => {}
    }
    if rules::who(t).is_some() {
        add(btn(p, "p-talk", t!("peek.talk"), Some("C"), false, true).on_click(cx.listener(|this, _, w, cx| this.board_talk(w, cx))));
    }
    if matches!(t.status, TicketStatus::InProgress | TicketStatus::Todo | TicketStatus::Waiting | TicketStatus::Review) {
        add(btn(p, "p-cancel", t!("peek.cancel_ticket"), Some("N"), false, can).on_click(cx.listener(move |this, _, _, cx| {
            this.board.flow = Some(Flow::Cancel(id));
            cx.notify();
        })));
    }
    let question = shell.board.flow.filter(|f| match f {
        Flow::Reply(x) | Flow::Cancel(x) | Flow::Method(x) => *x == id,
        Flow::Way { ticket, .. } => *ticket == id,
        _ => false,
    });
    let card = question.map(|flow| question_card(shell, flow, t, &project, cx));
    v_flex()
        .flex_none()
        .px_4()
        .py_3()
        .gap_3()
        .border_t_1()
        .border_color(p.hair)
        .children(card)
        .when(question.is_none_or(|f| matches!(f, Flow::Reply(_))), |c| c.child(h_flex().flex_wrap().gap_2().children(buttons)))
        .into_any_element()
}

fn question_card(shell: &Shell, flow: Flow, t: &Ticket, project: &str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let can = shell.can_act();
    let id = t.id;
    let card = || v_flex().gap_2().p_3().rounded(px(8.)).bg(p.accent_bg).border_1().border_color(p.hair);
    let ask = |text: String| div().text_size(px(13.)).line_height(px(20.)).text_color(p.text).child(text);
    let ways = |cx: &mut Context<Shell>| {
        v_flex().gap_1().children(MergeWay::ALL.into_iter().enumerate().map(|(i, way)| {
            h_flex()
                .id(("way", i))
                .h(px(28.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, _, _, cx| this.board_choose_way(way, cx)))
                .child(kbd(p, ["1", "2", "3"][i]))
                .child(way.label())
        }))
    };
    match flow {
        Flow::Way { remember, .. } => card()
            .child(ask(t!("peek.ask_way", project = project)))
            .child(ways(cx))
            .child(
                h_flex()
                    .id("remember")
                    .gap_2()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.text2)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board.flow = Some(Flow::Way { ticket: id, remember: !remember });
                        cx.notify();
                    }))
                    .child(div().size(px(14.)).flex().items_center().justify_center().rounded(px(3.)).border_1().border_color(p.accent).when(remember, |d| d.bg(p.accent).text_color(p.on_primary).text_size(px(10.)).child("✓")))
                    .child(t!("peek.remember"))
                    .child(kbd(p, "Space")),
            )
            .into_any_element(),
        Flow::Method(_) => card().child(ask(t!("peek.ask_method", id = id))).child(ways(cx)).into_any_element(),
        Flow::Cancel(_) => card()
            .child(ask(t!("peek.ask_cancel", id = id)))
            .child(
                h_flex()
                    .gap_2()
                    .child(btn(p, "cancel-yes", t!("peek.cancel_ticket_now"), Some("Y"), true, can).on_click(cx.listener(move |this, _, _, cx| this.board_cancel(id, cx))))
                    .child(btn(p, "cancel-no", t!("board.keep"), Some("N"), false, true).on_click(cx.listener(|this, _, _, cx| {
                        this.board.flow = None;
                        cx.notify();
                    }))),
            )
            .into_any_element(),
        Flow::Reply(_) => {
            let (ask_text, send) = match t.status {
                TicketStatus::Waiting => (t!("peek.ask_memo", id = id), t!("peek.memo_send")),
                TicketStatus::Proposed => (t!("peek.ask_reply_pm").to_owned(), t!("peek.reply_send")),
                TicketStatus::Review | TicketStatus::Done => (t!("peek.ask_changes", who = rules::who(t).unwrap_or("—")), t!("peek.send_back")),
                _ => (t!("peek.ask_member", who = rules::who(t).unwrap_or("—")), t!("peek.reply_send")),
            };
            let empty = shell.board.reply.read(cx).value().trim().is_empty() && t.status != TicketStatus::Waiting;
            v_flex()
                .gap_2()
                .child(div().text_size(px(12.)).text_color(p.muted).child(ask_text))
                .child(div().w_full().px_3().py_2().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(14.)).child(Textarea::new(&shell.board.reply).appearance(false).bordered(false)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1())
                        .child(btn(p, "reply-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, w, cx| {
                            this.board.flow = None;
                            w.blur(cx);
                            cx.notify();
                        })))
                        .child(btn(p, "reply-send", send, Some("⌘⏎"), true, can && !empty).on_click(cx.listener(|this, _, w, cx| this.board_send_reply(w, cx)))),
                )
                .into_any_element()
        }
        Flow::Bulk { .. } | Flow::SaveView => div().into_any_element(),
    }
}

/// The name box for ⌥V, under the bar.
pub fn save_view_bar(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    if shell.board.flow != Some(Flow::SaveView) {
        return None;
    }
    Some(
        h_flex()
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .bg(p.raised)
            .child(div().flex_none().text_size(px(SMALL)).text_color(p.text2).child(t!("board.save_view_as")))
            .child(div().w(px(260.)).px_2().py_1().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Input::new(&shell.board.naming).appearance(false).bordered(false)))
            .child(btn(p, "save-view", t!("board.save"), Some("⏎"), true, true).on_click(cx.listener(|this, _, w, cx| this.board_save_view(w, cx))))
            .child(btn(p, "save-view-cancel", t!("peek.cancel"), Some("Esc"), false, true).on_click(cx.listener(|this, _, _, cx| {
                this.board.flow = None;
                cx.notify();
            })))
            .into_any_element(),
    )
}
