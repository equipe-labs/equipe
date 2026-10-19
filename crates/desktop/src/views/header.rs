//! The conversation's header, which is the window's title bar on the right:
//! the conversation's name with ⌄ (the team, to switch), a small chip for the
//! project slice — opening a list with what works and what waits in each,
//! ⌃0–9 — and the context fill. The title bar's fixed buttons float over its
//! left with the sidebar away; its content slides beside them.

use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{Popover, dot, kbd};
use crate::state::{Member, Model, Presence};
use crate::theme::{PROJECT_HUES, Palette, R_MD, SANS, SMALL, look};
use crate::Shell;

/// The title bar's one height, the sidebar's strip's too.
pub const HEIGHT: f32 = super::sidebar::STRIP;
/// Past this many projects the list offers a search.
const SEARCH_FROM: usize = 7;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let member = model.selected_member();
    let open = shell.board.popover == Some(Popover::Members);
    let ticket = model.opened.and_then(|id| model.tickets.iter().find(|t| t.id == id));
    let who = member.map(|m| {
        div()
            .relative()
            .flex_none()
            .child(
                pill(p, "conversation-title")
                    .when(open, |b| b.bg(p.wash))
                    .hover(move |s| s.bg(p.wash))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.board.popover = if this.board.popover == Some(Popover::Members) { None } else { Some(Popover::Members) };
                        cx.notify();
                    }))
                    .child(title(p, m))
                    .children(m.engine.map(|e| super::switching::label(p, e)))
                    .children(ticket.map(|t| div().max_w(px(320.)).truncate().text_size(px(14.)).text_color(p.text2).child(format!("· {}", t.title))))
                    .child(super::sidebar::icon(IconName::ChevronDown, CHEVRON, p.muted)),
            )
            .children(open.then(|| members(shell, cx)))
    });
    let context = member.and_then(|m| m.context).map(|c| {
        div()
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
    h_flex()
        .h(px(HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(12., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .font_family(SANS)
        .window_control_area(WindowControlArea::Drag)
        .children(who)
        .child(slice_control(shell, "chat", cx))
        .child(div().flex_1())
        .children(screen)
        .children(context)
        .child(terminal_button(shell, cx))
        .child(browser_button(shell, cx))
}

/// The terminal's switch (⌃`): a shell on the daemon's machine.
fn terminal_button(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.terminal.as_ref().is_some_and(|t| t.open);
    super::sidebar::icon_button(p, "header-terminal", IconName::SquareTerminal, true, Some(format!("{} ⌃`", t!("term.tip"))))
        .when(on, |b| b.bg(p.wash))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx)))
}

/// The browser panel's switch (⌘⇧B), at the title bar's right as Claude's globe.
fn browser_button(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let on = shell.browser.read(cx).open;
    super::sidebar::icon_button(p, "header-browser", IconName::Globe, true, Some(format!("{} ⌘⇧B", t!("header.browser"))))
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
            let (mark, hue) = look(&m.role, &m.name);
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
                .child(div().w(px(16.)).text_center().text_color(p.member(hue)).child(mark))
                .child(div().flex_1().child(m.name.clone()))
                .into_any_element()
        })
        .collect();
    deferred(v_flex().id("member-list").absolute().top(px(32.)).left_0().w(px(220.)).p_1().rounded(px(8.)).bg(p.raised).border_1().border_color(p.hair).shadow_lg().occlude().children(rows)).into_any_element()
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
    let (mark, hue) = look(&member.role, &member.name);
    let color = if member.presence == Presence::Closed { p.muted } else { p.member(hue) };
    h_flex()
        .gap_2()
        .items_center()
        .flex_none()
        .text_color(color)
        .child(div().text_size(px(11.)).child(mark))
        .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(member.name.clone()))
}

/// A project's colour, by its place in the squad.
pub fn project_color(p: Palette, at: usize) -> Rgba {
    p.member(PROJECT_HUES[at % PROJECT_HUES.len()])
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
    let focus = model.focus.clone();
    let at = focus.as_ref().and_then(|f| names.iter().position(|n| n == f));
    let open = shell.board.popover == Some(Popover::Projects) && shell.board.slice_place == place;
    let label = focus.clone().unwrap_or_else(|| t!("header.all_projects").into());
    // The same pill as the conversation's title beside it: one height, one edge.
    let button = pill(p, SharedString::from(format!("slice-{place}")))
        .text_size(px(13.))
        .when(open, |b| b.bg(p.wash))
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, _, window, cx| {
            let opening = !(this.board.popover == Some(Popover::Projects) && this.board.slice_place == place);
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
        .when_some(at, |b, at| b.child(dot(project_color(p, at), 7.)))
        .child(div().max_w(px(160.)).truncate().text_color(if focus.is_some() { p.text } else { p.text2 }).child(label))
        .child(super::sidebar::icon(IconName::ChevronDown, CHEVRON, p.muted));
    div().relative().flex_none().child(button).children(open.then(|| list(shell, cx))).into_any_element()
}

fn list(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
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
                this.focus_project(target.clone(), window, cx);
                cx.notify();
            }))
            .child(div().w(px(10.)).flex().justify_center().children(color.map(|c| dot(c, 7.))))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().when(on, |d| d.font_weight(FontWeight::SEMIBOLD)).child(label.clone()))
                    // Where it is, on the daemon's machine.
                    .children(paths.get(&label).filter(|_| color.is_some()).map(|path| div().truncate().text_size(px(11.)).text_color(p.muted).child(path.clone()))),
            )
            .when(counts.0 > 0, |r| r.child(h_flex().gap_1().items_center().text_size(px(12.)).text_color(p.amber).child(dot(p.amber, 5.)).child(counts.0.to_string())))
            .when(counts.1 > 0, |r| r.child(div().text_size(px(12.)).text_color(p.accent).child(format!("⚑ {}", counts.1))))
            .children(key.map(|k| div().w(px(26.)).text_align(TextAlign::Right).text_size(px(11.)).text_color(p.muted).child(k)))
    };
    let keys = ["⌃1", "⌃2", "⌃3", "⌃4", "⌃5", "⌃6", "⌃7", "⌃8", "⌃9"];
    let mut rows: Vec<AnyElement> = vec![row("slice-all".into(), None, t!("header.all_projects").into(), project_counts(model, None), Some("⌃0"), model.focus.is_none(), None, cx).into_any_element()];
    for (i, name) in names.iter().enumerate() {
        if !typed.is_empty() && !name.to_lowercase().contains(&typed) {
            continue;
        }
        rows.push(
            row(SharedString::from(format!("slice-{name}")), Some(project_color(p, i)), name.clone(), project_counts(model, Some(name)), keys.get(i).copied(), model.focus.as_deref() == Some(name.as_str()), Some(name.clone()), cx)
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
