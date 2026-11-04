//! 보면서 말하기: the designer's conversation docked at the design screen's
//! right — the conversation as the chat screen draws it, live, and the one
//! composer under it — so the person says "이 주황 좀 덜 튀게" while looking.
//! Each colour, type style, logo, asset and UI-map state has a "말하기"
//! button on hover: it puts a chip for that thing in the composer (the
//! click on the thing itself still copies or shows it big), and the message
//! carries the chips as a block the member reads, the pictures attached
//! (`crate::design_talk`). With no designer on the squad, the PM's
//! conversation, said so. Folded to a rail as the person left it — and by
//! default when the window is narrow — and wider or narrower by its edge.

use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::design_talk::{self as rules, DesignContext};
use crate::icons::{Icon, icon};
use crate::state::{Member, Presence};
use crate::theme::{META, Palette, R_SM, SMALL, SMALL_LINE};
use crate::tips::TipExt as _;
use crate::{Screen, Shell, motion, ui};

/// The "말하기" button over a thing on the design screen: shown while the
/// pointer is on the thing (`group`), it attaches it beside.
pub fn say(p: Palette, id: impl Into<ElementId>, group: impl Into<SharedString>, ctx: DesignContext, cx: &mut Context<Shell>) -> Stateful<Div> {
    h_flex()
        .id(id)
        .occlude()
        .h(px(22.))
        .px_2()
        .gap_1()
        .items_center()
        .rounded(px(R_SM))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .shadow_sm()
        .text_size(px(META))
        .text_color(p.text)
        .cursor_pointer()
        .invisible()
        .group_hover(group, |s| s.visible())
        .hover(move |s| s.bg(p.raised))
        .child(icon(Icon::Message, 12., p.text2))
        .child(t!("design.talk_say"))
        .tip(p, t!("design.talk_say_tip"), None)
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.design_point_at(ctx.clone(), window, cx);
        }))
}

/// The panel at the right of the design screen, or its rail.
pub fn panel(shell: &Shell, window: &Window, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Design {
        return None;
    }
    let p = shell.palette;
    let to = shell.design_talk_to();
    if !shell.design_talk_open(window, cx) {
        return Some(rail(shell, to.map(|(m, _)| m), cx));
    }
    let width = rules::width(shell.remembered.design_talk_width, shell.design_talk_room(window, cx));
    let head = h_flex()
        .h(px(super::header::HEIGHT))
        .flex_none()
        .pl_4()
        .pr_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .window_control_area(WindowControlArea::Drag)
        .children(to.map(|(m, _)| who(p, m)))
        .child(div().flex_1())
        .when(to.is_some(), |bar| {
            bar.child(ui::icon_button(p, "design-talk-chat", Icon::Expand, t!("design.talk_open_chat"), None).on_click(cx.listener(|this, _, window, cx| {
                if let Some(session) = this.design_talk_to().map(|(m, _)| m.session) {
                    this.select_member(session, window, cx);
                }
            })))
        })
        .child(ui::icon_button(p, "design-talk-fold", Icon::ChevronRight, t!("design.talk_fold"), None).on_click(cx.listener(|this, _, window, cx| this.design_talk_fold(true, window, cx))));
    let note = to.filter(|(_, pm)| *pm).map(|_| {
        h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(p.hair)
            .text_size(px(META))
            .text_color(p.text2)
            .child(icon(Icon::Info, 13., p.muted))
            .child(div().min_w_0().child(t!("design.talk_no_designer")))
    });
    let viewing = to.is_some_and(|(m, _)| shell.model.shows_own(m.session));
    let body: AnyElement = match to {
        None => hint(p, t!("design.talk_nobody").into()),
        Some(_) if !viewing || shell.chat.blocks.is_empty() => hint(p, t!("design.talk_empty").into()),
        Some(_) => shell.panes.conversation.clone().cached(StyleRefinement::default().flex_1().min_h_0().w_full()).into_any_element(),
    };
    let panel = v_flex()
        .id("design-talk")
        .relative()
        .w(px(width))
        .flex_none()
        .h_full()
        .min_h_0()
        .border_l_1()
        .border_color(p.hair)
        .bg(p.bg)
        .child(head)
        .children(note)
        .child(v_flex().flex_1().min_h_0().w_full().items_center().child(body))
        .when(to.is_some(), |panel| panel.child(div().flex_none().w_full().px_3().flex().justify_center().child(super::composer::render(shell, cx))))
        .child(edge(shell, width, cx));
    Some(motion::fade(panel, "design-talk-in", motion::BASE).into_any_element())
}

/// Whose conversation it is: their face and name, in their colour.
fn who(p: Palette, m: &Member) -> impl IntoElement + use<> {
    let (_, hue) = crate::theme::look(&m.role, &m.name);
    h_flex()
        .min_w_0()
        .gap_2()
        .items_center()
        .child(crate::theme::avatar(p, &m.role, &m.name, 20.))
        .child(div().min_w_0().truncate().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(if m.presence == Presence::Closed { p.muted } else { p.member(hue) }).child(m.name.clone()))
}

