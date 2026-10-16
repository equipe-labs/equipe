//! The sidebar, after Claude's: a strip by the traffic lights (hide ⌘\,
//! search ⌘K, back and forward), then a few quiet rows — 새 메시지 ⌘N,
//! PM 대화, 나에게 (a badge), 작업 중 (a count), 모든 일 — and the live work as
//! Claude lists sessions: under a header per project, one line each for what
//! a member is at now (a spinner) and what waits on the person (an orange
//! dot), the title fading out at the edge, five at most and "+N개 더". A
//! project with nothing live is its header alone; its name switches the
//! project slice, its chevron folds it. Finished work is one line at the
//! list's foot. The team is pinned under the list, always in reach, then the
//! update when one is ready, the squad and machine, and settings. The edge
//! drags to resize. One left edge for marks, one for text.

use equipe_core::t;

use std::collections::HashMap;
use std::time::Instant;

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};
use crate::tips::TipExt as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::dot;
use super::header::project_color;
use crate::board::{HeadClick, Live, Menu, SideGroup, recent_done_view, side_groups};
use crate::state::{Conn, Member, Model, Need, Side, Status, short_span};
use crate::theme::{Palette, SANS, UI};
use crate::{Screen, Shell, motion};

pub const WIDTH: f32 = 260.;
pub const MIN_WIDTH: f32 = 200.;
pub const MAX_WIDTH: f32 = 420.;
/// The title bar: the sidebar's strip and every screen's header are this one
/// row, one height, one hairline under it.
pub const STRIP: f32 = 44.;
/// A traffic light, as AppKit draws it: 14 pt, 9 pt apart.
const LIGHT: f32 = 14.;
/// Where the traffic lights sit (the close button's top left): 14 pt in, and
/// centred on the title bar — the strip's buttons share that centre line.
pub const LIGHTS_AT: (f32, f32) = (14., (STRIP - LIGHT) / 2.);
/// The three lights, side to side, in points.
const LIGHTS_SPAN: f32 = 3. * LIGHT + 2. * 9.;
/// Where the traffic lights end, and a little air: the strip's buttons start here.
const LIGHTS: f32 = 78.;
/// The strip's square buttons and the space between them.
const BUTTON: f32 = 26.;
const BUTTON_GAP: f32 = 2.;

/// Where the traffic lights go with the window zoomed (`zoom.rs`): AppKit's
/// lights keep their size and their place from the left, and stay centred on
/// the title bar, which grows and shrinks with everything else.
pub fn lights_at(zoom: f32) -> (f32, f32) {
    (LIGHTS_AT.0, (STRIP * zoom - LIGHT) / 2.)
}

/// Where the strip's buttons start, in the window's (zoomed) points: after
/// the lights — which don't zoom — and the same air as at actual size.
pub fn lights_end(zoom: f32) -> f32 {
    (LIGHTS_AT.0 + LIGHTS_SPAN) / zoom + (LIGHTS - LIGHTS_AT.0 - LIGHTS_SPAN)
}

/// Where the fixed buttons (hide, back, forward) end, whatever the sidebar does.
pub fn buttons_end(zoom: f32) -> f32 {
    lights_end(zoom) + 3. * BUTTON + 2. * BUTTON_GAP
}
const ROW: f32 = 30.;
const TEXT: f32 = 13.;
/// Every row's mark sits in a box this wide, so every title starts at one x.
const MARK_BOX: f32 = 16.;
/// A member's row in the team block, compact.
pub const TEAM_ROW: f32 = 28.;
/// The team block shows this many rows; past that it scrolls inside.
pub const TEAM_SHOWN: usize = 6;

/// A small menu out of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    /// The squads and machines.
    Account,
}

/// The small mark at a row's left edge: how it stands, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    /// At work now.
    Spinner,
    /// Waits on the person.
    Waits,
    /// A member at rest: a ring in its colour.
    Ring(Rgba),
    /// A member that left: a faint ring.
    Hollow,
}

