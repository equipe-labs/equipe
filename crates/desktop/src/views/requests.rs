//! "Your requests", at the top of Needs you: each thing the person asked a
//! member for, its one line — where all of it stands — and what of it waits
//! on them, one click each to the ticket that asks. A click on a request
//! opens its chain of tickets as a compact timeline, and "Stop this" on it
//! stops that kind of work for good (two presses: it cancels tickets); the
//! stops in force are listed under the requests, each lifted with one click.
//! The rules are in `crate::requests`.

use equipe_core::protocol::requests::{Request, Stop};
use equipe_core::protocol::{Command, TicketStatus};
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::requests::{self as rules};
use crate::state::status_word;
use crate::theme::{MONO, SMALL};
use crate::Shell;

const HEADING: f32 = 30.;

/// The section, when the person has requests under way or finished lately.
pub fn section(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let all = shell.board.requests_all;
    let (list, more) = rules::listed(&shell.model.requests, all);
    if list.is_empty() && shell.model.stops.is_empty() && shell.model.decided.is_empty() {
        return None;
    }
    let open = list.iter().filter(|r| r.closed_at.is_none()).count();
    let mut section = v_flex()
        .id("your-requests")
        .pb_2()
        .mb_1()
        .border_b_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .h(px(HEADING))
                .mt(px(4.))
                .px_3()
                .gap_2()
                .items_center()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text2)
                .child(t!("requests.title"))
                .child(div().font_weight(FontWeight::NORMAL).text_color(p.muted).child(open.to_string())),
        );
    for r in list {
        section = section.child(request_row(shell, r, cx));
    }
    if more > 0 || all {
        let label = if all { t!("requests.fewer").to_owned() } else { t!("requests.more", count = more) };
        section = section.child(
            div()
                .id("requests-more")
                .mx_3()
                .mt_1()
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(p.text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.requests_all = !all;
                    cx.notify();
                }))
                .child(label),
        );
    }
    for stop in &shell.model.stops {
        section = section.child(stop_row(shell, stop, cx));
    }
    if !shell.model.decided.is_empty() {
        section = section.child(decided(shell, cx));
    }
    Some(section.into_any_element())
}

/// The daily digest: what the PM decided without a card — small, low-risk
/// work — folded to one line, opened to each ticket and what changed.
fn decided(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let open = shell.board.decided_open;
    let mut block = v_flex().mx_1().px_2().py_1().gap(px(2.)).child(
        h_flex()
            .id("decided-head")
            .gap_2()
            .items_center()
            .text_size(px(12.))
            .text_color(p.text2)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.decided_open = !open;
                cx.notify();
            }))
            .child(super::sidebar::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 12., p.muted))
            .child(t!("requests.decided"))
            .child(div().text_color(p.muted).child(shell.model.decided.len().to_string())),
    );
    if open {
        for d in &shell.model.decided {
            let ticket = d.ticket;
            let how = if d.accepted { t!("requests.decided_accepted") } else { t!("requests.decided_sent_back") };
            block = block.child(
                v_flex()
                    .id(SharedString::from(format!("decided-{ticket}")))
                    .pl(px(20.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.hair))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board_reveal(ticket);
                        this.board_changed(cx);
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_size(px(12.))
                            .child(div().flex_1().min_w_0().truncate().text_color(p.text).child(d.title.clone()))
                            .child(div().flex_none().text_color(p.muted).child(format!("{how} · {}", crate::state::local_hm(d.at)))),
                    )
                    .when(!d.what.is_empty(), |r| r.child(div().text_size(px(12.)).text_color(p.muted).truncate().child(d.what.clone()))),
            );
        }
    }
    block.into_any_element()
}

/// A stop in force: what, since when, what it turns away — and Lift.
fn stop_row(shell: &Shell, stop: &Stop, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = stop.id;
    let terms = stop.terms.iter().map(|t| format!("“{t}”")).collect::<Vec<_>>().join(", ");
    h_flex()
        .id(SharedString::from(format!("stop-{id}")))
        .mx_1()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .text_size(px(12.))
        .child(div().flex_none().size(px(8.)).rounded(px(2.)).bg(p.muted))
        .child(div().flex_none().text_color(p.text2).child(rules::stopped(stop)))
        .child(div().flex_1().min_w_0().truncate().text_color(p.muted).child(if terms.is_empty() { String::new() } else { t!("requests.stopped_terms", terms = terms) }))
        .child(
            div()
                .id(SharedString::from(format!("stop-{id}-lift")))
                .flex_none()
                .px_2()
                .rounded(px(5.))
                .text_color(p.text2)
                .cursor_pointer()
                .hover(move |s| s.bg(p.wash).text_color(p.text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.send_command(Command::LiftStop { squad: this.model.squad.clone(), stop: id });
                    cx.notify();
                }))
                .child(t!("requests.lift")),
        )
        .into_any_element()
}

