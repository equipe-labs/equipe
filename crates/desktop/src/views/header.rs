//! The conversation's header, which is the window's title bar on the right:
//! the conversation's name with ⌄ (the team, to switch), a small chip for the
//! project slice — opening a list with what works and what waits in each,
//! ⌃0–9 — the plan window used most while the sidebar is away, and the
//! context fill. The title bar's fixed buttons float over its
//! left with the sidebar away; its content slides beside them.

use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Popover, dot, kbd};
use crate::state::{Member, Model, Presence};
use crate::theme::{Palette, R_MD, SMALL, Sans, key_face, look};
use crate::Shell;
use crate::tips::TipExt as _;

/// The title bar's one height, the sidebar's strip's too.
pub const HEIGHT: f32 = super::sidebar::STRIP;
/// Past this many projects the list offers a search.
const SEARCH_FROM: usize = 7;
/// The least a conversation's title keeps while the bar is narrow.
const TITLE_MIN: f32 = 96.;
/// How much wider than where it last gave way the bar must be to show it again.
const GROW: f32 = 24.;
/// The empty room between the words and the buttons, at its least.
const GAP_MIN: f32 = 8.;

/// What the header leaves out while it is narrow, in the order it gives way
/// (T-621, the rule in design/specs/rename-conversation/spec.md): 0 nothing;
/// 1 the context figure (gone, not cut to "con…"); 2 the project folded to
/// its mark, its name on the pointer; 3 the project and its slash too. The
/// title keeps at least [`TITLE_MIN`]; ⋯ and the buttons always stay.
///
/// Measured as it is drawn: the room left between the words and the buttons
/// is down to its least when the words were squeezed, and the bar's width
/// when it gave way says when it may show more again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fit {
    pub level: u8,
    at: [f32; 3],
    key: u64,
    /// The bar's width as last drawn.
    pub seen: f32,
}

impl Fit {
    /// The deepest level.
    const ALL: u8 = 3;

    /// One frame's measure: what the bar shows (`key`), its `width`, and
    /// whether its words were `squeezed`. True when the level changed.
    pub fn measure(&mut self, key: u64, width: f32, squeezed: bool) -> bool {
        self.seen = width;
        if key != self.key {
            let was = self.level;
            *self = Fit { key, seen: width, ..Fit::default() };
            return was != 0;
        }
        let before = self.level;
        if squeezed && self.level < Self::ALL {
            self.at[self.level as usize] = width;
            self.level += 1;
        } else if self.level > 0 && width > self.at[self.level as usize - 1] + GROW {
            self.level -= 1;
        }
        self.level != before
    }
}