/// A plain icon, `size` px, in `color`.
pub fn icon(name: IconName, size: f32, color: Rgba) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// A small square button with an icon: the strip's, the foot's.
pub fn icon_button(p: Palette, id: impl Into<ElementId>, name: IconName, enabled: bool, tip: Option<String>) -> Stateful<Div> {
    div()
        .id(id.into())
        .size(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .when(enabled, |b| b.cursor_pointer().hover(move |s| s.bg(p.side_hover)))
        .when(!enabled, |b| b.opacity(0.35))
        .child(icon(name, 16., p.text2))
        .when_some(tip, |b, tip| b.tip(p, tip, None))
}

/// The title bar's fixed buttons — hide or show the sidebar, back, forward —
/// in one layer over the window's top left, so they never move: not when the
/// sidebar slides away, not from screen to screen. Only what is beside them moves.
pub fn title_buttons(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let ui = &shell.board;
    let tip = if ui.sidebar_hidden { t!("sidebar.show_tip") } else { t!("sidebar.hide_tip") };
    h_flex()
        .absolute()
        .top_0()
        .left(px(lights_end(crate::zoom::factor(shell.remembered.zoom.0))))
        .h(px(STRIP))
        .gap(px(BUTTON_GAP))
        .items_center()
        .child(icon_button(p, "toggle-sidebar", IconName::PanelLeft, true, Some(tip.into())).on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))))
        .child(icon_button(p, "back", IconName::ArrowLeft, ui.history_at > 0, Some(t!("sidebar.back").into())).on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))))
        .child(icon_button(p, "forward", IconName::ArrowRight, ui.history_at + 1 < ui.history.len(), Some(t!("sidebar.forward").into())).on_click(cx.listener(|this, _, window, cx| this.go_forward(window, cx))))
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let ui = &shell.board;
    let dim = matches!(model.conn, Conn::Down { had_snapshot: false, .. });
    let on_board = shell.screen == Screen::Board;
    let menu = ui.view.menu;
    let pm_on = shell.screen == Screen::Chat && model.side_open() == Some(Side::Pm);
    let needs = model.needs_you_count();
    let working = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress && model.in_focus(t)).count();

    // The strip: the title bar's fixed buttons float over its left (see
    // `title_buttons`); search, the sidebar's own, sits after them and leaves
    // with the sidebar.
    let strip = h_flex()
        .h(px(STRIP))
        .flex_none()
        .pl(px(buttons_end(crate::zoom::factor(shell.remembered.zoom.0)) + BUTTON_GAP))
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(icon_button(p, "search", IconName::Search, true, Some(t!("sidebar.search_tip").into())).on_click(cx.listener(|this, _, window, cx| this.search_everything(window, cx))));

    // A badge for what waits on the person; a quiet count for the rest.
    let nav = |id: &'static str, name: IconName, label: String, badge: usize, count: usize, on: bool| {
        row_shell(p, id, on)
            .h(px(32.))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(name, 16., if on { p.text } else { p.text2 })))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else { p.text2 }).child(label))
            .when(badge > 0, |r| {
                r.child(div().flex_none().min_w(px(18.)).h(px(18.)).px(px(5.)).flex().items_center().justify_center().rounded(px(9.)).bg(p.accent).text_color(p.on_accent).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(badge.to_string()))
            })
            .when(count > 0, |r| r.child(div().flex_none().pr(px(4.)).text_size(px(12.)).text_color(p.muted).child(count.to_string())))
    };
    let nav_rows = v_flex()
        .gap(px(1.))
        .child(nav("nav-new", IconName::Plus, t!("sidebar.new_message").into(), 0, 0, false).on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx))))
        .child(nav("nav-pm", IconName::Bot, t!("sidebar.pm_talk").into(), 0, 0, pm_on).on_click(cx.listener(|this, _, window, cx| this.open_side(Side::Pm, window, cx))))
        .child(nav("nav-inbox", IconName::Inbox, t!("board.menu_inbox").into(), needs, 0, on_board && menu == Menu::Inbox).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::Inbox, cx))))
        .child(nav("nav-working", IconName::LoaderCircle, t!("board.menu_working").into(), 0, working, on_board && menu == Menu::Working).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::Working, cx))))
        .child(nav("nav-all", IconName::LayoutDashboard, t!("board.menu_all").into(), 0, 0, on_board && matches!(menu, Menu::All | Menu::View(_))).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::All, cx))));

    // The live work, a group per project; rows that just came arrive, rows
    // that just went leave, eased (`SideMotion`).
    let selected = selected_ticket(shell);
    let groups = side_groups(model);
    let now = Instant::now();
    let animate = !cx.reduce_motion();
    let shown: Vec<Shown> = groups
        .iter()
        .filter(|g| !shell.remembered.folded.contains(&g.key()))
        .flat_map(|g| g.rows.iter().map(move |(id, live)| (*id, g.key(), *live)))
        .filter_map(|(id, key, live)| model.tickets.iter().find(|t| t.id == id).map(|t| Shown { id, key, live, title: t.title.clone() }))
        .collect();
    let (arriving, leaving) = {
        let mut m = ui.side_motion.borrow_mut();
        m.note(&shown, now, animate);
        (m.arriving(now), m.leaving(now))
    };
    let mut list: Vec<AnyElement> = Vec::new();
    let mut after_rows = false;
    for (ix, g) in groups.iter().enumerate() {
        let folded = shell.remembered.folded.contains(&g.key());
        let gap = if ix == 0 { 14. } else if after_rows { 10. } else { 2. };
        list.push(project_head(shell, g, folded, gap, cx));
        after_rows = !folded && (!g.is_quiet() || leaving.iter().any(|l| l.key == g.key()));
        if folded {
            continue;
        }
        for (id, live) in &g.rows {
            if let Some(t) = model.tickets.iter().find(|t| t.id == *id) {
                let arrives = arriving.contains(&(*id, *live));
                list.push(ticket_row(shell, t, *live, selected == Some(*id), arrives, cx));
            }
        }
        for gone in leaving.iter().filter(|l| l.key == g.key()) {
            list.push(leaving_row(p, gone));
        }
        if g.more > 0 {
            list.push(more_row(p, g, cx));
        }
    }
    list.push(done_row(shell, after_rows, cx));

    let body = v_flex()
        .id("side")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .pt_1()
        .pb_3()
        .when(dim, |l| l.opacity(0.8))
        .child(nav_rows)
        .children(list);

    v_flex()
        .w(px(ui.sidebar_width))
        .h_full()
        .flex_none()
        .relative()
        .bg(p.side)
        .font_family(SANS)
        .text_color(p.text)
        .child(strip)
        .child(body)
        .children(team_block(shell, cx))
        .children(relaunch(p, shell, cx))
        .child(footer(p, shell, cx))
        .children(popup(p, shell, cx))
        // The edge: drag to resize.
        .child(
            div()
                .id("sidebar-edge")
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-3.))
                .w(px(6.))
                .cursor_col_resize()
                .child(div().absolute().top_0().bottom_0().left(px(3.)).w(px(1.)).bg(if ui.sidebar_drag.is_some() { p.accent } else { p.hair }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        this.board.sidebar_drag = Some((f32::from(e.position.x), this.board.sidebar_width));
                        cx.notify();
                    }),
                ),
        )
}

