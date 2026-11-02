//! A project's page, where its name in the sidebar leads: its conversations
//! with the PM (whose turn it is) and its work (what waits on the person
//! first), in two tabs, with a new conversation about it one press away.
//! Nothing here filters the rest of the window: the page is the project's.

use equipe_core::protocol::TicketId;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::board::{ProjectTab, WorkPart, project_talks, project_work};
use crate::icons::Icon;
use crate::state::{Lane, Presence, Side, status_word};
use crate::theme::{MONO, R_MD, SMALL};
use crate::{Screen, Shell, ui};

/// The page's column, at its widest.
const COLUMN: f32 = 760.;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let name = shell.board.project_page.clone().unwrap_or_default();
    let names = model.project_names();
    let at = names.iter().position(|n| *n == name);
    let hq = model.is_hq(&name);
    let folder = model.projects.iter().find(|path| equipe_core::board::project_name(path) == name).map(|path| super::settings::short_path(path));
    let ident = match at {
        Some(_) if hq => super::sidebar::icon(IconName::Building2, 15., p.text2).into_any_element(),
        Some(at) => super::board::dot(super::header::project_color(p, at), 10.).into_any_element(),
        None => super::sidebar::icon(IconName::Folder, 15., p.muted).into_any_element(),
    };
    let new_in = name.clone();
    let header = crate::ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().flex_none().w(px(16.)).flex().justify_center().child(ident))
        .child(div().flex_none().max_w(px(280.)).truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(name.clone()))
        .when(hq, |h| h.child(super::sidebar::hq_badge(p)))
        .children(folder.map(|f| div().min_w_0().flex_shrink(1.).truncate().text_size(px(SMALL)).text_color(p.muted).child(f)))
        .child(div().flex_1())
        .when(at.is_some(), |h| {
            h.child(ui::button(p, "project-new-talk", ui::Kind::Primary, t!("project.new_talk")).icon(Icon::Plus).on_click(cx.listener(move |this, _, window, cx| this.start_conversation(Some(new_in.clone()), window, cx))))
        });
    let Some(_) = at else {
        return page(shell, header, div().py_8().text_size(px(13.)).text_color(p.muted).child(t!("project.gone")).into_any_element());
    };

    let (yours, answering) = project_talks(model, &name);
    let work = project_work(model, &name);
    let needs = work.iter().find(|(w, _)| *w == WorkPart::NeedsYou).map_or(0, |(_, ids)| ids.len());
    let tab = shell.board.project_tab;
    let tab_button = |id: &'static str, which: ProjectTab, label: String, count: usize, hot: bool, cx: &mut Context<Shell>| {
        let on = tab == which;
        h_flex()
            .id(id)
            .role(Role::Tab)
            .aria_selected(on)
            .aria_label(label.clone())
            .h(px(36.))
            .px_1()
            .gap(px(6.))
            .items_center()
            .cursor_pointer()
            .border_b_2()
            .border_color(if on { p.text } else { gpui_kit::transparent_black().into() })
            .text_size(px(14.))
            .text_color(if on { p.text } else { p.text2 })
            .when(on, |t| t.font_weight(FontWeight::MEDIUM))
            .hover(move |s| s.text_color(p.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.project_tab = which;
                cx.notify();
            }))
            .child(label)
            .when(count > 0, |t| {
                t.child(
                    div()
                        .min_w(px(18.))
                        .h(px(18.))
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .when(hot, |b| b.bg(p.orange).text_color(p.on_accent))
                        .when(!hot, |b| b.bg(p.wash).text_color(p.text2))
                        .child(count.to_string()),
                )
            })
    };
    let talks_count = yours.len() + answering.len();
    let work_count = work.iter().filter(|(w, _)| *w != WorkPart::Done).map(|(_, ids)| ids.len()).sum::<usize>();
    let tabs = h_flex()
        .w_full()
        .gap_5()
        .border_b_1()
        .border_color(p.hair)
        .child(tab_button("project-tab-talks", ProjectTab::Talks, t!("project.tab_talks").into(), talks_count, false, cx))
        .child(tab_button("project-tab-work", ProjectTab::Work, t!("project.tab_work").into(), if needs > 0 { needs } else { work_count }, needs > 0, cx));
    let body = match tab {
        ProjectTab::Talks => talks(shell, &name, &yours, &answering, cx),
        ProjectTab::Work => work_list(shell, &work, cx),
    };
    page(shell, header, v_flex().w_full().gap_4().child(tabs).child(body).into_any_element())
}

fn page(shell: &Shell, header: impl IntoElement, column: AnyElement) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("project-page")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        // The scroller spans the screen, so its bar is at the window's edge.
        .child(div().id("project-scroll").flex_1().min_h_0().overflow_y_scroll().child(h_flex().w_full().justify_center().child(v_flex().w_full().max_w(px(COLUMN)).px_6().pt_3().pb_10().child(column))))
        .into_any_element()
}

/// A part's heading: small, quiet — orange where it waits on the person.
fn heading(shell: &Shell, label: String, count: usize, hot: bool) -> Div {
    let p = shell.palette;
    h_flex()
        .pt_2()
        .pb_1()
        .gap_1p5()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if hot { p.orange } else { p.muted })
        .child(label)
        .child(div().font_weight(FontWeight::NORMAL).child(count.to_string()))
}

