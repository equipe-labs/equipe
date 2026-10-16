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

use equipe_core::protocol::{SessionId, Ticket, TicketId, TicketStatus};
use crate::convo::Sub;
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
/// A subagent's row under its member, shorter.
pub const SUB_ROW: f32 = 22.;
/// A member shows this many of its subagents as rows, then "+N개 더".
pub const SUBS_SHOWN: usize = 3;
/// The subagents' rows always have room for this many, however many members.
const SUB_ROOM: usize = 6;

/// A small menu out of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    /// The squads and machines.
    Account,
    /// Every subagent of a member at work, as "+N개 더" opens them.
    Subagents(SessionId),
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
        .child(nav("nav-all", IconName::LayoutDashboard, t!("board.menu_all").into(), 0, 0, on_board && matches!(menu, Menu::All | Menu::View(_))).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::All, cx))))
        .child(nav("nav-design", IconName::Palette, t!("design.title").into(), 0, 0, shell.screen == Screen::Design).on_click(cx.listener(|this, _, _, cx| this.open_design(cx))));

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
        .children(super::switching::meters(shell, cx))
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
        Screen::Settings | Screen::Design => None,
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
        .child(mark_element(p, mark, if mark == Mark::Spinner { shell.spin(cx) } else { 0. }))
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
/// the rest scrolling inside. A click opens its conversation. Under a member
/// at work, the subagents it handed work to, a short row each, easing in as
/// they start and out as they hand back (`SubMotion`); three at most, then
/// "+N개 더" — fewer when the block would outgrow its height, the members'
/// rows staying in sight.
fn team_block(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let members: Vec<&Member> = model.team().into_iter().filter_map(|s| model.members.iter().find(|m| m.session == s)).collect();
    if members.is_empty() {
        return None;
    }
    let subs: Vec<Vec<(SessionId, &Sub)>> = members.iter().map(|m| model.subagents_of(m.session)).collect();
    let cap = subs_cap(members.len(), &subs.iter().map(Vec::len).collect::<Vec<_>>());
    let now = Instant::now();
    let shown: Vec<SubShown> = members
        .iter()
        .zip(&subs)
        .flat_map(|(m, subs)| subs.iter().take(cap).map(|(_, s)| SubShown { member: m.session, id: s.id.clone(), what: s.what.clone() }))
        .collect();
    let live: Vec<&str> = subs.iter().flatten().map(|(_, s)| s.id.as_str()).collect();
    let (arriving, leaving) = {
        let mut motion = shell.board.sub_motion.borrow_mut();
        motion.note(&shown, &live, now, !cx.reduce_motion());
        (motion.arriving(now), motion.leaving(now))
    };
    let child_rows = subs.iter().map(|s| sub_rows(s.len(), cap)).sum::<usize>() + leaving.len();
    let mut rows: Vec<AnyElement> = Vec::new();
    for (m, subs) in members.iter().zip(&subs) {
        let folded = (cap == 0).then_some(subs.len()).filter(|n| *n > 0);
        rows.push(member_row(shell, m, folded, cx));
        for (session, sub) in subs.iter().take(cap) {
            rows.push(sub_row(shell, m.session, *session, sub, arriving.contains(&sub.id), cx));
        }
        for gone in leaving.iter().filter(|g| g.member == m.session) {
            rows.push(sub_leaving_row(p, gone));
        }
        if cap > 0 && subs.len() > cap {
            rows.push(sub_more_row(p, m.session, subs.len() - cap, cx));
        }
    }
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
            .child(v_flex().id("team-list").max_h(px(team_height(members.len(), child_rows))).overflow_y_scroll().children(rows))
            .into_any_element(),
    )
}

/// How tall the team's list may be: a row each, six members at most in
/// sight, and its subagents' rows below them, as far as they have room
/// ([`child_room`]); past that it scrolls inside.
pub fn team_height(members: usize, child_rows: usize) -> f32 {
    members.min(TEAM_SHOWN) as f32 * TEAM_ROW + (child_rows as f32 * SUB_ROW).min(child_room(members))
}