/// The ticket the screen shows, for the row's highlight.
fn selected_ticket(shell: &Shell) -> Option<TicketId> {
    match shell.screen {
        Screen::Chat => shell.model.opened,
        Screen::Board => match shell.picked_need() {
            Some(Need::Ticket(id)) => Some(id),
            _ => None,
        },
        Screen::Settings => None,
    }
}

/// A project's header: the chevron (fold), its colour and name (switch the
/// project slice to it) and, folded, how much is live in it — "2 · 1⚑", at
/// work and waiting on the person. A project with nothing live is one muted
/// line with no chevron.
fn project_head(shell: &Shell, g: &SideGroup, folded: bool, gap: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = g.key();
    let group = SharedString::from(format!("head-{key}"));
    let quiet = g.is_quiet();
    let focused = g.name.is_some() && shell.model.focus == g.name;
    let label = g.name.clone().unwrap_or_else(|| t!("board.squad_folder").into());
    let name_color = if focused { p.text } else if quiet { p.muted } else { p.text2 };
    let (fold_project, name_project, new_project) = (g.name.clone(), g.name.clone(), g.name.clone());
    let chevron = div()
        .id(SharedString::from(format!("fold-{key}")))
        .w(px(MARK_BOX))
        .h(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(!quiet, |c| {
            c.cursor_pointer()
                .hover(move |s| s.bg(p.side_hover))
                .tip(p, if folded { t!("sidebar.unfold") } else { t!("sidebar.fold") }, None)
                .child(icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 11., p.muted))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.side_head_click(HeadClick::Chevron, fold_project.clone(), window, cx);
                }))
        });
    let name_tip = match (&g.name, focused) {
        (None, _) => if folded { t!("sidebar.unfold") } else { t!("sidebar.fold") },
        (Some(_), true) => t!("sidebar.back_to_all"),
        (Some(_), false) => t!("sidebar.switch_project"),
    };
    let ident = match g.at {
        Some(at) => dot(project_color(p, at), 6.).into_any_element(),
        None => icon(IconName::Folder, 12., p.muted).into_any_element(),
    };
    let name = h_flex()
        .id(SharedString::from(format!("project-{key}")))
        .min_w_0()
        .h(px(24.))
        .ml(px(-4.))
        .px(px(4.))
        .gap(px(7.))
        .items_center()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .tip_at(p, name_tip, None, gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, _, window, cx| this.side_head_click(HeadClick::Name, name_project.clone(), window, cx)))
        .child(ident)
        .child(div().min_w_0().truncate().text_color(name_color).when(focused, |d| d.font_weight(FontWeight::SEMIBOLD)).child(label));
    let counts = (folded && !quiet).then(|| {
        h_flex()
            .flex_none()
            .gap(px(4.))
            .items_center()
            .text_size(px(11.))
            .when(g.working > 0, |c| c.child(div().text_color(p.muted).child(g.working.to_string())))
            .when(g.working > 0 && g.waiting > 0, |c| c.child(div().text_color(p.muted).child("·")))
            .when(g.waiting > 0, |c| c.child(div().text_color(p.orange).font_weight(FontWeight::SEMIBOLD).child(format!("{}⚑", g.waiting))))
    });
    let new_in = new_project.clone().map(|project| {
        div()
            .id(SharedString::from(format!("new-in-{key}")))
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .size(px(20.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(move |s| s.bg(p.side_hover))
            .tip(p, t!("sidebar.new_in_project"), None)
            .child(icon(IconName::Plus, 13., p.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.focus_project(Some(project.clone()), window, cx);
                this.new_message(window, cx);
            }))
    });
    h_flex()
        .id(SharedString::from(format!("head-row-{key}")))
        .group(group)
        .h(px(26.))
        .mt(px(gap))
        .mx_2()
        .pl_2()
        .pr_1()
        .gap(px(8.))
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(chevron)
        .child(name)
        .children(counts)
        .child(div().flex_1())
        .children(new_in)
        .into_any_element()
}

/// A row's frame: full width less the insets, rounded, a soft fill when open.
fn row_shell(p: Palette, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    h_flex()
        .id(id.into())
        .relative()
        .h(px(ROW))
        .flex_none()
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(8.))
        .text_size(px(UI))
        .cursor_pointer()
        .overflow_hidden()
        .when(on, |r| r.bg(p.side_sel))
        .when(!on, |r| r.hover(move |s| s.bg(p.side_hover)))
}

/// A title that fades out at the right edge instead of an ellipsis.
fn fading_title(p: Palette, title: String, on: bool, dim: bool) -> Div {
    let fade_to = if on { p.side_sel } else { p.side };
    let clear: Hsla = Rgba { a: 0., ..fade_to }.into();
    div()
        .flex_1()
        .min_w_0()
        .relative()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(if dim { p.muted } else if on { p.text } else { p.text2 })
        .child(title)
        .child(div().absolute().top_0().bottom_0().right_0().w(px(28.)).bg(linear_gradient(90., linear_color_stop(clear, 0.), linear_color_stop(Hsla::from(fade_to), 1.))))
}

/// The small mark before a row's text, in a box of one size.
fn mark_element(p: Palette, mark: Mark, turn: f32) -> AnyElement {
    let holder = || div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none().flex().items_center().justify_center();
    match mark {
        Mark::Spinner => holder().child(icon(IconName::LoaderCircle, 13., p.text2).rotate(radians(turn))).into_any_element(),
        Mark::Waits => holder().child(dot(p.orange, 7.)).into_any_element(),
        // A ring: who it is, not how it stands (filled dots are statuses).
        Mark::Ring(color) => holder().child(div().size(px(7.)).rounded_full().border_1().border_color(color)).into_any_element(),
        Mark::Hollow => holder().child(div().size(px(7.)).rounded_full().border_1().border_color(p.muted).opacity(0.6)).into_any_element(),
    }
}

