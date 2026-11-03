//! 진행 상황, where 작업 중 leads: since the person last looked (or today),
//! how many finished, how many need a look, how many are under way — then
//! the work that needs a look, in red, first; the rest under way by project,
//! each with who is on it, the step it is at and when it is expected done;
//! and what finished, on main or still on its branch.

use equipe_core::protocol::{Ticket, TicketId};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::board::{Progress, Risk, progress};
use crate::state::{local_midnight, short_span};
use crate::theme::{MONO, R_LG, R_MD, SMALL};
use crate::{Screen, Shell, ui};

/// The page's column, at its widest.
const COLUMN: f32 = 820.;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let today = shell.board.progress_today;
    let since = if today { local_midnight(model.unix) } else { shell.board.progress_since };
    let Progress { risks, working, done } = progress(model, since);
    let under_way = risks.len() + working.iter().map(|(_, ids)| ids.len()).sum::<usize>();
    let needs = model.needs_you_count();

    let seg = |id: &'static str, label: &'static str, on: bool, today: bool, cx: &mut Context<Shell>| {
        div()
            .id(id)
            .px_2()
            .h(px(24.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(if on { p.text } else { p.text2 })
            .when(on, |d| d.bg(p.wash))
            .role(Role::Tab)
            .aria_selected(on)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.progress_today = today;
                cx.notify();
            }))
            .child(label)
    };
    let header = ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_3()
        .pl(px(shell.header_inset(16., cx)))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("progress.title")))
        .child(div().flex_1())
        .child(
            h_flex()
                .p(px(2.))
                .gap(px(2.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.hair)
                .child(seg("progress-since", t!("progress.since_last"), !today, false, cx))
                .child(seg("progress-today", t!("progress.today"), today, true, cx)),
        );

    // How many, at a glance; 확인 필요 leads to its list.
    let tile = |id: &'static str, n: usize, label: &'static str, hot: Option<Rgba>| {
        v_flex()
            .id(id)
            .min_w(px(120.))
            .px_3()
            .py_2()
            .gap(px(2.))
            .rounded(px(R_MD))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(hot.filter(|_| n > 0).unwrap_or(p.text)).child(n.to_string()))
            .child(div().text_size(px(12.)).text_color(p.text2).child(label))
    };
    let tiles = h_flex()
        .w_full()
        .flex_wrap()
        .gap_2()
        .child(tile("progress-done", done.len(), t!("progress.chip_done"), None))
        .child(tile("progress-risks", risks.len(), t!("progress.chip_risks"), Some(p.danger())))
        .child(tile("progress-working", under_way, t!("progress.chip_working"), None))
        .child(tile("progress-needs", needs, t!("progress.chip_needs"), Some(p.orange)).cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(|this, _, _, cx| this.open_side_menu(crate::board::Menu::Inbox, cx))));

    let mut column = v_flex().w_full().gap_5().child(tiles);
    if risks.is_empty() && working.is_empty() && done.is_empty() {
        column = column.child(
            v_flex()
                .w_full()
                .py_10()
                .gap_1p5()
                .items_center()
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(t!("progress.empty_title")))
                .child(div().max_w(px(420.)).text_center().text_size(px(13.)).text_color(p.muted).child(t!("progress.empty"))),
        );
    }
    if !risks.is_empty() {
        let rows = risks.iter().filter_map(|(id, r)| model.tickets.iter().find(|t| t.id == *id).map(|t| row(shell, t, Some(*r), cx)));
        column = column.child(
            v_flex()
                .w_full()
                .gap_1()
                .child(heading(shell, Some(super::board::dot(p.danger(), 7.).into_any_element()), t!("progress.risks").into(), risks.len()))
                .child(v_flex().w_full().p_1().rounded(px(R_LG)).bg(p.danger_bg()).children(rows)),
        );
    }
    for (project, ids) in &working {
        let mark = project.is_some().then(|| crate::theme::project_mark(p, 12.));
        let label = project.clone().unwrap_or_else(|| t!("sidebar.outside").into());
        let rows = ids.iter().filter_map(|id| model.tickets.iter().find(|t| t.id == *id).map(|t| row(shell, t, None, cx)));
        column = column.child(v_flex().w_full().gap_1().child(heading(shell, mark, label, ids.len())).children(rows));
    }
    if !done.is_empty() {
        let rows = done.iter().filter_map(|id| model.tickets.iter().find(|t| t.id == *id).map(|t| done_row(shell, t, cx)));
        column = column.child(v_flex().w_full().gap_1().child(heading(shell, None, t!("progress.done").into(), done.len())).children(rows));
    }

    // The retros gone by: a day and a line each, opening the PM's conversation there.
    let retros = shell.model.pm().and_then(|pm| model.convos.get(&pm)).map(|c| crate::retro::past(c.transcript.entries())).unwrap_or_default();
    if !retros.is_empty() {
        let rows = retros.iter().enumerate().map(|(i, r)| retro_row(shell, i, r, cx));
        column = column.child(v_flex().w_full().gap_1().child(heading(shell, None, t!("progress.retros").into(), retros.len())).children(rows));
    }

    // What runs on its own, on a schedule: the routines, below the work.
    column = column.child(super::routines::section(shell, cx));
    v_flex()
        .id("progress-page")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        .child(div().id("progress-scroll").flex_1().min_h_0().overflow_y_scroll().child(h_flex().w_full().justify_center().child(v_flex().w_full().max_w(px(COLUMN)).px_6().pt_4().pb_10().child(column))))
        .into_any_element()
}

