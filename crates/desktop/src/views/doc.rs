//! The document panel at the right: a Markdown file a member wrote, drawn
//! as the conversation draws Markdown, beside the conversation it came up in
//! (`doc.rs` reads it). Its bar names it and where it is, reads it again,
//! opens it in its app, copies its path, closes. A brief or plan shows the
//! tickets it names under the bar, each opening its ticket.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::render::view::Look;
use crate::theme::{BODY, BODY_LINE, META, SMALL};
use crate::tips::tip;
use crate::ui;

/// The panel's left edge: drag to make it wider or narrower (kept), double-
/// click for the usual width. Shared by the document and the terminal.
pub fn edge(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let dragging = shell.side_drag.is_some();
    div()
        .id("side-edge")
        .group("side-edge")
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-4.))
        .w(px(8.))
        .cursor_col_resize()
        .child(div().absolute().top_0().bottom_0().left(px(4.)).w(px(1.)).bg(if dragging { p.accent } else { p.hair }).group_hover("side-edge", move |s| s.bg(p.accent).opacity(0.7)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, e: &MouseDownEvent, window, cx| {
                if e.click_count >= 2 {
                    this.remembered.side_width = None;
                    this.remembered.save(&equipe_providers::credentials::equipe_home());
                } else {
                    this.side_drag = Some((f32::from(e.position.x), this.side_width(window)));
                }
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

/// Folded: a rail at the window's edge — its name on the pointer, a click
/// opens it again.
fn rail(shell: &Shell, name: String, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("doc-rail")
        .w(px(crate::doc::RAIL))
        .flex_none()
        .h_full()
        .pt_2()
        .gap_2()
        .items_center()
        .border_l_1()
        .border_color(p.hair)
        .bg(p.bg)
        .child(ui::icon_button(p, "doc-unfold", Icon::ArrowLeft, t!("doc.unfold", name = name), None).on_click(cx.listener(|this, _, _, cx| this.fold_doc(false, cx))))
        .child(div().id("doc-rail-file").cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.fold_doc(false, cx))).child(icon(Icon::FileText, 15., p.muted)))
        .child(ui::icon_button(p, "doc-rail-close", Icon::X, t!("doc.close"), None).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx))))
        .into_any_element()
}

pub fn panel(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let doc = shell.doc.as_ref()?;
    let p = shell.palette;
    if doc.folded {
        return Some(rail(shell, doc.name(), cx));
    }
    let local = !shell.endpoint.is_remote();
    let path = doc.path.display().to_string();
    let wide = shell.side_width(window);
    let folder = crate::doc::folder_shown(&doc.path, &crate::render::base_dir(shell), std::env::home_dir().as_deref());
    let bar = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(icon(Icon::FileText, 15., p.muted))
        // One line: its name, then where it is, short — the whole path on hover.
        .child(tip(
            h_flex()
                .id("doc-where")
                .flex_1()
                .min_w_0()
                .gap(px(8.))
                .items_baseline()
                .child(div().min_w_0().flex_shrink_0().max_w(px(wide * 0.6)).text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(doc.name()))
                .child(div().flex_1().min_w_0().text_size(px(META)).text_color(p.muted).overflow_hidden().whitespace_nowrap().text_ellipsis().child(folder)),
            p,
            path.clone(),
            None,
        ))
        .when(doc.slides().is_some_and(|s| !s.is_empty()), |bar| bar.child(ui::icon_button(p, "doc-present", Icon::Play, t!("deck.present"), None).on_click(cx.listener(|this, _, _, cx| this.present(0, cx)))))
        // Kept words have no file to read again or open.
        .when(!doc.kept, |bar| bar.child(ui::icon_button(p, "doc-reload", Icon::Reload, t!("doc.reload"), None).on_click(cx.listener(|this, _, _, cx| this.reload_doc(cx)))))
        .when(local && !doc.kept, |bar| {
            let path = doc.path.clone();
            bar.child(ui::icon_button(p, "doc-open", Icon::External, t!("doc.open"), None).on_click(move |_, _, cx| crate::render::open_file(&path, crate::render::Open::Edit, cx)))
        })
        // Words with no file of their own (a hand-over packet) have no path to copy.
        .when(doc.path.is_absolute(), |bar| {
            bar.child(ui::icon_button(p, "doc-copy", Icon::Copy, t!("doc.copy_path"), None).on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                crate::render::flash(t!("doc.copied").into(), cx);
            }))
        })
        .child(ui::icon_button(p, "doc-fold", Icon::ChevronRight, t!("doc.fold"), None).on_click(cx.listener(|this, _, _, cx| this.fold_doc(true, cx))))
        .child(ui::icon_button(p, "doc-close", Icon::X, t!("doc.close"), None).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx))));
    let named = crate::docs::doc_at(&shell.docs, &doc.path.to_string_lossy()).map(|d| d.ticket_ids()).unwrap_or_default();
    let tickets = (!named.is_empty()).then(|| {
        h_flex()
            .flex_none()
            .px_3()
            .py_1p5()
            .gap_1()
            .flex_wrap()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .child(div().pr_1().text_size(px(META)).text_color(p.muted).child(t!("doc.tickets")))
            .children(named.iter().map(|&id| {
                let title = shell.model.tickets.iter().find(|t| t.id == id).map(|t| t.title.clone());
                super::ticket_card::hover_link(h_flex().id(("doc-ticket", id as usize)), id, super::ticket_card::Side::Below)
                    .h(px(24.))
                    .px(px(6.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.wash))
                    .aria_label(format!("T-{id}"))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_ticket_row(id, window, cx)))
                    .child(div().flex_none().font_family(crate::theme::MONO).text_size(px(META)).text_color(p.text2).child(format!("T-{id}")))
                    .children(title.map(|t| div().max_w(px(220.)).truncate().text_size(px(META)).text_color(p.muted).child(t)))
            }))
    });
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
                .when(doc.kept, |b| {
                    b.child(
                        h_flex()
                            .id("doc-kept")
                            .mb_3()
                            .gap_2()
                            .items_center()
                            .text_size(px(SMALL))
                            .text_color(p.muted)
                            .child(icon(Icon::Info, 13., p.muted))
                            // Words with no file of their own: a hand-over packet, as it was handed.
                            .child(if doc.path.is_absolute() { t!("doc.kept") } else { t!("moved.packet_note") }),
                    )
                })
                .child(crate::render::view::document(p, ("doc", crate::render::hash_of((&doc.path, doc.read))), &markdown, look))
                .when(doc.cut, |b| b.child(div().pt_3().text_size(px(SMALL)).text_color(p.muted).child(t!("doc.cut", size = crate::doc::MOST / 1024))))
                .into_any_element()
        }
    };
    Some(
        v_flex()
            .id("doc-panel")
            .relative()
            .w(px(wide))
            .flex_none()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(p.hair)
            .bg(p.bg)
            .child(bar)
            .children(tickets)
            .child(div().id("doc-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&doc.scroll).child(body))
            .child(edge(shell, cx))
            .into_any_element(),
    )
}