/// A row easing in (it just became live) or as it stands.
fn arrive(row: Stateful<Div>, id: ElementId, arrives: bool) -> AnyElement {
    if arrives {
        row.with_animation(id, Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.h(px(ROW * t)).opacity(t)).into_any_element()
    } else {
        row.into_any_element()
    }
}

/// A live ticket, as a session: its mark (a spinner, or the orange dot for
/// the person's turn), its title fading out, how long it has been so — and,
/// on the pointer, a quick yes for what waits. The number, the whole title,
/// where it stands and who is on it are on its card (`ticket_card`).
fn ticket_row(shell: &Shell, t: &Ticket, live: Live, on: bool, arrives: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let id = t.id;
    let group = SharedString::from(format!("tk-{id}"));
    let time = model.in_state_for(t).map(short_span);
    let yes = t.status == TicketStatus::Waiting && shell.can_act();
    let mark = match live {
        Live::Working => Mark::Spinner,
        Live::YourTurn => Mark::Waits,
    };
    // Its card on the pointer, beside the sidebar: the number, where it
    // stands, who is on it, what it asks.
    let row = super::ticket_card::hover_link(row_shell(p, ("ticket", id as usize), on), id, super::ticket_card::Side::Right)
        .group(group.clone())
        .on_click(cx.listener(move |this, _, window, cx| this.open_ticket_row(id, window, cx)))
        .child(mark_element(p, mark, shell.board.spin))
        .child(fading_title(p, t.title.clone(), on, false))
        .children(time.map(|time| div().flex_none().text_size(px(11.)).text_color(p.muted).when(yes, |d| d.group_hover(group.clone(), |s| s.invisible())).child(time)))
        .when(yes, |r| {
            r.child(
                div()
                    .id(("quick-yes", id as usize))
                    .absolute()
                    .right(px(6.))
                    .h(px(20.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .text_size(px(11.))
                    .text_color(p.text2)
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .hover(move |s| s.bg(p.side_hover))
                    .child(t!("sidebar.quick_yes"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.screen = Screen::Board;
                        this.board.view.selected = Some(Need::Ticket(id));
                        this.board_yes(cx);
                    })),
            )
        });
    let live_bit = u64::from(live == Live::YourTurn);
    arrive(row, ElementId::NamedInteger("side-in".into(), u64::from(id) << 1 | live_bit), arrives)
}

/// A row that just stopped being live, closing up as it fades.
fn leaving_row(p: Palette, gone: &Leaving) -> AnyElement {
    let mark = match gone.live {
        Live::Working => Mark::Spinner,
        Live::YourTurn => Mark::Waits,
    };
    row_shell(p, ("leaving", gone.id as usize), false)
        .cursor_default()
        .child(mark_element(p, mark, 0.))
        .child(fading_title(p, gone.title.clone(), false, true))
        .with_animation(ElementId::NamedInteger("side-out".into(), u64::from(gone.id)), Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.h(px(ROW * (1. - t))).opacity(1. - t))
        .into_any_element()
}

/// "+N개 더 ›": the rest of a project's live work, in 모든 일 narrowed to it.
fn more_row(p: Palette, g: &SideGroup, cx: &mut Context<Shell>) -> AnyElement {
    let words = g.name.as_ref().map(|n| format!("project:{n}")).unwrap_or_default();
    row_shell(p, SharedString::from(format!("more-{}", g.key())), false)
        .h(px(26.))
        .on_click(cx.listener(move |this, _, _, cx| this.board_find(&words, cx)))
        .child(div().w(px(MARK_BOX)).flex_none())
        .child(div().text_size(px(12.)).text_color(p.muted).child(t!("sidebar.more", n = g.more)))
        .child(icon(IconName::ChevronRight, 11., p.muted))
        .into_any_element()
}

/// The list's last line: "최근 끝남 · 오늘 N ›" — what finished today, local
/// time — or, nothing yet today, "최근 끝남 ›", muted. It opens 모든 일 on the
/// finished.
fn done_row(shell: &Shell, after_rows: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let today = shell.model.done_today();
    let on = shell.screen == Screen::Board && shell.board.view.menu == Menu::View(recent_done_view());
    let label = if today > 0 { t!("sidebar.recent_done_today", n = today) } else { t!("sidebar.recent_done").to_owned() };
    row_shell(p, "recent-done", on)
        .mt(px(if after_rows { 10. } else { 14. }))
        .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::View(recent_done_view()), cx)))
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(IconName::Check, 12., if today > 0 { p.text2 } else { p.muted })))
        .child(div().flex_none().text_size(px(TEXT)).text_color(if on { p.text } else if today > 0 { p.text2 } else { p.muted }).child(label))
        .child(icon(IconName::ChevronRight, 11., p.muted))
        .into_any_element()
}

