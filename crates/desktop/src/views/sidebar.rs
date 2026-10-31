//! The sidebar, after Claude's: a strip by the traffic lights (hide ⌘\,
//! search ⌘K, back and forward), then a few quiet rows — 새 메시지 ⌘N,
//! PM 대화, 나에게 (a badge), 작업 중 (a count), 모든 일 — and the live work as
//! Claude lists sessions: under a header per project, one line each for what
//! a member is at now (a spinner) and what waits on the person (an orange
//! dot), the title cut with an ellipsis, five at most and "+N개 더". A
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
use crate::side_split::Divider;
use crate::theme::{Palette, Sans, UI};
use crate::{Screen, Shell, motion};

pub const WIDTH: f32 = 260.;
pub const MIN_WIDTH: f32 = 200.;
pub const MAX_WIDTH: f32 = 480.;
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
/// A dragged width as kept: within bounds, and the usual width when within
/// a few points of it, so it is easy to come back to.
pub fn settle(width: f32) -> f32 {
    let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    if (width - WIDTH).abs() <= SETTLE { WIDTH } else { width }
}

/// How near the usual width a drag settles on it.
const SETTLE: f32 = 8.;

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

/// A project taken off the squad, while "되돌리기" can put it back.
#[derive(Debug, Clone)]
pub struct Removed {
    pub name: String,
    pub path: std::path::PathBuf,
    pub at: Instant,
}

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
    let pm_on = shell.screen == Screen::Chat && model.side_open() == Some(Side::Pm) && model.draft_talk.is_none();
    let new_on = shell.screen == Screen::Chat && model.draft_talk.as_ref().is_some_and(|(_, project)| project.is_none());
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
        .child(icon_button(p, "search", IconName::Search, true, Some(t!("sidebar.search_tip").into())).on_click(cx.listener(|this, _, window, cx| this.search_everything(window, cx))))
        .child(crate::ui::title_gap());

    // A badge for what waits on the person; a quiet count for the rest.
    let nav = |id: &'static str, name: IconName, label: String, badge: usize, count: usize, on: bool| {
        row_shell(p, id, on)
            .h(px(32.))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(name, 16., if on { p.text } else { p.text2 })))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else { p.text2 }).child(label))
            .when(badge > 0, |r| {
                r.child(div().flex_none().min_w(px(18.)).h(px(18.)).px(px(5.)).flex().items_center().justify_center().rounded(px(9.)).bg(p.accent).text_color(p.on_accent).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(badge.to_string()))
            })
            .when(count > 0, |r| r.child(div().flex_none().min_w(px(18.)).h(px(18.)).px(px(5.)).flex().items_center().justify_center().text_size(px(12.)).text_color(p.muted).child(count.to_string())))
    };
    let nav_rows = v_flex()
        .gap(px(1.))
        .child(
            nav("nav-new", IconName::Plus, t!("sidebar.new_conversation").into(), 0, 0, new_on)
                .child(div().flex_none().text_size(px(11.)).text_color(p.muted).child("⌘N"))
                .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx))),
        )
        .child(nav("nav-pm", IconName::Bot, t!("sidebar.pm_talk").into(), 0, 0, pm_on).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                this.open_side(Side::Pm, window, cx);
            }
        })))
        // The conversations about the whole squad, under the PM's own.
        .children(conversation_rows(shell, None, cx))
        .child(nav("nav-inbox", IconName::Inbox, t!("board.menu_inbox").into(), needs, 0, on_board && menu == Menu::Inbox).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }), window, cx) {
                this.open_side_menu(Menu::Inbox, cx);
            }
        })))
        .child(nav("nav-working", IconName::LoaderCircle, t!("board.menu_working").into(), 0, working, on_board && menu == Menu::Working).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Board, menu: Menu::Working, member: None, ticket: None }), window, cx) {
                this.open_side_menu(Menu::Working, cx);
            }
        })))
        .child(nav("nav-all", IconName::LayoutDashboard, t!("board.menu_all").into(), 0, 0, on_board && matches!(menu, Menu::All | Menu::View(_))).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Board, menu: Menu::All, member: None, ticket: None }), window, cx) {
                this.open_side_menu(Menu::All, cx);
            }
        })))
        .child(nav("nav-overview", IconName::ChartPie, t!("overview.title").into(), 0, 0, shell.screen == Screen::Overview).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Overview, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                this.open_overview(cx);
            }
        })))
        .child(nav("nav-roadmap", IconName::Map, t!("roadmap.title").into(), 0, 0, shell.screen == Screen::Roadmap).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Roadmap, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                this.open_roadmap(cx);
            }
        })))
        .child(nav("nav-design", IconName::Palette, t!("design.title").into(), 0, 0, shell.screen == Screen::Design).on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Design, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                this.open_design(cx);
            }
        })));

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
    // The projects under a heading of their own, with the button that adds one.
    if !groups.is_empty() || !model.projects.is_empty() {
        list.push(projects_heading(shell, cx));
    }
    let mut after_rows = false;
    // Every project with something in it; the quiet ones past a few behind "+N개 더".
    let talked: Vec<bool> = groups.iter().map(|g| !g.is_quiet() || g.name.as_ref().is_some_and(|n| model.conversations().iter().any(|l| l.project.as_deref() == Some(n.as_str())))).collect();
    let (shown_groups, hidden) = shown_projects(&talked, shell.board.side_projects_all, PROJECTS_SHOWN);
    for (ix, g) in shown_groups.iter().map(|&i| &groups[i]).enumerate() {
        let folded = shell.remembered.folded.contains(&g.key());
        let gap = if ix == 0 { 2. } else if after_rows { 8. } else { 1. };
        // A project's own conversations with the PM (the squad's folder has none:
        // those about the whole squad are under the PM).
        let talks = g.name.as_ref().map(|name| conversation_rows(shell, Some(name.as_str()), cx)).unwrap_or_default();
        list.push(project_head(shell, g, folded, gap, !talks.is_empty(), cx));
        after_rows = !folded && (!g.is_quiet() || !talks.is_empty() || leaving.iter().any(|l| l.key == g.key()));
        if folded {
            continue;
        }
        // What waits on the person first, in one line that opens to its tickets; then the conversations.
        if !g.is_quiet() {
            list.push(work_line(shell, g, cx));
        }
        if shell.board.side_work_open.contains(&g.key()) {
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
        list.extend(talks);
    }
    if hidden > 0 || shell.board.side_projects_all {
        list.push(projects_more_row(shell, hidden, cx));
    }
    list.extend(removed_row(shell, cx));
    list.push(done_row(shell, after_rows, cx));

    // The three areas: what each was drawn as goes to the hairlines' drags.
    let drawn = shell.board.side_drawn.clone();
    let measured = drawn.get();
    let room = measured.iter().sum::<f32>();
    let (usage_h, team_h) = if room > 0. { crate::side_split::fit(shell.remembered.side_usage, shell.remembered.side_team, room) } else { (shell.remembered.side_usage, shell.remembered.side_team) };
    let seen = drawn.clone();
    let body = v_flex()
        .id("side")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .pt_1()
        .pb_3()
        .when(dim, |l| l.opacity(0.8))
        .child(nav_rows)
        .children(list)
        .on_prepaint(move |b, _, _| seen.set([f32::from(b.size.height), seen.get()[1], seen.get()[2]]));
    let usage = super::switching::meters(shell, cx).map(|meters| split_area(shell, Divider::Usage, usage_h, meters, cx));
    if usage.is_none() {
        drawn.set([measured[0], 0., measured[2]]);
    }
    let team = team_block(shell, team_h, cx).map(|team| split_area(shell, Divider::Team, None, team, cx));

    v_flex()
        .w(px(ui.sidebar_width))
        .h_full()
        .flex_none()
        .relative()
        .bg(p.side)
        .sans()
        .text_color(p.text)
        .child(strip)
        .child(body)
        .children(usage)
        .children(team)
        .children(relaunch(p, shell, cx))
        .child(footer(p, shell, cx))
        .children(popup(p, shell, cx))
        .children(project_menu(shell, cx))
        // The edge: drag to resize (it settles on the usual width when near
        // it), double-click for the usual width again. Lit under the pointer.
        .child(
            div()
                .id("sidebar-edge")
                .group("sidebar-edge")
                .absolute()
                .top_0()
                .bottom_0()
                // Easy to catch: most of it inside the sidebar, where nothing
                // else takes the pointer, and a thick line under it.
                .right(px(-3.))
                .w(px(12.))
                .cursor_col_resize()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right(px(3.))
                        .w(px(if ui.sidebar_drag.is_some() { 3. } else { 1. }))
                        .bg(if ui.sidebar_drag.is_some() { p.accent } else { p.hair })
                        .group_hover("sidebar-edge", move |s| s.w(px(3.)).bg(p.accent).opacity(0.8)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        if e.click_count >= 2 {
                            this.board.sidebar_drag = Some((f32::from(e.position.x), WIDTH));
                            this.board.sidebar_width = WIDTH;
                            this.sidebar_released(cx);
                            return;
                        }
                        this.board.sidebar_drag = Some((f32::from(e.position.x), this.board.sidebar_width));
                        cx.notify();
                    }),
                ),
        )
}