/// A quiet word in the panel's middle.
fn hint(p: Palette, text: String) -> AnyElement {
    div().flex_1().w_full().flex().items_center().justify_center().px_6().child(div().max_w(px(300.)).text_center().text_size(px(SMALL)).line_height(px(SMALL_LINE)).text_color(p.muted).child(text)).into_any_element()
}

/// Folded: a rail at the window's edge — the face to open it by, and how
/// many things are attached, waiting.
fn rail(shell: &Shell, to: Option<&Member>, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let name = to.map(|m| m.name.clone()).unwrap_or_default();
    let pointed = shell.composer.context.len();
    v_flex()
        .id("design-talk-rail")
        .w(px(crate::doc::RAIL))
        .flex_none()
        .h_full()
        .pt_2()
        .gap_2()
        .items_center()
        .border_l_1()
        .border_color(p.hair)
        .bg(p.bg)
        .child(ui::icon_button(p, "design-talk-unfold", Icon::ArrowLeft, t!("design.talk_unfold", name = name.clone()), None).on_click(cx.listener(|this, _, window, cx| this.design_talk_fold(false, window, cx))))
        .children(to.map(|m| {
            div()
                .id("design-talk-face")
                .relative()
                .cursor_pointer()
                .child(crate::theme::avatar(p, &m.role, &m.name, 24.))
                .when(pointed > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(-6.))
                            .right(px(-8.))
                            .min_w(px(16.))
                            .h(px(16.))
                            .px(px(4.))
                            .rounded_full()
                            .bg(p.text)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.bg)
                            .child(pointed.to_string()),
                    )
                })
                .tip(p, t!("design.talk_unfold", name = name), None)
                .on_click(cx.listener(|this, _, window, cx| this.design_talk_fold(false, window, cx)))
        }))
        .into_any_element()
}