/// The height the subagents' rows may take: what six members' rows leave,
/// and never less than a few rows'.
fn child_room(members: usize) -> f32 {
    (TEAM_SHOWN.saturating_sub(members) as f32 * TEAM_ROW).max(SUB_ROOM as f32 * SUB_ROW)
}

/// How many rows `n` subagents take with `cap` shown: those, and "+N개 더"
/// for the rest. At `cap` 0 none: the member's row counts them.
pub fn sub_rows(n: usize, cap: usize) -> usize {
    if cap == 0 || n == 0 { 0 } else { n.min(cap) + usize::from(n > cap) }
}

/// How many of each member's subagents show as rows: three at most, fewer
/// when their rows would outgrow [`child_room`] — the members' rows stay in
/// sight, their subagents give way first, down to none (a count on the
/// member's row).
pub fn subs_cap(members: usize, counts: &[usize]) -> usize {
    let room = child_room(members);
    (0..=SUBS_SHOWN).rev().find(|&cap| counts.iter().map(|&n| sub_rows(n, cap)).sum::<usize>() as f32 * SUB_ROW <= room).unwrap_or(0)
}

/// A subagent at work, under its member: a small spinner, what it was
/// handed (fading out), how long it has been at it; its latest step on the
/// pointer. A click opens the member's conversation at the step that
/// handed it the work.
fn sub_row(shell: &Shell, member: SessionId, session: SessionId, sub: &Sub, arrives: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = sub.id.clone();
    let step = sub.step.clone().unwrap_or_else(|| t!("sidebar.subagent_starting").to_owned());
    let row = sub_shell(p, SharedString::from(format!("sub-{}", sub.id)))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .tip_at(p, sub.what.clone(), Some(&step), gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, _, window, cx| this.open_subagent(member, session, &id, window, cx)))
        .child(sub_mark(p, shell.spin(cx)))
        .child(fading_title(p, sub.what.clone(), false, false).text_size(px(12.)))
        .child(div().flex_none().text_size(px(11.)).text_color(p.muted).child(shell.counting(sub.since.elapsed(), cx)));
    if arrives {
        let key = ElementId::Name(SharedString::from(format!("sub-in-{}", sub.id)));
        row.with_animation(key, Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.h(px(SUB_ROW * t)).opacity(t)).into_any_element()
    } else {
        row.into_any_element()
    }
}

/// A subagent's row frame: shorter than its member's, its mark under the
/// member's name.
fn sub_shell(p: Palette, id: SharedString) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(SUB_ROW))
        .flex_none()
        .mx_2()
        .pl(px(8. + MARK_BOX + 8.))
        .pr_2()
        .gap(px(6.))
        .items_center()
        .rounded(px(6.))
        .overflow_hidden()
        .text_color(p.text2)
}

/// A subagent's mark: a small spinner, quieter than its member's.
fn sub_mark(p: Palette, turn: f32) -> AnyElement {
    div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(icon(IconName::LoaderCircle, 10., p.muted).rotate(radians(turn))).into_any_element()
}

/// A subagent that just handed back, closing up as it fades.
fn sub_leaving_row(p: Palette, gone: &SubLeaving) -> AnyElement {
    sub_shell(p, SharedString::from(format!("sub-gone-{}", gone.id)))
        .child(div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(icon(IconName::Check, 10., p.muted)))
        .child(fading_title(p, gone.what.clone(), false, true).text_size(px(12.)))
        .with_animation(ElementId::Name(SharedString::from(format!("sub-out-{}", gone.id))), Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.h(px(SUB_ROW * (1. - t))).opacity(1. - t))
        .into_any_element()
}

