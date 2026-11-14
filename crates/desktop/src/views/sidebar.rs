//! The sidebar, after Claude's: a strip by the traffic lights (hide ⌘\,
//! search ⌘K, back and forward), then a few quiet rows — New message ⌘N,
//! PM, Inbox (a badge), In progress (a count), All work — and the live work as
//! Claude lists sessions: under a header per project, one line each for what
//! a member is at now (a spinner) and what waits on the person (an orange
//! dot), the title cut with an ellipsis, five at most and "+N more". A
//! project with nothing live is its header alone; its name switches the
//! project slice, its chevron folds it. Finished work is one line at the
//! list's foot. The team is pinned under the list, always in reach, then the
//! update when one is ready, the squad and machine, and settings. The edge
//! drags to resize. One left edge for marks, one for text.

use equipe_core::t;

use std::collections::HashMap;
use std::time::Instant;

use equipe_core::protocol::{Engine, SessionId, TicketId, TicketStatus};
use crate::convo::Sub;
use crate::tips::TipExt as _;
use gpui_kit::component::*;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::dot;
use crate::board::{HeadClick, Menu, SideGroup, ideas_view, recent_done_view, side_groups};
use crate::state::{Conn, Member, Model, Presence, Side, Status};
use crate::theme::{Palette, Sans, UI, key_face};
use crate::{Screen, Shell, motion};

/// Every count at a row's end sits in a box this wide, its figures centred:
/// two of them fit, and the filled badge, the quiet counts and the spinner
/// share one column down the sidebar whether a row says 1 or 10.
const COUNT_BOX: f32 = 22.;
/// The dot before a count, saying how it stands (`docs/DESIGN-SYSTEM.md`,
/// "Counts and what waits").
const SIGN_DOT: f32 = 6.;

/// The box a row's count (or spinner) sits in, at its end.
fn count_box() -> Div {
    div().flex_none().min_w(px(COUNT_BOX)).h(px(18.)).flex().items_center().justify_center()
}

/// A count as the rows under the total show it: a dot in its state's colour
/// (a graphic, read at 3:1), then the number in muted ink — colour for the
/// mark, never for the figures, so a column of rows that all wait stays calm
/// and the one filled badge above them stays the thing that stands out.
/// No dot: the number alone; no number: the dot alone, in the dots' column.
fn count_sign(dot_color: Option<Rgba>, n: Option<usize>, p: Palette) -> Div {
    h_flex()
        .flex_none()
        .h(px(18.))
        .gap(px(4.))
        .items_center()
        .text_size(px(12.))
        .text_color(p.muted)
        .child(div().size(px(SIGN_DOT)).flex_none().rounded_full().when_some(dot_color, |d, c| d.bg(c)))
        .child(count_box().children(n.map(|n| n.to_string())))
}

/// The one sign at a project header's end, the most pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadSign {
    /// This many things in it wait on the person: the wait dot and the number.
    Waits(usize),
    /// Work under way in it needs a look: the red dot.
    Risky,
    /// This many at work in it: the spinner.
    Working(usize),
    Quiet,
}

/// Waiting on the person, else work that needs a look, else at work.
pub fn head_sign(waiting: usize, risky: bool, working: usize) -> HeadSign {
    match (waiting, risky, working) {
        (n @ 1.., _, _) => HeadSign::Waits(n),
        (0, true, _) => HeadSign::Risky,
        (0, false, n @ 1..) => HeadSign::Working(n),
        _ => HeadSign::Quiet,
    }
}

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
pub const MARK_BOX: f32 = 16.;
/// A member's row in the team block, compact.
pub const TEAM_ROW: f32 = 28.;
/// The team block shows this many rows; past that it scrolls inside.
pub const TEAM_SHOWN: usize = 6;
/// A subagent's row under its member, shorter.
pub const SUB_ROW: f32 = 22.;
/// A member shows this many of its subagents as rows, then "+N more".
pub const SUBS_SHOWN: usize = 3;
/// The subagents' rows always have room for this many, however many members.
const SUB_ROOM: usize = 6;

/// A project taken off the squad, while "Undo" can put it back.
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
    /// Every subagent of a member at work, as "+N more" opens them.
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
        .child(icon_button(p, "back", IconName::ArrowLeft, ui.nav.can_back(), Some(t!("sidebar.back").into())).on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))))
        .child(icon_button(p, "forward", IconName::ArrowRight, ui.nav.can_forward(), Some(t!("sidebar.forward").into())).on_click(cx.listener(|this, _, window, cx| this.go_forward(window, cx))))
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let ui = &shell.board;
    let dim = matches!(model.conn, Conn::Down { had_snapshot: false, .. });
    let on_board = shell.screen == Screen::Board;
    let menu = ui.view.menu;
    // A conversation being started shows as the first of the Conversations, selected.
    let new_on = false;
    let needs = model.needs_you_count();
    // The count's color: attention when something waiting holds up urgent or high work.
    let needs_hot = needs > 0 && model.needs_pressing();
    let working = model.tickets.iter().filter(|t| t.status == TicketStatus::InProgress).count();
    let risky = crate::board::progress(model, model.unix).risks.len();
    let progress_on = shell.screen == Screen::Progress;

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

    // The squad's total waiting on the person: the sidebar's one filled
    // badge. Every other count is a quiet number, a dot before it saying how
    // it stands (`count_sign`).
    // A row's mark: an icon, or the PM's own (◆ in its colour), as the header and the composer show it.
    let glyph = |name: IconName, on: bool| icon(name, 16., if on { p.text } else { p.text2 }).into_any_element();
    let nav = |id: &'static str, mark: AnyElement, label: String, badge: usize, count: usize, risky: bool, on: bool| {
        row_shell(p, id, on)
            .h(px(32.))
            .role(Role::Tab)
            .aria_selected(on)
            .aria_label(match badge.max(count) {
                0 => label.clone(),
                n => format!("{label}, {n}"),
            })
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(mark))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else { p.text2 }).child(label))
            .when(badge > 0, |r| r.child(count_box().rounded(px(9.)).px(px(5.)).bg(if id == "nav-inbox" && needs_hot { p.red } else { p.orange }).text_color(p.on_accent).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(badge.to_string())))
            // A red dot once work under way needs a look, where a project's wait dot sits.
            .when(count > 0 || risky, |r| r.child(count_sign(risky.then(|| p.danger()), (count > 0).then_some(count), p)))
    };
    let nav_rows = v_flex()
        .gap(px(1.))
        .child(
            nav("nav-new", glyph(IconName::Plus, new_on), t!("sidebar.new_conversation").into(), 0, 0, false, new_on)
                .child(div().flex_none().text_size(px(11.)).text_color(p.muted).font_features(key_face()).child("⌘N"))
                .on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx))),
        )
        .child(nav("nav-inbox", glyph(IconName::Inbox, on_board && menu == Menu::Inbox), t!("board.menu_inbox").into(), needs, 0, false, on_board && menu == Menu::Inbox)
            // What the badge counts: items, not people (the Team heading counts members).
            .when(needs > 0, |r| r.tip_at(p, format!("{}, {needs}\n{}", t!("board.menu_inbox"), t!("sidebar.needs_tip", n = needs)), None, gpui_kit::base::Placement::Right))
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }), window, cx) {
                this.open_side_menu(Menu::Inbox, cx);
            }
        })))
        // How the squad's work goes, at a glance: its dot says so (green, amber, red). Hidden
        // while the daemon is one that predates the request.
        .children(shell.status.offered().then(|| {
            let on = shell.screen == Screen::Status;
            let color = shell.status.mood(&model.squad).map_or(p.muted, |m| super::status::mood_color(p, m));
            let mark = div().size(px(9.)).flex_none().rounded_full().bg(color).into_any_element();
            let tip = shell.status.of(&model.squad).map(crate::status::headline);
            nav("nav-status", mark, t!("status.title").into(), 0, 0, false, on)
                .when_some(tip, |r, tip| r.tip_at(p, tip, None, gpui_kit::base::Placement::Right))
                .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                    if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Status, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                        this.open_status(cx);
                    }
                }))
        }))
        .child(
            nav("nav-working", glyph(IconName::LoaderCircle, progress_on), t!("progress.title").into(), 0, working, risky > 0, progress_on)
                .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                    if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Progress, menu: this.board.view.menu, member: None, ticket: None }), window, cx) {
                        this.open_progress(cx);
                    }
                })),
        )
        // The squad's other screens, named, behind "More".
        .children(more_rows(shell, cx));

    // The person's conversations with the PM, newest first, right under the
    // nav; then the projects — a project's own conversations are on its page.
    let groups: Vec<SideGroup> = side_groups(model);
    let projects: Vec<&SideGroup> = groups.iter().filter(|g| g.name.is_some()).collect();
    let mut list: Vec<AnyElement> = conversation_list(shell, cx);
    // The projects under a heading of their own, with the button that adds one.
    let folded = shell.remembered.projects_folded && shell.screen != Screen::Project;
    if !projects.is_empty() || !model.projects.is_empty() {
        list.push(projects_heading(shell, folded, cx));
    }
    // Every project with something in it; the quiet ones past a few behind "+N more".
    let talked: Vec<bool> = projects.iter().map(|g| !g.is_quiet() || g.name.as_ref().is_some_and(|n| model.conversations().iter().any(|l| l.project.as_deref() == Some(n.as_str())))).collect();
    let (shown_groups, hidden) = shown_projects(&talked, shell.board.side_projects_all, PROJECTS_SHOWN);
    if !folded {
        for (ix, g) in shown_groups.iter().map(|&i| projects[i]).enumerate() {
            list.push(project_head(shell, g, if ix == 0 { 2. } else { 1. }, cx));
        }
        if hidden > 0 || shell.board.side_projects_all {
            list.push(projects_more_row(shell, hidden, cx));
        }
    }
    list.extend(removed_row(shell, cx));

    // One column, one scroll: the nav and projects, then the plans' usage and
    // the team, each only as tall as its own content — nothing reserves room
    // it does not fill, nothing is clipped behind another area.
    let usage = super::switching::meters(shell, cx).map(|meters| section(p, meters));
    let team = team_block(shell, cx).map(|team| section(p, team));
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
        .children(usage)
        .children(team);

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
        .children(relaunch(p, shell, cx))
        .child(footer(p, shell, cx))
        .children(popup(p, shell, cx))
        .children(project_menu(shell, cx))
        .children(member_menu(shell, cx))
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

