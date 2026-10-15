//! The sidebar (248 wide): a menu, not a list of tickets. The squad, then
//! PM 대화 · 나에게 · 작업 중 · 모든 일, the views kept, the projects (a click
//! slices everything to one), and the team with what each member is on.
//! Plain, still text — nothing here moves.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{count_all, count_working, dot};
use super::header::project_color;
use super::icons;
use crate::board::Menu;
use crate::convo::Sub;
use crate::state::{Conn, Member, Model, Side, Status, Verb};
use crate::theme::{Palette, SANS, SMALL, UI, look};
use crate::{Screen, Shell};

pub const WIDTH: f32 = 248.;
const ITEM: f32 = 30.;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let dim = matches!(model.conn, Conn::Down { had_snapshot: false, .. });
    let on_board = shell.screen == Screen::Board;
    let menu = shell.board.view.menu;
    let pm_on = shell.screen == Screen::Chat && model.side_open() == Some(Side::Pm);
    let needs = model.needs_you_count();
    let pm = model.pm();

    let item = |id: &'static str, icon: AnyElement, label: String, count: Option<(usize, bool)>, on: bool| {
        h_flex()
            .id(id)
            .h(px(ITEM))
            .flex_none()
            .mx_2()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(UI))
            .cursor_pointer()
            .when(on, |r| r.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |r| r.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .child(div().w(px(16.)).flex_none().flex().items_center().justify_center().child(icon))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(count.filter(|(n, _)| *n > 0).map(|(n, accent)| {
                div()
                    .flex_none()
                    .px(px(6.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .rounded(px(9.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(accent, |d| d.bg(p.accent).text_color(p.on_primary))
                    .when(!accent, |d| d.text_color(p.muted))
                    .child(n.to_string())
            }))
    };
    let glyph = |g: &'static str, color: Rgba| div().text_size(px(12.)).text_color(color).child(g).into_any_element();
    let pm_look = pm.and_then(|s| model.members.iter().find(|m| m.session == s)).map(|m| look(&m.role, &m.name));

    let menu_items = v_flex()
        .gap(px(1.))
        .child(item("menu-pm", glyph(pm_look.map_or("◆", |l| l.0), pm_look.map_or(p.muted, |l| p.member(l.1))), t!("sidebar.pm_talk").into(), None, pm_on).on_click(cx.listener(|this, _, window, cx| this.open_side(Side::Pm, window, cx))))
        .child(item("menu-inbox", glyph("⚑", if needs > 0 { p.accent } else { p.muted }), t!("board.menu_inbox").into(), Some((needs, true)), on_board && menu == Menu::Inbox).on_click(cx.listener(|this, _, _, cx| this.open_menu(Menu::Inbox, cx))))
        .child(item("menu-working", icons::status(p, equipe_core::protocol::TicketStatus::InProgress, 13.).into_any_element(), t!("board.menu_working").into(), Some((count_working(model), false)), on_board && menu == Menu::Working).on_click(cx.listener(|this, _, _, cx| this.open_menu(Menu::Working, cx))))
        .child(item("menu-all", icons::status(p, equipe_core::protocol::TicketStatus::Todo, 13.).into_any_element(), t!("board.menu_all").into(), Some((count_all(model), false)), on_board && menu == Menu::All).on_click(cx.listener(|this, _, _, cx| this.open_menu(Menu::All, cx))));

    let views = shell.saved_views();
    let builtin = crate::board::builtin_views().len();
    let view_rows: Vec<_> = views.iter().enumerate().filter(|(_, v)| v.pinned).map(|(at, v)| {
        let on = on_board && menu == Menu::View(at);
        let removable = at >= builtin;
        h_flex()
            .id(("view", at))
            .group("view-row")
            .h(px(ITEM))
            .flex_none()
            .mx_2()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(UI))
            .cursor_pointer()
            .when(on, |r| r.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |r| r.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .on_click(cx.listener(move |this, _, _, cx| this.open_menu(Menu::View(at), cx)))
            .child(div().w(px(16.)).flex_none().text_center().text_size(px(12.)).text_color(p.muted).child("★"))
            .child(div().flex_1().min_w_0().truncate().child(v.name.clone()))
            .when(removable, |r| {
                r.child(div().id(("forget-view", at)).flex_none().text_color(p.muted).invisible().group_hover("view-row", |s| s.visible()).child("×").on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.board_forget_view(at, cx);
                })))
            })
    }).collect();

    let names = model.project_names();
    let project_rows: Vec<_> = names.iter().enumerate().map(|(at, name)| {
        let on = model.focus.as_deref() == Some(name.as_str());
        let open = model.tickets.iter().filter(|t| !t.status.is_closed() && model.ticket_project_name(t).as_deref() == Some(name.as_str())).count();
        let target = name.clone();
        h_flex()
            .id(("project", at))
            .h(px(ITEM))
            .flex_none()
            .mx_2()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(UI))
            .cursor_pointer()
            .when(on, |r| r.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |r| r.text_color(p.text2).hover(move |s| s.bg(p.wash)))
            .on_click(cx.listener(move |this, _, window, cx| {
                let next = if this.model.focus.as_deref() == Some(target.as_str()) { None } else { Some(target.clone()) };
                this.focus_project(next, window, cx);
            }))
            .child(div().w(px(16.)).flex_none().flex().justify_center().child(dot(project_color(p, at), 7.)))
            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            .when(open > 0, |r| r.child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(open.to_string())))
    }).collect();

    let team = model.team();

    v_flex()
        .w(px(WIDTH))
        .h_full()
        .flex_none()
        .bg(p.side)
        .border_r_1()
        .border_color(p.hair)
        .font_family(SANS)
        .text_color(p.text)
        .child(squad_switch(p, shell, cx))
        .child(
            v_flex()
                .id("side")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .pb_3()
                .when(dim, |l| l.opacity(0.8))
                .child(menu_items)
                .when(!views.is_empty(), |l| l.child(kicker(p, t!("sidebar.views"))).children(view_rows))
                .when(!names.is_empty(), |l| l.child(kicker(p, t!("sidebar.projects"))).children(project_rows))
                .when(!team.is_empty(), |l| l.child(kicker(p, t!("sidebar.team"))).children(team.into_iter().filter_map(|s| model.members.iter().find(|m| m.session == s)).map(|m| member_row(p, shell, m, cx)))),
        )
        .children(relaunch(p, shell, cx))
        .child(footer(p, shell, cx))
}