/// About how wide `text` is drawn at `size` (Latin narrower than CJK): enough
/// to keep a short title from being given room it doesn't need.
fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| if c.is_ascii() { size * 0.6 } else { size * 1.05 }).sum()
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let member = model.selected_member();
    let open = shell.board.popover == Some(Popover::Members);
    let ticket = model.opened.and_then(|id| model.tickets.iter().find(|t| t.id == id));
    let talk_title: Option<String> = match (&model.draft_talk, model.talking()) {
        (Some(_), _) => Some(t!("sidebar.new_conversation").into()),
        (None, Some(lane)) => Some(lane.title.clone().unwrap_or_else(|| t!("sidebar.new_conversation").into())),
        _ => None,
    };
    // The title gives way when the bar is narrow (a panel open beside it):
    // its words end in "…", the engine and the ⌄ stay whole.
    // With only a name and an engine there is nothing to give way: it keeps
    // its width, and what is beside it narrows instead of being drawn over.
    let shrinks = talk_title.is_some() || ticket.is_some();
    let who = member.map(|m| {
        div()
            .relative()
            .flex_shrink(1.)
            .when(shrinks, |d| d.min_w_0())
            .child(
                pill(p, "conversation-title")
                    .role(Role::Button)
                    .aria_label(match (talk_title.clone(), ticket) {
                        (Some(name), _) => name,
                        (None, Some(t)) => format!("{} · {}", m.name, t.title),
                        (None, None) => m.name.clone(),
                    })
                    .aria_expanded(open)
                    .flex_shrink(1.)
                    .when(shrinks, |b| b.min_w_0())
                    .max_w_full()
                    .overflow_hidden()
                    .when(open, |b| b.bg(p.wash))
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_popover(Popover::Members, cx)))
                    .child(match talk_title.clone() {
                        // A conversation with the PM: its own name.
                        Some(name) => div().min_w_0().flex_shrink(1.).max_w(px(320.)).truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(name).into_any_element(),
                        None => title(p, m).into_any_element(),
                    })
                    .children(m.engine.map(|e| engine_label(p, model, m, e)))
                    .children(ticket.map(|t| div().min_w_0().flex_shrink(1.).max_w(px(320.)).truncate().text_size(px(14.)).text_color(p.text2).child(format!("· {}", t.title))))
                    .child(div().flex_none().child(super::sidebar::icon(IconName::ChevronDown, CHEVRON, p.muted))),
            )
            .children(open.then(|| members(shell, cx)))
    });
    let fit = shell.header_fit.get();
    if shell.rename.as_ref().is_some_and(|r| shell.screen != crate::Screen::Chat || model.talk != Some(r.session) || model.draft_talk.is_some()) {
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |shell, cx| shell.rename_left(cx)).ok();
        });
    }
    // Whole, or not at all: the first thing a narrow bar gives up.
    let context = member.and_then(|m| m.context).filter(|_| fit.level < 1).map(|c| {
        div()
            .flex_none()
            .whitespace_nowrap()
            .text_size(px(SMALL))
            .text_color(if c >= 50 { p.text2 } else { p.muted })
            .when(c >= 50, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(t!("header.context", c = c))
    });
    // Who has the Mac's screen: mind it before asking a member for something there.
    let screen = crate::state::screen_line(&model.desktop, &model.squad).map(|line| {
        div()
            .flex_none()
            .px_2()
            .h(px(22.))
            .flex()
            .items_center()
            .rounded(px(11.))
            .bg(p.wash)
            .text_size(px(SMALL))
            .text_color(if model.desktop.stopped { p.muted } else { p.text2 })
            .child(line)
    });
    // A conversation with the PM: where it is, as a path — its project (to
    // its page), then its name — and its ⋯. Who it is with goes without saying.
    let talk = match talk_title.clone() {
        Some(title) => Some(crumb(shell, title, true, fit.level, cx)),
        // The PM's own conversation: the same path, nothing to move or put away.
        None if model.with_pm() => Some(crumb(shell, t!("sidebar.pm_talk").into(), false, fit.level, cx)),
        None => None,
    };
    // What the bar holds: when it changes, what it leaves out is worked out afresh.
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&talk_title, model.talk_project(), &model.focus, member.map(|m| (&m.name, m.context.is_some())), ticket.map(|t| &t.title), crate::state::screen_line(&model.desktop, &model.squad).is_some()).hash(&mut h);
        h.finish()
    };
    let (width, measured) = (std::rc::Rc::new(std::cell::Cell::new(0f32)), shell.header_fit.clone());
    let body = shell.panes.body.entity_id();
    let bar = {
        let width = width.clone();
        canvas(move |b, _, _| width.set(f32::from(b.size.width)), |_, _, _, _| {}).absolute().size_full()
    };
    let gap = crate::ui::title_gap().relative().child(
        canvas(
            move |b, window, cx| {
                let mut fit = measured.get();
                let changed = fit.measure(key, width.get(), f32::from(b.size.width) <= GAP_MIN + 0.5);
                measured.set(fit);
                if changed {
                    window.refresh();
                    cx.defer(move |cx| cx.notify(body));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full(),
    );
    let (who, slice) = match talk {
        Some(_) => (None, None),
        None => (who, Some(slice_control(shell, "chat", cx))),
    };
    h_flex()
        .h(px(HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(12., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .overflow_hidden()
        .sans()
        .window_control_area(WindowControlArea::Drag)
        .child(bar)
        .children(talk)
        .children(who)
        .children(slice)
        .child(gap)
        // The plan window used most, while the sidebar's meters are away.
        .children(super::switching::glance(shell, cx).map(|g| div().min_w_0().flex_shrink(1.).overflow_hidden().child(g)))
        .children(screen)
        .children(context)
        // The buttons stay whole; the words before them give way.
        .children(studio_button(shell, cx))
        .child(div().flex_none().child(terminal_button(shell, cx)))
        .children(screen_button(shell, cx))
        .child(div().flex_none().child(browser_button(shell, cx)))
}

/// The far machine's screen (⌘⇧D): only when the daemon is on another machine.
fn screen_button(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    crate::screen::source(&shell.endpoint)?;
    let p = shell.palette;
    let on = shell.remote_screen.read(cx).open;
    Some(
        div().flex_none().child(
            crate::ui::icon_button(p, "header-screen", crate::icons::Icon::Laptop, t!("screen.header_tip", host = shell.endpoint.label()), Some("⌘⇧D"))
                .selected(on)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_screen(window, cx))),
        ),
    )
}

/// The engine the member runs on now — and, when it moved between engines,
/// its engine history on the pointer: when and why each move, and whether
/// it was mid-ticket.
fn engine_label(p: Palette, model: &Model, member: &Member, engine: equipe_core::protocol::Engine) -> AnyElement {
    let history = model.engine_history(member.session);
    let label = super::switching::label(p, engine);
    if history.is_empty() {
        return label;
    }
    let words = format!("{}\n{}", t!("moved.history"), history.join("\n"));
    crate::tips::tip(div().id("engine-history").flex_none().child(label), p, words, None).into_any_element()
}

/// The designer studio, when the member in view is on a design ticket: its work, watched.
fn studio_button(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let p = shell.palette;
    let ticket = shell.model.opened.or_else(|| shell.model.selected_member().and_then(|m| m.ticket)).filter(|id| shell.studio_offered(*id))?;
    Some(
        div().flex_none().child(
            crate::ui::button(p, "header-studio", crate::ui::Kind::Ghost, t!("studio.open"))
                .icon(crate::icons::Icon::Eye)
                .tip(t!("studio.open_tip"), None)
                .on_click(cx.listener(move |this, _, window, cx| this.open_studio(ticket, window, cx))),
        ),
    )
}

/// The terminal's switch (⌃`): a shell on the daemon's machine.
fn terminal_button(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.terminal.as_ref().is_some_and(|t| t.open);
    super::sidebar::icon_button(p, "header-terminal", IconName::SquareTerminal, true, None)
        .tip(p, t!("term.tip"), Some("⌃`"))
        .when(on, |b| b.bg(p.wash))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx)))
}