/// The team, pinned above the foot so it never scrolls away: a compact row
/// each — its mark, its name, a few words of where it stands — six in sight,
/// the rest scrolling inside. A click opens its conversation.
fn team_block(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let members: Vec<&Member> = model.team().into_iter().filter_map(|s| model.members.iter().find(|m| m.session == s)).collect();
    if members.is_empty() {
        return None;
    }
    let rows: Vec<AnyElement> = members.iter().map(|m| member_row(shell, m, cx)).collect();
    Some(
        v_flex()
            .flex_none()
            .pt(px(6.))
            .pb_1()
            .border_t_1()
            .border_color(p.hair)
            .child(
                h_flex()
                    .h(px(22.))
                    .mx_2()
                    .px_2()
                    .gap(px(8.))
                    .items_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted)
                    .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(IconName::User, 12., p.muted)))
                    .child(t!("sidebar.team"))
                    .child(div().text_color(p.muted).font_weight(FontWeight::NORMAL).child(members.len().to_string())),
            )
            .child(v_flex().id("team-list").max_h(px(team_height(members.len()))).overflow_y_scroll().children(rows))
            .into_any_element(),
    )
}

/// How tall the team's list is: a row each, six at most in sight.
pub fn team_height(members: usize) -> f32 {
    members.min(TEAM_SHOWN) as f32 * TEAM_ROW
}

/// Where a member stands, in the team block's few words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamState {
    /// At work — on a ticket, when it is on one.
    Working(Option<TicketId>),
    /// It asks the person's permission.
    Asks,
    /// Its ticket waits on the person: 내 차례.
    YourTurn,
    Resting,
    Closed,
}

impl TeamState {
    pub fn of(model: &Model, member: &Member) -> Self {
        match model.status(member) {
            Status::NeedsYou(_) if model.approvals.first_for(member.session).is_some() => TeamState::Asks,
            Status::NeedsYou(_) => TeamState::YourTurn,
            Status::Working { ticket, .. } => TeamState::Working(ticket),
            Status::Ready { .. } => TeamState::Resting,
            Status::Closed => TeamState::Closed,
        }
    }

    /// "작업 중 · T-12", "내 차례", "쉬는 중".
    pub fn words(self) -> String {
        match self {
            TeamState::Working(Some(id)) => t!("sidebar.working_on", id = id),
            TeamState::Working(None) => t!("sidebar.working").to_owned(),
            TeamState::Asks => t!("state.section_permission").to_owned(),
            TeamState::YourTurn => t!("sidebar.needs_you").to_owned(),
            TeamState::Resting => t!("sidebar.idle").to_owned(),
            TeamState::Closed => t!("sidebar.closed").to_owned(),
        }
    }

    /// Whether it waits on the person (said in orange).
    pub fn waits(self) -> bool {
        matches!(self, TeamState::Asks | TeamState::YourTurn)
    }
}

/// Someone on the team, compact: a spinner at work, the orange dot when it
/// waits on the person, else its colour; the name; where it stands, muted —
/// and, on the pointer, "대화하기". A click opens its conversation.
fn member_row(shell: &Shell, member: &Member, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (_, hue) = crate::theme::look(&member.role, &member.name);
    let side = if Model::is_pm(member) { Side::Pm } else { Side::Member(member.session) };
    // The PM's conversation lights PM 대화 above; here only the others'.
    let on = side != Side::Pm && shell.screen == Screen::Chat && shell.model.opened.is_none() && shell.model.side_open() == Some(side);
    let session = member.session;
    let state = TeamState::of(&shell.model, member);
    let mark = match state {
        TeamState::Working(_) => Mark::Spinner,
        TeamState::Asks | TeamState::YourTurn => Mark::Waits,
        TeamState::Closed => Mark::Hollow,
        TeamState::Resting => Mark::Ring(p.member(hue)),
    };
    let group = SharedString::from(format!("mem-{session}"));
    row_shell(p, ("member", session), on)
        .h(px(TEAM_ROW))
        .group(group.clone())
        .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        .child(mark_element(p, mark, shell.board.spin))
        .child(div().flex_none().max_w(px(120.)).truncate().text_size(px(TEXT)).text_color(if on { p.text } else { p.text2 }).child(member.name.clone()))
        .child(fading_title(p, state.words(), on, true).text_size(px(12.)).when(state.waits(), |d| d.text_color(p.orange)).group_hover(group.clone(), |s| s.invisible()))
        .child(
            div()
                .absolute()
                .right(px(6.))
                .h(px(20.))
                .px(px(7.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .text_size(px(11.))
                .text_color(p.text2)
                .invisible()
                .group_hover(group, |s| s.visible())
                .child(t!("sidebar.talk")),
        )
        .into_any_element()
}

// ── rows arriving and leaving ──

/// A live row as drawn: its ticket, its group's key, how it is live, its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub id: TicketId,
    pub key: String,
    pub live: Live,
    pub title: String,
}

/// A row that stopped being live, still closing up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaving {
    pub id: TicketId,
    pub key: String,
    pub live: Live,
    pub title: String,
    pub since: Instant,
}

/// What the sidebar's live rows were at the last draw, so a row that comes
/// eases in and one that goes eases out instead of the list jumping.
#[derive(Debug, Default)]
pub struct SideMotion {
    last: Option<Vec<Shown>>,
    arrived: HashMap<(TicketId, Live), Instant>,
    leaving: Vec<Leaving>,
}

impl SideMotion {
    /// Take in the rows drawn now. The first draw (the window opening) eases
    /// nothing in; without `animate` ("Reduce motion"), nothing moves.
    pub fn note(&mut self, now_shown: &[Shown], now: Instant, animate: bool) {
        let window = motion::BASE;
        self.arrived.retain(|_, at| now.saturating_duration_since(*at) < window);
        self.leaving.retain(|l| now.saturating_duration_since(l.since) < window && !now_shown.iter().any(|s| s.id == l.id));
        if let Some(last) = self.last.as_ref().filter(|_| animate) {
            for s in now_shown {
                if !last.iter().any(|o| o.id == s.id && o.live == s.live) {
                    self.arrived.entry((s.id, s.live)).or_insert(now);
                }
            }
            for o in last {
                if !now_shown.iter().any(|s| s.id == o.id) && !self.leaving.iter().any(|l| l.id == o.id) {
                    self.leaving.push(Leaving { id: o.id, key: o.key.clone(), live: o.live, title: o.title.clone(), since: now });
                }
            }
        }
        if !animate {
            self.arrived.clear();
            self.leaving.clear();
        }
        self.last = Some(now_shown.to_vec());
    }

