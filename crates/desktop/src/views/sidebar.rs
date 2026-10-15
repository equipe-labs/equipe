//! The sidebar, after Claude's: a strip by the traffic lights (hide ⌘\,
//! search ⌘K, back and forward), then a few quiet rows — 새 메시지 ⌘N,
//! PM 대화, 나에게 (a badge), 작업 중, 모든 일 — the views kept, and the
//! tickets as Claude lists sessions: under a header per project, one line
//! each, a small mark for how it stands and the title fading out at the
//! edge. The finished fold into one line per project; the team sits last.
//! At the foot, the update when one is ready, the squad and machine, and
//! settings. The edge drags to resize. One left edge for marks, one for text.

use equipe_core::t;

use equipe_core::protocol::{Ticket, TicketId, TicketStatus};
use crate::tips::TipExt as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::dot;
use super::header::project_color;
use crate::board::{Menu, project_groups};
use crate::state::{Conn, Member, Model, Need, Side, Status, Verb, short_span};
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
/// Where the traffic lights end, and a little air: the strip's buttons start here.
pub const LIGHTS: f32 = 78.;
/// The strip's square buttons and the space between them.
const BUTTON: f32 = 26.;
const BUTTON_GAP: f32 = 2.;
/// Where the fixed buttons (hide, back, forward) end, whatever the sidebar does.
pub const BUTTONS_END: f32 = LIGHTS + 3. * BUTTON + 2. * BUTTON_GAP;
const ROW: f32 = 30.;
const TEXT: f32 = 13.;
/// Every row's mark sits in a box this wide, so every title starts at one x.
const MARK_BOX: f32 = 16.;

/// A small menu out of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    /// The squads and machines.
    Account,
}

/// How a ticket's row marks where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A member is at it now.
    Spinner,
    /// It waits for the person.
    Waits,
    /// Something happened since the person looked.
    News,
    /// Under way or picked up, nobody at it this moment.
    Idle,
    /// Not started, or only proposed.
    NotStarted,
    Done,
}

