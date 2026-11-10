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

/// Where the keyboard goes in focus view: the button that went in (back
/// there on the way out), the way back, the "more" button and its first item.
pub struct Handles {
    pub entry: FocusHandle,
    pub back: FocusHandle,
    pub more: FocusHandle,
    pub first: FocusHandle,
}

impl Handles {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self { entry: cx.focus_handle(), back: cx.focus_handle(), more: cx.focus_handle(), first: cx.focus_handle() }
    }
}

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
    let focused = doc.focus.is_some();
    let window_width: f32 = window.viewport_size().width.into();
    // Beside the conversation the panel's own width is the room; in focus view, the window's.
    let compact = crate::doc::compact(if focused { window_width } else { wide * 1.6 }, shell.remembered.zoom.0);
    let keys = &shell.doc_keys;
    let tools_inline = !compact;
    let copy_path = {
        let path = path.clone();
        move |cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
            crate::render::flash(t!("doc.copied").into(), cx);
        }
    };
    // Reading again, opening, copying the path: beside the name when there is
    // room, else under "more".
    let tools = |bar: Div, cx: &mut Context<Shell>| {
        bar.when(doc.slides().is_some_and(|s| !s.is_empty()), |bar| bar.child(ui::icon_button(p, "doc-present", Icon::Play, t!("deck.present"), None).on_click(cx.listener(|this, _, _, cx| this.present(0, cx)))))
            .when(tools_inline && !doc.kept, |bar| bar.child(ui::icon_button(p, "doc-reload", Icon::Reload, t!("doc.reload"), None).on_click(cx.listener(|this, _, _, cx| this.reload_doc(cx)))))
            .when(tools_inline && local && !doc.kept, |bar| {
                let path = doc.path.clone();
                bar.child(ui::icon_button(p, "doc-open", Icon::External, t!("doc.open"), None).on_click(move |_, _, cx| crate::render::open_file(&path, crate::render::Open::Edit, cx)))
            })
            .when(tools_inline && doc.path.is_absolute(), |bar| {
                let copy = copy_path.clone();
                bar.child(ui::icon_button(p, "doc-copy", Icon::Copy, t!("doc.copy_path"), None).on_click(move |_, _, cx| copy(cx)))
            })
    };
    let more_button = |cx: &mut Context<Shell>| {
        ui::icon_button(p, "doc-more", Icon::Compact, t!("doc.focus_more"), None)
            .focus(&keys.more)
            .selected(doc.more)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_doc_more(window, cx)))
    };
    let menu = doc.more.then(|| {
        let mut row = h_flex().flex_none().px_3().py_1().gap_1().flex_wrap().items_center().border_b_1().border_color(p.hair).id("doc-more-menu");
        let mut first = Some(&keys.first);
        let mut item = |b: ui::Button| -> ui::Button {
            match first.take() {
                Some(h) => b.focus(h),
                None => b,
            }
        };
        if !doc.kept {
            row = row.child(item(ui::button(p, "doc-more-reload", ui::Kind::Ghost, t!("doc.reload")).icon(Icon::Reload)).on_click(cx.listener(|this, _, _, cx| {
                this.reload_doc(cx);
                this.close_doc_more(cx);
            })));
        }
        if local && !doc.kept {
            let path = doc.path.clone();
            row = row.child(item(ui::button(p, "doc-more-open", ui::Kind::Ghost, t!("doc.open")).icon(Icon::External)).on_click(move |_, _, cx| crate::render::open_file(&path, crate::render::Open::Edit, cx)));
        }
        if doc.path.is_absolute() {
            let copy = copy_path.clone();
            row = row.child(item(ui::button(p, "doc-more-copy", ui::Kind::Ghost, t!("doc.copy_path")).icon(Icon::Copy)).on_click(move |_, _, cx| copy(cx)));
        }
        row.child(item(ui::button(p, "doc-more-close", ui::Kind::Ghost, t!("doc.close_doc")).icon(Icon::X)).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx))))
    });
    let bar = if let Some(focus) = &doc.focus {
        // Focus view: a way back on the left, the tools on the right; under
        // them the whole name and where it is, never cut.
        let back = match &focus.origin {
            crate::doc::Origin::Conversation(title) => t!("doc.focus_back_chat", title = title),
            crate::doc::Origin::Board(title) => t!("doc.focus_back_board", title = title),
        };
        let lights = super::sidebar::lights_end(crate::zoom::factor(shell.remembered.zoom.0));
        let first = ui::title_bar(
            h_flex().h(px(super::header::HEIGHT)).flex_none().pl(px(lights + 8.)).pr_3().gap_2().items_center().child(
                div().flex_shrink(1.).min_w_0().child(
                    ui::button(p, "doc-focus-back", ui::Kind::Ghost, back.clone())
                        .icon(Icon::ArrowLeft)
                        .focus(&keys.back)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_doc_focus(window, cx))),
                ),
            ),
        )
        .child(div().flex_1().min_w_0())
        .child(div().flex_none().text_size(px(META)).text_color(p.muted).child(if doc.more { t!("doc.focus_menu_esc").to_string() } else { "Esc".to_string() }));
        let first = tools(first, cx).when(compact, |bar| bar.child(more_button(cx))).when(!compact, |bar| bar.child(ui::icon_button(p, "doc-close", Icon::X, t!("doc.close_doc"), None).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx)))));
        v_flex().flex_none().border_b_1().border_color(p.hair).child(first).child(
            tip(
                h_flex()
                    .id("doc-where")
                    .px_4()
                    .pb_2()
                    .gap_2()
                    .items_start()
                    .child(div().flex_none().pt(px(2.)).child(icon(Icon::FileText, 15., p.muted)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap_x(px(8.))
                            .child(div().min_w_0().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(doc.name()))
                            .child(div().min_w_0().text_size(px(META)).text_color(p.muted).child(folder.clone())),
                    ),
                p,
                path.clone(),
                None,
            ),
        )
    } else {
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
                    .child(div().min_w_0().flex_shrink(1.).max_w(px(wide * 0.6)).text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).overflow_hidden().whitespace_nowrap().text_ellipsis().child(doc.name()))
                    .child(div().flex_1().min_w_0().text_size(px(META)).text_color(p.muted).overflow_hidden().whitespace_nowrap().text_ellipsis().child(folder.clone())),
                p,
                path.clone(),
                None,
            ));
        let enter = ui::icon_button(p, "doc-focus", Icon::Expand, t!("doc.focus"), Some("⇧⌘F")).focus(&keys.entry).on_click(cx.listener(|this, _, window, cx| this.toggle_doc_focus(window, cx)));
        // Kept words have no file to read again or open.
        tools(bar, cx)
            .child(enter)
            .when(compact, |bar| bar.child(more_button(cx)))
            .child(ui::icon_button(p, "doc-fold", Icon::ChevronRight, t!("doc.fold"), None).on_click(cx.listener(|this, _, _, cx| this.fold_doc(true, cx))))
            .when(!compact, |bar| bar.child(ui::icon_button(p, "doc-close", Icon::X, t!("doc.close_doc"), None).on_click(cx.listener(|this, _, _, cx| this.close_doc(cx)))))
    };
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
            .when(focused, |d| d.flex_1().min_w_0())
            .when(!focused, |d| d.w(px(wide)).flex_none().border_l_1().border_color(p.hair))
            .h_full()
            .min_h_0()
            .bg(p.bg)
            .child(bar)
            .children(menu)
            .children(tickets)
            .child(div().id("doc-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&doc.scroll).child(if focused { div().w_full().max_w(px(680.)).mx_auto().child(body).into_any_element() } else { body }))
            .children((!focused).then(|| edge(shell, cx)))
            .into_any_element(),
    )
}