/// "프로젝트", over the squad's projects, and its "+" (the same as the
/// project list's "Add a project…").
fn projects_heading(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let local = !shell.endpoint.is_remote();
    h_flex()
        .id("projects-heading")
        .group("projects-heading")
        .h(px(26.))
        .mt(px(14.))
        .mx_2()
        .pl_2()
        .pr_1()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(div().flex_1().child(t!("sidebar.projects")))
        .child(
            div()
                .id("projects-add")
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.side_hover))
                .tip(p, t!("header.add_project"), None)
                .child(icon(IconName::Plus, 13., p.muted))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if local {
                        this.pick_project_folder(cx);
                    } else {
                        this.board.popover = Some(super::board::Popover::Projects);
                        this.board.slice_place = "chat";
                        this.start_adding_project(window, cx);
                    }
                })),
        )
        .into_any_element()
}

/// A project's header being dragged to a new place.
#[derive(Clone)]
pub struct ProjectDrag {
    pub name: String,
    pub palette: Palette,
}

/// The project's name under the pointer while it is dragged.
struct ProjectGhost(ProjectDrag);

impl Render for ProjectGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.0.palette;
        div().px_3().py_1().rounded(px(6.)).bg(p.popover()).border_1().border_color(p.accent).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(self.0.name.clone())
    }
}

/// The ticket the screen shows, for the row's highlight.
fn selected_ticket(shell: &Shell) -> Option<TicketId> {
    match shell.screen {
        Screen::Chat => shell.model.opened,
        Screen::Board => match shell.picked_need() {
            Some(Need::Ticket(id)) => Some(id),
            _ => None,
        },
        Screen::Settings | Screen::Design | Screen::Overview | Screen::Roadmap => None,
    }
}