/// An icon from the kit, or one of equipe's own.
enum Glyph {
    Kit(IconName),
    Own(crate::icons::Icon),
}

/// Whether one of the screens behind "More" is on: then they show, so the
/// person sees where they are.
fn on_more(shell: &Shell) -> bool {
    let menu = shell.board.view.menu;
    matches!(shell.screen, Screen::Orchestration | Screen::Roadmap | Screen::Metrics | Screen::Design | Screen::Docs | Screen::Logins)
        || (shell.screen == Screen::Board && matches!(menu, Menu::All | Menu::View(_)))
}

/// The squad's other screens — every ticket, Status, Roadmap, Metrics, briefs and plans, Design, what
/// was finished, the ideas kept — named, one row each, behind "More".
fn more_rows(shell: &Shell, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let open = shell.remembered.nav_more || on_more(shell);
    let words = t!("sidebar.more_screens");
    let mut rows = vec![
        row_shell(p, "nav-more", false)
            .h(px(32.))
            .aria_expanded(open)
            .aria_label(words)
            .on_click(cx.listener(|this, _, _, cx| {
                this.remembered.nav_more = !(this.remembered.nav_more || on_more(this));
                this.remembered.save(&equipe_providers::credentials::equipe_home());
                cx.notify();
            }))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 14., p.muted)))
            .child(div().flex_1().min_w_0().truncate().text_color(p.muted).child(words))
            .into_any_element(),
    ];
    if !open {
        return rows;
    }
    let menu = shell.board.view.menu;
    let on_all = shell.screen == Screen::Board && matches!(menu, Menu::All);
    let tool = |id: &'static str, name: Glyph, label: String, on: bool| {
        row_shell(p, id, on)
            .pl(px(8. + CHILD_INDENT))
            .role(Role::Tab)
            .aria_selected(on)
            .aria_label(label.clone())
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(match name {
                Glyph::Kit(name) => icon(name, 14., if on { p.text } else { p.text2 }).into_any_element(),
                Glyph::Own(name) => crate::icons::icon(name, 14., if on { p.text } else { p.text2 }).into_any_element(),
            }))
            .child(row_title(p, label, on, false))
    };
    let place = |screen: Screen, menu: Menu| Place { screen, menu, member: None, ticket: None };
    // Who does what and waits on whom, beside the work's progress.
    rows.push(
        tool("nav-orchestra", Glyph::Kit(IconName::Network), t!("orchestra.title").into(), shell.screen == Screen::Orchestration)
            .child(div().flex_none().text_size(px(11.)).text_color(p.muted).font_features(key_face()).child("⌘⇧L"))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Orchestration, this.board.view.menu)), window, cx) {
                    this.open_orchestra(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-all", Glyph::Kit(IconName::LayoutDashboard), t!("board.menu_all").into(), on_all)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Board, Menu::All)), window, cx) {
                    this.open_side_menu(Menu::All, cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-roadmap", Glyph::Kit(IconName::Map), t!("roadmap.title").into(), shell.screen == Screen::Roadmap)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Roadmap, this.board.view.menu)), window, cx) {
                    this.open_roadmap(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-metrics", Glyph::Own(crate::icons::Icon::Chart), t!("metrics.title").into(), shell.screen == Screen::Metrics)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Metrics, this.board.view.menu)), window, cx) {
                    this.open_metrics(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-docs", Glyph::Own(crate::icons::Icon::FileText), t!("docs.title").into(), shell.screen == Screen::Docs)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Docs, this.board.view.menu)), window, cx) {
                    this.open_docs(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-design", Glyph::Kit(IconName::Palette), t!("design.title").into(), shell.screen == Screen::Design)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Design, this.board.view.menu)), window, cx) {
                    this.open_design(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(
        tool("nav-logins", Glyph::Own(crate::icons::Icon::Lock), t!("loginspage.title").into(), shell.screen == Screen::Logins)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if !this.open_elsewhere(e, super::elsewhere::Goal::Place(place(Screen::Logins, this.board.view.menu)), window, cx) {
                    this.open_logins(cx);
                }
            }))
            .into_any_element(),
    );
    rows.push(done_row(shell, cx));
    rows.extend(ideas_row(shell, cx));
    rows
}

/// A section's heading frame: the same muted line over every list.
fn heading_frame(p: Palette, id: &'static str) -> Stateful<Div> {
    h_flex().id(id).group(id).relative().h(px(26.)).mt(px(14.)).mx_2().pl_2().pr_1().items_center().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.muted)
}

/// "Projects", over the squad's projects, folding them on a click, and its
/// "+" (the same as the project list's "Add a project…").
fn projects_heading(shell: &Shell, folded: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let local = !shell.endpoint.is_remote();
    let words = t!("sidebar.projects");
    heading_frame(p, "projects-heading")
        .child(
            h_flex()
                .id("projects-fold")
                .flex_1()
                .min_w_0()
                .h_full()
                .gap(px(4.))
                .items_center()
                .cursor_pointer()
                .role(Role::Button)
                .aria_expanded(!folded)
                .aria_label(words)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.remembered.projects_folded = !(this.remembered.projects_folded && this.screen != Screen::Project);
                    this.remembered.save(&equipe_providers::credentials::equipe_home());
                    cx.notify();
                }))
                .child(div().child(words))
                .child(div().when(!folded, |d| d.invisible().group_hover("projects-heading", |s| s.visible())).child(icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 12., p.muted))),
        )
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
                        // Another machine's squad: its path, typed in a box
                        // that opens right here — the header's project pill
                        // isn't on screen outside a project's conversation.
                        this.board.popover = Some(super::board::Popover::Projects);
                        this.board.slice_place = "sidebar";
                        this.start_adding_project(window, cx);
                    }
                })),
        )
        .children((shell.board.popover == Some(super::board::Popover::Projects) && shell.board.slice_place == "sidebar").then(|| div().absolute().top_0().left_0().w_full().child(super::header::list(shell, cx))))
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
/// live in it — one sign (`head_sign`): what waits on the person, the wait
/// dot and its number in ink; else a red dot; else a spinner at work.
/// On the pointer the counts give way to "+" (a new conversation here) and
/// "⋯" (its menu, as a right-click opens it).
fn project_head(shell: &Shell, g: &SideGroup, gap: f32, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = g.key();
    let group = SharedString::from(format!("head-{key}"));
    // Its page open now.
    let focused = g.name.is_some() && shell.screen == Screen::Project && shell.board.project_page == g.name;
    let label = g.name.clone().unwrap_or_default();
    let name_project = g.name.clone();
    let name_tip = t!("sidebar.open_project");
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
        Some(_) => crate::theme::project_mark(p, 13.),
        // Outside every project: a ring, where a project has its colour.
        None => div().size(px(8.)).rounded_full().border_1().border_color(p.muted).into_any_element(),
    };
    let name_color = if focused { p.text } else { p.text2 };
    // Waiting on the person: its share of Needs you, so the rows add up to it.
    let waiting = shell.model.needs_by_project().get(&g.name).copied().unwrap_or(0);
    let risky = crate::board::progress(&shell.model, shell.model.unix).risks.iter().any(|(id, _)| shell.model.tickets.iter().find(|t| t.id == *id).is_some_and(|t| shell.model.ticket_project_name(t) == g.name));
    let spin = shell.spin(cx);
    let sign_id = SharedString::from(format!("sign-{key}"));
    let sign = match head_sign(waiting, risky, g.working) {
        HeadSign::Waits(n) => Some(count_sign(Some(p.orange), Some(n), p).id(sign_id).tip(p, t!("sidebar.work_needs_you", n = n), None)),
        HeadSign::Risky => Some(count_sign(Some(p.danger()), None, p).id(sign_id).tip(p, t!("progress.risks"), None)),
        HeadSign::Working(n) => Some(count_box().id(sign_id).tip(p, t!("sidebar.work_working", n = n), None).child(icon(IconName::LoaderCircle, 13., p.muted).rotate(radians(spin)))),
        HeadSign::Quiet => None,
    };
    let meta = h_flex().flex_none().items_center().group_hover(group.clone(), |s| s.invisible()).children(sign);
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
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(ident))
        .child(div().flex_1().min_w_0().truncate().text_color(name_color).font_weight(if waiting > 0 || focused { FontWeight::MEDIUM } else { FontWeight::NORMAL }).child(label))
        .when(hq, |r| r.child(hq_badge(p)))
        .child(meta)
        .children(actions);
    row.into_any_element()
}

/// The conversations shown at once under "Conversations", the newest first;
/// the rest behind "Show all (N)".
const CONVERSATIONS_SHOWN: usize = 10;

/// The longest a conversation's project label grows in its row.
const LABEL_MAX: f32 = 84.;

