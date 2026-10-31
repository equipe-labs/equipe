//! A look inside a subagent, beside the conversation: clicked from its row
//! under its member in the sidebar, or from the member's list of them. What
//! it was asked, each step it took (a tool use, opening to its command,
//! output or change) and what it said between them, live as it works — the
//! panel follows new steps unless the person scrolled up — and what it handed
//! back. ‹ › go through the member's other subagents; Esc closes.

use std::cell::Cell;
use std::collections::HashSet;

use equipe_core::protocol::SessionId;
use equipe_core::t;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::convo::{Block, SUB_KEPT, Status, Sub, ToolLine};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::render::view::Look;
use crate::theme::{ICON_SM, META, MONO, R_SM, SMALL, SMALL_LINE};
use crate::tips::tip;
use crate::Shell;

/// The panel's width, as the ticket's beside the conversation.
const WIDTH: f32 = 440.;

/// Which subagent is open beside the conversation, and how it is shown.
#[derive(Default)]
pub struct SubPanel {
    pub open: Option<Open>,
    /// Steps opened to their command, output or change, by place.
    pub expanded: HashSet<usize>,
    pub scroll: ScrollHandle,
    pub focus: Option<FocusHandle>,
    /// How many steps were drawn last, and whether the list sat at its end
    /// then: new ones scroll into sight only if it did.
    seen: Cell<usize>,
    at_end: Cell<bool>,
}

/// A subagent opened: its member, the conversation it works for, its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub member: SessionId,
    pub session: SessionId,
    pub id: String,
}

impl SubPanel {
    pub fn show(&mut self, open: Open) {
        if self.open.as_ref() != Some(&open) {
            self.expanded.clear();
            self.seen.set(0);
            self.at_end.set(true);
        }
        self.open = Some(open);
    }

    /// Closed; `true` when it was open.
    pub fn close(&mut self) -> bool {
        self.expanded.clear();
        self.open.take().is_some()
    }

    /// The member's subagent before (`-1`) or after (`1`) the open one, among
    /// `all` (oldest first: its conversation and id), if there is one.
    pub fn beside(&self, all: &[(SessionId, String)], by: isize) -> Option<Open> {
        let open = self.open.as_ref()?;
        let at = all.iter().position(|(s, id)| *s == open.session && *id == open.id)?;
        let to = at.checked_add_signed(by)?;
        all.get(to).map(|(session, id)| Open { member: open.member, session: *session, id: id.clone() })
    }

    pub fn toggle_step(&mut self, at: usize) {
        if !self.expanded.remove(&at) {
            self.expanded.insert(at);
        }
    }

    /// Drawn with `steps` steps: whether to scroll to the end — new ones
    /// came while the list sat at its end.
    fn follows(&self, steps: usize) -> bool {
        let max = self.scroll.max_offset().y;
        let at_end = max <= px(1.) || -self.scroll.offset().y >= max - px(8.);
        let grew = steps > self.seen.get();
        let follow = grew && self.at_end.get();
        self.seen.set(steps);
        self.at_end.set(at_end || follow);
        follow
    }
}

/// Where a subagent stands, in a line, and its mark.
fn state_line(shell: &Shell, sub: &Sub, cx: &mut Context<Shell>) -> (AnyElement, String) {
    let p = shell.palette;
    match &sub.ended {
        None => (motion::spinner(12., p.muted), t!("subagent.working", time = shell.counting(sub.since.elapsed(), cx))),
        Some(end) => {
            let took = crate::render::tools::duration(u64::try_from(end.took.as_millis()).unwrap_or(u64::MAX));
            if end.failed {
                (icon(Icon::Alert, 12., p.danger()).into_any_element(), t!("subagent.failed", time = took))
            } else if end.stopped {
                (icon(Icon::Info, 12., p.muted).into_any_element(), t!("subagent.stopped", time = took))
            } else {
                (icon(Icon::Check, 12., p.accent).into_any_element(), t!("subagent.done", time = took))
            }
        }
    }
}

/// A section's quiet label.
fn label(p: crate::theme::Palette, text: String) -> Div {
    div().pt_3().pb_1().text_size(px(META)).font_weight(FontWeight::MEDIUM).text_color(p.muted).child(text)
}