/// The browser panel's switch (⌘⇧B), at the title bar's right as Claude's globe.
fn browser_button(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.browser.read(cx).open;
    super::sidebar::icon_button(p, "header-browser", IconName::Globe, true, None)
        .tip(p, t!("header.browser"), Some("⌘⇧B"))
        .when(on, |b| b.bg(p.wash))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_browser(window, cx)))
}

/// The team, to switch the conversation: under the title's ⌄.
fn members(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let rows: Vec<AnyElement> = model
        .team()
        .into_iter()
        .filter_map(|s| model.members.iter().find(|m| m.session == s))
        .map(|m| {
            let session = m.session;
            let on = model.selected == Some(session) && model.opened.is_none();
            h_flex()
                .id(("switch-member", session))
                .h(px(30.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(6.))
                .text_size(px(SMALL))
                .cursor_pointer()
                .when(on, |r| r.bg(p.wash))
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.board.popover = None;
                    this.select_member(session, window, cx);
                }))
                .child(crate::theme::avatar(p, &m.role, &m.name, 18.))
                .child(div().flex_1().child(m.name.clone()))
                .into_any_element()
        })
        .collect();
    deferred(
        v_flex()
            .id("member-list")
            .absolute()
            .top(px(32.))
            .left_0()
            .w(px(220.))
            .p_1()
            .rounded(px(8.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popover_outside(cx)))
            .children(rows),
    )
    .into_any_element()
}