/// The "Conversations" section: every conversation the person opened with the
/// PM, the most lately used first and in the days they were last used (Today,
/// Yesterday, Previous 7 days, Older) — one being started on top, selected —
/// each naming its project when it has one. Under the heading, once equipe has
/// put old ones away that day, a quiet note says how many. "Show all" opens
/// the rest, and a filter over their titles. No conversation, no section.
fn conversation_list(shell: &Shell, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let model = &shell.model;
    let talks = model.conversations();
    let away_today = crate::talk_days::put_away_today(&model.put_away, model.unix, crate::state::local_midnight);
    if talks.is_empty() && model.draft_talk.is_none() && model.put_away.is_empty() {
        return Vec::new();
    }
    let mut rows = vec![heading_frame(p, "talks-heading").child(div().flex_1().child(t!("sidebar.conversations"))).into_any_element()];
    if away_today > 0 {
        rows.extend(tidied_note(shell, away_today, true, cx));
    }
    if let Some((draft, about)) = &model.draft_talk {
        let on = shell.screen == Screen::Chat;
        let title: String = t!("sidebar.new_conversation").into();
        rows.push(
            row_shell(p, ("conv-draft", *draft as usize), on)
                .aria_label(title.clone())
                .aria_selected(on)
                .child(div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none())
                .child(row_title(p, title, on, true))
                .children(project_label(p, about.as_deref()))
                .into_any_element(),
        );
    }
    let all = shell.board.side_talks_all.contains("");
    let long = talks.len() > CONVERSATIONS_SHOWN;
    // The full list can be filtered by title; what is typed counts only while it is open.
    let typed = if all && long { shell.board.talks_filter.read(cx).value().to_string() } else { String::new() };
    let filtering = !typed.trim().is_empty();
    let untitled: String = t!("sidebar.new_conversation").into();
    let pool = if filtering { crate::talk_days::matching(&talks, &typed, &untitled) } else { talks.clone() };
    let shown = if all { pool.len() } else { pool.len().min(CONVERSATIONS_SHOWN) };
    if all && long {
        rows.push(
            h_flex()
                .mx_2()
                .mb_1()
                .px_2()
                .h(px(26.))
                .items_center()
                .rounded(px(6.))
                .bg(p.side_hover)
                .text_size(px(12.))
                .child(Input::new(&shell.board.talks_filter).appearance(false).bordered(false))
                .into_any_element(),
        );
    }
    for (day, range) in crate::talk_days::groups(&pool, shown, model.unix, crate::state::local_midnight) {
        rows.push(div().mx_2().px_2().pt(px(6.)).pb(px(2.)).text_size(px(11.)).text_color(p.muted).child(crate::i18n::catalog().get(day.label_key()).to_owned()).into_any_element());
        for lane in &pool[range] {
            rows.push(conversation_row(shell, lane, cx));
        }
    }
    if filtering && pool.is_empty() {
        rows.push(div().mx_2().px_2().py_1().text_size(px(12.)).text_color(p.muted).child(t!("sidebar.no_conversation_match")).into_any_element());
    }
    if long {
        let words = if all { t!("sidebar.fewer").to_owned() } else { t!("sidebar.show_all", n = talks.len()) };
        rows.push(
            row_shell(p, "talks-more", false)
                .h(px(26.))
                .aria_label(words.clone())
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.board.side_talks_all.remove("") {
                        this.board.side_talks_all.insert(String::new());
                    }
                    cx.notify();
                }))
                .child(div().w(px(MARK_BOX)).flex_none())
                .child(div().text_size(px(12.)).text_color(p.muted).child(words))
                .into_any_element(),
        );
    }
    // Put away on another day: still one click away, at the list's end, so
    // nothing equipe tidied is ever out of reach.
    if away_today == 0 && !model.put_away.is_empty() {
        rows.extend(tidied_note(shell, model.put_away.len(), false, cx));
    }
    rows
}

/// "N older conversations put away · Show": one quiet line when equipe put
/// old conversations away today (`today`), or "N put-away conversations ·
/// Show" at the list's end otherwise. "Show" lists them (newest first) right
/// under it, each taken up again with a click.
fn tidied_note(shell: &Shell, n: usize, today: bool, cx: &mut Context<Shell>) -> Vec<AnyElement> {
    let p = shell.palette;
    let open = shell.board.side_put_away_open;
    let said = match (today, n) {
        (true, 1) => t!("sidebar.tidied_one").to_owned(),
        (true, _) => t!("sidebar.tidied_many", n = n),
        (false, 1) => t!("sidebar.put_away_one").to_owned(),
        (false, _) => t!("sidebar.put_away_many", n = n),
    };
    let toggle = if open { t!("sidebar.tidied_hide") } else { t!("sidebar.tidied_show") };
    let mut rows = vec![
        h_flex()
            .id("talks-tidied")
            .mx_2()
            .px_2()
            .h(px(24.))
            .gap(px(4.))
            .items_center()
            .text_size(px(12.))
            .text_color(p.muted)
            .aria_label(said.clone())
            .child(div().flex_none().child(format!("{said} ·")))
            .child(
                div()
                    .id("talks-tidied-show")
                    .flex_none()
                    .cursor_pointer()
                    .text_color(p.text2)
                    .hover(move |s| s.text_color(p.text))
                    .role(Role::Button)
                    .aria_label(toggle.to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.board.side_put_away_open = !this.board.side_put_away_open;
                        cx.notify();
                    }))
                    .child(toggle.to_string()),
            )
            .into_any_element(),
    ];
    if open {
        for c in shell.model.put_away.iter().take(PUT_AWAY_ROWS) {
            let id = c.session;
            let title = c.title.clone().unwrap_or_else(|| t!("sidebar.new_conversation").into());
            rows.push(
                row_shell(p, ("conv-away-row", id as usize), false)
                    .h(px(26.))
                    .aria_label(title.clone())
                    .tip(p, t!("project.reopen_tip"), None)
                    .on_click(cx.listener(move |this, _, _, cx| this.reopen_conversation(id, cx)))
                    .child(div().w(px(MARK_BOX)).flex_none())
                    .child(row_title(p, title, false, true))
                    .children(project_label(p, c.project.as_deref()))
                    .into_any_element(),
            );
        }
    }
    rows
}

/// The conversations put away listed under the note at most (as many as a
/// snapshot carries).
const PUT_AWAY_ROWS: usize = 50;

/// The project a conversation is about, small and muted at its row's end.
fn project_label(p: Palette, project: Option<&str>) -> Option<Div> {
    project.map(|name| div().flex_none().max_w(px(LABEL_MAX)).truncate().text_size(px(11.)).text_color(p.muted).child(name.to_owned()))
}

/// A conversation with the PM: a spinner while it answers, its first words,
/// its project at the end, and on the pointer the way to rename it and put it
/// away. ⌘-click: in a window of its own.
fn conversation_row(shell: &Shell, lane: &crate::state::Lane, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = lane.session;
    let on = shell.screen == Screen::Chat && shell.model.talk == Some(id) && shell.model.draft_talk.is_none();
    let group = SharedString::from(format!("conv-{id}"));
    // A spinner while the PM answers, else an empty mark: every title starts at the nav's x.
    let mark = if lane.presence == crate::state::Presence::Working { mark_element(p, Mark::Spinner, shell.spin(cx)) } else { div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none().into_any_element() };
    let pinned = lane.pinned;
    let title = lane.title.clone().unwrap_or_else(|| t!("sidebar.new_conversation").into());
    let label = match &lane.project {
        Some(project) => format!("{title}, {project}"),
        None => title.clone(),
    };
    // On the pointer, over the row's end: rename (it opens the conversation, and the card under its title) and put away.
    let actions = h_flex()
        .absolute()
        .right(px(4.))
        .top_0()
        .bottom_0()
        .items_center()
        .invisible()
        .group_hover(group.clone(), |s| s.visible())
        .child(
            div()
                .id(("conv-pin", id as usize))
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(p.side_hover)
                .hover(move |s| s.bg(p.side_sel))
                .role(Role::Button)
                .aria_label(if lane.pinned { t!("sidebar.unpin") } else { t!("sidebar.pin") })
                .tip(p, if lane.pinned { t!("sidebar.unpin") } else { t!("sidebar.pin") }, None)
                .child(crate::icons::icon(crate::icons::Icon::Pin, 12., if lane.pinned { p.text } else { p.muted }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.pin_conversation(id, !pinned, cx);
                })),
        )
        .child(
            div()
                .id(("conv-rename", id as usize))
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(p.side_hover)
                .hover(move |s| s.bg(p.side_sel))
                .role(Role::Button)
                .aria_label(t!("talk.rename"))
                .tip(p, t!("talk.rename"), None)
                .child(crate::icons::icon(crate::icons::Icon::Pencil, 12., p.muted))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_side(Side::Conversation(id), window, cx);
                    this.open_rename(id, None, window, cx);
                })),
        )
        .child(
            div()
                .id(("conv-away", id as usize))
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(p.side_hover)
                .hover(move |s| s.bg(p.side_sel))
                .role(Role::Button)
                .aria_label(t!("sidebar.put_away"))
                .tip(p, t!("sidebar.put_away"), None)
                .child(crate::icons::icon(crate::icons::Icon::X, 12., p.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.put_away_conversation(id, cx);
                })),
        );
    row_shell(p, ("conv", id as usize), on)
        .aria_label(label)
        .aria_selected(on)
        .group(group.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Conversation(id), window, cx) {
                this.open_side(Side::Conversation(id), window, cx);
            }
        }))
        .child(mark)
        .child(row_title(p, title, on, false))
        .children(project_label(p, lane.project.as_deref()).map(|label| label.group_hover(group.clone(), |s| s.invisible())))
        .children(pinned.then(|| div().flex_none().group_hover(group, |s| s.invisible()).child(crate::icons::icon(crate::icons::Icon::Pin, 11., p.muted))))
        .child(actions)
        .into_any_element()
}

/// "+N more" under the projects shown, or "Show fewer" once all are.
fn projects_more_row(shell: &Shell, hidden: usize, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let words = if shell.board.side_projects_all { t!("sidebar.fewer").to_owned() } else { t!("sidebar.more", n = hidden) };
    row_shell(p, "projects-more", false)
        .h(px(26.))
        .aria_label(words.clone())
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
        option: None,
    }
}

/// Every project with something in it stays; past `cap` the quiet ones go
/// behind "+N more", unless all are asked for. The indices shown, in order,
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

/// Projects shown before "+N more", the busy ones always.
const PROJECTS_SHOWN: usize = 6;
/// How far a group's conversations sit in from its header: each mark under
/// the group's own, each title under its name.
const CHILD_INDENT: f32 = MARK_BOX + 8.;

/// How long "Removed · Undo" stays.
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

/// The list's last line: "Recently done · N today ›" — what finished today, local
/// time — or, nothing yet today, "Recently done ›", muted. It opens All work on the
/// finished.
fn done_row(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let today = shell.model.done_today();
    let on = shell.screen == Screen::Board && shell.board.view.menu == Menu::View(recent_done_view());
    let label = if today > 0 { t!("sidebar.recent_done_today", n = today) } else { t!("sidebar.recent_done").to_owned() };
    row_shell(p, "recent-done", on)
        .pl(px(8. + CHILD_INDENT))
        .aria_label(label.clone())
        .aria_selected(on)
        .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::View(recent_done_view()), cx)))
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(IconName::Check, 14., if today > 0 { p.text2 } else { p.muted })))
        .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else if today > 0 { p.text2 } else { p.muted }).child(label))
        .into_any_element()
}

