//! A ticket's status as a 14px mark, drawn with paths so every one sits on the
//! same circle: proposed dashed, to do empty, in progress half full (amber),
//! 내 차례 a pause, 검토 three quarters, done a filled check, cancelled struck.

use equipe_core::protocol::TicketStatus;
use gpui_kit::*;

use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Dashed,
    Empty,
    Half,
    Pause,
    ThreeQuarters,
    Check,
    Struck,
}

fn shape(status: TicketStatus) -> Shape {
    match status {
        TicketStatus::Proposed => Shape::Dashed,
        TicketStatus::Todo => Shape::Empty,
        TicketStatus::InProgress => Shape::Half,
        TicketStatus::Waiting => Shape::Pause,
        TicketStatus::Review => Shape::ThreeQuarters,
        TicketStatus::Done => Shape::Check,
        TicketStatus::Cancelled => Shape::Struck,
    }
}

/// The colour a status is drawn in.
pub fn color(p: Palette, status: TicketStatus) -> Rgba {
    match status {
        TicketStatus::InProgress => p.amber,
        TicketStatus::Waiting | TicketStatus::Review => p.accent,
        _ => p.muted,
    }
}

/// A status mark, `size` px square.
pub fn status(p: Palette, status: TicketStatus, size: f32) -> impl IntoElement {
    let shape = shape(status);
    let ink = color(p, status);
    let paper = p.bg;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let s = f32::from(bounds.size.width.min(bounds.size.height));
            let c = point(bounds.origin.x + px(s / 2.), bounds.origin.y + px(s / 2.));
            let r = s / 2. - 1.25;
            let line = (s / 9.).max(1.2);
            let at = |dx: f32, dy: f32| point(c.x + px(dx), c.y + px(dy));
            let ring = |dash: bool| {
                let mut b = PathBuilder::stroke(px(line));
                if dash {
                    b = b.dash_array(&[px(r * 0.62), px(r * 0.5)]);
                }
                b.move_to(at(0., -r));
                b.arc_to(point(px(r), px(r)), px(0.), false, true, at(0., r));
                b.arc_to(point(px(r), px(r)), px(0.), false, true, at(0., -r));
                b.build().ok()
            };
            // A pie from 12 o'clock, clockwise, of `turn` (0–1) of a circle `pr` wide.
            let pie = |pr: f32, turn: f32| {
                let mut b = PathBuilder::fill();
                b.move_to(c);
                b.line_to(at(0., -pr));
                let angle = turn * std::f32::consts::TAU;
                let end = at(pr * angle.sin(), -pr * angle.cos());
                let radii = point(px(pr), px(pr));
                if turn >= 0.5 {
                    b.arc_to(radii, px(0.), false, true, at(0., pr));
                }
                if (turn - 0.5).abs() > 0.01 {
                    b.arc_to(radii, px(0.), false, true, end);
                }
                b.close();
                b.build().ok()
            };
            let mut paths: Vec<(Path<Pixels>, Rgba)> = Vec::new();
            let mut add = |path: Option<Path<Pixels>>, color: Rgba| {
                if let Some(path) = path {
                    paths.push((path, color));
                }
            };
            match shape {
                Shape::Dashed => add(ring(true), ink),
                Shape::Empty => add(ring(false), ink),
                Shape::Half => {
                    add(ring(false), ink);
                    add(pie(r - line * 1.6, 0.5), ink);
                }
                Shape::ThreeQuarters => {
                    add(ring(false), ink);
                    add(pie(r - line * 1.6, 0.75), ink);
                }
                Shape::Pause => {
                    add(ring(false), ink);
                    for dx in [-1.4f32, 1.4] {
                        let mut b = PathBuilder::stroke(px(line));
                        b.move_to(at(dx, -r * 0.42));
                        b.line_to(at(dx, r * 0.42));
                        add(b.build().ok(), ink);
                    }
                }
                Shape::Check => {
                    let mut b = PathBuilder::fill();
                    b.move_to(at(0., -r - line / 2.));
                    let full = r + line / 2.;
                    b.arc_to(point(px(full), px(full)), px(0.), false, true, at(0., full));
                    b.arc_to(point(px(full), px(full)), px(0.), false, true, at(0., -full));
                    b.close();
                    add(b.build().ok(), ink);
                    let mut b = PathBuilder::stroke(px(line));
                    b.move_to(at(-r * 0.45, 0.));
                    b.line_to(at(-r * 0.1, r * 0.35));
                    b.line_to(at(r * 0.5, -r * 0.35));
                    add(b.build().ok(), paper);
                }
                Shape::Struck => {
                    add(ring(false), ink);
                    let mut b = PathBuilder::stroke(px(line));
                    let k = r * 0.62;
                    b.move_to(at(-k, k));
                    b.line_to(at(k, -k));
                    add(b.build().ok(), ink);
                }
            }
            for (path, color) in paths {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// The pulse beside a member mid-turn: a dot whose strength is `phase` (0–1).
pub fn live_dot(p: Palette, phase: f32) -> impl IntoElement {
    div().size(px(6.)).flex_none().rounded_full().bg(p.amber).opacity(0.35 + 0.65 * phase)
}