/// Markdown in the panel's own size.
fn words(shell: &Shell, id: (&'static str, u64), text: &str, color: Rgba) -> impl IntoElement {
    let look = Look { size: SMALL, line: SMALL_LINE, color, live: false };
    crate::render::view::nested(shell.palette, id, text, look)
}

/// One step: its verb, what it acted on, how long it took — opening to its
/// command, output or change.
fn step_row(shell: &Shell, at: usize, line: &ToolLine, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let open = shell.chat.sub.expanded.contains(&at);
    let failed = line.status == Status::Failed;
    let more = crate::activity::has_more(line);
    let head = h_flex()
        .id(("sub-step", at))
        .h(px(28.))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(R_SM))
        .text_size(px(SMALL))
        .when(more, |h| h.cursor_pointer().hover(move |s| s.bg(p.wash)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.chat.sub.toggle_step(at);
            cx.notify();
        }))
        .children(failed.then(|| icon(Icon::Alert, 12., p.danger())))
        .child(div().flex_none().text_color(if failed { p.danger() } else { p.muted }).child(crate::activity::line_phrase(line, false)))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(p.text2).child(line.text()))
        .children(line.ms.map(|ms| div().flex_none().text_size(px(META)).text_color(p.muted).child(crate::render::tools::duration(ms))))
        .when(more, |h| h.child(icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, 12., p.muted)));
    let body = open.then(|| {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(line.full.clone());
        parts.extend(line.output.as_ref().map(|(out, _)| out.clone()));
        parts.extend(line.diff.clone());
        div()
            .id(("sub-step-body", at))
            .mx_2()
            .mb_1()
            .p_2()
            .max_h(px(240.))
            .overflow_y_scroll()
            .rounded(px(R_SM))
            .bg(if failed { p.danger_bg() } else { p.code_bg })
            .font_family(MONO)
            .text_size(px(11.5))
            .text_color(if failed { p.danger() } else { p.text2 })
            .child(parts.join("\n\n"))
    });
    v_flex().child(head).children(body).into_any_element()
}

pub fn panel(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let open = shell.chat.sub.open.clone()?;
    let p = shell.palette;
    let model = &shell.model;
    let member = model.members.iter().find(|m| m.session == open.member)?.name.clone();
    let sub = model.convos.get(&open.session).and_then(|c| c.subagent(&open.id))?.clone();
    let all: Vec<(SessionId, String)> = model.subagents_all(open.member).into_iter().map(|(s, sub)| (s, sub.id.clone())).collect();
    let (prev, next) = (shell.chat.sub.beside(&all, -1), shell.chat.sub.beside(&all, 1));
    let base = model.convos.get(&open.session).map(|c| c.base.clone()).unwrap_or_default();

    let button = |id: &'static str, glyph: Icon, label: &'static str, on: bool| {
        let b = div().id(id).size(px(26.)).flex().flex_none().items_center().justify_center().rounded(px(R_SM)).when(on, |b| b.cursor_pointer().hover(move |s| s.bg(p.wash))).child(icon(glyph, ICON_SM, if on { p.text2 } else { p.hair }));
        tip(b, p, label, None)
    };
    let go = |to: Option<Open>| {
        cx.listener(move |this: &mut Shell, _: &ClickEvent, window, cx| {
            if let Some(to) = to.clone() {
                this.open_subagent_panel(to.member, to.session, &to.id, window, cx);
            }
        })
    };
    let head = h_flex()
        .h(px(44.))
        .flex_none()
        .px_3()
        .gap_1()
        .items_center()
        .border_b_1()
        .border_color(p.hair)
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .px_1()
                .gap_1p5()
                .items_center()
                .text_size(px(SMALL))
                .child(div().flex_none().text_color(p.muted).child(member))
                .child(div().flex_none().text_color(p.muted).child("›"))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(sub.what.clone())),
        )
        .child(button("sub-prev", Icon::ArrowLeft, t!("subagent.prev"), prev.is_some()).on_click(go(prev)))
        .child(button("sub-next", Icon::ArrowRight, t!("subagent.next"), next.is_some()).on_click(go(next)))
        .child(button("sub-close", Icon::X, t!("subagent.close"), true).on_click(cx.listener(|this, _, _, cx| {
            this.chat.sub.close();
            cx.notify();
        })));
    let (mark, said) = state_line(shell, &sub, cx);
    let status = h_flex().px_4().pt_3().gap_2().items_center().text_size(px(SMALL)).text_color(p.text2).child(mark).child(said);

    // What it was asked: the whole brief, else what the member called it.
    let brief = if sub.brief.trim().is_empty() { sub.what.clone() } else { sub.brief.clone() };
    let key = crate::render::hash_of((&open.id, open.session));
    let entries: Vec<std::borrow::Cow<'_, equipe_core::protocol::Entry>> = sub.entries.iter().map(std::borrow::Cow::Borrowed).collect();
    let blocks = crate::convo::build_blocks(&entries, &base);
    let mut steps: Vec<AnyElement> = Vec::new();
    let mut at = 0;
    for (n, block) in blocks.iter().enumerate() {
        match block {
            Block::Steps(lines) => {
                for line in lines {
                    steps.push(step_row(shell, at, line, cx));
                    at += 1;
                }
            }
            Block::Reply { text, .. } => steps.push(div().px_2().py_1().child(words(shell, ("sub-said", key ^ n as u64), text, p.prose)).into_any_element()),
            _ => {}
        }
    }
    // At it now: its latest step, as it says it.
    let now = sub.ended.is_none().then(|| sub.step.clone()).flatten().map(|step| {
        h_flex().h(px(28.)).px_2().gap_2().items_center().text_size(px(SMALL)).text_color(p.text2).child(motion::spinner(11., p.muted)).child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(t!("subagent.now", step = step)))
    });
    let nothing = (steps.is_empty() && now.is_none()).then(|| div().px_2().text_size(px(SMALL)).text_color(p.muted).child(if sub.ended.is_some() { t!("subagent.no_steps_kept") } else { t!("subagent.no_steps") }));
    let cut = (sub.entries.len() == SUB_KEPT).then(|| div().px_2().pb_1().text_size(px(META)).text_color(p.muted).child(t!("subagent.kept", n = SUB_KEPT)));
    let result = sub.ended.as_ref().and_then(|e| e.result.clone().map(|r| (e.failed, r))).map(|(failed, text)| {
        v_flex()
            .child(label(p, t!("subagent.result").to_owned()))
            .child(div().p_3().rounded(px(R_SM)).bg(if failed { p.danger_bg() } else { p.wash }).child(words(shell, ("sub-result", key), &text, if failed { p.danger() } else { p.prose })))
    });

    if shell.chat.sub.follows(steps.len()) {
        shell.chat.sub.scroll.scroll_to_bottom();
    }
    let body = v_flex()
        .id("sub-panel-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&shell.chat.sub.scroll)
        .px_4()
        .pb_6()
        .child(label(p, t!("subagent.brief").to_owned()))
        .child(div().text_color(p.text2).child(words(shell, ("sub-brief", key), &brief, p.text2)))
        .child(label(p, t!("subagent.steps").to_owned()))
        .children(cut)
        .children(steps)
        .children(now)
        .children(nothing)
        .children(result);

    let mut panel = v_flex()
        .id("sub-panel")
        .w(px(WIDTH))
        .flex_none()
        .h_full()
        .bg(p.bg)
        .border_l_1()
        .border_color(p.hair)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" && this.chat.sub.close() {
                cx.stop_propagation();
                cx.notify();
            }
        }));
    if let Some(focus) = &shell.chat.sub.focus {
        panel = panel.track_focus(focus);
    }
    Some(motion::fade(panel.child(head).child(status).child(body), "sub-panel-in", motion::BASE).into_any_element())
}