/// The person's ideas, kept for later, under what was finished: how many,
/// which opens them. None kept: no row.
fn ideas_row(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let kept = shell.model.tickets.iter().filter(|t| t.status == TicketStatus::Idea && shell.model.in_focus(t)).count();
    if kept == 0 {
        return None;
    }
    let on = shell.screen == Screen::Board && shell.board.view.menu == Menu::View(ideas_view());
    let label = t!("sidebar.ideas", n = kept);
    Some(
        row_shell(p, "ideas", on)
            .pl(px(8. + CHILD_INDENT))
            .on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::View(ideas_view()), cx)))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(crate::icons::icon(crate::icons::Icon::Lightbulb, 14., p.text2)))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else { p.text2 }).child(label))
            .into_any_element(),
    )
}

/// The team, pinned above the foot so it never scrolls away: a compact row
/// each — its mark, its name, a few words of where it stands — six in sight,
/// the rest scrolling inside. A click opens its conversation. Under a member
/// at work, the subagents it handed work to, a short row each, easing in as
/// they start and out as they hand back (`SubMotion`); three at most, then
/// "+N more" — fewer when the block would outgrow its height, the members'
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
    // Each row's engine only where it tells something; shared by all, once on the heading.
    let (engine_shown, shared_engine) = engine_marks(&members.iter().map(|m| m.engine).collect::<Vec<_>>());
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut labelled = false;
    for ((m, subs), show_engine) in members.iter().zip(&subs).zip(engine_shown) {
        let rests = model.is_resting(m);
        if rests && !labelled {
            labelled = true;
            rows.push(resting_label(p));
        }
        let folded = (cap == 0).then_some(subs.len()).filter(|n| *n > 0);
        rows.push(member_row(shell, m, folded, show_engine, rests, cx));
        // Its tickets, each run under it, when it runs two or more: one row
        // each, opened like the ticket. A single run is its member's row.
        let runs = model.runs_of(m.session);
        let runs_open = shell.board.side_talks_all.contains(&runs_key(m.session));
        if runs.len() > 1 {
            for run in runs.iter().take(if runs_open { runs.len() } else { RUNS_SHOWN }) {
                rows.push(run_row(shell, run, cx));
            }
        }
        if runs.len() > RUNS_SHOWN {
            rows.push(runs_more_row(shell, m.session, runs.len() - RUNS_SHOWN, runs_open, cx));
        }
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
    // As tall as its rows: the sidebar's one scroll holds them.
    let list = v_flex().id("team-list").children(rows);
    // Folded, the usual: one row of faces, each with how it stands; a click
    // on the heading shows the rows.
    let open = shell.remembered.team_open;
    let turns = members.iter().filter(|m| TeamState::of(model, m).waits()).count();
    let heading_tip = shared_engine.map(|e| t!("sidebar.team_engine", engine = equipe_core::controls::engine_name(e)));
    let heading = h_flex()
        .id("team-heading")
        .h(px(24.))
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .aria_expanded(open)
        .aria_label(t!("sidebar.team"))
        .on_click(cx.listener(|this, _, _, cx| {
            this.remembered.team_open = !this.remembered.team_open;
            this.remembered.save(&equipe_providers::credentials::equipe_home());
            cx.notify();
        }))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 12., p.muted)))
        .when_some(heading_tip, |h, tip| h.tip(p, tip, None))
        .child(t!("sidebar.team"))
        .child(div().font_weight(FontWeight::NORMAL).child(members.len().to_string()))
        .child(div().flex_1())
        // How many wait on the person: the wait dot, the words in the heading's
        // ink. A click opens Needs you — everything waiting on them, each
        // with its ticket, who asks, what and how long.
        .when(turns > 0, |h| {
            h.child(
                h_flex()
                    .id("team-turns")
                    .gap(px(5.))
                    .items_center()
                    // The padding (for the hover background) hangs outside the
                    // heading's own, so the words end at the rows' right edge.
                    .px(px(4.))
                    .mr(px(-4.))
                    .rounded(px(4.))
                    .font_weight(FontWeight::NORMAL)
                    .hover(move |s| s.bg(p.side_sel))
                    // Members, where Needs you counts items: the tip says so.
                    .tip(p, format!("{}\n{}", t!("sidebar.team_turns_open", n = turns), t!("sidebar.team_turns_tip", n = turns)), None)
                    .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if !this.open_elsewhere(e, super::elsewhere::Goal::Place(Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }), window, cx) {
                            this.open_side_menu(Menu::Inbox, cx);
                        }
                    }))
                    .child(dot(p.orange, SIGN_DOT))
                    .child(t!("sidebar.team_turns", n = turns)),
            )
        });
    let body = if open { list.into_any_element() } else { faces(shell, &members, cx) };
    Some(
        v_flex()
            .flex_none()
            .pt(px(6.))
            .pb_1()
            .child(heading)
            .child(body)
            .into_any_element(),
    )
}

/// Someone on the team, in a row: their avatar, as the folded row and every
/// card shows them, its dot saying how they stand — orange waiting on the
/// person, the accent at work — dimmed once they left.
fn face(p: Palette, member: &Member, mark: Mark) -> AnyElement {
    let dot = match mark {
        Mark::Waits => Some(p.orange),
        Mark::Spinner => Some(p.accent),
        _ => None,
    };
    div()
        .relative()
        .size(px(MARK_BOX + 2.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(mark == Mark::Hollow, |d| d.opacity(0.5))
        .child(crate::theme::avatar(p, &member.role, &member.name, 16.))
        .children(dot.map(|c| div().absolute().right(px(-2.)).bottom(px(-2.)).size(px(7.)).rounded_full().border_1().border_color(p.side).bg(c)))
        .into_any_element()
}

/// The team folded: a face each, its dot saying how it stands — orange
/// waiting on the person, the accent at work — its name on the pointer.
fn faces(shell: &Shell, members: &[&Member], cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let row = members.iter().map(|m| {
        let state = TeamState::of(&shell.model, m);
        let session = m.session;
        let dot = if state.waits() {
            Some(p.orange)
        } else if matches!(state, TeamState::Working(_) | TeamState::Compacting(_)) {
            Some(p.accent)
        } else {
            None
        };
        div()
            .id(("face", session as usize))
            .relative()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(p.side_hover))
            .aria_label(format!("{}, {}", m.name, state.words()))
            .tip(p, format!("{} · {}", m.name, state.words()), None)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                let goal = super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: Some(session), ticket: None });
                if !this.open_elsewhere(e, goal, window, cx) {
                    this.select_member(session, window, cx);
                }
            }))
            .when(shell.model.is_resting(m), |d| d.opacity(0.5))
            .child(crate::theme::avatar(p, &m.role, &m.name, 20.))
            .children(dot.map(|c| div().absolute().right(px(0.)).bottom(px(0.)).size(px(8.)).rounded_full().border_1().border_color(p.side).bg(c)))
    });
    h_flex().mx_2().px_2().pt_1().flex_wrap().gap(px(4.)).children(row).into_any_element()
}

/// One of the sidebar's lower areas (usage, the team) under a hairline of
/// its own, as tall as its content.
fn section(p: Palette, area: AnyElement) -> AnyElement {
    v_flex().flex_none().mt(px(10.)).border_t_1().border_color(p.hair).child(area).into_any_element()
}

/// The height the subagents' rows may take: what six members' rows leave,
/// and never less than a few rows'.
fn child_room(members: usize) -> f32 {
    (TEAM_SHOWN.saturating_sub(members) as f32 * TEAM_ROW).max(SUB_ROOM as f32 * SUB_ROW)
}

/// How many of a member's runs show as rows under it; the rest behind "+N more".
pub const RUNS_SHOWN: usize = 4;

/// How many rows `n` runs take: those shown, and "+N more" for the rest —
/// none for a single run, which its member's row says and opens.
#[cfg(test)]
pub fn run_rows(n: usize) -> usize {
    if n < 2 { 0 } else { n.min(RUNS_SHOWN) + usize::from(n > RUNS_SHOWN) }
}

/// "Resting", small and muted, over the members the person rested.
fn resting_label(p: Palette) -> AnyElement {
    h_flex()
        .h(px(SUB_ROW))
        .flex_none()
        .mx_2()
        .px_2()
        .pt(px(4.))
        .items_center()
        .text_size(px(11.))
        .text_color(p.muted)
        .child(t!("sidebar.resting"))
        .into_any_element()
}

/// One ticket a member runs, under the member: its mark (a spinner at
/// work, the wait dot when it asks the person), its number and title, and
/// where it stands. A click opens it as the ticket — its conversation,
/// steering, stop and requests all that run's.
fn run_row(shell: &Shell, run: &crate::state::Run, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let ticket = run.ticket;
    let title = model.tickets.iter().find(|t| t.id == ticket).map(|t| t.title.clone()).unwrap_or_default();
    let asks = model.approvals.first_for(run.session).is_some();
    let on = shell.screen == Screen::Chat && model.opened == Some(ticket);
    // A run equipe paused (an engine's limit, a retry, the squad on hold) is
    // waiting on purpose, not failed: say why and when it goes on.
    let pause = model.tickets.iter().find(|t| t.id == ticket).and_then(|t| t.execution.as_ref()).and_then(|e| e.pause.as_ref());
    let (mark, words) = if asks {
        (div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(dot(p.orange, SIGN_DOT)).into_any_element(), t!("state.section_permission").to_owned())
    } else if run.presence == Presence::Working {
        (sub_mark(p, shell.spin(cx)), t!("sidebar.working").to_owned())
    } else if let Some(pause) = pause {
        (div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(dot(p.muted, SIGN_DOT)).into_any_element(), crate::state::pause_words(pause, model.unix as i64))
    } else if run.failed {
        (div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(crate::icons::icon(crate::icons::Icon::Warning, 10., crate::render::tones(&p).warning)).into_any_element(), t!("sidebar.run_failed").to_owned())
    } else {
        (div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(dot(p.muted, SIGN_DOT)).into_any_element(), t!("sidebar.idle").to_owned())
    };
    let label = format!("T-{ticket} {title}");
    sub_shell(p, SharedString::from(format!("run-{}", run.session)))
        .cursor_pointer()
        .when(on, |r| r.bg(p.side_sel))
        .hover(move |s| s.bg(p.side_hover))
        .aria_label(format!("{label}, {words}"))
        .aria_selected(on)
        .tip_at(p, label.clone(), Some(&words), gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, _, window, cx| this.open_side(Side::Ticket(ticket), window, cx)))
        .child(mark)
        .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(format!("T-{ticket}")))
        .child(row_title(p, title, on, false).text_size(px(12.)))
        .child(div().flex_none().text_size(px(11.)).text_color(if asks { p.text2 } else { p.muted }).child(words))
        .into_any_element()
}