/// The mark a ticket's row wears.
pub fn mark(model: &Model, t: &Ticket) -> Mark {
    if t.status.is_closed() {
        return Mark::Done;
    }
    if model.is_live(t) {
        return Mark::Spinner;
    }
    if matches!(t.status, TicketStatus::Review | TicketStatus::Waiting) || t.not_on_main() {
        return Mark::Waits;
    }
    if model.news.contains(&t.id) {
        return Mark::News;
    }
    match t.status {
        TicketStatus::InProgress => Mark::Idle,
        _ => Mark::NotStarted,
    }
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
        .left(px(LIGHTS))
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

    // The strip: the title bar's fixed buttons float over its left (see
    // `title_buttons`); search, the sidebar's own, sits after them and leaves
    // with the sidebar.
    let strip = h_flex()
        .h(px(STRIP))
        .flex_none()
        .pl(px(BUTTONS_END + BUTTON_GAP))
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(icon_button(p, "search", IconName::Search, true, Some(t!("sidebar.search_tip").into())).on_click(cx.listener(|this, _, window, cx| this.search_everything(window, cx))));

    let nav = |id: &'static str, name: IconName, label: String, badge: usize, on: bool| {
        row_shell(p, id, on)
            .h(px(32.))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(name, 16., if on { p.text } else { p.text2 })))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else { p.text2 }).child(label))
            .when(badge > 0, |r| {
                r.child(div().flex_none().min_w(px(18.)).h(px(18.)).px(px(5.)).flex().items_center().justify_center().rounded(px(9.)).bg(p.accent).text_color(p.on_primary).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(badge.to_string()))
            })
    };
    let nav_rows = v_flex()
        .gap(px(1.))
        .child(nav("nav-new", IconName::Plus, t!("sidebar.new_message").into(), 0, false).on_click(cx.listener(|this, _, window, cx| this.new_message(window, cx))))
        .child(nav("nav-pm", IconName::Bot, t!("sidebar.pm_talk").into(), 0, pm_on).on_click(cx.listener(|this, _, window, cx| this.open_side(Side::Pm, window, cx))))
        .child(nav("nav-inbox", IconName::Inbox, t!("board.menu_inbox").into(), needs, on_board && menu == Menu::Inbox).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::Inbox, cx))))
        .child(nav("nav-working", IconName::LoaderCircle, t!("board.menu_working").into(), 0, on_board && menu == Menu::Working).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::Working, cx))))
        .child(nav("nav-all", IconName::LayoutDashboard, t!("board.menu_all").into(), 0, on_board && menu == Menu::All).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(Menu::All, cx))));

    // The views kept: pinned, each with its icon.
    let views = shell.saved_views();
    let builtin = crate::board::builtin_views().len();
    let pinned: Vec<AnyElement> = views
        .iter()
        .enumerate()
        .filter(|(_, v)| v.pinned)
        .map(|(at, v)| {
            let on = on_board && menu == Menu::View(at);
            let removable = at >= builtin;
            let group = SharedString::from(format!("view-{at}"));
            row_shell(p, ("view", at), on)
                .group(group.clone())
                .on_click(cx.listener(move |this, _, _, cx| this.open_side_menu(Menu::View(at), cx)))
                .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(IconName::Star, 13., p.muted)))
                .child(fading_title(p, v.name.clone(), on, false))
                .when(removable, |r| {
                    r.child(
                        div()
                            .id(("forget-view", at))
                            .absolute()
                            .right(px(8.))
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .child(icon(IconName::Close, 12., p.muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.board_forget_view(at, cx);
                            })),
                    )
                })
                .into_any_element()
        })
        .collect();

    // The tickets, as sessions: a group per project.
    let selected = selected_ticket(shell);
    let mut groups: Vec<AnyElement> = Vec::new();
    for g in project_groups(model) {
        let key = g.name.clone().unwrap_or_default();
        let folded = ui.folded.contains(&key);
        let label = g.name.clone().unwrap_or_else(|| t!("board.squad_folder").into());
        let fold_key = key.clone();
        let group = SharedString::from(format!("head-{key}"));
        let focus_target = g.name.clone();
        let project = g.name.clone();
        let hover_button = |id: SharedString, name: IconName, tip: &'static str| {
            div()
                .id(id)
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .hover(move |s| s.bg(p.side_hover))
                .tip(p, tip, None)
                .child(icon(name, 13., p.muted))
        };
        let head = section_head(p, SharedString::from(format!("project-{key}")), folded)
            .group(group.clone())
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.board.folded.remove(&fold_key) {
                    this.board.folded.insert(fold_key.clone());
                }
                cx.notify();
            }))
            .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(match g.at {
                Some(at) => dot(project_color(p, at), 6.).into_any_element(),
                None => icon(IconName::Folder, 12., p.muted).into_any_element(),
            }))
            .child(div().flex_none().child(label))
            .child(icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 11., p.muted))
            .child(div().flex_1())
            .child(hover_button(SharedString::from(format!("focus-{key}")), IconName::SortAscending, t!("sidebar.only_this_project")).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                let next = if this.model.focus == focus_target { None } else { focus_target.clone() };
                this.focus_project(next, window, cx);
            })))
            .child(hover_button(SharedString::from(format!("new-in-{key}")), IconName::Plus, t!("sidebar.new_in_project")).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if project.is_some() {
                    this.focus_project(project.clone(), window, cx);
                }
                this.new_message(window, cx);
            })));
        groups.push(head.into_any_element());
        if folded {
            continue;
        }
        for t in &g.open {
            groups.push(ticket_row(shell, t, selected == Some(t.id), cx));
        }
        if g.open.is_empty() && g.done.is_empty() {
            groups.push(row_shell(p, SharedString::from(format!("none-{key}")), false).cursor_default().child(div().w(px(MARK_BOX)).flex_none()).child(div().text_size(px(12.)).text_color(p.muted).child(t!("sidebar.nothing_yet"))).into_any_element());
        }
        if !g.done.is_empty() {
            let shown = ui.done_shown.contains(&key);
            let done_key = key.clone();
            groups.push(
                row_shell(p, SharedString::from(format!("done-{key}")), false)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.board.done_shown.remove(&done_key) {
                            this.board.done_shown.insert(done_key.clone());
                        }
                        cx.notify();
                    }))
                    .child(mark_element(p, Mark::Done, 0.))
                    .child(div().text_size(px(12.)).text_color(p.muted).child(t!("sidebar.done_count", n = g.done.len())))
                    .child(icon(if shown { IconName::ChevronDown } else { IconName::ChevronRight }, 11., p.muted))
                    .into_any_element(),
            );
            if shown {
                for t in g.done.iter().take(30) {
                    groups.push(ticket_row(shell, t, selected == Some(t.id), cx));
                }
            }
        }
    }

    let team = model.team();
    let team_folded = ui.folded.contains("\u{0}team");
    let team_rows: Vec<AnyElement> = if team_folded { Vec::new() } else { team.iter().filter_map(|s| model.members.iter().find(|m| m.session == *s)).map(|m| member_row(p, shell, m, cx)).collect() };
    let team_head = section_head(p, "team-head", team_folded)
        .on_click(cx.listener(|this, _, _, cx| {
            if !this.board.folded.remove("\u{0}team") {
                this.board.folded.insert("\u{0}team".into());
            }
            cx.notify();
        }))
        .child(div().w(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(icon(IconName::User, 12., p.muted)))
        .child(t!("sidebar.team"))
        .child(icon(if team_folded { IconName::ChevronRight } else { IconName::ChevronDown }, 11., p.muted));

    let body = v_flex()
        .id("side")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .pt_1()
        .pb_4()
        .when(dim, |l| l.opacity(0.8))
        .child(nav_rows)
        .when(!pinned.is_empty(), |l| l.child(section_head(p, "pinned-head", false).cursor_default().child(div().w(px(MARK_BOX)).flex_none()).child(t!("sidebar.pinned"))).children(pinned))
        .children(groups)
        .when(!team.is_empty(), |l| l.child(team_head).children(team_rows));

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

/// A section's header: muted small text at the rows' own left edges, a click folds it.
fn section_head(p: Palette, id: impl Into<ElementId>, _folded: bool) -> Stateful<Div> {
    h_flex()
        .id(id.into())
        .h(px(28.))
        .mt(px(12.))
        .mx_2()
        .px_2()
        .gap(px(8.))
        .items_center()
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .cursor_pointer()
        .hover(move |s| s.text_color(p.text2))
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

/// The small mark before a ticket's title, in a box of one size.
fn mark_element(p: Palette, mark: Mark, turn: f32) -> AnyElement {
    let holder = || div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none().flex().items_center().justify_center();
    match mark {
        Mark::Spinner => holder().child(icon(IconName::LoaderCircle, 13., p.text2).rotate(radians(turn))).into_any_element(),
        Mark::Waits => holder().child(dot(p.orange, 7.)).into_any_element(),
        Mark::News => holder().child(dot(p.accent, 7.)).into_any_element(),
        Mark::Idle => holder().child(dot(p.muted, 7.)).into_any_element(),
        Mark::NotStarted => holder().child(div().size(px(7.)).rounded_full().border_1().border_color(p.muted)).into_any_element(),
        Mark::Done => holder().child(icon(IconName::Check, 12., p.muted)).into_any_element(),
    }
}

/// A ticket, as a session: its mark, its title fading out, how long it has
/// been so — and, on the pointer, a quick yes for what waits. The number is
/// in the tooltip.
fn ticket_row(shell: &Shell, t: &Ticket, on: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let id = t.id;
    let group = SharedString::from(format!("tk-{id}"));
    let tip = format!("T-{id} · {}", if t.not_on_main() { t!("state.not_on_main") } else { crate::state::status_word(t.status) });
    let time = model.in_state_for(t).map(short_span);
    let yes = (matches!(t.status, TicketStatus::Review | TicketStatus::Waiting | TicketStatus::Proposed) || t.not_on_main()) && shell.can_act();
    row_shell(p, ("ticket", id as usize), on)
        .group(group.clone())
        .tip_at(p, tip, None, gpui_kit::base::Placement::Right)
        .on_click(cx.listener(move |this, _, window, cx| this.open_ticket_row(id, window, cx)))
        .child(mark_element(p, mark(model, t), shell.board.spin))
        .child(fading_title(p, t.title.clone(), on, t.status.is_closed()))
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
        })
        .into_any_element()
}

