//! The conversation with a member. Its own ticket fills this in (T-57); until
//! then it holds the place.

use gpui_kit::*;

use crate::Shell;
use crate::theme::BODY;

pub fn render(shell: &Shell) -> impl IntoElement + use<> {
    let p = shell.palette;
    div().flex_1().flex().items_center().justify_center().text_size(px(BODY)).text_color(p.muted).child("대화 화면은 준비 중입니다")
}