/// A conversation's path: its project (its colour and name, to its page; or
/// "Outside projects"), "/", its name — then its ⋯.
fn crumb(shell: &Shell, title: String, menu: bool, level: u8, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    // The conversation's project; the PM's own is about the one in focus.
    let project = match model.talk_project() {
        Some(project) => project,
        None => model.focus.clone(),
    };
    let names = model.project_names();
    let at = project.as_ref().and_then(|name| names.iter().position(|n| n == name));
    let open = shell.board.popover == Some(Popover::Talk);
    // Narrow, the place folds to its mark (its name on the pointer), then goes.
    let folded = level >= 2;
    let place = match (project.clone(), at) {
        _ if level >= 3 => None,
        (Some(name), Some(_)) => {
            let to = name.clone();
            let tip = name.clone();
            Some(h_flex()
                .id("crumb-project")
                .min_w_0()
                .flex_shrink(1.)
                .overflow_hidden()
                .h(px(PILL))
                .px_2()
                .gap(px(6.))
                .items_center()
                .rounded(px(R_MD))
                .cursor_pointer()
                .text_size(px(13.5))
                .text_color(p.text2)
                .hover(move |s| s.bg(p.wash).text_color(p.text))
                .aria_label(t!("sidebar.open_project"))
                .on_click(cx.listener(move |this, _, window, cx| this.open_project_page(to.clone(), window, cx)))
                .child(div().flex_none().child(crate::theme::project_mark(p, 13.)))
                .when(!folded, |d| d.child(div().min_w_0().max_w(px(200.)).truncate().child(name)))
                .when(folded, |d| d.tip(p, tip, None))
                .into_any_element())
        }
        _ if folded => Some(
            div()
                .id("crumb-outside")
                .flex_none()
                .size(px(PILL))
                .flex()
                .items_center()
                .justify_center()
                .aria_label(t!("sidebar.outside"))
                .tip(p, t!("sidebar.outside"), None)
                .child(div().size(px(7.)).rounded_full().border_1().border_color(p.muted))
                .into_any_element(),
        ),
        _ => Some(div().min_w_0().flex_shrink(1.).truncate().px_2().text_size(px(13.5)).text_color(p.muted).child(t!("sidebar.outside")).into_any_element()),
    };
    let more = div()
        .relative()
        .flex_none()
        .child(
            div()
                .id("talk-menu")
                .size(px(PILL))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_MD))
                .cursor_pointer()
                .when(open, |b| b.bg(p.wash))
                .hover(move |s| s.bg(p.wash))
                .aria_label(t!("header.talk_menu"))
                .aria_expanded(open)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_popover(Popover::Talk, cx)))
                .child(super::sidebar::icon(IconName::Ellipsis, 15., p.text2)),
        )
        .children(open.then(|| talk_menu(shell, project.clone(), cx)));
    // The title keeps its least (or its own width, if shorter) and gives way
    // after the place; ⋯ never shrinks. The path's own least width is what its
    // parts keep, so ⋯ is never drawn over what follows (T-621).
    let least = (text_width(&title, 14.) + 8.).min(TITLE_MIN);
    let slash = place.is_some().then(|| div().flex_none().text_color(p.muted).child("/"));
    let words = div().min_w_0().flex_shrink(1.).truncate().child(title.clone());
    // A conversation of the person's: its title is the way to rename it (T-642).
    let session = model.talk.filter(|_| menu && model.draft_talk.is_none());
    let name: AnyElement = match session {
        Some(session) => {
            let editing = shell.rename.as_ref().is_some_and(|r| r.session == session);
            // Its full name first when it may be cut (long, or the bar narrow), then what a click does.
            let tip = if text_width(&title, 14.) + 8. > 420. || level >= 2 { format!("{title}\n{}", t!("talk.rename")) } else { t!("talk.rename").into() };
            let focus = shell.title_focus.clone();
            div()
                .relative()
                .min_w(px(least + 4.))
                .flex_shrink(0.25)
                .max_w(px(440.))
                .child(
                    h_flex()
                        .id("conversation-title")
                        .track_focus(&focus)
                        .group("conversation-title")
                        .h(px(PILL))
                        .px_1()
                        .gap(px(8.))
                        .items_center()
                        .rounded(px(R_MD))
                        .cursor_pointer()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text)
                        .role(Role::Button)
                        .aria_label(title.clone())
                        .aria_description(t!("talk.rename"))
                        .aria_expanded(editing)
                        .when(editing, |b| b.bg(p.wash))
                        .hover(move |s| s.bg(p.wash))
                        .border_1()
                        .border_color(gpui_kit::transparent_black())
                        .focus_visible(move |s| s.border_color(p.accent))
                        .tip(p, tip, None)
                        .on_click(cx.listener(move |this, _, window, cx| this.rename_from_title(session, window, cx)))
                        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                            if matches!(e.keystroke.key.as_str(), "enter" | "space") && !e.keystroke.modifiers.modified() {
                                cx.stop_propagation();
                                this.rename_from_title(session, window, cx);
                            }
                        }))
                        .child(words)
                        // The pencil shows on the pointer (its room is kept: a hover may not change the layout).
                        .child(div().flex_none().invisible().group_hover("conversation-title", |s| s.visible()).child(crate::icons::icon(crate::icons::Icon::Pencil, 12., p.muted))),
                )
                .children(super::rename::card(shell, shell.header_fit.get().seen, cx).map(|card| div().absolute().top(px(PILL + 6.)).left_0().child(card)))
                .into_any_element()
        }
        None => words.min_w(px(least)).flex_shrink(0.25).max_w(px(420.)).px_1().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).into_any_element(),
    };
    h_flex()
        .flex_shrink(1.)
        .gap_1()
        .items_center()
        .children(place)
        .children(slash)
        .child(name)
        .when(menu, |c| c.child(more))
        .into_any_element()
}