/// "+N more": the rest of a member's runs; a click opens the first of them.
/// The key a member's runs are kept open under, beside "Show all" of the
/// conversations (one set of what the sidebar has opened).
fn runs_key(member: SessionId) -> String {
    format!("runs-{member}")
}

/// "+N more" under a member's runs: a click shows the rest right there, and
/// "Show fewer" folds them again.
fn runs_more_row(shell: &Shell, member: SessionId, more: usize, open: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let words = if open { t!("sidebar.fewer").to_owned() } else { t!("sidebar.more", n = more) };
    sub_shell(p, SharedString::from(format!("runs-more-{member}")))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .role(Role::Button)
        .aria_expanded(open)
        .aria_label(words.clone())
        .on_click(cx.listener(move |this, _, _, cx| {
            let key = runs_key(member);
            if !this.board.side_talks_all.remove(&key) {
                this.board.side_talks_all.insert(key);
            }
            cx.notify();
        }))
        .child(div().w(px(12.)).flex_none())
        .child(div().text_size(px(11.5)).text_color(p.muted).child(words))
        .child(icon(if open { IconName::ChevronUp } else { IconName::ChevronDown }, 10., p.muted))
        .into_any_element()
}

/// How many rows `n` subagents take with `cap` shown: those, and "+N more"
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
    let step = sub_step(shell, sub, cx);
    let row = sub_shell(p, SharedString::from(format!("sub-{}", sub.id)))
        .cursor_pointer()
        .hover(move |s| s.bg(p.side_hover))
        .tip_at(p, sub.what.clone(), Some(&step), gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, _, window, cx| this.open_subagent(member, session, &id, window, cx)))
        .child(sub_mark_of(shell, sub, cx))
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

/// What a subagent's row says under it: that it looks stuck, else its latest step.
fn sub_step(shell: &Shell, sub: &Sub, cx: &mut Context<Shell>) -> String {
    match sub.stuck {
        Some(quiet) => t!("sidebar.subagent_stuck", time = shell.counting(quiet.elapsed(), cx)),
        None => sub.step.clone().unwrap_or_else(|| t!("sidebar.subagent_starting").to_owned()),
    }
}

/// A subagent's mark: its spinner — amber warning once it looks stuck.
fn sub_mark_of(shell: &Shell, sub: &Sub, cx: &mut Context<Shell>) -> AnyElement {
    match sub.stuck {
        Some(_) => {
            let warning = crate::render::tones(&shell.palette).warning;
            div().w(px(12.)).h(px(12.)).flex_none().flex().items_center().justify_center().child(crate::icons::icon(crate::icons::Icon::Warning, 10., warning)).into_any_element()
        }
        None => sub_mark(shell.palette, shell.spin(cx)),
    }
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

/// "+N more": the rest of a member's subagents, all of them in a popover.
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
    /// Running this many tickets at once (two or more): "3 tickets".
    Runs(usize),
    /// Its conversation is being summarized, for this many seconds.
    Compacting(u64),
    /// It asks the person's permission.
    Asks,
    /// Its ticket waits on the person: Your turn.
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
            Status::Working { .. } | Status::Ready { .. } if model.runs_of(member.session).len() > 1 => TeamState::Runs(model.runs_of(member.session).len()),
            Status::Working { ticket, .. } => TeamState::Working(ticket),
            Status::Ready { .. } => TeamState::Resting,
            Status::Closed => TeamState::Closed,
        }
    }

    /// "Working · T-12", "Your turn", "Idle".
    pub fn words(self) -> String {
        match self {
            TeamState::Working(Some(id)) => t!("sidebar.working_on", id = id),
            TeamState::Working(None) => t!("sidebar.working").to_owned(),
            TeamState::Runs(n) => t!("sidebar.runs", n = n),
            TeamState::Compacting(secs) => t!("compact.sidebar", elapsed = crate::convo::elapsed(std::time::Duration::from_secs(secs))),
            TeamState::Asks => t!("state.section_permission").to_owned(),
            TeamState::YourTurn => t!("sidebar.needs_you").to_owned(),
            TeamState::Resting => t!("sidebar.idle").to_owned(),
            TeamState::Closed => t!("sidebar.closed").to_owned(),
        }
    }

    /// Whether it waits on the person (its face's orange dot).
    pub fn waits(self) -> bool {
        matches!(self, TeamState::Asks | TeamState::YourTurn)
    }
}

/// What a member waiting on the person waits for, as its row says it: the
/// ticket, what is asked, and how long it has waited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub ticket: Option<TicketId>,
    /// What is asked, in full.
    pub what: String,
    /// How long it has waited, in seconds, when known.
    pub waited: Option<u64>,
}

/// How much of what is asked a team row shows before it is cut.
const WAIT_WORDS: usize = 28;

impl Wait {
    /// The row's few words: "T-584 · Create the store account", cut to fit.
    pub fn short(&self) -> String {
        let what = cut(&self.what, WAIT_WORDS);
        match self.ticket {
            Some(id) => format!("T-{id} · {what}"),
            None => what,
        }
    }

    /// Waiting past three days: worth asking whether it is still needed.
    pub fn stale(&self) -> bool {
        self.waited.is_some_and(|w| w >= equipe_core::heartbeat::STALE_WAIT_AFTER)
    }

    /// On the pointer, under the ticket: all that is asked, and how long it waited.
    pub fn detail(&self) -> String {
        let mut out = self.what.clone();
        if let Some(waited) = self.waited.filter(|w| *w > 0) {
            let line = if self.stale() { t!("sidebar.waited_stale", span = crate::state::short_span(waited)) } else { t!("sidebar.waited", span = crate::state::short_span(waited)) };
            out = format!("{out}\n{line}");
        }
        out
    }
}

/// `text` on one line, cut at a word near `most` characters with an ellipsis.
pub fn cut(text: &str, most: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    if line.chars().count() <= most {
        return line.to_owned();
    }
    let head: String = line.chars().take(most).collect();
    // Back to the last space, unless that leaves too little (a script without spaces).
    let at = head.rfind(' ').filter(|at| head[..*at].chars().count() >= most / 2).unwrap_or(head.len());
    format!("{}…", head[..at].trim_end())
}

/// What `member` waits on the person for, as [`Model::status`] reads it: a
/// request for permission (its own or one of its runs'), its questions, or
/// a ticket of its put down for the person. None when it waits for nothing.
pub fn wait_of(model: &Model, member: &Member) -> Option<Wait> {
    if !matches!(model.status(member), Status::NeedsYou(_)) {
        return None;
    }
    let waiting = model.tickets.iter().find(|t| t.status == TicketStatus::Waiting && t.owner.as_deref() == Some(member.name.as_str()));
    let asking = model.approvals.first_for(member.session).or_else(|| model.runs_of(member.session).into_iter().find_map(|r| model.approvals.first_for(r.session)));
    if let Some(a) = asking {
        let what = t!("approvals.allow_question", what = super::approvals::Card::of(a).noun);
        return Some(Wait { ticket: a.ticket.or(member.ticket).or(waiting.map(|t| t.id)), what, waited: None });
    }
    if let Some(set) = model.asked.from(&member.name) {
        let what = set.questions.first().map(|q| q.shown().0).unwrap_or_else(|| t!("state.section_decide").to_owned());
        let waited = (set.asked_at > 0).then(|| model.unix.saturating_sub(set.asked_at));
        return Some(Wait { ticket: set.ticket, what, waited });
    }
    let t = waiting?;
    let asks = t.asks();
    let what = if asks.is_empty() { t!("sidebar.needs_you_what").to_owned() } else { asks.iter().map(|a| equipe_core::protocol::links::words(a)).collect::<Vec<_>>().join("; ") };
    Some(Wait { ticket: Some(t.id), what, waited: model.in_state_for(t) })
}

/// The ticket of `member`'s that equipe paused while it has no run — an
/// engine's limit, a retry, the squad on hold — with why: what its row says
/// in place of "Idle". Tickets in progress first, then the lowest number.
pub fn paused_ticket<'a>(model: &'a Model, member: &Member) -> Option<(TicketId, &'a equipe_core::protocol::RunPause)> {
    model
        .tickets
        .iter()
        .filter(|t| t.owner.as_deref() == Some(member.name.as_str()))
        .filter_map(|t| t.execution.as_ref().and_then(|e| e.pause.as_ref()).map(|pause| (t, pause)))
        .min_by_key(|(t, _)| (t.status != TicketStatus::InProgress, t.id))
        .map(|(t, pause)| (t.id, pause))
}

/// Which members' rows show their engine: only where it tells something —
/// those on another engine than most of the team (all of them when it is a
/// tie). The engine everyone shares, when they all do, is said once, on the
/// team's heading instead.
pub fn engine_marks(engines: &[Option<Engine>]) -> (Vec<bool>, Option<Engine>) {
    let known: Vec<Engine> = engines.iter().flatten().copied().collect();
    let count = |e: Engine| known.iter().filter(|k| **k == e).count();
    let (claude, codex) = (count(Engine::Claude), count(Engine::Codex));
    let most = match claude.cmp(&codex) {
        std::cmp::Ordering::Greater => Some(Engine::Claude),
        std::cmp::Ordering::Less => Some(Engine::Codex),
        std::cmp::Ordering::Equal => None,
    };
    let shared = most.filter(|_| claude == 0 || codex == 0);
    let shown = engines.iter().map(|e| e.is_some_and(|e| Some(e) != most)).collect();
    (shown, shared)
}

