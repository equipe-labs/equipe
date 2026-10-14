//! The sidebar (248 wide): the squads, the board, and the selected squad's
//! members with how each one stands. Plain, still text — nothing here moves.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::convo::Sub;
use crate::state::{Conn, Member, Status, Verb};
use crate::theme::{Palette, SANS, SMALL, UI, look};
use crate::{Screen, Shell};

pub const WIDTH: f32 = 248.;

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let model = &shell.model;
    let for_you = model.for_you();
    let dim = matches!(model.conn, Conn::Down { had_snapshot: false, .. });

    let squads = shell.squads.iter().enumerate().map(|(ix, name)| {
        let selected = *name == model.squad;
        let target = name.clone();
        h_flex()
            .id(("squad", ix))
            .h(px(36.))
            .mx_2()
            .px_3()
            .rounded(px(6.))
            .items_center()
            .text_size(px(UI))
            .when(selected, |row| row.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
            .when(!selected, |row| row.text_color(p.text2).hover(move |s| s.bg(p.wash)).cursor_pointer())
            .on_click(cx.listener(move |this, _, window, cx| this.select_squad(&target, window, cx)))
            .child(name.clone())
    });

    let board = h_flex()
        .id("board")
        .h(px(36.))
        .mx_2()
        .px_3()
        .rounded(px(6.))
        .items_center()
        .justify_between()
        .text_size(px(UI))
        .when(shell.screen == Screen::Board, |row| row.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
        .when(shell.screen != Screen::Board, |row| row.hover(move |s| s.bg(p.wash)).cursor_pointer())
        .on_click(cx.listener(|this, _, _, cx| this.show(Screen::Board, cx)))
        .child("보드")
        .when(for_you > 0, |row| row.child(div().text_size(px(SMALL)).text_color(p.accent).child(format!("나에게 {for_you}"))));

    v_flex()
        .w(px(WIDTH))
        .h_full()
        .flex_none()
        .bg(p.side)
        .border_r_1()
        .border_color(p.hair)
        .font_family(SANS)
        .text_color(p.text)
        .child(kicker(p, "스쿼드"))
        .children(squads)
        .child(div().h(px(8.)))
        .child(board)
        .child(kicker(p, "멤버"))
        // Behind a daemon that is off the list is only what we last saw.
        .child(v_flex().flex_1().min_h_0().overflow_hidden().when(dim, |l| l.opacity(0.8)).children(model.ordered().into_iter().map(|m| member_row(p, shell, m, cx))))
        .child(footer(p, shell))
}

fn kicker(p: Palette, text: &'static str) -> impl IntoElement {
    div().h(px(32.)).px_5().pt_3().text_size(px(11.)).text_color(p.muted).child(text)
}

fn member_row(p: Palette, shell: &Shell, member: &Member, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let status = shell.model.status(member);
    let (mark, hue) = look(&member.role, &member.name);
    let color = p.member(hue);
    let needs = matches!(status, Status::NeedsYou(_));
    let working = matches!(status, Status::Working { .. });
    let closed = status == Status::Closed;
    let (line, line_color) = status_line(member, &status, p);

    let on = shell.screen == Screen::Chat && shell.model.selected == Some(member.session);
    let session = member.session;
    let subs: Vec<Sub> = shell.model.convos.get(&member.session).map(|c| c.subagents.clone()).unwrap_or_default();
    let row = h_flex()
        .id(("member", member.session))
        .h(px(44.))
        .flex_none()
        .cursor_pointer()
        .when(on, |r| r.bg(p.wash))
        .when(!on, |r| r.hover(move |s| s.bg(p.wash)))
        .on_click(cx.listener(move |this, _, window, cx| this.select_member(session, window, cx)))
        .items_center()
        .gap_2()
        .pr_3()
        .when(needs, |row| row.border_l_2().border_color(p.accent).pl(px(14.)))
        .when(!needs, |row| row.pl(px(16.)))
        .child(
            div()
                .w(px(16.))
                .flex_none()
                .text_size(px(14.))
                .text_color(if needs { p.accent } else if closed { p.muted } else { color })
                .when(status_is_ready(&status), |m| m.opacity(0.62))
                .child(if needs { "✻" } else if closed { "·" } else { mark }),
        )
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(UI))
                        .line_height(px(18.))
                        .text_color(if closed { p.muted } else { color })
                        .when(working, |n| n.font_weight(FontWeight::SEMIBOLD))
                        .child(member.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(SMALL))
                        .line_height(px(16.))
                        .text_color(line_color)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(line),
                ),
        );
    v_flex().flex_none().child(row).children(sub_rows(p, &subs))
}

/// The subagents a member handed work to, under it: at most three, then a count.
fn sub_rows(p: Palette, subs: &[Sub]) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = subs
        .iter()
        .take(3)
        .map(|sub| {
            let line = sub.step.clone().unwrap_or_else(|| sub.what.clone());
            h_flex()
                .h(px(24.))
                .pl(px(40.))
                .pr_3()
                .gap_2()
                .items_center()
                .text_size(px(SMALL))
                .text_color(p.muted)
                .child(div().flex_none().child("└"))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(line))
                .into_any_element()
        })
        .collect();
    if subs.len() > 3 {
        rows.push(div().h(px(24.)).pl(px(40.)).text_size(px(SMALL)).text_color(p.muted).child(format!("외 {}개", subs.len() - 3)).into_any_element());
    }
    rows
}

fn status_is_ready(status: &Status) -> bool {
    matches!(status, Status::Ready { .. })
}

/// The second line of a member row, and its colour.
pub fn status_line(member: &Member, status: &Status, p: Palette) -> (String, Rgba) {
    let ticket = |t: Option<u32>| t.map(|t| format!("T-{t}"));
    let join = |parts: Vec<Option<String>>| parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let memos = (member.memos > 0).then(|| format!("메모 {}개", member.memos));
    let context = member.context.filter(|c| *c >= 50).map(|c| format!("{c}%"));
    match status {
        Status::NeedsYou(t) => (join(vec![Some("사람 대기".into()), ticket(*t)]), p.accent),
        Status::Working { ticket: t, verb } => (join(vec![memos, ticket(*t), Some(verb_text(*verb).into()), context]), p.muted),
        Status::Ready { ticket: t, .. } => (join(vec![memos, Some("쉬는 중".into()), ticket(*t), context]), p.muted),
        Status::Closed => ("닫힘".into(), p.muted),
    }
}

fn verb_text(verb: Verb) -> &'static str {
    match verb {
        Verb::Thinking => "생각 중",
        Verb::Reading => "읽는 중",
        Verb::Editing => "수정 중",
        Verb::Running => "실행 중",
        Verb::Searching => "찾는 중",
        Verb::Writing => "작성 중",
        Verb::Working => "작업 중",
    }
}

fn footer(p: Palette, shell: &Shell) -> impl IntoElement + use<> {
    let text = match &shell.model.conn {
        Conn::Connecting => "연결 중…".to_owned(),
        Conn::Connected { last_event: Some(at) } => format!("데몬 연결됨 · {}", at.hm()),
        Conn::Connected { last_event: None } => "데몬 연결됨".to_owned(),
        Conn::Down { .. } => "데몬 끊김".to_owned(),
    };
    h_flex()
        .h(px(36.))
        .flex_none()
        .px_5()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .text_color(p.muted)
        .child(div().text_size(px(9.)).text_color(if matches!(shell.model.conn, Conn::Connected { .. }) { p.accent } else { p.muted }).child("●"))
        .child(text)
}