/// A conversation's ⋯: the projects to move it to (where it is, checked),
/// and putting it away.
fn talk_menu(shell: &Shell, current: Option<String>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let item = |id: SharedString, mark: AnyElement, label: String, on: bool| {
        h_flex()
            .id(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(div().w(px(14.)).flex().justify_center().child(mark))
            .child(div().flex_1().min_w_0().truncate().when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(label))
            .when(on, |r| r.child(super::sidebar::icon(IconName::Check, 13., p.text2)))
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    // Renaming comes first (T-642): a conversation begun has a name to change.
    if let Some(session) = model.talk.filter(|_| model.draft_talk.is_none()) {
        rows.push(
            item("talk-rename".into(), crate::icons::icon(crate::icons::Icon::Pencil, 12., p.text2).into_any_element(), t!("talk.rename").into(), false)
                .on_click(cx.listener(move |this, _, window, cx| this.open_rename(session, None, window, cx)))
                .into_any_element(),
        );
        rows.push(div().my_1().h(px(1.)).bg(p.hair).into_any_element());
    }
    rows.push(div().px_2().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).child(t!("header.move_to")).into_any_element());
    let mut targets: Vec<(Option<String>, Option<usize>)> = model.project_names().into_iter().enumerate().map(|(i, n)| (Some(n), Some(i))).collect();
    targets.push((None, None));
    for (target, at) in targets {
        let on = target == current;
        let label = target.clone().unwrap_or_else(|| t!("sidebar.outside").into());
        let mark = match at {
            Some(_) => crate::theme::project_mark(p, 12.),
            None => div().size(px(7.)).rounded_full().border_1().border_color(p.muted).into_any_element(),
        };
        let to = target.clone();
        rows.push(
            item(SharedString::from(format!("move-to-{}", target.as_deref().unwrap_or(""))), mark, label, on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.popover = None;
                    this.move_conversation(to.clone(), cx);
                }))
                .into_any_element(),
        );
    }
    // A conversation not yet begun has nothing to put away.
    if let Some(session) = model.talk.filter(|_| model.draft_talk.is_none()) {
        rows.push(div().my_1().h(px(1.)).bg(p.hair).into_any_element());
        rows.push(
            item("talk-put-away".into(), crate::icons::icon(crate::icons::Icon::X, 12., p.text2).into_any_element(), t!("sidebar.put_away").into(), false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.popover = None;
                    this.put_away_conversation(session, cx);
                }))
                .into_any_element(),
        );
    }
    deferred(
        v_flex()
            .id("talk-menu-list")
            .absolute()
            .top(px(32.))
            .left_0()
            .w(px(240.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p_1()
            .rounded(px(8.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popover_outside(cx)))
            .children(rows),
    )
    .into_any_element()
}

/// The header's controls' shape: the conversation's title and the projects'
/// switch share it, so they sit on one line at one height.
fn pill(p: Palette, id: impl Into<ElementId>) -> Stateful<Div> {
    h_flex()
        .id(id.into())
        .h(px(PILL))
        .pl(px(10.))
        .pr(px(8.))
        .gap(px(6.))
        .flex_none()
        .items_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .cursor_pointer()
}

const PILL: f32 = 28.;
const CHEVRON: f32 = 12.;

/// `◆ pm`, in the member's colour; a member who left is dimmed.
fn title(p: Palette, member: &Member) -> impl IntoElement + use<> {
    let (_, hue) = look(&member.role, &member.name);
    let color = if member.presence == Presence::Closed { p.muted } else { p.member(hue) };
    h_flex()
        .gap_2()
        .items_center()
        .flex_none()
        .text_color(color)
        .child(crate::theme::avatar(p, &member.role, &member.name, 18.))
        .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(member.name.clone()))
}

