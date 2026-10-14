//! The 'All' control tower: over the PM's conversation, a line a project —
//! who is at it and on what, what waits for the person, what is open — and a
//! click goes into that project (as the terminal console's overview does).

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::{Busy, ProjectLine};
use crate::theme::{PROJECT_HUES, SMALL};
use crate::Shell;

/// Past this many lines the overview scrolls rather than push the conversation off.
const MAX_HEIGHT: f32 = 168.;

/// Who is at a project and on what: `frontend T-12 제목 · pm ‘이건 어때요’`.
pub fn busy_text(busy: &[Busy]) -> String {
    let one = |b: &Busy| match (&b.ticket, &b.about) {
        (Some((id, title)), _) => format!("{} T-{id} {title}", b.member),
        (None, Some(about)) if !about.is_empty() => format!("{} ‘{about}’", b.member),
        _ => format!("{} 대화 중", b.member),
    };
    busy.iter().map(one).collect::<Vec<_>>().join(" · ")
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let rows = shell.model.overview().into_iter().map(|line| row(shell, line, cx));
    v_flex()
        .id("overview")
        .w_full()
        .flex_none()
        .max_h(px(MAX_HEIGHT))
        .overflow_y_scroll()
        .py_2()
        .border_b_1()
        .border_color(p.hair)
        .children(rows)
}

fn row(shell: &Shell, line: ProjectLine, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let hue = p.member(PROJECT_HUES[line.at % PROJECT_HUES.len()]);
    let target = line.name.clone();
    let text = if line.busy.is_empty() { "아무도 일하고 있지 않음".to_owned() } else { busy_text(&line.busy) };
    let working = line.busy.iter().any(|b| b.working);
    h_flex()
        .id(("project-line", line.at))
        .h(px(32.))
        .px_3()
        .gap_3()
        .items_center()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .cursor_pointer()
        .hover(move |s| s.bg(p.wash))
        .on_click(cx.listener(move |this, _, window, cx| this.focus_project(Some(target.clone()), window, cx)))
        .child(h_flex().w(px(120.)).flex_none().gap_2().items_center().child(div().text_size(px(9.)).text_color(hue).child("●")).child(div().font_weight(FontWeight::SEMIBOLD).child(line.name.clone())))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(if working { p.text2 } else { p.muted }).child(text))
        .when(line.for_you > 0, |r| r.child(div().flex_none().text_color(p.accent).font_weight(FontWeight::SEMIBOLD).child(format!("나에게 {}", line.for_you))))
        .when(line.open > 0, |r| r.child(div().flex_none().text_color(p.muted).child(format!("열림 {}", line.open))))
}