/// "+N개 더": the rest of a member's subagents, all of them in a popover.
fn sub_more_row(p: Palette, member: SessionId, more: usize, cx: &mut Context<Shell>) -> AnyElement {
    sub_shell(p, SharedString::from(format!("sub-more-{member}")))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_subagents(member, cx)))
        .child(div().w(px(12.)).flex_none())
        .child(div().text_size(px(11.5)).text_color(p.muted).child(t!("sidebar.more", n = more)))
        .child(icon(IconName::ChevronRight, 10., p.muted))
        .into_any_element()
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
fn member_row(shell: &Shell, member: &Member, folded: Option<usize>, cx: &mut Context<Shell>) -> AnyElement {
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
        .child(mark_element(p, mark, if mark == Mark::Spinner { shell.spin(cx) } else { 0. }))
        .child(div().flex_none().max_w(px(120.)).truncate().text_size(px(TEXT)).text_color(if on { p.text } else { p.text2 }).child(member.name.clone()))
        .children(member.engine.map(|e| super::switching::badge(p, e)))
        .child(fading_title(p, state.words(), on, true).text_size(px(12.)).when(state.waits(), |d| d.text_color(p.orange)).group_hover(group.clone(), |s| s.invisible()))
        // No room for its subagents' rows: how many, which opens them all.
        .children(folded.map(|n| {
            h_flex()
                .id(("sub-count", session))
                .flex_none()
                .gap(px(3.))
                .items_center()
                .text_size(px(11.))
                .text_color(p.muted)
                .group_hover(group.clone(), |s| s.invisible())
                .tip_at(p, t!("sidebar.subagents", n = n), None, gpui_kit::base::Placement::Right)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_subagents(session, cx);
                }))
                .child(sub_mark(p, shell.spin(cx)))
                .child(n.to_string())
        }))
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

/// A subagent's row as drawn: whose, which, what it was handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubShown {
    pub member: SessionId,
    pub id: String,
    pub what: String,
}

/// A subagent that handed back, its row still closing up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubLeaving {
    pub member: SessionId,
    pub id: String,
    pub what: String,
    pub since: Instant,
}

/// The team block's subagent rows at the last draw: one that starts eases
/// in, one that hands back eases out, so the block grows and shrinks rather
/// than jumps. A row no longer shown while its subagent works on (another
/// took its place) just goes.
#[derive(Debug, Default)]
pub struct SubMotion {
    last: Option<Vec<SubShown>>,
    arrived: HashMap<String, Instant>,
    leaving: Vec<SubLeaving>,
}

impl SubMotion {
    /// Take in the rows drawn now and the subagents at work (`live`, shown
    /// or not). The first draw eases nothing in; without `animate`, nothing moves.
    pub fn note(&mut self, shown: &[SubShown], live: &[&str], now: Instant, animate: bool) {
        let window = motion::BASE;
        self.arrived.retain(|_, at| now.saturating_duration_since(*at) < window);
        self.leaving.retain(|l| now.saturating_duration_since(l.since) < window && !live.contains(&l.id.as_str()));
        if let Some(last) = self.last.as_ref().filter(|_| animate) {
            for s in shown {
                if !last.iter().any(|o| o.id == s.id) {
                    self.arrived.entry(s.id.clone()).or_insert(now);
                }
            }
            for o in last {
                if !live.contains(&o.id.as_str()) && !self.leaving.iter().any(|l| l.id == o.id) {
                    self.leaving.push(SubLeaving { member: o.member, id: o.id.clone(), what: o.what.clone(), since: now });
                }
            }
        }
        if !animate {
            self.arrived.clear();
            self.leaving.clear();
        }
        self.last = Some(shown.to_vec());
    }

    /// The subagents whose rows ease in now.
    pub fn arriving(&self, now: Instant) -> Vec<String> {
        self.arrived.iter().filter(|(_, at)| now.saturating_duration_since(**at) < motion::BASE).map(|(id, _)| id.clone()).collect()
    }