/// How a project stands: members at work on it, things waiting for the person.
pub fn project_counts(model: &Model, name: Option<&str>) -> (usize, usize) {
    let here = |t: &equipe_core::protocol::Ticket| name.is_none() || model.ticket_project_name(t).as_deref() == name;
    let working = model.tickets.iter().filter(|t| t.status == equipe_core::protocol::TicketStatus::InProgress && here(t)).count();
    let needs = model.tickets.iter().filter(|t| here(t) && (model.section_of(t).is_some())).count();
    (working, needs)
}

/// The project slice: one control, and its list when open.
pub fn slice_control(shell: &Shell, place: &'static str, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let names = model.project_names();
    if names.is_empty() {
        return div().into_any_element();
    }
    // Over a conversation with the PM: the project it is about, which it moves to.
    let talk = if place == "chat" { model.talk_project() } else { None };
    // Nothing slices the whole window any more (a project's page is its
    // own): only a conversation's project, to move it, or the way back from
    // a focus `/project` put on.
    if talk.is_none() && model.focus.is_none() {
        return div().into_any_element();
    }
    let focus = talk.clone().unwrap_or_else(|| model.focus.clone());
    let at = focus.as_ref().and_then(|f| names.iter().position(|n| n == f));
    let open = shell.board.popover == Some(Popover::Projects) && shell.board.slice_place == place;
    let label = focus.clone().unwrap_or_else(|| if talk.is_some() { t!("header.squad_wide").into() } else { t!("header.all_projects").into() });
    // The same pill as the conversation's title beside it: one height, one edge.
    let button = pill(p, SharedString::from(format!("slice-{place}")))
        .role(Role::Button)
        .aria_label(label.clone())
        .aria_expanded(open)
        .flex_shrink(1.)
        .min_w_0()
        .max_w_full()
        .text_size(px(13.))
        .when(open, |b| b.bg(p.wash))
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, _, window, cx| {
            let just_closed = this.popover_just_closed(Popover::Projects);
            let opening = !(this.board.popover == Some(Popover::Projects) && this.board.slice_place == place) && !just_closed;
            this.board.popover = opening.then_some(Popover::Projects);
            this.board.slice_place = place;
            if opening {
                this.board.adding_project = false;
                this.board.slice_search.update(cx, |s, cx| {
                    s.set_value(String::new(), window, cx);
                    s.set_placeholder(t!("header.find_project"), window, cx);
                    s.focus(window, cx);
                });
            }
            cx.notify();
        }))
        .when(at.is_some(), |b| b.child(crate::theme::project_mark(p, 12.)))
        .child(div().min_w_0().flex_shrink(1.).max_w(px(160.)).truncate().text_color(if focus.is_some() { p.text } else { p.text2 }).child(label))
        .child(div().flex_none().child(super::sidebar::icon(IconName::ChevronDown, CHEVRON, p.muted)));
    // It gives way too, after the title has: the title's words matter more.
    div().relative().flex_shrink(0.4).min_w(px(64.)).child(button).children(open.then(|| list(shell, cx))).into_any_element()
}