/// Someone on the team: a mark at the tickets' left edge, the name, and what
/// it is doing, fading. A click opens its conversation.
fn member_row(p: Palette, shell: &Shell, member: &Member, cx: &mut Context<Shell>) -> AnyElement {
    let status = shell.model.status(member);
    let (_, hue) = crate::theme::look(&member.role, &member.name);
    let side = if Model::is_pm(member) { Side::Pm } else { Side::Member(member.session) };
    let on = shell.screen == Screen::Chat && side != Side::Pm && shell.model.side_open() == Some(side);
    let session = member.session;
    let (line, line_color) = status_line(member, &status, shell.model.at_once(member), shell.model.approvals.first_for(member.session).is_some(), p);
    let mark = match status {
        Status::Working { .. } => mark_element(p, Mark::Spinner, shell.board.spin),
        Status::NeedsYou(_) => mark_element(p, Mark::Waits, 0.),
        Status::Closed => mark_element(p, Mark::NotStarted, 0.),
        Status::Ready { .. } => div().w(px(MARK_BOX)).h(px(MARK_BOX)).flex_none().flex().items_center().justify_center().child(dot(p.member(hue), 7.)).into_any_element(),
    };
    row_shell(p, ("member", session), on)
        .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        .child(mark)
        .child(div().flex_none().text_color(if on { p.text } else { p.text2 }).child(member.name.clone()))
        .child(fading_title(p, line, on, true).text_size(px(12.)).text_color(line_color))
        .into_any_element()
}