    /// The rows easing in now.
    pub fn arriving(&self, now: Instant) -> Vec<(TicketId, Live)> {
        self.arrived.iter().filter(|(_, at)| now.saturating_duration_since(**at) < motion::BASE).map(|(k, _)| *k).collect()
    }

    /// The rows easing out now.
    pub fn leaving(&self, now: Instant) -> Vec<Leaving> {
        self.leaving.iter().filter(|l| now.saturating_duration_since(l.since) < motion::BASE).cloned().collect()
    }
}

/// Where an update stands, in one pill above the foot: a new version being
/// downloaded (quiet), downloaded and ready — one full-width button puts it
/// in and starts the app again (as Claude's "Relaunch to update") — or
/// failed, to try again (or to move the app where it can update itself).
fn relaunch(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    use crate::relaunch::{Update, Why};
    let words = crate::relaunch::pill(&shell.update)?;
    let pill = h_flex().id("relaunch").h(px(34.)).flex_none().mx_3().mb_1().px_3().rounded(px(8.)).items_center().justify_between().text_size(px(TEXT));
    Some(match &shell.update {
        Update::Ready(_) => pill
            .bg(p.primary)
            .text_color(p.on_primary)
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .on_click(cx.listener(|this, _, _, cx| this.relaunch_to_update(cx)))
            .child(words)
            .child("→"),
        // Downloading: said quietly, nothing to press.
        Update::Found(_) | Update::Downloading(_) => pill.border_1().border_color(p.hair).text_color(p.muted).child(div().min_w_0().truncate().child(words)),
        // The app can't replace itself where it is: show it in Finder.
        Update::Failed { why: Why::Move | Why::Permission, .. } => pill
            .border_1()
            .border_color(p.hair)
            .text_color(p.text)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .on_click(|_, _, _| crate::relaunch::reveal())
            .child(div().min_w_0().truncate().child(words))
            .child("→"),
        _ => pill
            .border_1()
            .border_color(p.hair)
            .text_color(p.text)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .on_click(|_, _, cx| crate::relaunch::check_now(cx))
            .child(div().min_w_0().truncate().child(words))
            .child("↻"),
    })
}

/// The foot, one row: the squad and machine with the connection's dot (a
/// click switches), and settings.
fn footer(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let squad = if shell.model.squad.is_empty() { t!("sidebar.squads").to_owned() } else { shell.model.squad.clone() };
    let tip = match &shell.model.conn {
        Conn::Connecting => t!("sidebar.connecting").to_owned(),
        Conn::Connected { last_event: Some(at) } => t!("sidebar.connected_at", at = at.hm()),
        Conn::Connected { last_event: None } => t!("sidebar.connected").to_owned(),
        Conn::Down { .. } => t!("sidebar.disconnected").to_owned(),
    };
    h_flex()
        .h(px(44.))
        .flex_none()
        .mx_2()
        .mb_1()
        .gap_1()
        .items_center()
        .child(
            h_flex()
                .id("account")
                .flex_1()
                .min_w_0()
                .h(px(34.))
                .px_2()
                .gap(px(8.))
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.side_hover))
                .tip_at(p, tip, None, gpui_kit::base::Placement::Right)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board.popup = if this.board.popup == Some(Popup::Account) { None } else { Some(Popup::Account) };
                    cx.notify();
                }))
                .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(dot(if connected { p.accent } else { p.muted }, 7.)))
                .child(div().flex_none().text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(squad))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(shell.endpoint.label()))
                .child(icon(IconName::ChevronsUpDown, 12., p.muted)),
        )
        .child(icon_button(p, "settings", IconName::Settings, true, None).tip(p, t!("sidebar.settings"), Some("⌘,")).on_click(cx.listener(|this, _, window, cx| this.open_settings(None, window, cx))))
}