/// "HQ", beside the name of the project that is the squad's HQ.
pub fn hq_badge(p: Palette) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(5.))
        .h(px(16.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(p.wash)
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .child("HQ")
}

/// A project's header, as tall and as large as the rows above: the chevron
/// (fold), its colour and name (switch the project slice to it), and what is
/// live in it — a quiet count at work, an orange badge waiting on the person.
/// On the pointer the counts give way to "+" (a new conversation here) and
/// "⋯" (its menu, as a right-click opens it).
fn project_head(shell: &Shell, g: &SideGroup, folded: bool, gap: f32, talks: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = g.key();
    let group = SharedString::from(format!("head-{key}"));
    // Nothing in it to fold: no live work, no conversation.
    let quiet = g.is_quiet() && !talks;
    let focused = g.name.is_some() && shell.model.focus == g.name;
    let label = g.name.clone().unwrap_or_else(|| t!("board.squad_folder").into());
    let (fold_project, name_project) = (g.name.clone(), g.name.clone());
    // The chevron: open, always; folded, on the pointer — the name starts at one x either way.
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
                .child(div().when(folded, |d| d.invisible().group_hover(group.clone(), |s| s.visible())).child(icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 12., p.muted)))
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
    // Where the project is, on the daemon's machine: on the pointer, and to copy.
    let folder = g.name.as_ref().and_then(|n| shell.model.projects.iter().find(|p| &equipe_core::board::project_name(p) == n)).cloned();
    let name_tip = match &folder {
        Some(path) => format!("{label}\n{name_tip}\n{}", path.display()),
        None => name_tip.to_owned(),
    };
    let hq = g.name.as_deref().is_some_and(|n| shell.model.is_hq(n));
    let ident = match g.at {
        // The HQ: a building, not a dot — the squad's own place, not one of its products.
        Some(_) if hq => icon(IconName::Building2, 13., p.text2).into_any_element(),
        Some(at) => dot(project_color(p, at), 8.).into_any_element(),
        None => icon(IconName::Folder, 13., p.muted).into_any_element(),
    };
    let name_color = if focused { p.text } else { p.text2 };
    // What is live in it, at the end: at work (a quiet count), waiting on the person (an orange badge).
    let meta = h_flex()
        .flex_none()
        .gap(px(6.))
        .items_center()
        .group_hover(group.clone(), |s| s.invisible())
        .when(g.working > 0, |m| {
            m.child(
                h_flex()
                    .id(SharedString::from(format!("working-{key}")))
                    .gap(px(3.))
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .tip(p, t!("sidebar.work_working", n = g.working), None)
                    .child(icon(IconName::LoaderCircle, 12., p.muted))
                    .child(g.working.to_string()),
            )
        })
        .when(g.waiting > 0, |m| {
            m.child(
                div()
                    .id(SharedString::from(format!("waiting-{key}")))
                    .min_w(px(18.))
                    .h(px(18.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .bg(p.orange)
                    .text_color(p.on_accent)
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .tip(p, t!("sidebar.work_needs_you", n = g.waiting), None)
                    .child(g.waiting.to_string()),
            )
        });
    // On the pointer, in the counts' place: a new conversation here, and the project's menu.
    let small = |id: SharedString, name: IconName, tip: String| {
        div()
            .id(id)
            .size(px(22.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |s| s.bg(p.side_hover))
            .tip(p, tip, None)
            .child(icon(name, 14., p.text2))
    };
    let actions = g.name.clone().map(|project| {
        let (new_in, menu_of) = (project.clone(), project);
        h_flex()
            .absolute()
            .right(px(4.))
            .top_0()
            .bottom_0()
            .items_center()
            .gap(px(1.))
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .child(small(SharedString::from(format!("new-in-{key}")), IconName::Plus, t!("sidebar.new_in_project").into()).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.start_conversation(Some(new_in.clone()), window, cx);
            })))
            .child(small(SharedString::from(format!("menu-of-{key}")), IconName::Ellipsis, t!("sidebar.project_menu").into()).on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_project_menu(menu_of.clone(), e.position(), cx);
            })))
    });
    let drag = g.name.clone().map(|name| ProjectDrag { name, palette: p });
    let target = g.name.clone();
    let menu_target = g.name.clone();
    let row = h_flex()
        .id(SharedString::from(format!("head-row-{key}")))
        .group(group)
        .relative()
        // Dragged onto another project's header, it goes before that one.
        .when_some(drag, |row, drag| row.on_drag(drag, |d, _, _, cx| cx.new(|_| ProjectGhost(d.clone()))))
        .when_some(target, |row, target| {
            row.drag_over::<ProjectDrag>(move |style, _, _, _| style.border_t_2().border_color(p.accent))
                .on_drop(cx.listener(move |this, d: &ProjectDrag, _, cx| this.reorder_project(&d.name, Some(&target), cx)))
        })
        // Right-click: the same menu as its ⋯.
        .when_some(menu_target, |row, project| {
            row.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_project_menu(project.clone(), e.position, cx);
                }),
            )
        })
        .h(px(ROW))
        .mt(px(gap))
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .when(focused, |r| r.bg(p.side_hover))
        .tip_at(p, name_tip, None, gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            // ⌘-click: the project in a window of its own (the squad's folder has no slice to open).
            if let Some(project) = name_project.clone()
                && this.open_elsewhere(e, super::elsewhere::Goal::Project(project), window, cx)
            {
                return;
            }
            this.side_head_click(HeadClick::Name, name_project.clone(), window, cx)
        }))
        .text_size(px(UI))
        .child(chevron)
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(ident))
        .child(div().flex_1().min_w_0().truncate().text_color(name_color).font_weight(if g.waiting > 0 || focused { FontWeight::MEDIUM } else { FontWeight::NORMAL }).child(label))
        .when(hq, |r| r.child(hq_badge(p)))
        .when(head_counts(folded), |r| r.child(meta))
        .children(actions);
    row.into_any_element()
}

/// The conversations with the PM about `project` (`None`: the whole squad),
/// most lately used first — and, while one is being started there, it.
fn conversation_rows(shell: &Shell, project: Option<&str>, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    // Inside a project, under its colour: one step in.
    let indent = inset(SideRow::Talk { in_project: project.is_some() });
    let mut rows = Vec::new();
    if let Some((draft, about)) = &model.draft_talk
        && about.as_deref() == project
        && project.is_some()
    {
        let on = shell.screen == Screen::Chat;
        rows.push(
            row_shell(p, ("conv-draft", *draft as usize), on)
                .pl(px(8. + indent))
                .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(crate::icons::icon(crate::icons::Icon::Message, 13., p.text2)))
                .child(row_title(p, t!("sidebar.new_conversation").into(), on, true))
                .into_any_element(),
        );
    }
    let talks: Vec<&crate::state::Lane> = model.conversations().into_iter().filter(|l| l.project.as_deref() == project).collect();
    let all = shell.board.side_talks_all.contains(project.unwrap_or(""));
    let shown = if all { talks.len() } else { talks.len().min(TALKS_SHOWN) };
    for lane in &talks[..shown] {
        rows.push(conversation_row(shell, lane, indent, cx));
    }
    if talks.len() > TALKS_SHOWN {
        let key = project.unwrap_or("").to_owned();
        let words = if all { t!("sidebar.fewer").to_owned() } else { t!("sidebar.more", n = talks.len() - TALKS_SHOWN) };
        rows.push(
            row_shell(p, SharedString::from(format!("talks-more-{key}")), false)
                .h(px(26.))
                .pl(px(8. + indent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.board.side_talks_all.remove(&key) {
                        this.board.side_talks_all.insert(key.clone());
                    }
                    cx.notify();
                }))
                .child(div().w(px(MARK_BOX)).flex_none())
                .child(div().text_size(px(12.)).text_color(p.muted).child(words))
                .into_any_element(),
        );
    }
    rows
}

