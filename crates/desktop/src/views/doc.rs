//! The document panel at the right: a Markdown file a member wrote, drawn
//! as the conversation draws Markdown, beside the conversation it came up in
//! (`doc.rs` reads it). Its bar names it and where it is, reads it again,
//! opens it in its app, copies its path, closes.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::render::view::Look;
use crate::theme::{BODY, BODY_LINE, MONO, META, SMALL};
use crate::ui;

pub fn panel(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let doc = shell.doc.as_ref()?;
    let p = shell.palette;
    let local = !shell.endpoint.is_remote();
    let path = doc.path.display().to_string();
    let bar = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::FileText, 15., p.muted))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(doc.name()))
                .child(div().text_size(px(META)).text_color(p.muted).font_family(MONO).overflow_hidden().whitespace_nowrap().text_ellipsis().child(path.clone())),
        )
        .child(ui::icon_button(p, "doc-reload", Icon::Reload, t!("doc.reload"), None).on_click(cx.listener(|this, _, _, cx| this.reload_doc(cx))))
        .when(local, |bar| {
            let path = doc.path.clone();
            bar.child(ui::icon_button(p, "doc-open", Icon::External, t!("doc.open"), None).on_click(move |_, _, cx| crate::render::open_file(&path, crate::render::Open::Edit, cx)))
        })
        .child(ui::icon_button(p, "doc-copy", Icon::Copy, t!("doc.copy_path"), None).on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
            crate::render::flash(t!("doc.copied").into(), cx);
        }))
        .child(ui::icon_button(p, "doc-close", Icon::X, t!("doc.close"), None).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx))));
    let body: AnyElement = match &doc.text {
        None => div().p_4().flex().items_center().gap_2().text_size(px(SMALL)).text_color(p.muted).child(crate::motion::spinner(13., p.muted)).child(t!("doc.reading")).into_any_element(),
        Some(Err(why)) => v_flex()
            .p_4()
            .gap_1()
            .text_size(px(SMALL))
            .child(div().text_color(p.text).child(t!("doc.unreadable")))
            .child(div().text_color(p.muted).child(why.clone()))
            .into_any_element(),
        Some(Ok(_)) => {
            let markdown = doc.markdown().unwrap_or_default();
            let look = Look { size: BODY, line: BODY_LINE, color: p.prose, live: false };
            v_flex()
                .px_5()
                .pt_4()
                .pb_8()
                .min_w_0()
                .child(crate::render::view::nested(p, ("doc", crate::render::hash_of((&doc.path, doc.read))), &markdown, look))
                .when(doc.cut, |b| b.child(div().pt_3().text_size(px(SMALL)).text_color(p.muted).child(t!("doc.cut", size = crate::doc::MOST / 1024))))
                .into_any_element()
        }
    };
    let wide = crate::doc::width(window.viewport_size().width.into());
    Some(
        v_flex()
            .id("doc-panel")
            .w(px(wide))
            .flex_none()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(p.hair)
            .bg(p.bg)
            .child(bar)
            .child(div().id("doc-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&doc.scroll).child(body))
            .into_any_element(),
    )
}