fn request_row(shell: &Shell, r: &Request, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = r.id;
    let open = shell.board.requests_open.contains(&id);
    let done = r.closed_at.is_some();
    let session = r.session;
    let head = h_flex()
        .id(SharedString::from(format!("request-{id}")))
        .gap_2()
        .items_center()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.board.requests_open.remove(&id) {
                this.board.requests_open.insert(id);
            }
            cx.notify();
        }))
        .child(super::sidebar::icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 12., p.muted))
        .child(div().flex_1().min_w_0().truncate().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(if done { p.muted } else { p.text }).child(r.title.clone()))
        .child(div().flex_none().text_size(px(12.)).text_color(p.muted).child(rules::asked(r)));
    let mut row = v_flex()
        .mx_1()
        .px_2()
        .py_1()
        .gap(px(2.))
        .rounded(px(6.))
        .hover(move |s| s.bg(p.wash))
        .child(head)
        .child(div().pl(px(20.)).text_size(px(12.)).line_height(px(18.)).text_color(if done { p.muted } else { p.text2 }).child(rules::line(r)));
    // What waits on the person: one click each, to the ticket that asks.
    if !r.status.waits.is_empty() {
        let mut waits = h_flex().pl(px(20.)).pt(px(2.)).gap_1().flex_wrap();
        for w in &r.status.waits {
            let ticket = w.ticket;
            waits = waits.child(
                h_flex()
                    .id(SharedString::from(format!("request-{id}-wait-{ticket}")))
                    .h(px(22.))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded(px(5.))
                    .bg(p.accent_bg)
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board_reveal(ticket);
                        this.board_changed(cx);
                    }))
                    .child(div().font_family(MONO).child(format!("T-{ticket}")))
                    .child(div().max_w(px(260.)).truncate().child(w.title.clone())),
            );
        }
        row = row.child(waits);
    }
    if open {
        row = row.child(timeline(shell, r, cx));
        if !done {
            row = row.child(stop_button(shell, r, cx));
        }
        if let Some(session) = session {
            row = row.child(
                div()
                    .id(SharedString::from(format!("request-{id}-talk")))
                    .pl(px(20.))
                    .pt_1()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(p.text))
                    .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
                    .child(t!("requests.open_talk")),
            );
        }
    }
    row.into_any_element()
}

/// "Stop this": pressed once it says what it cancels; pressed again it stops.
fn stop_button(shell: &Shell, r: &Request, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = r.id;
    let asked = shell.board.stop_asked == Some(id);
    let open = rules::open_tickets(r, &shell.model.tickets);
    let label = if asked { t!("requests.stop_confirm", count = open) } else { t!("requests.stop").to_owned() };
    let button = div()
        .id(SharedString::from(format!("request-{id}-stop")))
        .ml(px(20.))
        .mt_1()
        .px_2()
        .h(px(22.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(if asked { p.accent } else { p.hair })
        .text_size(px(12.))
        .text_color(if asked { p.accent } else { p.text2 })
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.board.stop_asked == Some(id) {
                this.board.stop_asked = None;
                this.send_command(Command::StopWork { squad: this.model.squad.clone(), request: Some(id), ticket: None, words: String::new() });
            } else {
                this.board.stop_asked = Some(id);
            }
            cx.notify();
        }))
        .child(label);
    h_flex().child(crate::tips::tip(button, p, t!("requests.stop_tip"), None)).into_any_element()
}

/// The chain of tickets: each part's, then what came of it a step in —
/// number, title, how it stands, who, when. equipe's own checks quieter.
fn timeline(shell: &Shell, r: &Request, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let id = r.id;
    let mut list = v_flex().pl(px(20.)).pt_1().gap(px(1.));
    for row in rules::timeline(r) {
        let step = row.step;
        let ticket = step.ticket;
        let check = rules::is_check(step);
        let status = if step.merged { t!("requests.on_main").to_owned() } else { status_word(step.status).to_owned() };
        let tone = match step.status {
            TicketStatus::Waiting => p.accent,
            _ if check => p.muted,
            _ => p.text2,
        };
        let meta = [step.who.clone(), step.at.map(crate::state::local_hm)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        list = list.child(
            h_flex()
                .id(SharedString::from(format!("request-{id}-step-{ticket}")))
                .h(px(22.))
                .pl(px(row.depth as f32 * 14.))
                .gap_2()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(move |s| s.bg(p.hair))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board_reveal(ticket);
                    this.board_changed(cx);
                }))
                .child(div().w(px(56.)).flex_none().font_family(MONO).text_color(p.muted).child(format!("T-{ticket}")))
                .child(div().flex_1().min_w_0().truncate().text_color(if check { p.muted } else { p.text }).child(step.title.clone()))
                .when(check, |r| r.child(div().flex_none().text_color(p.muted).child(t!("requests.check"))))
                .child(div().flex_none().text_color(tone).child(status))
                .child(div().flex_none().w(px(120.)).truncate().text_color(p.muted).child(meta)),
        );
    }
    list.into_any_element()
}