    /// The rows easing out now.
    pub fn leaving(&self, now: Instant) -> Vec<SubLeaving> {
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
    if let Popup::Subagents(member) = which {
        return subagents_popover(shell, member, cx);
    }
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
        Popup::Subagents(_) => {}
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

/// All of a member's subagents at work, over the team block: each what it
/// was handed, its latest step under it, how long it has been at it. A click
/// goes to where it was handed the work; a click outside closes it.
fn subagents_popover(shell: &Shell, member: SessionId, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let name = model.members.iter().find(|m| m.session == member)?.name.clone();
    let subs = model.subagents_of(member);
    if subs.is_empty() {
        return None;
    }
    let rows = subs.iter().map(|(session, sub)| {
        let (session, id) = (*session, sub.id.clone());
        let step = sub.step.clone().unwrap_or_else(|| t!("sidebar.subagent_starting").to_owned());
        h_flex()
            .id(SharedString::from(format!("sub-pop-{}", sub.id)))
            .px_2()
            .py(px(5.))
            .gap(px(8.))
            .items_start()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(p.side_hover))
            .on_click(cx.listener(move |this, _, window, cx| this.open_subagent(member, session, &id, window, cx)))
            .child(div().pt(px(3.)).child(sub_mark(p, shell.spin(cx))))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(12.5)).text_color(p.text).truncate().child(sub.what.clone()))
                    .child(div().text_size(px(11.5)).text_color(p.muted).truncate().child(step)),
            )
            .child(div().flex_none().pt(px(1.)).text_size(px(11.)).text_color(p.muted).child(shell.counting(sub.since.elapsed(), cx)))
    }).collect::<Vec<_>>();
    let surface = v_flex()
        .id("sub-popover")
        .relative()
        .w(px((shell.board.sidebar_width - 16.).max(220.)))
        .max_h(px(360.))
        .overflow_y_scroll()
        .p_1()
        .rounded(px(10.))
        .bg(p.raised)
        .border_1()
        .border_color(p.hair)
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.board.popup = None;
            cx.notify();
        }))
        .child(
            h_flex()
                .px_2()
                .pt_1()
                .pb(px(4.))
                .gap_1()
                .text_size(px(11.))
                .text_color(p.muted)
                .child(div().font_weight(FontWeight::MEDIUM).text_color(p.text2).child(name))
                .child("·")
                .child(t!("sidebar.subagents", n = subs.len())),
        )
        .children(rows);
    Some(deferred(div().absolute().bottom(px(50.)).left(px(8.)).child(motion::pop(surface, ("sub-popover-in", member), true))).into_any_element())
}

impl Shell {
    /// "+N개 더" under a member: all its subagents, or away again.
    pub fn toggle_subagents(&mut self, member: SessionId, cx: &mut Context<Self>) {
        let which = Popup::Subagents(member);
        self.board.popup = if self.board.popup == Some(which) { None } else { Some(which) };
        cx.notify();
    }

    /// A subagent's row: its member's conversation, at the step that handed
    /// it the work (`session`: the conversation it works for — the member's
    /// own, or a lane's, which opens as the member's does).
    pub fn open_subagent(&mut self, member: SessionId, session: SessionId, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.board.popup = None;
        self.select_member(member, window, cx);
        self.chat_sync(cx);
        let sub = self.model.convos.get(&session).and_then(|c| c.subagents.iter().find(|s| s.id == id)).cloned();
        if let Some(row) = sub.and_then(|s| self.chat.open_delegation(&s, cx)) {
            self.chat.scroll_to(row, cx);
        }
        cx.notify();
    }

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
            Screen::Design => self.open_design(cx),
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

#[cfg(test)]
mod tests {
    use super::{Leaving, SUB_ROW, SUBS_SHOWN, SideMotion, Shown, SubLeaving, SubMotion, SubShown, TEAM_ROW, TeamState, sub_rows, subs_cap, team_height};
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
        assert_eq!(team_height(0, 0), 0.);
        assert_eq!(team_height(4, 0), 4. * TEAM_ROW);
        assert_eq!(team_height(6, 0), 6. * TEAM_ROW);
        assert_eq!(team_height(9, 0), 6. * TEAM_ROW, "past six, the block keeps its height and scrolls");
        const { assert!(TEAM_ROW <= 28.) };
    }

