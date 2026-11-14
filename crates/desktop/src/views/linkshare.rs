//! "Copy link": a small button (a link mark) with a menu under it that puts a
//! link to a place on the clipboard — the `equipe://` link, that link with its
//! title as text (chat apps show plain text, so the words say what the link is),
//! or the https form for chat apps that don't make a custom scheme clickable.
//! The rules — what a link holds, where it goes — are `crate::deeplink`'s.
//!
//! Used in a ticket's header, a conversation's header, a member's menu, a
//! decision's card, Needs-you's rows and Status; a ticket's ⌘⇧C copies the link at once.

use equipe_core::protocol::deeplink::Target;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::Popover;
use crate::deeplink::CopyAs;
use crate::icons::{Icon, icon};
use crate::theme::{Palette, R_MD, SMALL};
use crate::tips::TipExt as _;
use crate::Shell;

/// How long the button shows a check after copying.
const COPIED: std::time::Duration = std::time::Duration::from_millis(1600);

/// What "Copy link" is open for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMenu {
    /// The control that opened it (one of the screen's, by name).
    pub key: &'static str,
    pub target: Target,
    /// What the link is called, for "Copy as text".
    pub title: String,
}

/// The link button for `target`, titled `title`; its menu opens under it, its
/// right edge to the button's. `key` tells this button from the others on screen.
pub fn button(shell: &Shell, key: &'static str, target: Target, title: String, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let open = shell.board.popover == Some(Popover::Link) && shell.board.link.as_ref().is_some_and(|l| l.key == key);
    let copied = shell.board.link_copied.is_some_and(|(k, at)| k == key && at.elapsed() < COPIED);
    if let Some((k, at)) = shell.board.link_copied
        && k == key
        && at.elapsed() < COPIED
    {
        shell.wake_at(at + COPIED, cx);
    }
    let menu = LinkMenu { key, target, title };
    let shown = menu.clone();
    div()
        .relative()
        .flex_none()
        .child(
            div()
                .id(key)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_MD))
                .cursor_pointer()
                .when(open, |b| b.bg(p.wash))
                .hover(move |s| s.bg(p.wash))
                .role(Role::Button)
                .aria_label(t!("link.copy"))
                .aria_expanded(open)
                .tip(p, t!("link.tip"), None)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.board.link = Some(menu.clone());
                    this.toggle_popover(Popover::Link, cx);
                }))
                .child(icon(if copied { Icon::Check } else { Icon::Link }, 14., if copied { p.accent } else { p.text2 })),
        )
        .children(open.then(|| popup(shell, p, &shown, cx)))
        .into_any_element()
}

/// The menu's three rows, over the link they copy.
fn popup(shell: &Shell, p: Palette, menu: &LinkMenu, cx: &mut Context<Shell>) -> AnyElement {
    let link = shell.link_to(menu.target.clone());
    let row = |id: &'static str, mark: Icon, label: String, hint: Option<&'static str>, how: CopyAs, cx: &mut Context<Shell>| {
        h_flex()
            .id(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(div().w(px(14.)).flex().justify_center().child(icon(mark, 12., p.text2)))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .children(hint.map(|h| div().flex_none().text_size(px(11.)).text_color(p.muted).child(h)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.popover = None;
                this.copy_link(how, cx);
            }))
    };
    deferred(
        v_flex()
            .id("link-menu")
            .absolute()
            .top(px(32.))
            .right_0()
            .w(px(280.))
            .p_1()
            .rounded(px(8.))
            .bg(p.raised)
            .border_1()
            .border_color(p.hair)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.popover_outside(cx)))
            .child(div().px_2().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).truncate().child(link.uri()))
            .child(row("link-copy", Icon::Link, t!("link.copy").into(), matches!(menu.target, Target::Ticket(_)).then_some("⌘⇧C"), CopyAs::Link, cx))
            .child(row("link-copy-text", Icon::Copy, t!("link.copy_text").into(), None, CopyAs::Text, cx))
            .child(row("link-copy-web", Icon::Globe, t!("link.copy_web").into(), None, CopyAs::Web, cx)),
    )
    .into_any_element()
}