/// Someone on the team, compact: a spinner at work, the orange dot when it
/// waits on the person, else its colour; the name; where it stands, muted —
/// what it waits for when that is the person ("T-584 · Create the store
/// account", all of it on the pointer) — its engine where that tells
/// something, and, on the pointer, "Talk". A click opens its conversation —
/// or, running a single ticket, that ticket, as its run's row would. Rested
/// by the person, it is dimmed. A right-click offers to rest it or wake it.
fn member_row(shell: &Shell, member: &Member, folded: Option<usize>, show_engine: bool, rests: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let (_, hue) = crate::theme::look(&member.role, &member.name);
    let side = if Model::is_pm(member) { Side::Pm } else { Side::Member(member.session) };
    let session = member.session;
    let runs = shell.model.runs_of(session);
    // A single run: the member's row is that ticket's.
    let single = (runs.len() == 1).then(|| runs[0].ticket);
    // The PM's conversation lights the PM row above; here only the others'.
    let on = match single {
        Some(ticket) => side != Side::Pm && shell.screen == Screen::Chat && shell.model.opened == Some(ticket),
        None => side != Side::Pm && shell.screen == Screen::Chat && shell.model.opened.is_none() && shell.model.side_open() == Some(side),
    };
    let state = TeamState::of(&shell.model, member);
    // Its seconds count up: drawn again when the next one is.
    if matches!(state, TeamState::Compacting(_)) {
        shell.wake_at(std::time::Instant::now() + std::time::Duration::from_secs(1), cx);
    }
    let at_work = runs.iter().any(|r| r.presence == Presence::Working) || member.presence == Presence::Working;
    let paused = single.and_then(|id| shell.model.tickets.iter().find(|t| t.id == id)).and_then(|t| t.execution.as_ref()).and_then(|e| e.pause.as_ref());
    // Idle, but only because a ticket of its is paused (an engine's limit, a
    // retry): the row says that instead of "Idle".
    let parked = if state == TeamState::Resting && runs.is_empty() { paused_ticket(&shell.model, member) } else { None };
    let failed = single.is_some() && runs[0].failed && runs[0].presence != Presence::Working && paused.is_none();
    // Running several tickets, it says how many — "frontend · 3 tickets" —
    // whatever one of them waits for: its face's dot and that run's row say so.
    // Waiting on the person, it says for what.
    let wait = if runs.len() > 1 { None } else { wait_of(&shell.model, member) };
    let stale = wait.as_ref().is_some_and(Wait::stale);
    let words = match (&wait, single) {
        _ if runs.len() > 1 => t!("sidebar.runs", n = runs.len()),
        (Some(wait), _) => wait.short(),
        (None, Some(ticket)) if failed => format!("{} · T-{ticket}", t!("sidebar.run_failed")),
        (None, Some(ticket)) if let Some(pause) = paused => format!("{} · T-{ticket}", crate::state::pause_words(pause, shell.model.unix as i64)),
        (None, None) if let Some((ticket, pause)) = parked => format!("{} · T-{ticket}", crate::state::pause_words(pause, shell.model.unix as i64)),
        _ => state.words(),
    };
    // On the pointer: the ticket and all that is asked, how long it has waited.
    let tip = wait.as_ref().map(|w| {
        let title = w.ticket.and_then(|id| shell.model.tickets.iter().find(|t| t.id == id).map(|t| format!("T-{id} {}", t.title)));
        (title.unwrap_or_else(|| member.name.clone()), w.detail())
    });
    let mark = match state {
        TeamState::Working(_) | TeamState::Compacting(_) => Mark::Spinner,
        TeamState::Runs(_) if at_work => Mark::Spinner,
        TeamState::Runs(_) => Mark::Ring(p.member(hue)),
        TeamState::Asks | TeamState::YourTurn => Mark::Waits,
        TeamState::Closed => Mark::Hollow,
        TeamState::Resting => Mark::Ring(p.member(hue)),
    };
    let group = SharedString::from(format!("mem-{session}"));
    let menu = (side != Side::Pm).then(|| member.name.clone());
    let said = match &tip {
        Some((_, detail)) => format!("{}, {}, {}", member.name, words, detail.replace('\n', ", ")),
        None => format!("{}, {}", member.name, words),
    };
    row_shell(p, ("member", session), on)
        .h(px(TEAM_ROW))
        .aria_label(if rests { format!("{said}, {}", t!("sidebar.resting")) } else { said })
        .aria_selected(on)
        .group(group.clone())
        .when(rests, |r| r.opacity(0.55))
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if let Some(ticket) = single {
                let goal = super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: None, ticket: Some(ticket) });
                if !this.open_elsewhere(e, goal, window, cx) {
                    this.open_side(Side::Ticket(ticket), window, cx);
                }
                return;
            }
            let goal = super::elsewhere::Goal::Place(Place { screen: Screen::Chat, menu: this.board.view.menu, member: Some(session), ticket: None });
            if !this.open_elsewhere(e, goal, window, cx) {
                this.select_member(session, window, cx);
            }
        }))
        // Right-click: rest it, or wake it — the person's choice.
        .when_some(menu, |row, _| {
            row.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_member_menu(session, e.position, cx);
                }),
            )
        })
        .child(face(p, member, mark))
        // Name, then what it is doing against the right edge, then its engine:
        // each in the same place on every row.
        .child(div().flex_1().min_w(px(36.)).truncate().text_size(px(TEXT)).text_color(if on { p.text } else { p.text2 }).child(member.name.clone()))
        .child(
            div()
                .id(("member-words", session))
                .flex_none()
                .max_w(relative(0.6))
                .truncate()
                .text_right()
                .text_size(px(12.))
                // Waiting on the person, its face's dot says so in orange; the
                // words stay in ink, a step stronger — amber once it waited for days.
                .text_color(if stale { p.amber } else if state.waits() { p.text2 } else { p.muted })
                .when(state.waits(), |d| d.font_weight(FontWeight::MEDIUM))
                .group_hover(group.clone(), |s| s.invisible())
                .when_some(tip, |d, (title, detail)| d.tip_at(p, format!("{title}\n{detail}"), None, gpui_kit::base::Placement::Right))
                .child(words.clone()),
        )
        .children(member.engine.filter(|_| show_engine).map(|e| div().flex_none().group_hover(group.clone(), |s| s.invisible()).child(super::switching::mark(p, ("member-engine", session), e))))
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
                .child(if single.is_some() { t!("sidebar.open_ticket") } else { t!("sidebar.talk") }),
        )
        .into_any_element()
}

/// A member's menu, open where the right-click was: rest it — letting what
/// it works on finish, or stopping that too — or wake it.
fn member_menu(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let (session, at) = shell.board.member_menu?;
    let member = shell.model.members.iter().find(|m| m.session == session)?;
    let name = member.name.clone();
    let rests = shell.model.is_resting(member);
    let busy = !shell.model.runs_of(session).is_empty() || member.presence == Presence::Working;
    let can = shell.can_act();
    let item = |id: &'static str, name: IconName, label: String, hint: Option<String>| {
        h_flex()
            .id(id)
            .min_h(px(30.))
            .py_1()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(TEXT))
            .text_color(if can { p.text } else { p.muted })
            .when(can, |r| r.cursor_pointer().hover(move |s| s.bg(p.side_hover)))
            .child(icon(name, 14., if can { p.text } else { p.muted }))
            .child(v_flex().flex_1().min_w_0().child(div().truncate().child(label)).children(hint.map(|h| div().text_size(px(11.)).text_color(p.muted).child(h))))
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    if rests {
        let who = name.clone();
        rows.push(
            item("mm-wake", IconName::Play, t!("sidebar.wake").to_owned(), Some(t!("sidebar.wake_hint").to_owned()))
                .on_click(cx.listener(move |this, _, _, cx| this.rest_member(&who, false, false, cx)))
                .into_any_element(),
        );
    } else {
        let who = name.clone();
        rows.push(
            item("mm-rest", IconName::Pause, t!("sidebar.rest").to_owned(), Some(if busy { t!("sidebar.rest_hint_busy") } else { t!("sidebar.rest_hint") }.to_owned()))
                .on_click(cx.listener(move |this, _, _, cx| this.rest_member(&who, true, false, cx)))
                .into_any_element(),
        );
        if busy {
            let who = name.clone();
            rows.push(
                item("mm-rest-stop", IconName::Square, t!("sidebar.rest_stop").to_owned(), None)
                    .on_click(cx.listener(move |this, _, _, cx| this.rest_member(&who, true, true, cx)))
                    .into_any_element(),
            );
        }
    }
    Some(
        deferred(
            anchored().position(at).snap_to_window().child(
                v_flex()
                    .id("member-menu")
                    .w(px(240.))
                    .p_1()
                    .rounded(px(10.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.board.member_menu = None;
                        cx.notify();
                    }))
                    .child(div().px_2().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).child(name))
                    .children(rows),
            ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

// ── rows arriving and leaving ──

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
    // The mark as an icon, in the words' colour: typed, "→" and "↻" sat off the line.
    let mark = |m: crate::icons::Icon, color: Rgba| crate::icons::icon(m, crate::theme::ICON_SM, color);
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
            .child(mark(crate::icons::Icon::ArrowRight, p.on_primary)),
        _ => pill
            .border_1()
            .border_color(if act == Act::RestartDaemonNow { p.red } else { p.hair })
            .text_color(if act == Act::RestartDaemonNow { p.red } else { p.text })
            .cursor_pointer()
            .hover(|s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, _, cx| this.update_act(act, cx)))
            .child(words)
            .child(mark(if matches!(act, Act::CheckNow | Act::UpdateDaemon) { crate::icons::Icon::Reload } else { crate::icons::Icon::ArrowRight }, if act == Act::RestartDaemonNow { p.red } else { p.text })),
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
                .child(div().flex_none().max_w(relative(0.6)).truncate().text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(squad))
                .child(machine_label(p, shell))
                .child(icon(IconName::ChevronsUpDown, 12., p.muted)),
        )
        .child(icon_button(p, "settings", IconName::Settings, true, None).tip(p, t!("sidebar.settings"), Some("⌘,")).on_click(cx.listener(|this, _, window, cx| this.open_settings(None, window, cx))))
}