/// A project's conversations with the PM shown at once; the rest behind "+N개 더".
const TALKS_SHOWN: usize = 5;

/// A conversation with the PM: a spinner while it answers, its first words,
/// and on the pointer the way to put it away. ⌘-click: in a window of its own.
fn conversation_row(shell: &Shell, lane: &crate::state::Lane, indent: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = lane.session;
    let on = shell.screen == Screen::Chat && shell.model.talk == Some(id) && shell.model.draft_talk.is_none();
    let group = SharedString::from(format!("conv-{id}"));
    let mark = if lane.presence == crate::state::Presence::Working {
        mark_element(p, Mark::Spinner, shell.spin(cx))
    } else {
        div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(crate::icons::icon(crate::icons::Icon::Message, 13., if on { p.text } else { p.muted })).into_any_element()
    };
    let title = lane.title.clone().unwrap_or_else(|| t!("sidebar.new_conversation").into());
    row_shell(p, ("conv", id as usize), on)
        .pl(px(8. + indent))
        .group(group.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Conversation(id), window, cx) {
                this.open_side(Side::Conversation(id), window, cx);
            }
        }))
        .child(mark)
        .child(row_title(p, title, on, false))
        .child(
            div()
                .id(("conv-away", id as usize))
                .size(px(20.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .invisible()
                .group_hover(group, |s| s.visible())
                .hover(move |s| s.bg(p.side_hover))
                .tip(p, t!("sidebar.put_away"), None)
                .child(crate::icons::icon(crate::icons::Icon::X, 12., p.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.put_away_conversation(id, cx);
                })),
        )
        .into_any_element()
}

/// A project's live work in one line, one step in — what waits on the
/// person first ("확인 필요 1 · 작업 중 3") — that opens to its tickets.
fn work_line(shell: &Shell, g: &SideGroup, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = g.key();
    let open = shell.board.side_work_open.contains(&key);
    let toggle = key.clone();
    let mark = if g.waiting > 0 { dot(p.orange, 7.).into_any_element() } else { icon(IconName::LoaderCircle, 12., p.muted).into_any_element() };
    row_shell(p, SharedString::from(format!("work-{key}")), false)
        .pl(px(8. + inset(SideRow::Work)))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.board.side_work_open.remove(&toggle) {
                this.board.side_work_open.insert(toggle.clone());
            }
            cx.notify();
        }))
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(mark))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .text_size(px(TEXT))
                .when(g.waiting > 0, |r| r.child(div().text_color(p.orange).font_weight(FontWeight::MEDIUM).child(t!("sidebar.work_needs_you", n = g.waiting))))
                .when(g.working > 0 && g.waiting > 0, |r| r.child(div().text_color(p.muted).child("·")))
                .when(g.working > 0, |r| r.child(div().text_color(p.text2).child(t!("sidebar.work_working", n = g.working)))),
        )
        .child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 12., p.muted))
        .into_any_element()
}

/// "+N개 더" under the projects shown, or "접기" once all are.
fn projects_more_row(shell: &Shell, hidden: usize, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let words = if shell.board.side_projects_all { t!("sidebar.fewer").to_owned() } else { t!("sidebar.more", n = hidden) };
    row_shell(p, "projects-more", false)
        .h(px(26.))
        .on_click(cx.listener(|this, _, _, cx| {
            this.board.side_projects_all = !this.board.side_projects_all;
            cx.notify();
        }))
        .child(div().w(px(MARK_BOX)).flex_none())
        .child(div().w(px(MARK_BOX)).flex_none())
        .child(div().text_size(px(12.)).text_color(p.muted).child(words))
        .into_any_element()
}

/// A project just taken off the squad: said, with the way to put it back,
/// for a few seconds.
fn removed_row(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let removed = shell.board.removed_project.as_ref()?;
    let left = REMOVED_SHOWN.saturating_sub(removed.at.elapsed());
    if left.is_zero() {
        return None;
    }
    shell.wake_at(Instant::now() + left, cx);
    Some(
        h_flex()
            .id("project-removed")
            .h(px(32.))
            .mx_2()
            .mt_1()
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(8.))
            .bg(p.wash)
            .border_1()
            .border_color(p.hair)
            .text_size(px(TEXT))
            .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(t!("sidebar.removed", name = removed.name)))
            .child(
                div()
                    .id("project-removed-undo")
                    .flex_none()
                    .px_1()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(p.accent)
                    .font_weight(FontWeight::MEDIUM)
                    .hover(move |s| s.bg(p.side_hover))
                    .child(t!("sidebar.undo_remove"))
                    .on_click(cx.listener(|this, _, _, cx| this.undo_remove_project(cx))),
            )
            .into_any_element(),
    )
}