/// The project list and its "Add a project…" (or the path box, adding), as
/// a popover under whatever opened it.
pub(crate) fn list(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let names = model.project_names();
    let adding = shell.board.adding_project;
    let typed = if adding { String::new() } else { shell.board.slice_search.read(cx).value().trim().to_lowercase() };
    let paths: std::collections::HashMap<String, String> = model.projects.iter().map(|p| (equipe_core::board::project_name(p), super::settings::short_path(p))).collect();
    let row = |id: SharedString, color: Option<Rgba>, label: String, counts: (usize, usize), key: Option<&'static str>, on: bool, target: Option<String>, cx: &mut Context<Shell>| {
        h_flex()
            .id(id)
            .min_h(px(30.))
            .py_1()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .when(on, |r| r.bg(p.wash))
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.board.popover = None;
                if this.board.slice_place == "chat" && this.model.talk_project().is_some() {
                    this.move_conversation(target.clone(), cx);
                } else {
                    this.focus_project(target.clone(), window, cx);
                }
                cx.notify();
            }))
            .child(div().w(px(14.)).flex().justify_center().children(color.map(|_| crate::theme::project_mark(p, 12.))))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(label.clone()))
                    // Where it is, on the daemon's machine.
                    .children(paths.get(&label).filter(|_| color.is_some()).map(|path| div().truncate().text_size(px(11.)).text_color(p.muted).child(path.clone())))
                    // All of the squad: also what belongs to no project.
                    .when(color.is_none(), |d| d.child(div().truncate().text_size(px(11.)).text_color(p.muted).child(t!("header.all_projects_note")))),
            )
            .when(counts.0 > 0, |r| r.child(h_flex().gap_1().items_center().text_size(px(12.)).text_color(p.amber).child(dot(p.amber, 5.)).child(counts.0.to_string())))
            .when(counts.1 > 0, |r| r.child(h_flex().gap_1().items_center().text_size(px(12.)).text_color(p.accent).child(crate::icons::icon(crate::icons::Icon::Flag, 11., p.accent)).child(counts.1.to_string())))
            .children(key.map(|k| div().w(px(26.)).text_align(TextAlign::Right).text_size(px(11.)).text_color(p.muted).font_features(key_face()).child(k)))
    };
    let keys = ["⌃1", "⌃2", "⌃3", "⌃4", "⌃5", "⌃6", "⌃7", "⌃8", "⌃9"];
    let talk = if shell.board.slice_place == "chat" { model.talk_project() } else { None };
    let current = talk.clone().unwrap_or_else(|| model.focus.clone());
    let all = if talk.is_some() { t!("header.squad_wide") } else { t!("header.all_projects") };
    let mut rows: Vec<AnyElement> = vec![row("slice-all".into(), None, all.into(), project_counts(model, None), Some("⌃0"), current.is_none(), None, cx).into_any_element()];
    for (i, name) in names.iter().enumerate() {
        if !typed.is_empty() && !name.to_lowercase().contains(&typed) {
            continue;
        }
        rows.push(
            row(SharedString::from(format!("slice-{name}")), Some(p.muted), name.clone(), project_counts(model, Some(name)), keys.get(i).copied(), current.as_deref() == Some(name.as_str()), Some(name.clone()), cx)
                .into_any_element(),
        );
    }
    // Adding a project: the system's folder picker on this Mac; for another
    // machine, its path typed in the box above.
    let local = !shell.endpoint.is_remote();
    let add = h_flex()
        .id("slice-add")
        .h(px(30.))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .text_color(p.text2)
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .child(div().w(px(10.)).flex().justify_center().child(super::sidebar::icon(IconName::Plus, 12., p.muted)))
        .child(div().flex_1().child(if adding { t!("header.add_project_typed") } else { t!("header.add_project") }))
        .on_click(cx.listener(move |this, _, window, cx| {
            if local && !this.board.adding_project {
                this.pick_project_folder(cx);
            } else {
                this.start_adding_project(window, cx);
            }
        }));
    let show_box = adding || names.len() >= SEARCH_FROM;
    deferred(
        v_flex()
            .id("slice-list")
            .absolute()
            .top(px(PILL + 4.))
            .left_0()
            .w(px(320.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p_1()
            .rounded(px(8.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popover_outside(cx)))
            .when(show_box, |c| {
                c.child(h_flex().mx_1().mb_1().px_2().h(px(28.)).items_center().rounded(px(6.)).bg(p.code_bg).border_1().border_color(p.hair).text_size(px(SMALL)).child(Input::new(&shell.board.slice_search).appearance(false).bordered(false)))
            })
            .when(!adding, |c| c.children(rows))
            .when(adding, |c| c.child(div().px_2().py_1().text_size(px(11.)).text_color(p.muted).child(if local { t!("header.add_project_hint_here") } else { t!("header.add_project_hint_there") })))
            .child(div().mx_1().my_1().h(px(1.)).bg(p.hair))
            .child(add)
            .when(!adding, |c| c.child(h_flex().px_2().pt_1().gap_1().items_center().text_size(px(11.)).text_color(p.muted).child(kbd(p, "⌃0–9")).child(t!("header.slice_hint")))),
    )
    .into_any_element()
}

impl Shell {
    /// The project list's box takes a path: a folder on the daemon's machine.
    pub fn start_adding_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.board.adding_project = true;
        self.board.slice_search.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.set_placeholder(t!("header.project_path_placeholder"), window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// On this Mac: the system's folder picker, then added.
    pub fn pick_project_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t!("header.add_project_prompt").into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| this.add_project(path.display().to_string(), cx)).ok();
            }
        })
        .detach();
    }

    /// Ask the daemon to add `path` as a project of the squad shown; the list
    /// comes back with it, or the reason it wasn't.
    pub fn add_project(&mut self, path: String, cx: &mut Context<Self>) {
        self.board.adding_project = false;
        self.board.popover = None;
        self.send_command(equipe_core::protocol::Command::AddProject { squad: self.model.squad.clone(), path: path.clone().into() });
        self.composer.flash = Some(t!("header.adding_project", path = path));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{Fit, GROW};

    #[test]
    fn a_narrow_bar_gives_way_a_step_a_frame_and_takes_it_back_only_once_wider() {
        let mut fit = Fit::default();
        // The first frame learns what the bar shows; nothing to give up yet.
        assert!(!fit.measure(7, 1200., false));
        assert_eq!(fit.level, 0);
        // Squeezed at 300: the context, then the project's name, then the project go, one frame each.
        assert!(fit.measure(7, 300., true));
        assert!(fit.measure(7, 300., true));
        assert!(fit.measure(7, 300., true));
        assert_eq!(fit.level, 3);
        // Nothing further to give: it stays, squeezed or not.
        assert!(!fit.measure(7, 300., true));
        assert!(!fit.measure(7, 300. + GROW, false));
        // Wider than where it last gave way: one step back.
        assert!(fit.measure(7, 300. + GROW + 1., false));
        assert_eq!(fit.level, 2);
        // Still squeezed there: the step again, with the new width remembered — no flicker after.
        assert!(fit.measure(7, 330., true));
        assert_eq!(fit.level, 3);
        assert!(!fit.measure(7, 330., false));
    }

    #[test]
    fn a_new_title_or_place_starts_the_bar_over() {
        let mut fit = Fit::default();
        fit.measure(1, 900., false);
        fit.measure(1, 400., true);
        assert_eq!(fit.level, 1);
        // Another conversation: everything shown again, then measured afresh.
        assert!(fit.measure(2, 400., true), "redrawn with everything");
        assert_eq!(fit.level, 0);
        assert!(fit.measure(2, 400., true));
        assert_eq!(fit.level, 1);
    }

    #[test]
    fn a_short_title_keeps_its_own_width_a_long_one_the_least() {
        assert!(super::text_width("PM", 14.) + 8. < super::TITLE_MIN);
        assert!(super::text_width("Tidy the checkout page copy", 14.) > super::TITLE_MIN);
        assert!(super::text_width("결제 화면 문구 정리", 14.) > super::text_width("abcdefghij", 14.));
    }
}