/// What a member row says after the name, and its colour.
pub fn status_line(member: &Member, status: &Status, at_once: usize, asks: bool, p: Palette) -> (String, Rgba) {
    let join = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let memos = (member.memos > 0).then(|| t!("sidebar.memos", n = member.memos));
    let context = member.context.filter(|c| *c >= 50).map(|c| format!("{c}%"));
    match status {
        Status::NeedsYou(_) if asks => (t!("state.section_permission").into(), p.orange),
        Status::NeedsYou(_) => (t!("sidebar.needs_you").into(), p.orange),
        Status::Working { .. } if at_once > 1 => (join(vec![memos, Some(t!("sidebar.at_once", at_once = at_once)), context]), p.muted),
        Status::Working { verb, .. } => (join(vec![memos, Some(verb_text(*verb).into()), context]), p.muted),
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

/// An update downloaded in the background, ready: one full-width button puts
/// it in and starts the app again (as Claude's "Relaunch to update").
fn relaunch(p: Palette, shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let version = shell.update_ready.as_deref()?;
    Some(
        h_flex()
            .id("relaunch")
            .h(px(34.))
            .flex_none()
            .mx_3()
            .mb_1()
            .px_3()
            .rounded(px(8.))
            .items_center()
            .justify_between()
            .bg(p.primary)
            .text_color(p.on_primary)
            .text_size(px(TEXT))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .on_click(cx.listener(|this, _, _, cx| this.relaunch_to_update(cx)))
            .child(crate::relaunch::label(version))
            .child("→"),
    )
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
        (BUTTONS_END + 8. - self.sidebar_on_screen(cx)).max(pad)
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
    use super::{Mark, mark, spin};
    use crate::state::Model;
    use equipe_core::protocol::Ticket;

    fn ticket(id: u32, status: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({"id": id, "title": "t", "body": "", "status": status, "owner": "backend", "assignee": null, "project": null, "depends_on": [], "branch": null, "created_by": "pm", "note": null})).unwrap()
    }

    #[test]
    fn a_row_marks_what_waits_what_is_news_what_idles_and_what_has_not_started() {
        let mut model = Model::new("acme");
        assert_eq!(mark(&model, &ticket(1, "review")), Mark::Waits);
        assert_eq!(mark(&model, &ticket(1, "waiting")), Mark::Waits);
        assert_eq!(mark(&model, &ticket(1, "proposed")), Mark::NotStarted);
        assert_eq!(mark(&model, &ticket(1, "todo")), Mark::NotStarted);
        assert_eq!(mark(&model, &ticket(1, "in_progress")), Mark::Idle, "nobody at it this moment");
        assert_eq!(mark(&model, &ticket(1, "done")), Mark::Done);
        model.news.insert(1);
        assert_eq!(mark(&model, &ticket(1, "todo")), Mark::News);
        model.seen(1);
        assert_eq!(mark(&model, &ticket(1, "todo")), Mark::NotStarted);
        assert_eq!(spin(12), 0.);
    }
}