/// The squads and machines, or settings, over the foot.
fn popup(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let which = shell.board.popup?;
    let item = |id: SharedString, label: String, on: bool| {
        h_flex()
            .id(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(TEXT))
            .cursor_pointer()
            .hover(move |s| s.bg(p.side_hover))
            .child(div().w(px(14.)).flex_none().children(on.then(|| icon(IconName::Check, 13., p.accent))))
            .child(div().flex_1().min_w_0().truncate().child(label))
    };
    let label = |text: &'static str| div().px_2().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).child(text).into_any_element();
    let mut rows: Vec<AnyElement> = Vec::new();
    match which {
        Popup::Account => {
            rows.push(label(t!("sidebar.squads")));
            for (ix, name) in shell.squads.iter().enumerate() {
                let target = name.clone();
                rows.push(
                    item(SharedString::from(format!("squad-{ix}")), name.clone(), *name == shell.model.squad)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.board.popup = None;
                            this.select_squad(&target, window, cx);
                        }))
                        .into_any_element(),
                );
            }
            rows.push(div().h(px(1.)).my_1().bg(p.hair).into_any_element());
            rows.push(
                item("machine".into(), t!("sidebar.other_machine").into(), false)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.board.popup = None;
                        this.open_chooser(window, cx);
                    }))
                    .into_any_element(),
            );
            rows.push(
                item("pair-phone".into(), t!("pair.menu_item").into(), false)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.board.popup = None;
                        this.open_pairing(window, cx);
                    }))
                    .into_any_element(),
            );
        }
    }
    Some(
        deferred(
            v_flex()
                .id("side-popup")
                .absolute()
                .bottom(px(50.))
                .left(px(8.))
                .w(px(240.))
                .max_h(px(440.))
                .overflow_y_scroll()
                .p_1()
                .rounded(px(10.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .shadow_lg()
                .occlude()
                .children(rows),
        )
        .into_any_element(),
    )
}

impl Shell {
    /// ⌘\: the sidebar away, or back.
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        // Turned back mid-slide, it goes back from where it stands.
        let now = std::time::Instant::now();
        let shown = self.sidebar_shown(cx);
        let gone = if self.board.sidebar_hidden { shown } else { 1. - shown };
        let into = motion::SLIDE.mul_f32(motion::slide_inverse(gone));
        self.board.sidebar_hidden = !self.board.sidebar_hidden;
        self.board.sidebar_shown_at = Some(now.checked_sub(into).unwrap_or(now));
        cx.notify();
    }

    /// Whether the sidebar is sliding now.
    pub fn sidebar_sliding(&self, cx: &App) -> bool {
        !cx.reduce_motion() && self.board.sidebar_shown_at.is_some_and(|at| at.elapsed() < motion::SLIDE)
    }

    /// How much of the sidebar shows, 0–1, through its slide after ⌘\.
    pub fn sidebar_shown(&self, cx: &App) -> f32 {
        let hidden = self.board.sidebar_hidden;
        let Some(at) = self.board.sidebar_shown_at.filter(|_| self.sidebar_sliding(cx)) else {
            return if hidden { 0. } else { 1. };
        };
        let t = motion::slide(at.elapsed().as_secs_f32() / motion::SLIDE.as_secs_f32());
        if hidden { 1. - t } else { t }
    }

    /// The sidebar's width on screen now.
    pub fn sidebar_on_screen(&self, cx: &App) -> f32 {
        self.board.sidebar_width * self.sidebar_shown(cx)
    }

    /// A header's left padding: its own `pad` beside the sidebar, and room for
    /// the title bar's fixed buttons without it — its content going with the
    /// sidebar's edge as that slides, and stopping by the buttons.
    pub fn header_inset(&self, pad: f32, cx: &App) -> f32 {
        (buttons_end(crate::zoom::factor(self.remembered.zoom.0)) + 8. - self.sidebar_on_screen(cx)).max(pad)
    }

    /// ⌘N: a new message to the PM.
    pub fn new_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_side(Side::Pm, window, cx);
    }

    /// ⌘K: find anything — 모든 일, its search box.
    pub fn search_everything(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_side_menu(Menu::All, cx);
        self.board.search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// A ticket's row: its work in the conversation's place when there is
    /// any, else the ticket on the board.
    pub fn open_ticket_row(&mut self, id: TicketId, window: &mut Window, cx: &mut Context<Self>) {
        self.model.seen(id);
        self.board.popup = None;
        self.open_side(Side::Ticket(id), window, cx);
    }

    /// A click on a project's header: its name switches the project slice
    /// (the conversation and the board follow), its chevron folds it — kept
    /// for the next launch.
    pub fn side_head_click(&mut self, click: HeadClick, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let mut folded = self.remembered.folded.clone();
        match crate::board::head_click(click, project.as_deref(), self.model.focus.as_deref(), &mut folded) {
            Some(slice) => self.focus_project(slice, window, cx),
            None => {
                self.remembered.folded = folded;
                self.remembered.save(&equipe_providers::credentials::equipe_home());
            }
        }
        cx.notify();
    }

    /// Dragging the sidebar's edge.
    pub fn sidebar_dragged(&mut self, x: f32, cx: &mut Context<Self>) {
        if let Some((x0, w0)) = self.board.sidebar_drag {
            self.board.sidebar_width = (w0 + x - x0).clamp(MIN_WIDTH, MAX_WIDTH);
            cx.notify();
        }
    }

    pub fn sidebar_released(&mut self, cx: &mut Context<Self>) {
        if self.board.sidebar_drag.take().is_some() {
            self.remembered.sidebar_width = Some(self.board.sidebar_width);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }
    }

    // ── back and forward ──

    /// Where the window is now, for back and forward.
    fn place(&self) -> Place {
        Place { screen: self.screen, menu: self.board.view.menu, member: self.model.selected, ticket: self.model.opened }
    }

    /// Called from each draw: a new place goes on the history.
    pub fn note_place(&mut self) {
        let here = self.place();
        let ui = &mut self.board;
        if ui.history.get(ui.history_at) == Some(&here) {
            return;
        }
        ui.history.truncate(ui.history_at + 1);
        ui.history.push(here);
        if ui.history.len() > 100 {
            ui.history.remove(0);
        }
        ui.history_at = ui.history.len() - 1;
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.board.history_at > 0 {
            self.board.history_at -= 1;
            self.go_to(self.board.history[self.board.history_at], window, cx);
        }
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.board.history_at + 1 < self.board.history.len() {
            self.board.history_at += 1;
            self.go_to(self.board.history[self.board.history_at], window, cx);
        }
    }

    fn go_to(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        match place.screen {
            Screen::Board => {
                self.board.view.menu = place.menu;
                self.show(Screen::Board, cx);
            }
            Screen::Chat => match (place.ticket, place.member) {
                (Some(id), _) => self.open_side(Side::Ticket(id), window, cx),
                (None, Some(session)) => self.select_member(session, window, cx),
                (None, None) => self.open_side(Side::Pm, window, cx),
            },
            Screen::Settings => self.open_settings(None, window, cx),
        }
        cx.notify();
    }
}