/// A project's menu, where its ⋯ or a right-click opened it: a new
/// conversation, where it is, the HQ, and taking it off the squad — set
/// apart, in red, last.
fn project_menu(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let (name, at) = shell.board.project_menu.clone()?;
    let path = shell.model.projects.iter().find(|path| equipe_core::board::project_name(path) == name)?.clone();
    let hq = shell.model.is_hq(&name);
    let local = !shell.endpoint.is_remote();
    let item = |id: &'static str, name: IconName, label: String, color: Rgba| {
        h_flex()
            .id(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(TEXT))
            .text_color(color)
            .cursor_pointer()
            .hover(move |s| s.bg(p.side_hover))
            .child(icon(name, 14., color))
            .child(div().flex_1().min_w_0().truncate().child(label))
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    let project = name.clone();
    rows.push(
        item("pm-new", IconName::Plus, t!("sidebar.new_in_project").into(), p.text)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.board.project_menu = None;
                this.start_conversation(Some(project.clone()), window, cx);
            }))
            .into_any_element(),
    );
    if local {
        let reveal = path.clone();
        rows.push(
            item("pm-reveal", IconName::Folder, t!("render.file_reveal").into(), p.text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.project_menu = None;
                    cx.reveal_path(&reveal);
                    cx.notify();
                }))
                .into_any_element(),
        );
    }
    let copy = path.clone();
    rows.push(
        item("pm-copy", IconName::Copy, t!("sidebar.copy_project_path").into(), p.text)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.project_menu = None;
                cx.write_to_clipboard(ClipboardItem::new_string(copy.display().to_string()));
                cx.notify();
            }))
            .into_any_element(),
    );
    let mark_to = if hq { None } else { Some(path.clone()) };
    rows.push(
        item("pm-hq", IconName::Building2, if hq { t!("sidebar.hq_unset") } else { t!("sidebar.hq_set") }.into(), p.text)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.project_menu = None;
                this.set_hq(mark_to.clone(), cx);
            }))
            .into_any_element(),
    );
    rows.push(div().h(px(1.)).mx_1().my_1().bg(p.hair).into_any_element());
    rows.push(
        item("pm-remove", IconName::Close, t!("sidebar.remove_project").into(), p.red)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.board.project_menu = None;
                this.ask_remove_project(&name, window, cx);
            }))
            .into_any_element(),
    );
    Some(
        deferred(
            anchored().position(at).snap_to_window().child(
                v_flex()
                    .id("project-menu")
                    .w(px(220.))
                    .p_1()
                    .rounded(px(10.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.board.project_menu = None;
                        cx.notify();
                    }))
                    .children(rows),
            ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

/// The question before `name` comes off the squad: its folder, tickets and
/// branches stay; the tickets at work in it now are said.
pub fn remove_ask(model: &Model, name: &str, path: std::path::PathBuf) -> crate::sheet::Ask {
    let working = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress && model.ticket_project_name(t).as_deref() == Some(name)).count();
    let mut body = vec![t!("sidebar.remove_body").to_owned()];
    if working > 0 {
        body.push(t!("sidebar.remove_working", n = working));
    }
    crate::sheet::Ask {
        title: t!("sidebar.remove_ask", name = name),
        body,
        yes: t!("sidebar.remove_yes").into(),
        then: crate::sheet::Then::Send(vec![equipe_core::protocol::Command::RemoveProject { squad: model.squad.clone(), path }]),
    }
}

/// Every project with something in it stays; past `cap` the quiet ones go
/// behind "+N개 더", unless all are asked for. The indices shown, in order,
/// and how many are behind.
pub fn shown_projects(busy: &[bool], all: bool, cap: usize) -> (Vec<usize>, usize) {
    if all || busy.len() <= cap {
        return ((0..busy.len()).collect(), 0);
    }
    let room = cap.saturating_sub(busy.iter().filter(|b| **b).count());
    let mut quiet_left = room;
    let mut shown = Vec::new();
    for (i, b) in busy.iter().enumerate() {
        if *b {
            shown.push(i);
        } else if quiet_left > 0 {
            quiet_left -= 1;
            shown.push(i);
        }
    }
    let hidden = busy.len() - shown.len();
    (shown, hidden)
}

/// Projects shown before "+N개 더", the busy ones always.
const PROJECTS_SHOWN: usize = 6;
/// How far a project's own rows sit in from its header: under its name.
const CHILD_INDENT: f32 = MARK_BOX + 8.;

/// A row under a project's header (or the PM's), as far as where it sits goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideRow {
    /// Its live work in one line.
    Work,
    /// A live ticket of it, or one leaving.
    Ticket,
    /// "+N개 더" after its tickets.
    More,
    /// A conversation with the PM: about a project, or the whole squad.
    Talk { in_project: bool },
}

/// How far in from its header a row starts: every row of a project one step,
/// its mark under the project's dot and its title under the project's name.
pub fn inset(row: SideRow) -> f32 {
    match row {
        SideRow::Talk { in_project: false } => 0.,
        SideRow::Work | SideRow::Ticket | SideRow::More | SideRow::Talk { in_project: true } => CHILD_INDENT,
    }
}

/// Whether a project's header shows its counts (at work, waiting on the
/// person): only folded — open, the line under it says them.
pub fn head_counts(folded: bool) -> bool {
    folded
}
/// How long "뺐어요 · 되돌리기" stays.
const REMOVED_SHOWN: std::time::Duration = std::time::Duration::from_secs(8);

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

/// A row's title on one line, cut with an ellipsis before what follows it.
fn row_title(p: Palette, title: String, on: bool, dim: bool) -> Div {
    div().flex_1().min_w_0().truncate().text_color(if dim { p.muted } else if on { p.text } else { p.text2 }).child(title)
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
/// the person's turn), its title (an ellipsis where it is cut), how long it has been so — and,
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
    let row = super::ticket_card::hover_link(row_shell(p, ("ticket", id as usize), on).pl(px(8. + inset(SideRow::Ticket))), id, super::ticket_card::Side::Right)
        .group(group.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            let goal = super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: None, ticket: Some(id) });
            if !this.open_elsewhere(e, goal, window, cx) {
                this.open_ticket_row(id, window, cx);
            }
        }))
        .child(mark_element(p, mark, if mark == Mark::Spinner { shell.spin(cx) } else { 0. }))
        .child(row_title(p, t.title.clone(), on, false))
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
        .pl(px(8. + inset(SideRow::Ticket)))
        .cursor_default()
        .child(mark_element(p, mark, 0.))
        .child(row_title(p, gone.title.clone(), false, true))
        .with_animation(ElementId::NamedInteger("side-out".into(), u64::from(gone.id)), Animation::new(motion::BASE).with_easing(motion::ease_out), |el, t| el.h(px(ROW * (1. - t))).opacity(1. - t))
        .into_any_element()
}