/// The foot's machine: this Mac quietly; another machine plainly, with its
/// mark — with windows on two machines, which one this is is never a guess.
fn machine_label(p: Palette, shell: &Shell) -> AnyElement {
    let name = shell.place_name();
    if !shell.endpoint.is_remote() {
        return div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted).child(name).into_any_element();
    }
    h_flex()
        .flex_1()
        .min_w_0()
        .gap(px(4.))
        .items_center()
        .text_size(px(12.))
        .text_color(p.text2)
        .child(div().flex_none().child(crate::icons::icon(crate::icons::Icon::Server, 11., p.text2)))
        .child(div().min_w_0().truncate().child(name))
        .into_any_element()
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
    // A row's "open in a new window", on the pointer — what ⌘-click on the
    // row does — so it is found without knowing the keys.
    let away = |id: SharedString, group: SharedString| {
        div()
            .id(id)
            .size(px(22.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .invisible()
            .group_hover(group, |s| s.visible())
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .aria_label(t!("sidebar.open_in_new_window"))
            .tip_at(p, t!("sidebar.open_in_new_window"), Some(t!("sidebar.cmd_click")), gpui_kit::base::Placement::Right)
            .child(crate::icons::icon(crate::icons::Icon::External, 12., p.text2))
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    match which {
        Popup::Account => {
            rows.push(label(t!("sidebar.squads")));
            for (ix, name) in shell.squads.iter().enumerate() {
                let (target, away_to) = (name.clone(), name.clone());
                let group = SharedString::from(format!("squad-row-{ix}"));
                rows.push(
                    item(SharedString::from(format!("squad-{ix}")), name.clone(), *name == shell.model.squad)
                        .group(group.clone())
                        // ⌘-click: that squad in a new window; this one stays where it is.
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.board.popup = None;
                            if super::elsewhere::wants_window(&e.modifiers()) {
                                this.new_window(Some(target.clone()), cx);
                            } else {
                                this.select_squad(&target, window, cx);
                            }
                            cx.notify();
                        }))
                        .child(away(SharedString::from(format!("squad-away-{ix}")), group).on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.board.popup = None;
                            this.new_window(Some(away_to.clone()), cx);
                            cx.notify();
                        })))
                        .into_any_element(),
                );
            }
            // One more squad, on the machine the window is on — or why not now.
            let route = shell.new_squad_route();
            let new = h_flex()
                .id("new-squad")
                .h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(TEXT))
                .child(div().w(px(14.)).flex_none().child(icon(IconName::Plus, 13., if route.available() { p.text2 } else { p.muted })))
                .child(div().flex_1().min_w_0().truncate().child(t!("sidebar.new_squad")));
            rows.push(match super::new_squad::unavailable_tip(route, &shell.endpoint.label()) {
                Some(why) => new.text_color(p.muted).tip_at(p, why, None, gpui_kit::base::Placement::Right).into_any_element(),
                None => new
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.side_hover))
                    .on_click(cx.listener(|this, _, window, cx| this.open_new_squad(window, cx)))
                    .into_any_element(),
            });
            // Deleting the squad is not here, beside switching: it is in the squad's Settings, asked first.
            rows.push(div().h(px(1.)).my_1().bg(p.hair).into_any_element());
            // One more window, on this machine and squad, to point elsewhere.
            rows.push(
                h_flex()
                    .id("new-window")
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(TEXT))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.side_hover))
                    .child(div().w(px(14.)).flex_none().child(crate::icons::icon(crate::icons::Icon::AppWindow, 13., p.text2)))
                    .child(div().flex_1().min_w_0().truncate().child(t!("sidebar.new_window")))
                    .child(div().flex_none().text_size(px(11.)).text_color(p.muted).child("⌘⇧N"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.board.popup = None;
                        this.new_window(None, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            // Another machine — here, or (⌘-click, the row's button) in a new window.
            rows.push(
                item("machine".into(), t!("sidebar.other_machine").into(), false)
                    .group("machine-row")
                    .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                        this.board.popup = None;
                        if super::elsewhere::wants_window(&e.modifiers()) {
                            this.new_window_elsewhere(cx);
                        } else {
                            this.open_chooser(window, cx);
                        }
                        cx.notify();
                    }))
                    .child(away("machine-away".into(), "machine-row".into()).on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.board.popup = None;
                        this.new_window_elsewhere(cx);
                        cx.notify();
                    })))
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
        let step = sub_step(shell, sub, cx);
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
            .child(div().pt(px(3.)).child(sub_mark_of(shell, sub, cx)))
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
    /// "+N more" under a member: all its subagents, or away again.
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

    /// Pin a conversation with the PM, or let it go: equipe never puts a
    /// pinned one away by itself.
    pub fn pin_conversation(&mut self, session: SessionId, pinned: bool, cx: &mut Context<Self>) {
        self.send_command(equipe_core::protocol::Command::PinConversation { session, pinned });
        if let Some(lane) = self.model.lanes.iter_mut().find(|l| l.session == session) {
            lane.pinned = pinned;
        }
        cx.notify();
    }

    /// A conversation put away, taken up again: it opens once it is back.
    pub fn reopen_conversation(&mut self, session: SessionId, cx: &mut Context<Self>) {
        self.board.reopening = Some(session);
        self.send_command(equipe_core::protocol::Command::ReopenConversation { squad: self.model.squad.clone(), session });
        cx.notify();
    }

    /// ⌘K: find anything — All work, its search box.
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

    /// A member's menu, at `at`.
    pub fn open_member_menu(&mut self, member: SessionId, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.board.member_menu = Some((member, at));
        cx.notify();
    }

    /// Rest member `name` — no new work; with `stop`, what it works on stops
    /// too — or wake it (`resting` false). The daemon keeps it and tells every screen.
    pub fn rest_member(&mut self, name: &str, resting: bool, stop: bool, cx: &mut Context<Self>) {
        self.board.member_menu = None;
        if self.can_act() {
            self.send_command(equipe_core::protocol::Command::SetResting { squad: self.model.squad.clone(), member: name.to_owned(), resting, stop });
        }
        cx.notify();
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

    /// A click on a project's header: its name opens the project's page, its
    /// chevron folds it — kept for the next launch.
    pub fn side_head_click(&mut self, click: HeadClick, project: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let mut folded = self.remembered.folded.clone();
        match crate::board::head_click(click, project.as_deref(), &mut folded) {
            Some(name) => self.open_project_page(name, window, cx),
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
        Place { screen: self.screen, menu: self.board.view.menu, member: self.model.selected, ticket: if self.screen == Screen::Studio { self.studio.ticket } else { self.model.opened } }
    }

    /// Called from each draw: a new place goes on the history.
    pub fn note_place(&mut self) {
        let here = self.place();
        self.board.nav.note(here);
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(place) = self.board.nav.back() {
            self.go_to(place, window, cx);
        }
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(place) = self.board.nav.forward() {
            self.go_to(place, window, cx);
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
            Screen::Roadmap => self.open_roadmap(cx),
            Screen::Metrics => self.open_metrics(cx),
            Screen::Progress => self.open_progress(cx),
            Screen::Usage => self.open_usage(cx),
            Screen::Status => self.open_status(cx),
            Screen::Logins => self.open_logins(cx),
            Screen::Orchestration => self.open_orchestra(cx),
            Screen::Docs => self.open_docs(cx),
            Screen::Studio => match place.ticket.or(self.studio.ticket) {
                Some(id) => self.open_studio(id, window, cx),
                None => self.open_side(Side::Pm, window, cx),
            },
            Screen::Project => match self.board.project_page.clone() {
                Some(name) => self.open_project_page(name, window, cx),
                None => self.open_side(Side::Pm, window, cx),
            },
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

/// The places a window has been, and where in them it is: what back and
/// forward (the title bar's arrows, ⌘[ and ⌘], the mouse's side buttons) walk.
#[derive(Debug, Default)]
pub struct NavHistory {
    places: Vec<Place>,
    at: usize,
}

impl NavHistory {
    const MAX: usize = 100;

    /// The window is at `here`: a new place goes after the current one, and
    /// what was ahead of it (the forward trail) is dropped, as a browser does.
    pub fn note(&mut self, here: Place) {
        if self.places.get(self.at) == Some(&here) {
            return;
        }
        self.places.truncate(self.at + 1);
        self.places.push(here);
        if self.places.len() > Self::MAX {
            self.places.remove(0);
        }
        self.at = self.places.len() - 1;
    }

    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.places.len()
    }

    /// One step back: the place now current, if there was one before.
    pub fn back(&mut self) -> Option<Place> {
        self.can_back().then(|| {
            self.at -= 1;
            self.places[self.at]
        })
    }

    /// One step forward: the place now current, if there is one ahead.
    pub fn forward(&mut self) -> Option<Place> {
        self.can_forward().then(|| {
            self.at += 1;
            self.places[self.at]
        })
    }
}

/// The sidebar's top items in order, ⌘1, ⌘2…: what Needs you, Status (when
/// the daemon offers it) and Progress open.
pub fn top_places(status_offered: bool, menu: Menu) -> Vec<Place> {
    let at = |screen| Place { screen, menu, member: None, ticket: None };
    let mut places = vec![Place { screen: Screen::Board, menu: Menu::Inbox, member: None, ticket: None }];
    if status_offered {
        places.push(at(Screen::Status));
    }
    places.push(at(Screen::Progress));
    places
}

#[cfg(test)]
mod tests {
    use super::{HeadSign, MAX_WIDTH, MIN_WIDTH, SUB_ROW, SUBS_SHOWN, SubLeaving, SubMotion, SubShown, TeamState, WIDTH, Wait, cut, engine_marks, head_sign, remove_ask, run_rows, settle, shown_projects, sub_rows, subs_cap, paused_ticket, wait_of, CONVERSATIONS_SHOWN};
    use equipe_core::protocol::Engine;
    use crate::state::{Member, Model, Presence, Verb};
    use equipe_core::protocol::Ticket;
    use std::time::{Duration, Instant};

    /// A project's header says one thing at its end, the most pressing: what
    /// waits on the person (with how many), else work that needs a look, else
    /// work under way — and nothing when nothing is live.
    #[test]
    fn a_project_header_shows_one_sign_what_waits_on_the_person_first() {
        assert_eq!(head_sign(4, true, 3), HeadSign::Waits(4));
        assert_eq!(head_sign(1, false, 0), HeadSign::Waits(1));
        assert_eq!(head_sign(0, true, 2), HeadSign::Risky);
        assert_eq!(head_sign(0, true, 0), HeadSign::Risky);
        assert_eq!(head_sign(0, false, 2), HeadSign::Working(2));
        assert_eq!(head_sign(0, false, 0), HeadSign::Quiet);
    }

    /// With nearly every project waiting, the headers' counts are the total's
    /// breakdown and add up to it; one with nothing waiting shows its work, not a zero.
    #[test]
    fn the_headers_counts_add_up_to_the_one_total() {
        let rows = [4, 2, 2, 1, 1, 0, 10];
        let signs: Vec<HeadSign> = rows.iter().map(|&n| head_sign(n, false, 1)).collect();
        let total: usize = signs.iter().map(|s| if let HeadSign::Waits(n) = s { *n } else { 0 }).sum();
        assert_eq!(total, 20);
        assert_eq!(signs[5], HeadSign::Working(1));
    }

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
        assert_eq!(ask.title, "site를 스쿼드에서 뺄까요?");
        assert_eq!(ask.body.len(), 2, "what stays, and the one ticket at work: {:?}", ask.body);
        assert!(ask.destructive(), "its yes is red, and Enter doesn't say it");
        assert!(matches!(&ask.then, crate::sheet::Then::Send(c) if matches!(c[..], [equipe_core::protocol::Command::RemoveProject { .. }])));
        assert_eq!(remove_ask(&model, "app", "/code/app".into()).body.len(), 1, "nothing at work: no second line");
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
        Member { session, name: name.into(), role: name.into(), presence, verb: Verb::Working, ticket, memos: 0, context: None, model: None, asked: None, engine: None, estimate: None, target: None, last_seen: 0, failed: false }
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

    /// A member with nothing running whose ticket equipe paused says why,
    /// where it used to say "Idle".
    #[test]
    fn an_idle_member_whose_ticket_is_paused_says_why_instead_of_idle() {
        let mut model = Model::new("acme");
        let mut limited = ticket(12, "in_progress", "frontend");
        limited.execution = serde_json::from_value(serde_json::json!({"state": "active", "session": 2, "operation": "op", "generation": 1, "heartbeat_at": 0, "lease_expires_at": 0, "pause": {"why": "engine_limit", "engine": "claude", "resumes_at": 99_000}})).ok();
        model.tickets = vec![ticket(5, "done", "frontend"), limited, ticket(13, "in_progress", "backend")];
        model.members = vec![member(2, "frontend", Presence::Ready, Some(12)), member(3, "backend", Presence::Ready, Some(13))];
        let (id, pause) = paused_ticket(&model, &model.members[0]).expect("its ticket is paused");
        assert_eq!(id, 12);
        let words = crate::state::pause_words(pause, 1_000);
        assert!(words.contains("Claude") && !words.is_empty(), "{words}");
        assert!(paused_ticket(&model, &model.members[1]).is_none(), "an unpaused ticket stays idle");
        assert_eq!(TeamState::of(&model, &model.members[1]).words(), TeamState::Resting.words());
    }

    #[test]
    fn the_conversations_list_shows_a_screenful_then_all_behind_one_row() {
        assert_eq!(CONVERSATIONS_SHOWN, 10);
    }

    #[test]
    fn a_member_on_one_ticket_is_one_row_and_two_or_more_list_each_under_it() {
        assert_eq!(run_rows(0), 0);
        assert_eq!(run_rows(1), 0, "a single run: the member's row says it and opens it");
        assert_eq!(run_rows(2), 2);
        assert_eq!(run_rows(4), 4);
        assert_eq!(run_rows(6), 5, "four, then +2 more");
        // The member's row says the one ticket.
        let mut model = Model::new("acme");
        model.tickets = vec![ticket(669, "in_progress", "frontend")];
        model.members = vec![member(1, "pm", Presence::Ready, None), member(2, "frontend", Presence::Ready, None)];
        let run = |session, ticket| crate::state::Run { session, of: 2, ticket, presence: Presence::Working, verb: Verb::Working, failed: false };
        model.runs = vec![run(20, 669)];
        assert_eq!(TeamState::of(&model, &model.members[1]), TeamState::Working(Some(669)));
        model.runs.push(run(21, 670));
        assert_eq!(TeamState::of(&model, &model.members[1]), TeamState::Runs(2), "two: \"2 tickets\", each a row under it");
    }

    #[test]
    fn your_turn_says_the_ticket_and_what_is_asked_and_goes_stale_after_three_days() {
        const DAY: u64 = 24 * 60 * 60;
        let mut model = Model::new("acme");
        model.unix = 2_000_000_000;
        let mut account = ticket(584, "waiting", "backend");
        account.title = "Store listing".into();
        account.note = Some("Waiting on you:\n- [ ] Create the App Store account\n- [ ] Check it on a real phone".into());
        let bare = ticket(590, "waiting", "designer");
        model.tickets = vec![account, bare, ticket(12, "in_progress", "frontend")];
        model.members = vec![member(1, "pm", Presence::Ready, None), member(2, "backend", Presence::Ready, None), member(3, "designer", Presence::Ready, None), member(4, "frontend", Presence::Working, Some(12))];
        model.since.insert(584, model.unix - 9 * DAY);
        model.since.insert(590, model.unix - DAY);
        let wait = wait_of(&model, &model.members[1]).unwrap();
        assert_eq!(wait, Wait { ticket: Some(584), what: "Create the App Store account; Check it on a real phone".into(), waited: Some(9 * DAY) });
        assert_eq!(wait.short(), "T-584 · Create the App Store…", "the ticket, then what, cut at a word");
        assert!(wait.stale() && wait.detail().starts_with("Create the App Store account; Check it on a real phone\n"), "all of it on the pointer, then how long");
        // Nothing listed: the ticket and that it needs the person.
        let bare = wait_of(&model, &model.members[2]).unwrap();
        assert_eq!((bare.ticket, bare.short()), (Some(590), "T-590 · 내 차례".to_owned()));
        assert!(!bare.stale(), "a day: not stale");
        // A request for permission says what it asks for, on its ticket.
        let ask: equipe_core::protocol::Approval = serde_json::from_value(serde_json::json!({"id": 1, "session": 4, "member": "frontend", "ticket": 12, "tool": "Bash", "action": "git push", "reason": null})).unwrap();
        model.approvals.push(ask);
        let asks = wait_of(&model, &model.members[3]).unwrap();
        assert!(asks.short().starts_with("T-12 · ") && asks.short().ends_with('?'), "{}", asks.short());
        assert_eq!(wait_of(&model, &model.members[0]), None, "waits for nothing");
        assert_eq!(cut("short", 28), "short");
        assert_eq!(cut("가나다라마바사아자차카타파하가나다라마바사아자차카타파하가나다", 28).chars().count(), 29, "no spaces: cut where it must");
    }

    #[test]
    fn a_rows_engine_shows_only_where_it_differs_and_one_shared_engine_is_said_once() {
        let (c, x) = (Some(Engine::Claude), Some(Engine::Codex));
        assert_eq!(engine_marks(&[c, c, c]), (vec![false, false, false], Some(Engine::Claude)), "all the same: once, on the heading");
        assert_eq!(engine_marks(&[c, c, x, c]), (vec![false, false, true, false], None), "the odd one out");
        assert_eq!(engine_marks(&[x, x, c]), (vec![false, false, true], None));
        assert_eq!(engine_marks(&[c, x]), (vec![true, true], None), "a tie: each says its own");
        assert_eq!(engine_marks(&[c, None, c]), (vec![false, false, false], Some(Engine::Claude)), "one not known yet");
        assert_eq!(engine_marks(&[]), (vec![], None));
    }

    #[test]
    fn a_resting_member_comes_last() {
        let mut model = Model::new("acme");
        model.tickets = vec![ticket(13, "waiting", "backend")];
        model.members = vec![member(1, "pm", Presence::Ready, None), member(2, "designer", Presence::Working, None), member(3, "backend", Presence::Ready, None), member(4, "qa", Presence::Ready, None)];
        assert_eq!(model.team(), [1, 3, 2, 4]);
        model.resting = vec!["backend".into(), "designer".into()];
        assert_eq!(model.team(), [1, 4, 3, 2], "under Resting at the foot, in the team's order");
        assert!(model.is_resting(&model.members[2]) && !model.is_resting(&model.members[3]));
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
            assert!(rows as f32 * SUB_ROW <= super::child_room(4), "{counts:?}");
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

    fn place(ticket: u32) -> super::Place {
        super::Place { screen: crate::Screen::Chat, menu: crate::board::Menu::Inbox, member: None, ticket: Some(ticket) }
    }

    /// Back and forward walk the places the window has been, as the title bar's arrows and ⌘[ ⌘] do.
    #[test]
    fn back_and_forward_move_through_the_history() {
        let mut h = super::NavHistory::default();
        assert!(!h.can_back() && !h.can_forward());
        assert_eq!((h.back(), h.forward()), (None, None));
        for n in 1..=3 {
            h.note(place(n));
        }
        h.note(place(3));
        assert_eq!(h.back(), Some(place(2)));
        assert_eq!(h.back(), Some(place(1)));
        assert_eq!(h.back(), None, "nothing before the first place");
        assert_eq!(h.forward(), Some(place(2)));
        assert!(h.can_back() && h.can_forward());
        // A new place after going back drops what was ahead, as a browser does.
        h.note(place(9));
        assert!(!h.can_forward());
        assert_eq!(h.back(), Some(place(2)));
        assert_eq!(h.forward(), Some(place(9)));
    }

    #[test]
    fn the_sidebars_top_items_are_numbered_in_order_skipping_status_when_it_is_not_offered() {
        use crate::Screen;
        let menu = crate::board::Menu::Inbox;
        let with: Vec<Screen> = super::top_places(true, menu).iter().map(|p| p.screen).collect();
        assert_eq!(with, [Screen::Board, Screen::Status, Screen::Progress]);
        let without: Vec<Screen> = super::top_places(false, menu).iter().map(|p| p.screen).collect();
        assert_eq!(without, [Screen::Board, Screen::Progress]);
    }
}