    #[test]
    fn the_team_block_grows_by_its_subagents_rows_as_far_as_they_have_room() {
        // Two members and three subagent rows: all in sight.
        assert_eq!(team_height(2, 3), 2. * TEAM_ROW + 3. * SUB_ROW);
        // Six members leave no room of their own: still six rows', then it scrolls.
        assert_eq!(team_height(6, 9), 6. * TEAM_ROW + 6. * SUB_ROW);
        assert_eq!(team_height(9, 2), 6. * TEAM_ROW + 2. * SUB_ROW);
    }

    #[test]
    fn three_subagents_show_under_a_member_then_more_and_fewer_when_the_block_is_full() {
        assert_eq!((sub_rows(0, 3), sub_rows(2, 3), sub_rows(3, 3), sub_rows(5, 3)), (0, 2, 3, 4), "three, then a row for the rest");
        assert_eq!(sub_rows(5, 0), 0, "none: the member's row counts them");
        // Two members, room for six rows: three each at most, then fewer.
        assert_eq!(subs_cap(2, &[2, 5]), SUBS_SHOWN, "2 and 3+more: six rows");
        assert_eq!(subs_cap(2, &[5, 5]), 2, "2+more twice");
        // Four members (room for six rows too): fewer each, the members' rows staying.
        assert_eq!(subs_cap(4, &[1, 1]), 3);
        assert_eq!(subs_cap(4, &[5, 0, 5, 0]), 2);
        assert_eq!(subs_cap(4, &[5, 5, 5, 0]), 1, "1+more three times");
        // Five members at it with subagents: no room for rows, counts on the members'.
        assert_eq!(subs_cap(6, &[2, 2, 2, 2, 2, 0]), 0);
        // Whatever the cap, the rows fit the room the members leave.
        for counts in [[3usize, 3, 3, 3], [9, 0, 0, 1], [1, 1, 1, 1]] {
            let cap = subs_cap(4, &counts);
            let rows: usize = counts.iter().map(|&n| sub_rows(n, cap)).sum();
            assert!(rows as f32 * SUB_ROW <= team_height(4, rows) - 4. * TEAM_ROW, "{counts:?}");
        }
    }

    fn sub(member: u64, id: &str) -> SubShown {
        SubShown { member, id: id.into(), what: format!("task {id}") }
    }

    #[test]
    fn subagent_rows_ease_in_as_they_start_and_out_as_they_hand_back() {
        let t0 = Instant::now();
        let mut m = SubMotion::default();
        m.note(&[sub(2, "a")], &["a"], t0, true);
        assert!(m.arriving(t0).is_empty(), "the window opening eases nothing in");
        // b starts; a hands back.
        let t1 = t0 + Duration::from_millis(500);
        m.note(&[sub(2, "b")], &["b"], t1, true);
        assert_eq!(m.arriving(t1), ["b"]);
        assert_eq!(m.leaving(t1).into_iter().map(|l: SubLeaving| (l.member, l.id)).collect::<Vec<_>>(), [(2, "a".to_owned())]);
        // c starts past the cap: no row of its own, nothing eases. b's row gives way to
        // another member's while b works on: it just goes, it didn't hand back.
        let t2 = t1 + crate::motion::BASE;
        m.note(&[sub(3, "d")], &["b", "c", "d"], t2, true);
        assert_eq!(m.arriving(t2), ["d"]);
        assert!(m.leaving(t2).is_empty(), "{:?}", m.leaving(t2));
        // Reduce motion: nothing moves.
        m.note(&[], &[], t2 + Duration::from_millis(10), false);
        assert!(m.arriving(t2).is_empty() && m.leaving(t2 + Duration::from_millis(10)).is_empty());
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
    }
}
