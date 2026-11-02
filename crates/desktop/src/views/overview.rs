//! 현황: the squad's projects side by side (`portfolio.rs`) — the squad's
//! numbers over a card per project, the HQ first: what is stuck, waiting for
//! review, under way and finished this week, with its latest release. A row
//! opens its ticket; a project's name opens its conversation. Without an HQ
//! yet, a line on what one is, with the projects to make it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use equipe_core::protocol::TicketId;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::portfolio::{self, Card, Release};
use crate::state::Side;
use crate::theme::{MONO, R_LG, R_MD, SMALL};
use crate::{Screen, Shell, ui};

/// How long the releases found stay fresh before the screen asks again.
const RELEASES_FRESH: Duration = Duration::from_secs(5 * 60);
/// A card's width before it grows into its row, and the least it takes.
const CARD_BASIS: f32 = 360.;
const CARD_MIN: f32 = 300.;

#[derive(Default)]
pub struct OverviewUi {
    /// Each project's newest tag, as last found.
    pub releases: Vec<(PathBuf, Release)>,
    pub asked: Option<Instant>,
    /// 정기 업무's form and second clicks.
    pub routines: super::routines::RoutinesUi,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let now = now_secs();
    let cards = portfolio::cards(&shell.model, now);
    let totals = portfolio::totals(&cards);
    // Nothing to press in it: the whole bar is the title bar's.
    let header = crate::ui::title_bar(h_flex())
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("overview.title")))
        .child(div().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(t!("overview.subtitle")));
    let tile = |n: usize, label: String, hot: bool| {
        v_flex()
            .flex_1()
            .min_w(px(120.))
            .px_4()
            .py_3()
            .gap_0p5()
            .rounded(px(R_LG))
            .border_1()
            .border_color(p.hair)
            .child(div().text_size(px(22.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(if hot && n > 0 { p.orange } else { p.text }).child(n.to_string()))
            .child(div().text_size(px(12.)).text_color(p.muted).child(label))
    };
    let tiles = h_flex()
        .w_full()
        .gap_3()
        .flex_wrap()
        .child(tile(totals.stuck, t!("overview.stuck").into(), true))
        .child(tile(totals.review, t!("overview.review").into(), false))
        .child(tile(totals.working, t!("overview.working").into(), false))
        .child(tile(totals.shipped, t!("overview.shipped").into(), false));
    let hint = (shell.model.hq.is_none() && !shell.model.projects.is_empty()).then(|| hq_hint(shell, cx));
    // An odd card out keeps a column's width: an empty one beside it.
    let filler = (cards.len() % 2 == 1).then(|| div().flex_1().flex_basis(px(CARD_BASIS)).min_w(px(CARD_MIN)));
    let grid = h_flex().w_full().flex_wrap().gap_3().items_start().children(cards.iter().map(|c| card(shell, c, now, cx))).children(filler);
    let empty = shell.model.projects.is_empty().then(|| div().py_8().text_size(px(13.)).text_color(p.muted).child(t!("overview.no_projects")));
    let column = v_flex().w_full().max_w(px(1040.)).px_6().pt_5().pb_10().gap_5().child(tiles).children(hint).child(super::routines::section(shell, cx)).children(empty).child(grid);
    v_flex()
        .id("overview")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        // The scroller spans the screen, so its bar is at the window's edge.
        .child(div().id("overview-scroll").flex_1().min_h_0().overflow_y_scroll().child(h_flex().w_full().justify_center().child(column)))
        .into_any_element()
}

/// No HQ yet: what one is for, and the projects to make it.
fn hq_hint(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let picks = shell.model.projects.iter().take(6).map(|path| {
        let name = equipe_core::board::project_name(path);
        let to = path.clone();
        ui::button(p, SharedString::from(format!("hq-pick-{name}")), ui::Kind::Secondary, name).on_click(cx.listener(move |this, _, _, cx| this.set_hq(Some(to.clone()), cx)))
    });
    h_flex()
        .w_full()
        .p_4()
        .gap_3()
        .items_start()
        .rounded(px(R_LG))
        .bg(p.wash)
        .child(div().pt(px(2.)).child(super::sidebar::icon(IconName::Building2, 16., p.text2)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(t!("overview.hq_title")))
                .child(div().text_size(px(13.)).line_height(px(20.)).text_color(p.text2).child(t!("overview.hq_text")))
                .child(h_flex().pt_2().gap_2().flex_wrap().items_center().children(picks).child(div().text_size(px(12.)).text_color(p.muted).child(t!("overview.hq_or_menu")))),
        )
}

fn card(shell: &Shell, c: &Card, now: u64, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let key = c.name.clone().unwrap_or_default();
    let ident = match (c.at, c.hq) {
        (Some(_), true) => super::sidebar::icon(IconName::Building2, 13., p.text2).into_any_element(),
        (Some(at), false) => super::board::dot(super::header::project_color(p, at), 8.).into_any_element(),
        (None, _) => super::sidebar::icon(IconName::Folder, 13., p.muted).into_any_element(),
    };
    let label = c.name.clone().unwrap_or_else(|| t!("board.squad_folder").into());
    let release = c.path.as_deref().and_then(|path| portfolio::release_of(&shell.overview.releases, path)).map(|r| {
        div().flex_none().text_size(px(12.)).text_color(p.muted).child(t!("overview.release", tag = r.tag, ago = crate::activity::ago(now.saturating_sub(r.at))))
    });
    let open_project = c.name.clone();
    let name = h_flex()
        .id(SharedString::from(format!("overview-name-{key}")))
        .min_w_0()
        .gap_2()
        .items_center()
        .px_1()
        .ml(px(-4.))
        .rounded(px(R_MD))
        .when(open_project.is_some(), |n| {
            n.cursor_pointer().hover(move |s| s.bg(p.wash)).on_click(cx.listener(move |this, _, window, cx| {
                this.focus_project(open_project.clone(), window, cx);
                this.open_side(Side::Pm, window, cx);
            }))
        })
        .child(ident)
        .child(div().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(label))
        .when(c.hq, |n| n.child(super::sidebar::hq_badge(p)));
    let count = |n: usize, word: String, hot: bool| div().text_color(if hot && n > 0 { p.orange } else if n > 0 { p.text2 } else { p.muted }).child(format!("{word} {n}"));
    let counts = h_flex()
        .gap_3()
        .flex_wrap()
        .text_size(px(12.))
        .child(count(c.stuck.len(), t!("overview.stuck_short").into(), true))
        .child(count(c.review.len(), t!("overview.review_short").into(), false))
        .child(count(c.working.len(), t!("overview.working_short").into(), false))
        .child(count(c.todo, t!("overview.todo_short").into(), false));
    let row = |id: TicketId, meta: Option<String>, hot: bool| {
        let title = model.tickets.iter().find(|t| t.id == id).map(|t| t.title.clone()).unwrap_or_default();
        h_flex()
            .id(SharedString::from(format!("overview-{key}-{id}")))
            .w_full()
            .h(px(26.))
            .px_1()
            .ml(px(-4.))
            .gap_2()
            .items_center()
            .rounded(px(R_MD))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .on_click(cx.listener(move |this, _, window, cx| this.open_side(Side::Ticket(id), window, cx)))
            .child(div().flex_none().font_family(MONO).text_size(px(12.)).text_color(if hot { p.orange } else { p.muted }).child(format!("T-{id}")))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(p.text).child(title))
            .children(meta.map(|m| div().flex_none().text_size(px(12.)).text_color(p.muted).child(m)))
    };
    let section = |label: String, ids: &[TicketId], meta: &dyn Fn(TicketId) -> Option<String>, hot: bool| {
        (!ids.is_empty()).then(|| {
            let more = ids.len().saturating_sub(portfolio::ROWS);
            v_flex()
                .gap_0p5()
                .child(div().pb_0p5().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(if hot { p.orange } else { p.muted }).child(label))
                .children(ids.iter().take(portfolio::ROWS).map(|id| row(*id, meta(*id), hot)))
                .when(more > 0, |s| s.child(div().pl_1().text_size(px(12.)).text_color(p.muted).child(t!("overview.more", count = more))))
        })
    };
    let who = |id: TicketId| model.member_on(id).map(|m| m.name.clone()).or_else(|| model.tickets.iter().find(|t| t.id == id).and_then(|t| t.owner.clone()));
    let when = |id: TicketId| model.tickets.iter().find(|t| t.id == id).and_then(|t| t.updated_at).map(|at| crate::activity::ago(now.saturating_sub(at)));
    let quiet = c.is_quiet().then(|| div().text_size(px(13.)).text_color(p.muted).child(t!("overview.quiet")));
    v_flex()
        .id(SharedString::from(format!("overview-card-{key}")))
        .flex_1()
        .flex_basis(px(CARD_BASIS))
        .min_w(px(CARD_MIN))
        .p_4()
        .gap_3()
        .rounded(px(R_LG))
        .border_1()
        .border_color(if c.hq { Rgba { a: 0.35, ..p.text2 } } else { p.hair })
        .child(h_flex().w_full().gap_2().items_center().child(div().flex_1().min_w_0().child(name)).children(release))
        .child(counts)
        .children(section(t!("overview.section_stuck").into(), &c.stuck, &when, true))
        .children(section(t!("overview.section_review").into(), &c.review, &when, false))
        .children(section(t!("overview.section_working").into(), &c.working, &who, false))
        .children(section(t!("overview.section_shipped").into(), &c.shipped, &when, false))
        .children(quiet)
        .into_any_element()
}

impl Shell {
    /// 현황, with each project's latest release asked again when it's been a while.
    pub fn open_overview(&mut self, cx: &mut Context<Self>) {
        self.show(Screen::Overview, cx);
        if self.overview.asked.is_some_and(|at| at.elapsed() < RELEASES_FRESH) {
            return;
        }
        self.overview.asked = Some(Instant::now());
        let (endpoint, projects) = (self.endpoint.clone(), self.model.projects.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(portfolio::releases(&endpoint, &projects));
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(found)) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.overview.releases = found;
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