/// A right-click on a row opens this menu where the pointer is: the same three rows.
pub fn context(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let (menu, at) = shell.board.link_at.clone()?;
    let link = shell.link_to(menu.target.clone());
    let row = |id: &'static str, mark: Icon, label: String, how: CopyAs, cx: &mut Context<Shell>| {
        let (key, target) = (menu.key, menu.target.clone());
        h_flex()
            .id(id)
            .h(px(30.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(6.))
            .text_size(px(SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(p.wash))
            .child(div().w(px(14.)).flex().justify_center().child(icon(mark, 12., p.text2)))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.board.link_at = None;
                this.copy_link_to(key, target.clone(), how, cx);
            }))
    };
    Some(
        deferred(
            anchored().position(at).snap_to_window().child(
                v_flex()
                    .id("link-context")
                    .w(px(280.))
                    .p_1()
                    .rounded(px(8.))
                    .bg(p.raised)
                    .border_1()
                    .border_color(p.hair)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.board.link_at = None;
                        cx.notify();
                    }))
                    .child(div().px_2().pt_1().pb(px(2.)).text_size(px(11.)).text_color(p.muted).truncate().child(link.uri()))
                    .child(row("ctx-copy", Icon::Link, t!("link.copy").into(), CopyAs::Link, cx))
                    .child(row("ctx-copy-text", Icon::Copy, t!("link.copy_text").into(), CopyAs::Text, cx))
                    .child(row("ctx-copy-web", Icon::Globe, t!("link.copy_web").into(), CopyAs::Web, cx)),
            ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

/// A right-click on `el` opens "Copy link" for `target` at the pointer.
pub fn on_right_click<E: InteractiveElement>(el: E, key: &'static str, target: Target, cx: &mut Context<Shell>) -> E {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.board.link_at = Some((LinkMenu { key, title: String::new(), target: target.clone() }, e.position));
            cx.notify();
        }),
    )
}

/// The links pasted into the composer, over its box: a chip each (the title,
/// not the address), a click opens it, ✕ takes it off.
pub fn chips(shell: &Shell, inset: f32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let refs = &shell.composer.refs;
    if refs.is_empty() {
        return None;
    }
    let p = shell.palette;
    let row = h_flex().px(px(inset)).pt(px(inset)).gap_1p5().flex_wrap().children(refs.iter().enumerate().map(|(n, item)| {
        let uri = item.link.uri();
        let mark = match item.link.target {
            Target::Ticket(_) => Icon::Ticket,
            Target::Conversation(_) | Target::Member(_) => Icon::Message,
            _ => Icon::Link,
        };
        let face = h_flex()
            .id(("link-chip", n))
            .max_w_full()
            .h(px(24.))
            .pl_2()
            .pr(px(3.))
            .gap_1()
            .items_center()
            .rounded(px(12.))
            .bg(p.wash)
            .border_1()
            .border_color(p.hair)
            .text_size(px(crate::theme::META))
            .text_color(p.text)
            .cursor_pointer()
            .child(icon(mark, 12., p.text2))
            .child(div().min_w_0().truncate().child(item.label.clone()))
            .tip(p, t!("link.chip_open"), None)
            .on_click(cx.listener(move |_, _, _, cx| crate::deeplink::handle(&uri, cx)))
            .child(
                crate::ui::x_box(("link-chip-x", n), p.muted, t!("composer.tip_remove"))
                    .cursor_pointer()
                    .rounded_full()
                    .hover(move |s| s.bg(p.popover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_ref(n, cx);
                    })),
            );
        crate::motion::rise(face, ("link-chip-in", n)).into_any_element()
    }));
    Some(row.into_any_element())
}