// ── what the panel does ──

impl Shell {
    /// A subagent's row clicked: a look inside it, beside its member's conversation.
    pub fn open_subagent_panel(&mut self, member: SessionId, session: SessionId, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.chat.side.clear();
        self.chat.sub.show(Open { member, session, id: id.to_owned() });
        let focus = self.chat.sub.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{Open, SubPanel};

    #[test]
    fn the_arrows_go_through_the_members_subagents_and_stop_at_the_ends() {
        let all = vec![(1, "a".to_owned()), (77, "b".to_owned()), (1, "c".to_owned())];
        let mut panel = SubPanel::default();
        assert_eq!(panel.beside(&all, 1), None, "nothing open");
        panel.show(Open { member: 1, session: 77, id: "b".into() });
        assert_eq!(panel.beside(&all, -1), Some(Open { member: 1, session: 1, id: "a".into() }));
        assert_eq!(panel.beside(&all, 1), Some(Open { member: 1, session: 1, id: "c".into() }));
        panel.show(Open { member: 1, session: 1, id: "a".into() });
        assert_eq!(panel.beside(&all, -1), None, "the first has none before it");
    }

    #[test]
    fn steps_open_and_close_and_another_subagent_starts_folded() {
        let mut panel = SubPanel::default();
        panel.show(Open { member: 1, session: 1, id: "a".into() });
        panel.toggle_step(2);
        assert!(panel.expanded.contains(&2));
        panel.toggle_step(2);
        assert!(panel.expanded.is_empty());
        panel.toggle_step(0);
        panel.show(Open { member: 1, session: 1, id: "b".into() });
        assert!(panel.expanded.is_empty(), "another's steps start folded");
        assert!(panel.close() && !panel.close(), "closed once");
    }

    #[test]
    fn new_steps_scroll_into_sight_only_while_the_list_sits_at_its_end() {
        let panel = SubPanel::default();
        panel.at_end.set(true);
        assert!(panel.follows(1), "a new step at the end: followed");
        assert!(!panel.follows(1), "nothing new: stays");
        // Scrolled up: the list doesn't sit at its end any more.
        panel.at_end.set(false);
        assert!(!panel.follows(2), "a new step while reading back: left where it is");
    }
}