/// The squad followed, and — open — the others to switch to.
fn squad_switch(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let open = shell.board.squads_open;
    let current = if shell.model.squad.is_empty() { t!("sidebar.squads").to_owned() } else { shell.model.squad.clone() };
    let others: Vec<String> = shell.squads.iter().filter(|s| **s != shell.model.squad).cloned().collect();
    v_flex()
        .flex_none()
        .pt_2()
        .pb_1()
        .child(
            h_flex()
                .id("squad-switch")
                .h(px(34.))
                .mx_2()
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board.squads_open = !this.board.squads_open;
                    cx.notify();
                }))
                .child(div().size(px(18.)).flex_none().flex().items_center().justify_center().rounded(px(5.)).bg(p.primary).text_color(p.on_primary).text_size(px(11.)).font_weight(FontWeight::BOLD).child(current.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()))
                .child(div().flex_1().min_w_0().truncate().text_size(px(UI)).font_weight(FontWeight::SEMIBOLD).child(current))
                .when(!others.is_empty(), |r| r.child(div().text_size(px(11.)).text_color(p.muted).child(if open { "▴" } else { "▾" }))),
        )
        .when(open, |c| {
            c.children(others.into_iter().enumerate().map(|(ix, name)| {
                let target = name.clone();
                h_flex()
                    .id(("squad", ix))
                    .h(px(ITEM))
                    .mx_2()
                    .pl(px(34.))
                    .pr_2()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(UI))
                    .text_color(p.text2)
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.board.squads_open = false;
                        this.select_squad(&target, window, cx);
                    }))
                    .child(name)
            }))
        })
}

/// An update downloaded in the background, ready: one click puts it in and
/// starts the app again (as Claude's "Relaunch to update").
fn relaunch(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let version = shell.update_ready.as_deref()?;
    Some(
        h_flex()
            .id("relaunch")
            .h(px(40.))
            .flex_none()
            .mx_2()
            .mb_2()
            .px_3()
            .rounded(px(8.))
            .items_center()
            .justify_between()
            .bg(p.wash)
            .text_size(px(UI))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(move |s| s.bg(p.hair))
            .on_click(cx.listener(|this, _, _, cx| this.relaunch_to_update(cx)))
            .child(crate::relaunch::label(version))
            .child(div().text_color(p.text2).child("→")),
    )
}

fn kicker(p: Palette, text: &'static str) -> impl IntoElement {
    div().h(px(30.)).flex_none().px(px(18.)).pt(px(14.)).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(text)
}