/// Nothing yet: what will gather here, and how it starts.
fn empty(shell: &Shell, title: String, text: String, project: Option<String>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .w_full()
        .py_10()
        .gap_1p5()
        .items_center()
        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(title))
        .child(div().max_w(px(420.)).text_center().text_size(px(13.)).line_height(px(20.)).text_color(p.muted).child(text))
        .children(project.map(|project| {
            div().pt_3().child(ui::button(p, "project-empty-new", ui::Kind::Secondary, t!("project.new_talk")).icon(Icon::Plus).on_click(cx.listener(move |this, _, window, cx| this.start_conversation(Some(project.clone()), window, cx))))
        }))
        .into_any_element()
}

fn talks(shell: &Shell, project: &str, yours: &[&Lane], answering: &[&Lane], cx: &mut Context<Shell>) -> AnyElement {
    if yours.is_empty() && answering.is_empty() {
        return empty(shell, t!("project.talks_empty_title").into(), t!("project.talks_empty").into(), Some(project.to_owned()), cx);
    }
    let mut out = v_flex().w_full().gap_0p5();
    for (label, lanes) in [(t!("project.talks_yours"), yours), (t!("project.talks_answering"), answering)] {
        if lanes.is_empty() {
            continue;
        }
        out = out.child(heading(shell, label.into(), lanes.len(), false)).children(lanes.iter().map(|lane| talk_row(shell, lane, cx)));
    }
    out.into_any_element()
}

/// A conversation: a spinner while the PM answers, its first words, when it last moved.
fn talk_row(shell: &Shell, lane: &Lane, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = lane.session;
    let working = lane.presence == Presence::Working;
    let mark = if working {
        super::sidebar::icon(IconName::LoaderCircle, 14., p.text2).rotate(radians(shell.spin(cx))).into_any_element()
    } else {
        crate::icons::icon(Icon::Message, 14., p.text2).into_any_element()
    };
    let title = lane.title.clone().unwrap_or_else(|| t!("sidebar.new_conversation").into());
    let when = (lane.active > 0 && shell.model.unix >= lane.active).then(|| crate::activity::ago(shell.model.unix - lane.active));
    h_flex()
        .id(("project-talk", id as usize))
        .w_full()
        .h(px(36.))
        .px_2()
        .ml(px(-8.))
        .gap_3()
        .items_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .aria_label(title.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Conversation(id), window, cx) {
                this.open_side(Side::Conversation(id), window, cx);
            }
        }))
        .child(div().w(px(16.)).flex_none().flex().justify_center().child(mark))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(p.text).child(title))
        .children(when.map(|w| div().flex_none().text_size(px(12.)).text_color(p.muted).child(w)))
        .into_any_element()
}

fn work_list(shell: &Shell, work: &[(WorkPart, Vec<TicketId>)], cx: &mut Context<Shell>) -> AnyElement {
    if work.is_empty() {
        return empty(shell, t!("project.work_empty_title").into(), t!("project.work_empty").into(), None, cx);
    }
    let mut out = v_flex().w_full().gap_0p5();
    for (part, ids) in work {
        let label = match part {
            WorkPart::NeedsYou => t!("project.work_needs"),
            WorkPart::Working => t!("project.work_working"),
            WorkPart::Next => t!("project.work_next"),
            WorkPart::Done => t!("project.work_done"),
        };
        let hot = *part == WorkPart::NeedsYou;
        out = out.child(heading(shell, label.into(), ids.len(), hot)).children(ids.iter().map(|id| ticket_row(shell, *id, hot, cx)));
    }
    out.into_any_element()
}

/// A ticket: its number, its title, who is on it or how it stands.
fn ticket_row(shell: &Shell, id: TicketId, hot: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let Some(t) = model.tickets.iter().find(|t| t.id == id) else { return div().into_any_element() };
    let who = model.member_on(id).map(|m| m.name.clone()).or_else(|| t.owner.clone());
    // Waiting on the person: why — a request, a hand-in, a proposal — not its status.
    let asks = hot && model.approvals.open_items().any(|a| a.ticket == Some(id));
    let state = match model.section_of(t) {
        _ if asks => t!("state.section_permission"),
        Some(section) if hot => section.label(),
        _ => status_word(t.status),
    };
    let meta = match who {
        Some(who) if !t.status.is_closed() => format!("{who} · {state}"),
        _ => state.to_owned(),
    };
    h_flex()
        .id(("project-ticket", id as usize))
        .w_full()
        .h(px(36.))
        .px_2()
        .ml(px(-8.))
        .gap_3()
        .items_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .aria_label(t.title.clone())
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) {
                this.open_ticket_row(id, window, cx);
            }
        }))
        .child(div().w(px(48.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(if hot { p.orange } else { p.muted }).child(format!("T-{id}")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(if t.status.is_closed() { p.text2 } else { p.text }).child(t.title.clone()))
        .child(div().flex_none().max_w(px(200.)).truncate().text_size(px(12.)).text_color(p.muted).child(meta))
        .into_any_element()
}

impl Shell {
    /// A project's page, from its name in the sidebar (or ⌃1–⌃9, 현황).
    pub fn open_project_page(&mut self, name: String, _window: &mut Window, cx: &mut Context<Self>) {
        // Another project: its conversations first — its work when it has none.
        if self.board.project_page.as_deref() != Some(name.as_str()) {
            let (yours, answering) = project_talks(&self.model, &name);
            self.board.project_tab = if yours.is_empty() && answering.is_empty() && !project_work(&self.model, &name).is_empty() { ProjectTab::Work } else { ProjectTab::Talks };
        }
        self.board.project_page = Some(name);
        self.show(Screen::Project, cx);
        cx.notify();
    }
}