/// "+N개 더 ›": the rest of a project's live work, in 모든 일 narrowed to it.
fn more_row(p: Palette, g: &SideGroup, cx: &mut Context<Shell>) -> AnyElement {
    let words = g.name.as_ref().map(|n| format!("project:{n}")).unwrap_or_default();
    row_shell(p, SharedString::from(format!("more-{}", g.key())), false)
        .h(px(26.))
        .pl(px(8. + inset(SideRow::More)))
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
fn team_block(shell: &Shell, height: Option<f32>, cx: &mut Context<Shell>) -> Option<AnyElement> {
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
    // Dragged to a height: the list takes what the heading leaves, scrolling.
    let list = v_flex().id("team-list").overflow_y_scroll().children(rows);
    let list = match height {
        Some(_) => list.flex_1().min_h_0(),
        None => list.max_h(px(team_height(members.len(), child_rows))),
    };
    Some(
        v_flex()
            .flex_none()
            .when_some(height, |b, h| b.h(px(h)))
            .pt(px(6.))
            .pb_1()
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
            .child(list)
            .into_any_element(),
    )
}

/// One of the sidebar's lower areas (usage, the team) under its hairline:
/// held and dragged, the room moves between it and the area above
/// (`side_split.rs`); double-clicked, the areas take their own heights again.
/// `height`: what it was dragged to, the area scrolling inside — none, its own.
fn split_area(shell: &Shell, held: Divider, height: Option<f32>, area: AnyElement, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let dragging = shell.board.side_split_drag.is_some_and(|(d, ..)| d == held);
    let seen = shell.board.side_drawn.clone();
    let at = if held == Divider::Usage { 1 } else { 2 };
    let id = if held == Divider::Usage { "side-split-usage" } else { "side-split-team" };
    let content = match height {
        Some(h) => v_flex().id(id).h(px(h)).overflow_y_scroll().child(area).into_any_element(),
        None => area,
    };
    let group = SharedString::from(format!("{id}-line"));
    v_flex()
        .flex_none()
        .relative()
        .child(content)
        .on_prepaint(move |b, _, _| {
            let mut now = seen.get();
            now[at] = f32::from(b.size.height);
            seen.set(now);
        })
        // The hairline, lit under the pointer and while held; easy to catch.
        .child(
            div()
                .id(SharedString::from(format!("{id}-grip")))
                .group(group.clone())
                .absolute()
                .top(px(-4.))
                .left_0()
                .right_0()
                .h(px(9.))
                .cursor_row_resize()
                .child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left_0()
                        .right_0()
                        .h(px(if dragging { 2. } else { 1. }))
                        .bg(if dragging { p.accent } else { p.hair })
                        .group_hover(group, move |s| s.h(px(2.)).bg(p.accent).opacity(0.8)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if e.click_count >= 2 {
                            this.side_split_reset(cx);
                            return;
                        }
                        let [projects, usage, team] = this.board.side_drawn.get();
                        let drawn = crate::side_split::Drawn { projects, usage: (usage > 0.).then_some(usage), team };
                        this.board.side_split_drag = Some((held, f32::from(e.position.y), drawn));
                        cx.notify();
                    }),
                ),
        )
        .into_any_element()
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
/// handed (cut with an ellipsis), how long it has been at it; its latest step on the
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
        .child(row_title(p, sub.what.clone(), false, false).text_size(px(12.)))
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
        .child(row_title(p, gone.what.clone(), false, true).text_size(px(12.)))
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
    /// Its conversation is being summarized, for this many seconds.
    Compacting(u64),
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
            Status::Working { .. } | Status::Ready { .. } if model.compacting_of(member.session).is_some() => TeamState::Compacting(model.compacting_of(member.session).map_or(0, |d| d.as_secs())),
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
            TeamState::Compacting(secs) => t!("compact.sidebar", elapsed = crate::convo::elapsed(std::time::Duration::from_secs(secs))),
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
    // Its seconds count up: drawn again when the next one is.
    if matches!(state, TeamState::Compacting(_)) {
        shell.wake_at(std::time::Instant::now() + std::time::Duration::from_secs(1), cx);
    }
    let mark = match state {
        TeamState::Working(_) | TeamState::Compacting(_) => Mark::Spinner,
        TeamState::Asks | TeamState::YourTurn => Mark::Waits,
        TeamState::Closed => Mark::Hollow,
        TeamState::Resting => Mark::Ring(p.member(hue)),
    };
    let group = SharedString::from(format!("mem-{session}"));
    row_shell(p, ("member", session), on)
        .h(px(TEAM_ROW))
        .group(group.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            let goal = super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: Some(session), ticket: None });
            if !this.open_elsewhere(e, goal, window, cx) {
                this.select_member(session, window, cx);
            }
        }))
        .child(mark_element(p, mark, if mark == Mark::Spinner { shell.spin(cx) } else { 0. }))
        // Name, then what it is doing against the right edge, then its engine:
        // each in the same place on every row.
        .child(div().flex_1().min_w(px(36.)).truncate().text_size(px(TEXT)).text_color(if on { p.text } else { p.text2 }).child(member.name.clone()))
        .child(
            div()
                .flex_none()
                .max_w(relative(0.55))
                .truncate()
                .text_right()
                .text_size(px(12.))
                .text_color(if state.waits() { p.orange } else { p.muted })
                .group_hover(group.clone(), |s| s.invisible())
                .child(state.words()),
        )
        .children(member.engine.map(|e| div().flex_none().group_hover(group.clone(), |s| s.invisible()).child(super::switching::mark(p, ("member-engine", session), e))))
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