/// Someone on the team: mark, name, what it is doing — and, under it, the
/// ticket it is on. A click opens its conversation.
fn member_row(p: Palette, shell: &Shell, member: &Member, cx: &mut Context<Shell>) -> AnyElement {
    let status = shell.model.status(member);
    let (mark, hue) = look(&member.role, &member.name);
    let color = p.member(hue);
    let needs = matches!(status, Status::NeedsYou(_));
    let working = matches!(status, Status::Working { .. });
    let closed = status == Status::Closed;
    let (line, line_color) = status_line(member, &status, shell.model.at_once(member), shell.model.approvals.first_for(member.session).is_some(), p);
    let side = if Model::is_pm(member) { Side::Pm } else { Side::Member(member.session) };
    // The PM's own line is PM 대화, above.
    let on = shell.screen == Screen::Chat && side != Side::Pm && shell.model.side_open() == Some(side);
    let session = member.session;
    let subs: Vec<Sub> = shell.model.convos.get(&member.session).map(|c| c.subagents.clone()).unwrap_or_default();
    let ticket = member.ticket.and_then(|id| shell.model.tickets.iter().find(|t| t.id == id)).map(|t| (t.id, t.title.clone(), t.status));
    let row = h_flex()
        .id(("member", member.session))
        .min_h(px(36.))
        .flex_none()
        .mx_2()
        .px_2()
        .py(px(4.))
        .rounded(px(6.))
        .cursor_pointer()
        .when(on, |r| r.bg(p.wash))
        .when(!on, |r| r.hover(move |s| s.bg(p.wash)))
        .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(16.))
                .pt(px(1.))
                .flex_none()
                .text_center()
                .text_size(px(13.))
                .text_color(if needs { p.accent } else if closed { p.muted } else { color })
                .when(matches!(status, Status::Ready { .. }), |m| m.opacity(0.62))
                .child(if needs { "⚑" } else if closed { "·" } else { mark }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_none().text_size(px(UI)).line_height(px(18.)).text_color(if closed { p.muted } else { p.text }).when(working || on, |n| n.font_weight(FontWeight::SEMIBOLD)).child(member.name.clone()))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).line_height(px(18.)).text_color(line_color).child(line)),
                )
                .children(ticket.map(|(id, title, status)| {
                    h_flex()
                        .gap(px(5.))
                        .items_center()
                        .text_size(px(12.))
                        .line_height(px(17.))
                        .text_color(p.muted)
                        .child(icons::status(p, status, 11.))
                        .child(div().flex_none().child(format!("T-{id}")))
                        .child(div().flex_1().min_w_0().truncate().child(title))
                })),
        );
    v_flex().flex_none().child(row).children(sub_rows(p, &subs)).into_any_element()
}

/// The subagents a member handed work to, under it: at most three, then a count.
fn sub_rows(p: Palette, subs: &[Sub]) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = subs
        .iter()
        .take(3)
        .map(|sub| {
            let line = sub.step.clone().unwrap_or_else(|| sub.what.clone());
            h_flex()
                .h(px(22.))
                .pl(px(42.))
                .pr_3()
                .gap_2()
                .items_center()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(div().flex_none().child("└"))
                .child(div().min_w_0().truncate().child(line))
                .into_any_element()
        })
        .collect();
    if subs.len() > 3 {
        rows.push(div().h(px(22.)).pl(px(42.)).text_size(px(12.)).text_color(p.muted).child(t!("sidebar.more_subs", n = subs.len() - 3)).into_any_element());
    }
    rows
}

/// What a member row says after the name, and its colour.
pub fn status_line(member: &Member, status: &Status, at_once: usize, asks: bool, p: Palette) -> (String, Rgba) {
    let join = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let memos = (member.memos > 0).then(|| t!("sidebar.memos", n = member.memos));
    let context = member.context.filter(|c| *c >= 50).map(|c| format!("{c}%"));
    match status {
        Status::NeedsYou(_) if asks => (t!("state.section_permission").into(), p.accent),
        Status::NeedsYou(_) => (t!("sidebar.needs_you").into(), p.accent),
        // Several of its conversations at work at once.
        Status::Working { .. } if at_once > 1 => (join(vec![memos, Some(t!("sidebar.at_once", at_once = at_once)), context]), p.muted),
        Status::Working { verb, .. } => (join(vec![memos, Some(verb_text(*verb).into()), context]), p.amber),
        Status::Ready { .. } => (join(vec![memos, Some(t!("sidebar.idle").into()), context]), p.muted),
        Status::Closed => (t!("sidebar.closed").into(), p.muted),
    }
}

fn verb_text(verb: Verb) -> &'static str {
    match verb {
        Verb::Thinking => t!("sidebar.thinking"),
        Verb::Reading => t!("sidebar.reading"),
        Verb::Editing => t!("sidebar.editing"),
        Verb::Running => t!("sidebar.running"),
        Verb::Searching => t!("sidebar.searching"),
        Verb::Writing => t!("sidebar.writing"),
        Verb::Working => t!("sidebar.working"),
    }
}

fn footer(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let text = match &shell.model.conn {
        Conn::Connecting => t!("sidebar.connecting").to_owned(),
        Conn::Connected { last_event: Some(at) } => t!("sidebar.connected_at", at = at.hm()),
        Conn::Connected { last_event: None } => t!("sidebar.connected").to_owned(),
        Conn::Down { .. } => t!("sidebar.disconnected").to_owned(),
    };
    // Which machine's daemon this is; a click chooses another.
    let machine = shell.endpoint.label();
    h_flex()
        .id("machine")
        .h(px(48.))
        .flex_none()
        .px(px(18.))
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx)))
        .child(dot(if matches!(shell.model.conn, Conn::Connected { .. }) { p.accent } else { p.muted }, 6.))
        .child(v_flex().flex_1().min_w_0().child(div().child(text)).child(div().truncate().text_color(p.text2).child(machine)))
        .child(div().child("▾"))
}
