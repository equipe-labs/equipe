//! 로드맵 (`roadmap.rs`): the squad's work as one picture — a lane per
//! project, the HQ first; across it what finished lately, what is under way,
//! what comes next and what waits; an arrow from each ticket to the ones
//! waiting on it, orange while it holds them up. A card opens its ticket.
//! It scrolls both ways when the work is wider or taller than the window.

use equipe_core::protocol::TicketStatus;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::roadmap::{self, Layout, Stage};
use crate::state::Side;
use crate::theme::{MONO, R_LG, R_MD, SMALL};
use crate::{Screen, Shell};

/// The picture's measures, in px.
const PAD: f32 = 24.;
const LABEL_W: f32 = 132.;
const COL_W: f32 = 236.;
const COL_GAP: f32 = 40.;
const CARD_H: f32 = 76.;
const ROW_GAP: f32 = 10.;
const LANE_PAD: f32 = 14.;
const HEAD_H: f32 = 44.;

/// An arrow: from where, to where (a card's top-left), and whether it holds its end up.
type Line = ((f32, f32), (f32, f32), bool);

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn lane_height(rows: usize) -> f32 {
    let rows = rows.max(1) as f32;
    rows * CARD_H + (rows - 1.) * ROW_GAP + 2. * LANE_PAD
}

fn column_x(stage: Stage) -> f32 {
    PAD + LABEL_W + stage.column() as f32 * (COL_W + COL_GAP)
}

/// Where each lane starts, down the picture, and the picture's height.
fn lane_tops(layout: &Layout) -> (Vec<f32>, f32) {
    let mut tops = Vec::new();
    let mut y = HEAD_H;
    for lane in &layout.lanes {
        tops.push(y);
        y += lane_height(lane.rows);
    }
    (tops, y + PAD)
}