/// The panel's left edge: drag to make it wider or narrower (kept),
/// double-click for its usual width.
fn edge(shell: &Shell, width: f32, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let p = shell.palette;
    let dragging = shell.design.talk.drag.is_some();
    div()
        .id("design-talk-edge")
        .group("design-talk-edge")
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-4.))
        .w(px(8.))
        .cursor_col_resize()
        .child(div().absolute().top_0().bottom_0().left(px(4.)).w(px(1.)).bg(if dragging { p.accent } else { p.hair }).group_hover("design-talk-edge", move |s| s.bg(p.accent).opacity(0.7)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                if e.click_count >= 2 {
                    this.remembered.design_talk_width = None;
                    this.remembered.save(&equipe_providers::credentials::equipe_home());
                } else {
                    this.design.talk.drag = Some((f32::from(e.position.x), width));
                }
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

/// What is pointed at, over the composer's box: a chip each, ✕ to take it off.
pub fn chips(shell: &Shell, inset: f32, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let pointed = &shell.composer.context;
    if pointed.is_empty() {
        return None;
    }
    let p = shell.palette;
    let row = h_flex().px(px(inset)).pt(px(inset)).gap_1p5().flex_wrap().children(pointed.iter().enumerate().map(|(n, item)| {
        let face = h_flex()
            .id(("design-chip", n))
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
            .text_size(px(META))
            .text_color(p.text)
            .children(swatch_of(item))
            .when(!matches!(item, DesignContext::Token { .. }), |c| c.child(icon(glyph(item), 12., p.text2)))
            .child(div().min_w_0().truncate().child(item.label()))
            .child(
                ui::x_box(("design-unpoint", n), p.muted, t!("composer.tip_remove"))
                    .cursor_pointer()
                    .rounded_full()
                    .hover(move |s| s.bg(p.popover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.design_unpoint(n, cx))),
            );
        motion::rise(face, ("design-chip-in", crate::render::hash_of(item.key()))).into_any_element()
    }));
    Some(row.into_any_element())
}

/// A colour's chip shows the colour.
fn swatch_of(item: &DesignContext) -> Option<impl IntoElement + use<>> {
    let DesignContext::Token { value_light, .. } = item else { return None };
    let c = equipe_core::design::values::color(&serde_json::Value::String(value_light.clone()))?;
    Some(div().size(px(12.)).flex_none().rounded(px(3.)).border_1().border_color(rgba(0x00000026)).bg(rgba(c.rgba())))
}

fn glyph(item: &DesignContext) -> Icon {
    match item {
        DesignContext::Token { .. } | DesignContext::TypeStyle { .. } => Icon::Pencil,
        DesignContext::Asset { .. } => Icon::File,
        DesignContext::Screen { .. } => Icon::AppWindow,
    }
}

/// In a message of the person's, what it pointed at: the same chips, quiet.
pub fn in_bubble(p: Palette, labels: &[String]) -> AnyElement {
    h_flex()
        .flex_wrap()
        .gap_1()
        .children(labels.iter().map(|label| {
            h_flex()
                .max_w_full()
                .h(px(22.))
                .px_2()
                .gap_1()
                .items_center()
                .rounded(px(11.))
                .border_1()
                .border_color(p.hair)
                .text_size(px(META))
                .text_color(p.text2)
                .child(icon(Icon::Locate, 11., p.muted))
                .child(div().min_w_0().truncate().child(label.clone()))
        }))
        .into_any_element()
}

// ── what the panel does ──

impl Shell {
    /// Whom the panel talks to: the designer, else the PM (`true`).
    pub fn design_talk_to(&self) -> Option<(&Member, bool)> {
        self.model.design_talker()
    }

    /// The room beside the sidebar.
    pub fn design_talk_room(&self, window: &Window, cx: &App) -> f32 {
        f32::from(window.viewport_size().width) - self.board.sidebar_width * self.sidebar_shown(cx)
    }

    /// Whether the panel is open now (not its rail).
    pub fn design_talk_open(&self, window: &Window, cx: &App) -> bool {
        rules::shown(self.remembered.design_talk, self.design.talk.opened_narrow, self.design_talk_room(window, cx))
    }

    /// Folded to its rail, or open again: kept on this Mac where there is
    /// room; this launch only where the window is narrow.
    pub fn design_talk_fold(&mut self, folded: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.design_talk_room(window, cx) < rules::NARROW {
            self.design.talk.opened_narrow = !folded;
        } else {
            self.design.talk.opened_narrow = false;
            self.remembered.design_talk = Some(!folded);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        if !folded {
            self.design_talk_follow(window, cx);
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    /// The panel shows whom it talks to, and the composer writes to them.
    pub fn design_talk_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.design_talk_to().map(|(m, _)| m.session) else { return };
        if self.model.shows_own(session) {
            return;
        }
        self.model.select(session);
        self.refresh(window, cx);
        cx.notify();
    }

    /// Whether the conversation beside isn't the one it should show yet.
    pub fn design_talk_behind(&self) -> bool {
        self.design_talk_to().is_some_and(|(m, _)| !self.model.shows_own(m.session))
    }

    /// An ask from the design screen — carry on with the setup, a video of a
    /// flow, a fix, a picture to draw…: `to_designer` to the designer, else
    /// `to_pm` to the PM, about the system on show. It goes in the
    /// conversation beside, opened if folded, the person staying on the
    /// design screen — never to a conversation open elsewhere, and never with
    /// what the box holds, which stays the person's.
    pub fn design_ask_beside(&mut self, to_designer: String, to_pm: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pm) = self.design_talk_to().map(|(_, pm)| pm) else {
            self.composer.flash = Some(t!("design.no_one_to_ask").into());
            cx.notify();
            return;
        };
        if self.screen != Screen::Design {
            self.open_design(cx);
        }
        if self.design_talk_open(window, cx) {
            self.design_talk_follow(window, cx);
        } else {
            self.design_talk_fold(false, window, cx);
        }
        let project = self.design.current().map_or_else(|| self.model.focus.clone(), |s| s.project.clone());
        let plan = crate::compose::ask(&self.model, &to_designer, &to_pm, project.clone());
        let sent = !plan.commands.is_empty();
        for command in plan.commands {
            self.send_command(command);
        }
        // The PM's conversation beside shows another project's thread: where it went.
        let shown = self.model.chat_slice(self.chat.follow).flatten();
        let elsewhere = pm && sent && shown.is_some() && shown != project;
        self.composer.flash = match (plan.flash, project) {
            (Some(flash), _) => Some(flash),
            (None, Some(project)) if elsewhere => Some(t!("design.ask_sent_to_pm_about", project = project)),
            _ => None,
        };
        self.chat.follow_tail(cx);
        cx.notify();
    }

    /// Point at `ctx` from any view: the panel opens on the design screen's
    /// right, and its composer has the chip.
    pub fn design_point_at(&mut self, ctx: DesignContext, window: &mut Window, cx: &mut Context<Self>) {
        rules::add(&mut self.composer.context, ctx);
        if self.screen != Screen::Design {
            self.open_design(cx);
        }
        if !self.design_talk_open(window, cx) {
            self.design_talk_fold(false, window, cx);
        } else {
            self.design_talk_follow(window, cx);
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    /// A chip's ✕.
    pub fn design_unpoint(&mut self, n: usize, cx: &mut Context<Self>) {
        if n < self.composer.context.len() {
            self.composer.context.remove(n);
        }
        cx.notify();
    }

    /// The edge held and moved: wider as it goes left.
    pub fn design_talk_dragged(&mut self, x: f32, pressed: bool, cx: &mut Context<Self>) {
        let Some((x0, w0)) = self.design.talk.drag else { return };
        if pressed {
            self.remembered.design_talk_width = Some((w0 + x0 - x).max(rules::MIN_WIDTH));
        } else {
            self.design.talk.drag = None;
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        cx.notify();
    }
}