/// Where updates stand, in one pill above the foot — this app's and the
/// daemon's together (`relaunch::both`): a new version being downloaded
/// (quiet), the daemon's machine at work on its own (quiet, a spinner),
/// something to press — restart into the new version (as Claude's "Relaunch
/// to update"), bring the daemon up to this app, or both in one press — or a
/// failure to try again.
fn relaunch(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    use crate::relaunch::Act;
    let daemon = shell.daemon_view();
    let (words, act, strong) = crate::relaunch::both(&shell.update, &daemon, env!("CARGO_PKG_VERSION"))?;
    // The whole of it on the pointer: which machine, which versions, why it failed.
    let whole = match &daemon.update {
        Some(crate::relaunch::DaemonUpdate::Failed(why)) => Some(t!("main.update_daemon_failed", why = why)),
        Some(crate::relaunch::DaemonUpdate::Done) => Some(t!("main.update_daemon_done").to_owned()),
        None if daemon.older || act == Act::UpdateBoth => Some(t!("main.update_daemon_line", place = daemon.place.as_str(), theirs = daemon.theirs.as_str(), ours = env!("CARGO_PKG_VERSION"))),
        _ => None,
    };
    // Words that shorten with an ellipsis, then the mark, inside the pill whatever the width.
    let pill = h_flex().id("relaunch").h(px(34.)).flex_none().mx_2().mb_1().px_3().gap_2().overflow_hidden().rounded(px(8.)).items_center().justify_between().text_size(px(TEXT));
    let pill = match whole {
        Some(whole) => pill.tip(p, whole, None),
        None => pill,
    };
    let mark = |m: &'static str| div().flex_none().child(m);
    let words = div().flex_1().min_w_0().truncate().child(words);
    Some(match act {
        Act::None => pill
            .border_1()
            .border_color(p.hair)
            .text_color(p.muted)
            .child(words)
            .when(matches!(daemon.update, Some(crate::relaunch::DaemonUpdate::Running | crate::relaunch::DaemonUpdate::Restarting)), |b| b.child(motion::spinner(13., p.muted))),
        _ if strong => pill
            .bg(p.primary)
            .text_color(p.on_primary)
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .on_click(cx.listener(move |this, _, _, cx| this.update_act(act, cx)))
            .child(words)
            .child(mark("→")),
        _ => pill
            .border_1()
            .border_color(if act == Act::RestartDaemonNow { p.red } else { p.hair })
            .text_color(if act == Act::RestartDaemonNow { p.red } else { p.text })
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| this.update_act(act, cx)))
            .child(words)
            .child(mark(if matches!(act, Act::CheckNow | Act::UpdateDaemon) { "↻" } else { "→" })),
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
                .on_click(cx.listener(|this, _, _, cx| this.toggle_popup(Popup::Account, cx)))
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
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popup_outside(cx)))
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
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popup_outside(cx)))
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
        self.toggle_popup(Popup::Subagents(member), cx);
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
        // And a look inside it, beside the conversation.
        self.open_subagent_panel(member, session, id, window, cx);
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

    /// ⌘N: a new conversation with the PM, about the whole squad.
    pub fn new_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_conversation(None, window, cx);
    }

    /// A new conversation with the PM about `project` (`None`: the whole
    /// squad), empty until its first message makes it.
    pub fn start_conversation(&mut self, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.model.draft_conversation(project);
        self.screen = Screen::Chat;
        self.refresh(window, cx);
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// The open conversation with the PM moves to `project` — one not made
    /// yet just changes where it will be.
    pub fn move_conversation(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        if let Some((_, draft)) = self.model.draft_talk.as_mut() {
            *draft = project;
            self.model.revision += 1;
        } else if let Some(session) = self.model.talk {
            self.send_command(equipe_core::protocol::Command::MoveConversation { session, project });
        }
        cx.notify();
    }

    /// Put a conversation with the PM away: it leaves the list.
    pub fn put_away_conversation(&mut self, session: SessionId, cx: &mut Context<Self>) {
        self.send_command(equipe_core::protocol::Command::CloseConversation { session });
        cx.notify();
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
    /// The update pill pressed.
    pub fn update_act(&mut self, act: crate::relaunch::Act, cx: &mut Context<Self>) {
        use crate::relaunch::Act;
        match act {
            Act::None => {}
            Act::Relaunch => self.relaunch_to_update(cx),
            Act::UpdateDaemon => self.update_daemon(cx),
            Act::UpdateBoth => self.update_both(cx),
            Act::ArmRestart => {
                self.daemon_restart_armed = true;
                cx.notify();
            }
            Act::RestartDaemonNow => self.restart_daemon_now(cx),
            Act::Reveal => crate::relaunch::reveal(),
            Act::CheckNow => crate::relaunch::check_now(cx),
        }
    }

    /// A project's menu, at `at`.
    pub fn open_project_menu(&mut self, project: String, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.board.project_menu = Some((project, at));
        cx.notify();
    }

    /// Ask before taking `name` off the squad: what stays, and the work at it now.
    pub fn ask_remove_project(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.model.projects.iter().find(|p| equipe_core::board::project_name(p) == name).cloned() else { return };
        self.confirm = Some(remove_ask(&self.model, name, path));
        let _ = window;
        cx.notify();
    }

    /// The project just taken off, back on the squad.
    pub fn undo_remove_project(&mut self, cx: &mut Context<Self>) {
        let Some(removed) = self.board.removed_project.take() else { return };
        self.add_project(removed.path.display().to_string(), cx);
    }

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

    /// Dragging a hairline between the sidebar's areas.
    pub fn side_split_dragged(&mut self, y: f32, cx: &mut Context<Self>) {
        let Some((held, y0, at)) = self.board.side_split_drag else { return };
        let (usage, team) = crate::side_split::dragged(held, at, y - y0);
        self.remembered.side_usage = usage.or(self.remembered.side_usage);
        // The team as drawn stays its own height until a drag moves it.
        if held == Divider::Team || team != at.team {
            self.remembered.side_team = Some(team);
        }
        cx.notify();
    }

    pub fn side_split_released(&mut self, cx: &mut Context<Self>) {
        if self.board.side_split_drag.take().is_some() {
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }
    }

    /// A hairline double-clicked: usage and the team take their own heights again.
    pub fn side_split_reset(&mut self, cx: &mut Context<Self>) {
        self.board.side_split_drag = None;
        (self.remembered.side_usage, self.remembered.side_team) = (None, None);
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        cx.notify();
    }

    /// Dragging the sidebar's edge.
    pub fn sidebar_dragged(&mut self, x: f32, cx: &mut Context<Self>) {
        if let Some((x0, w0)) = self.board.sidebar_drag {
            self.board.sidebar_width = settle(w0 + x - x0);
            cx.notify();
        }
    }

    pub fn sidebar_released(&mut self, cx: &mut Context<Self>) {
        // (A double-click on the edge comes here with the usual width set.)
        if self.board.sidebar_drag.take().is_some() {
            self.remembered.sidebar_width = Some(self.board.sidebar_width);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }
    }

    // ── back and forward ──

    /// Where the window is now, for back and forward.
    pub fn place(&self) -> Place {
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

    pub fn go_to(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
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
            Screen::Overview => self.open_overview(cx),
            Screen::Roadmap => self.open_roadmap(cx),
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
    use super::{CHILD_INDENT, Leaving, MARK_BOX, MAX_WIDTH, MIN_WIDTH, SUB_ROW, SideRow, head_counts, inset, SUBS_SHOWN, SideMotion, Shown, SubLeaving, SubMotion, SubShown, TEAM_ROW, TeamState, WIDTH, remove_ask, settle, shown_projects, sub_rows, subs_cap, team_height};
    use crate::board::Live;
    use crate::state::{Member, Model, Presence, Verb};
    use equipe_core::protocol::Ticket;
    use std::time::{Duration, Instant};

    #[test]
    fn busy_projects_always_show_and_quiet_ones_past_a_few_go_behind_more() {
        assert_eq!(shown_projects(&[false; 4], false, 6), (vec![0, 1, 2, 3], 0));
        // Ten, two of them busy: the busy two and the first four quiet ones.
        let busy = [false, false, true, false, false, false, false, false, true, false];
        assert_eq!(shown_projects(&busy, false, 6), (vec![0, 1, 2, 3, 4, 8], 4));
        assert_eq!(shown_projects(&busy, true, 6).1, 0, "all, when asked");
        // More busy ones than room: every busy one still shows.
        assert_eq!(shown_projects(&[true; 8], false, 6), ((0..8).collect(), 0));
    }

    #[test]
    fn taking_a_project_off_asks_first_saying_what_stays_and_what_is_at_work() {
        let mut model = Model::new("acme");
        model.projects = vec!["/code/site".into(), "/code/app".into()];
        let mut at_work = ticket(3, "in_progress", "frontend");
        at_work.project = Some("/code/site".into());
        model.tickets = vec![at_work, ticket(4, "todo", "backend")];
        let ask = remove_ask(&model, "site", "/code/site".into());
        assert_eq!(ask.title, "site을(를) 스쿼드에서 뺄까요?");
        assert_eq!(ask.body.len(), 2, "what stays, and the one ticket at work: {:?}", ask.body);
        assert!(ask.destructive(), "its yes is red, and Enter doesn't say it");
        assert!(matches!(&ask.then, crate::sheet::Then::Send(c) if matches!(c[..], [equipe_core::protocol::Command::RemoveProject { .. }])));
        assert_eq!(remove_ask(&model, "app", "/code/app".into()).body.len(), 1, "nothing at work: no second line");
    }

    #[test]
    fn a_projects_rows_sit_on_one_grid_under_its_name() {
        // Its summary line, its tickets (live, leaving, "+N"), its conversations:
        // one step in, each mark under the project's dot, each title under its name.
        let children = [SideRow::Work, SideRow::Ticket, SideRow::More, SideRow::Talk { in_project: true }];
        assert!(children.iter().all(|&row| inset(row) == CHILD_INDENT), "{:?}", children.map(inset));
        assert_eq!(CHILD_INDENT, MARK_BOX + 8., "one step: the chevron's box and the gap after it");
        assert_eq!(inset(SideRow::Talk { in_project: false }), 0., "the squad's own talks sit under the PM, not in a project");
    }

    #[test]
    fn a_projects_counts_show_once() {
        // Open, its summary line under it says them; folded, its header does.
        assert!(!head_counts(false));
        assert!(head_counts(true));
    }

    #[test]
    fn a_dragged_width_stays_in_bounds_and_settles_on_the_usual_one() {
        assert_eq!(settle(100.), MIN_WIDTH);
        assert_eq!(settle(900.), MAX_WIDTH);
        assert_eq!(settle(WIDTH + 6.), WIDTH);
        assert_eq!(settle(WIDTH - 7.), WIDTH);
        assert_eq!(settle(WIDTH + 30.), WIDTH + 30.);
    }

    fn ticket(id: u32, status: &str, owner: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": owner, "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()
    }

    fn member(session: u64, name: &str, presence: Presence, ticket: Option<u32>) -> Member {
        Member { session, name: name.into(), role: name.into(), presence, verb: Verb::Working, ticket, memos: 0, context: None, model: None, engine: None, estimate: None, target: None, last_seen: 0, failed: false }
    }

    #[test]
    fn a_member_whose_conversation_is_summarized_says_so_with_the_seconds() {
        let mut model = Model::new("acme");
        model.members = vec![member(1, "pm", Presence::Ready, None), member(2, "frontend", Presence::Working, Some(12))];
        let mut convo = crate::convo::Convo::new(std::path::PathBuf::from("/work"), false);
        convo.compact = Some(crate::convo::Compact::Running { manual: false, since: std::time::Instant::now() - std::time::Duration::from_secs(72) });
        model.convos.insert(2, convo);
        let state = TeamState::of(&model, &model.members[1]);
        assert_eq!(state, TeamState::Compacting(72));
        assert_eq!(state.words(), "대화 요약 중 · 1분 12초");
        assert!(!state.waits());
        // Done: back to what it was.
        model.convos.get_mut(&2).unwrap().compact = Some(crate::convo::Compact::Done { at: std::time::Instant::now() });
        assert_eq!(TeamState::of(&model, &model.members[1]), TeamState::Working(Some(12)));
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