fn stage_word(stage: Stage) -> &'static str {
    match stage {
        Stage::Done => t!("roadmap.done"),
        Stage::Now => t!("roadmap.now"),
        Stage::Next => t!("roadmap.next"),
        Stage::Later => t!("roadmap.later"),
    }
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let model = &shell.model;
    let layout = roadmap::layout(model, now_secs());
    let (tops, height) = lane_tops(&layout);
    let width = column_x(Stage::Later) + COL_W + PAD;
    let header = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pr_4()
        .pl(px(shell.header_inset(16., cx)))
        .gap_3()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t!("roadmap.title")))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).text_color(p.muted).child(t!("roadmap.subtitle")))
        .child(legend(shell));
    if layout.cards.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .bg(p.bg)
            .child(header)
            .child(div().flex_1().flex().items_center().justify_center().text_size(px(13.)).text_color(p.muted).child(t!("roadmap.empty")))
            .into_any_element();
    }
    // Stage headings across the top, with how many each holds.
    let heads = Stage::ALL.into_iter().map(|stage| {
        div()
            .absolute()
            .left(px(column_x(stage)))
            .top(px(12.))
            .w(px(COL_W))
            .flex()
            .gap_2()
            .items_baseline()
            .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(stage_word(stage)))
            .child(div().text_size(px(12.)).text_color(p.muted).child(layout.count(stage).to_string()))
    });
    // A band per lane, its name at the left.
    let bands = layout.lanes.iter().enumerate().map(|(i, lane)| {
        let top = tops[i];
        let name = lane.name.clone().unwrap_or_else(|| t!("board.squad_folder").into());
        let ident = match (lane.at, lane.hq) {
            (Some(_), true) => super::sidebar::icon(IconName::Building2, 12., p.text2).into_any_element(),
            (Some(at), false) => super::board::dot(super::header::project_color(p, at), 7.).into_any_element(),
            (None, _) => super::sidebar::icon(IconName::Folder, 12., p.muted).into_any_element(),
        };
        div()
            .absolute()
            .left(px(PAD / 2.))
            .top(px(top))
            .w(px(width - PAD))
            .h(px(lane_height(lane.rows)))
            .rounded(px(R_LG))
            .when(i % 2 == 0, |b| b.bg(p.wash))
            .child(
                h_flex()
                    .absolute()
                    .left(px(PAD / 2. + 4.))
                    .top(px(LANE_PAD))
                    .w(px(LABEL_W - 16.))
                    .gap_2()
                    .items_center()
                    .child(ident)
                    .child(div().min_w_0().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(name))
                    .when(lane.hq, |r| r.child(super::sidebar::hq_badge(p))),
            )
    });
    // The arrows, under the cards.
    let at = |id| {
        layout.card(id).map(|c| {
            let x = column_x(c.stage);
            let y = tops[c.lane] + LANE_PAD + c.row as f32 * (CARD_H + ROW_GAP);
            (x, y)
        })
    };
    let lines: Vec<Line> = layout.edges.iter().filter_map(|e| Some((at(e.from)?, at(e.to)?, e.holding))).collect();
    let (calm, holding) = (Rgba { a: 0.45, ..p.muted }, Rgba { a: 0.85, ..p.orange });
    let arrows = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
            for ((fx, fy), (tx, ty), hold) in &lines {
                let (sy, ey) = (fy + CARD_H / 2., ty + CARD_H / 2.);
                let mut b = PathBuilder::stroke(px(1.5));
                // Forward: from the right of the one before to the left of the one after;
                // within a column: out to the right and back.
                let (start, end, c1, c2, back) = if tx > fx {
                    let (sx, ex) = (fx + COL_W, *tx);
                    let mid = (ex - sx) / 2.;
                    ((sx, sy), (ex, ey), (sx + mid, sy), (ex - mid, ey), false)
                } else {
                    let (sx, ex) = (fx + COL_W, tx + COL_W);
                    ((sx, sy), (ex, ey), (sx + 28., sy), (ex + 28., ey), true)
                };
                b.move_to(pt(start.0, start.1));
                b.cubic_bezier_to(pt(end.0, end.1), pt(c1.0, c1.1), pt(c2.0, c2.1));
                let ink = if *hold { holding } else { calm };
                if let Ok(path) = b.build() {
                    window.paint_path(path, ink);
                }
                // The head, pointing into the card it leads to.
                let dir = if back { -1. } else { 1. };
                let mut head = PathBuilder::fill();
                head.move_to(pt(end.0, end.1));
                head.line_to(pt(end.0 - 7. * dir, end.1 - 4.));
                head.line_to(pt(end.0 - 7. * dir, end.1 + 4.));
                head.close();
                if let Ok(path) = head.build() {
                    window.paint_path(path, ink);
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .w(px(width))
    .h(px(height));
    let cards = layout.cards.iter().filter_map(|c| {
        let ticket = model.tickets.iter().find(|t| t.id == c.id)?;
        let (x, y) = at(c.id)?;
        let id = c.id;
        let who = model.member_on(id).map(|m| m.name.clone()).or_else(|| ticket.owner.clone()).unwrap_or_default();
        let edge = match ticket.status {
            TicketStatus::Waiting => p.orange,
            TicketStatus::Review => p.accent,
            TicketStatus::InProgress => p.text2,
            _ => p.hair,
        };
        let faded = c.stage == Stage::Done;
        let waits = c.stage == Stage::Later;
        Some(
            v_flex()
                .id(SharedString::from(format!("roadmap-{id}")))
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(COL_W))
                .h(px(CARD_H))
                .px_3()
                .py_2()
                .gap_1()
                .rounded(px(R_MD))
                .bg(p.bg)
                .border_1()
                .border_color(p.hair)
                .overflow_hidden()
                .when(faded, |c| c.opacity(0.6))
                // Its state at its left edge.
                .child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(edge))
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash))
                .on_click(cx.listener(move |this, _, window, cx| this.open_side(Side::Ticket(id), window, cx)))
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(super::icons::status(p, ticket.status, 11.))
                        .child(div().flex_none().font_family(MONO).text_size(px(11.)).text_color(if waits { p.orange } else { p.muted }).child(format!("T-{id}")))
                        .child(div().flex_1())
                        .child(div().min_w_0().truncate().text_size(px(11.)).text_color(p.muted).child(who)),
                )
                .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(if faded { p.text2 } else { p.text }).line_clamp(2).text_ellipsis().child(ticket.title.clone())),
        )
    });
    let picture = div().relative().w(px(width)).h(px(height)).children(bands).child(arrows).children(heads).children(cards);
    v_flex()
        .id("roadmap")
        .flex_1()
        .min_h_0()
        .bg(p.bg)
        .child(header)
        .child(div().id("roadmap-scroll").flex_1().min_h_0().overflow_scroll().child(picture))
        .into_any_element()
}

/// What the marks mean: under way, waiting on the person, in review; an arrow holding work up.
fn legend(shell: &Shell) -> impl IntoElement + use<> {
    let p = shell.palette;
    let item = |color: Rgba, words: &'static str| h_flex().gap(px(5.)).items_center().child(div().w(px(3.)).h(px(12.)).rounded(px(1.)).bg(color)).child(words);
    h_flex()
        .flex_none()
        .gap_3()
        .text_size(px(11.))
        .text_color(p.muted)
        .child(item(p.text2, t!("roadmap.legend_working")))
        .child(item(p.accent, t!("roadmap.legend_review")))
        .child(item(p.orange, t!("roadmap.legend_waiting")))
        .child(h_flex().gap(px(5.)).items_center().child(div().w(px(14.)).h(px(2.)).bg(Rgba { a: 0.85, ..p.orange })).child(t!("roadmap.legend_holding")))
}

impl Shell {
    pub fn open_roadmap(&mut self, cx: &mut Context<Self>) {
        self.show(Screen::Roadmap, cx);
    }
}