/// A place in the window's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub screen: Screen,
    pub menu: Menu,
    pub member: Option<equipe_core::protocol::SessionId>,
    pub ticket: Option<TicketId>,
}

/// How far the spinner has turned, from a step count.
pub fn spin(step: u32) -> f32 {
    (step % 12) as f32 * std::f32::consts::TAU / 12.
}

#[cfg(test)]
mod tests {
    use super::{Leaving, SideMotion, Shown, TEAM_ROW, TeamState, spin, team_height};
    use crate::board::Live;
    use crate::state::{Member, Model, Presence, Verb};
    use equipe_core::protocol::Ticket;
    use std::time::{Duration, Instant};

    fn ticket(id: u32, status: &str, owner: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": owner, "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()
    }

    fn member(session: u64, name: &str, presence: Presence, ticket: Option<u32>) -> Member {
        Member { session, name: name.into(), role: name.into(), presence, verb: Verb::Working, ticket, memos: 0, context: None, model: None, engine: None, estimate: None, target: None, last_seen: 0, failed: false }
    }

    #[test]
    fn a_team_row_says_where_each_stands_in_a_few_words() {
        let mut model = Model::new("acme");
        model.tickets = vec![ticket(12, "in_progress", "frontend"), ticket(13, "waiting", "backend")];
        model.members = vec![member(1, "pm", Presence::Ready, None), member(2, "frontend", Presence::Working, Some(12)), member(3, "backend", Presence::Ready, Some(13)), member(4, "qa", Presence::Closed, None), member(5, "data", Presence::Working, None)];
        let words: Vec<(TeamState, String)> = model.members.iter().map(|m| TeamState::of(&model, m)).map(|s| (s, s.words())).collect();
        assert_eq!(
            words,
            [
                (TeamState::Resting, "쉬는 중".to_owned()),
                (TeamState::Working(Some(12)), "작업 중 · T-12".to_owned()),
                (TeamState::YourTurn, "내 차례".to_owned()),
                (TeamState::Closed, "닫힘".to_owned()),
                (TeamState::Working(None), "작업 중".to_owned()),
            ]
        );
        assert!(TeamState::YourTurn.waits() && TeamState::Asks.waits() && !TeamState::Resting.waits());
        let ask: equipe_core::protocol::Approval = serde_json::from_value(serde_json::json!({"id": 1, "session": 2, "member": "frontend", "ticket": 12, "tool": "Bash", "action": "ls", "reason": null})).unwrap();
        model.approvals.push(ask);
        assert_eq!(TeamState::of(&model, &model.members[1]).words(), "허락 요청");
        // Every member is listed, the PM first, whatever the sidebar holds above.
        assert_eq!(model.team().len(), 5);
        assert_eq!(model.team()[0], 1);
    }

    #[test]
    fn the_team_block_shows_six_rows_then_scrolls_inside() {
        assert_eq!(team_height(0), 0.);
        assert_eq!(team_height(4), 4. * TEAM_ROW);
        assert_eq!(team_height(6), 6. * TEAM_ROW);
        assert_eq!(team_height(9), 6. * TEAM_ROW, "past six, the block keeps its height and scrolls");
        const { assert!(TEAM_ROW <= 28.) };
    }

    fn shown(id: u32, key: &str, live: Live) -> Shown {
        Shown { id, key: key.into(), live, title: format!("t{id}") }
    }

    #[test]
    fn rows_ease_in_when_they_become_live_and_out_when_they_stop() {
        let t0 = Instant::now();
        let mut m = SideMotion::default();
        m.note(&[shown(1, "web", Live::Working)], t0, true);
        assert!(m.arriving(t0).is_empty(), "the window opening eases nothing in");
        // T-2 starts; T-1 hands in.
        let t1 = t0 + Duration::from_millis(500);
        m.note(&[shown(2, "web", Live::Working)], t1, true);
        assert_eq!(m.arriving(t1), [(2, Live::Working)]);
        let gone: Vec<(u32, String)> = m.leaving(t1).into_iter().map(|l: Leaving| (l.id, l.key)).collect();
        assert_eq!(gone, [(1, "web".to_owned())]);
        // Its spinner turns to the orange dot: it eases in as that.
        let t2 = t1 + Duration::from_millis(20);
        m.note(&[shown(2, "web", Live::YourTurn)], t2, true);
        assert!(m.arriving(t2).contains(&(2, Live::YourTurn)));
        assert!(m.leaving(t2).iter().all(|l| l.id != 2), "it stays: only its mark changed");
        // A moment later all of it has settled.
        let t3 = t2 + crate::motion::BASE;
        m.note(&[shown(2, "web", Live::YourTurn)], t3, true);
        assert!(m.arriving(t3).is_empty() && m.leaving(t3).is_empty());
        // Back before it finished leaving: it stops leaving.
        m.note(&[], t3, true);
        m.note(&[shown(2, "web", Live::YourTurn)], t3 + Duration::from_millis(10), true);
        assert!(m.leaving(t3 + Duration::from_millis(10)).is_empty());
        // Reduce motion: nothing moves.
        m.note(&[], t3 + Duration::from_millis(20), false);
        assert!(m.leaving(t3 + Duration::from_millis(20)).is_empty());
        assert_eq!(spin(12), 0.);
    }
}