/// A part's heading: its mark (a project's folder, red for what needs a look), its name, how many.
fn heading(shell: &Shell, mark: Option<AnyElement>, label: String, count: usize) -> Div {
    let p = shell.palette;
    h_flex()
        .gap_1p5()
        .items_center()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text2)
        .children(mark)
        .child(label)
        .child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(count.to_string()))
}

/// A ticket under way: its number, title, who is on it and the step it is
/// at — then why it needs a look, or when it is expected done.
fn row(shell: &Shell, t: &Ticket, risk: Option<Risk>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let id = t.id;
    let member = model.member_on(id);
    let step = member.and_then(|m| crate::board::step_words(m.estimate.as_ref().and_then(|e| e.what.as_deref()), m.target.as_deref()));
    let right = match risk {
        Some(Risk::Failed) => t!("progress.failed").to_owned(),
        Some(Risk::Stuck(s)) => t!("progress.stuck", span = short_span(s)),
        Some(Risk::Late(s)) => t!("progress.late", span = short_span(s)),
        None => match model.eta(t) {
            Some(eta) if eta.at > model.unix => t!("progress.eta", at = super::board::eta_text(eta.at, eta.guessed), left = short_span(eta.at - model.unix)),
            _ => String::new(),
        },
    };
    let hot = risk.is_some();
    let who = member.map(|m| {
        h_flex()
            .flex_none()
            .gap(px(5.))
            .items_center()
            .child(crate::theme::avatar(p, &m.role, &m.name, 15.))
            .child(div().text_color(p.text2).child(m.name.clone()))
    });
    base(shell, id, cx)
        .child(div().w(px(52.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(if hot { p.danger() } else { p.muted }).child(format!("T-{id}")))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_size(px(14.)).text_color(p.text).child(t.title.clone()))
                .child(h_flex().gap_2().items_center().text_size(px(12.)).children(who).children(step.map(|s| div().min_w_0().truncate().text_color(p.muted).child(s)))),
        )
        .child(div().flex_none().max_w(px(220.)).truncate().text_size(px(12.)).text_color(if hot { p.danger() } else { p.text2 }).child(right))
        .into_any_element()
}

/// A ticket finished: on main, or still on its branch (for the person to merge).
fn done_row(shell: &Shell, t: &Ticket, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let waits = t.waits_to_merge();
    base(shell, t.id, cx)
        .child(div().w(px(52.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(format!("T-{}", t.id)))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(p.text2).child(t.title.clone()))
        .child(div().flex_none().text_size(px(12.)).text_color(if waits { p.orange } else { p.muted }).child(if waits { t!("progress.on_branch") } else { t!("progress.on_main") }))
        .into_any_element()
}

/// A retro gone by: the day its week runs up to, and a line of what it said.
fn retro_row(shell: &Shell, i: usize, past: &crate::retro::Past, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let to = past.to.clone();
    h_flex()
        .id(("progress-retro", i))
        .w_full()
        .min_h(px(36.))
        .px_2()
        .py_1()
        .gap_3()
        .items_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .text_size(px(SMALL))
        .role(Role::Button)
        .aria_label(format!("{} · {}", past.to, past.line))
        .child(div().w(px(88.)).flex_none().font_family(MONO).text_size(px(12.)).text_color(p.muted).child(past.to.clone()))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(p.text2).child(past.line.clone()))
        .on_click(cx.listener(move |this, _, window, cx| this.open_retro(&to, window, cx)))
        .into_any_element()
}

fn base(shell: &Shell, id: TicketId, cx: &mut Context<Shell>) -> Stateful<Div> {
    let p = shell.palette;
    h_flex()
        .id(("progress-row", id as usize))
        .w_full()
        .min_h(px(44.))
        .px_2()
        .py_1()
        .gap_3()
        .items_center()
        .rounded(px(R_MD))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .text_size(px(SMALL))
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if !this.open_elsewhere(e, super::elsewhere::Goal::Ticket(id), window, cx) {
                this.open_ticket_row(id, window, cx);
            }
        }))
}

impl Shell {
    /// A past retro's row: the PM's conversation, at the retro's run.
    pub fn open_retro(&mut self, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pm) = self.model.pm() else { return };
        self.select_member(pm, window, cx);
        self.chat_sync(cx);
        if let Some(row) = crate::retro::row_of(&self.chat.blocks, to) {
            self.chat.scroll_to(row, cx);
        }
        cx.notify();
    }

    /// 진행 상황, from 작업 중: "since you last looked" means since the last
    /// time it was opened on this Mac (the start of today, the first time).
    pub fn open_progress(&mut self, cx: &mut Context<Self>) {
        if self.screen != Screen::Progress {
            self.board.progress_since = self.remembered.progress_seen.unwrap_or_else(|| local_midnight(self.model.unix));
            self.remembered.progress_seen = Some(self.model.unix);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        self.show(Screen::Progress, cx);
        cx.notify();
    }
}
