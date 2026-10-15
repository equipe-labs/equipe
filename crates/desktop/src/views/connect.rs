//! Everything about *which machine*: the chooser, the box that answers `ssh`'s
//! questions (password, code, "do you trust this host?"), and the guide shown
//! when the window can't attach — one per cause, in words for someone who has
//! never used SSH.

use equipe_core::t;
use std::collections::VecDeque;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::askpass::{Answer, Ask, Prompt};
use crate::failure::Guide;
use crate::target::Endpoint;
use crate::theme::{BODY, MONO, Palette, SMALL};
use crate::Shell;

/// The chooser's state.
pub struct ConnectUi {
    pub open: bool,
    pub input: Entity<InputState>,
    /// Hosts `~/.ssh/config` names.
    pub hosts: Vec<String>,
    /// Why the last typed destination was refused.
    pub error: Option<String>,
}

/// Questions from `ssh`, oldest first; the front one is on screen.
pub struct AskUi {
    pub queue: VecDeque<Prompt>,
    pub input: Entity<InputState>,
}

impl ConnectUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>, hosts: Vec<String>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("connect.host_placeholder")));
        Self { open: false, input, hosts, error: None }
    }
}

impl AskUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("connect.secret_placeholder")));
        Self { queue: VecDeque::new(), input }
    }
}

fn button(p: Palette, id: &'static str, label: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(30.))
        .px_3()
        .items_center()
        .rounded(px(6.))
        .text_size(px(SMALL))
        .cursor_pointer()
        .when(primary, |b| b.bg(p.primary).text_color(p.on_primary).font_weight(FontWeight::SEMIBOLD))
        .when(!primary, |b| b.border_1().border_color(p.hair).hover(move |s| s.bg(p.wash)))
        .child(label.into())
}

/// A one-line text box in the window's own colours.
fn field(p: Palette, state: &Entity<InputState>) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .text_size(px(BODY))
        .text_color(p.text)
        .child(Input::new(state).appearance(false).bordered(false))
}

/// A line to paste in a terminal, with a copy button.
fn command_line(p: Palette, ix: usize, text: String) -> impl IntoElement {
    let copy = text.clone();
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .py_1()
                .rounded(px(8.))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .font_family(MONO)
                .text_size(px(SMALL))
                .text_color(p.text2)
                .child(text),
        )
        .child(button(p, "copy", t!("connect.copy"), false).id(("copy", ix)).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))))
}

/// The body when nothing is attached: the cause, what to do, the lines to paste.
pub fn guide_screen(shell: &Shell, guide: &Guide, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    v_flex()
        .id("guide")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .items_center()
        .py_8()
        .child(
            v_flex()
                .w_full()
                .max_w(px(560.))
                .px_4()
                .gap_3()
                .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(guide.title.clone()))
                .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(guide.text.clone()))
                .children(guide.steps.iter().enumerate().map(|(i, step)| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .text_size(px(BODY))
                        .line_height(px(24.))
                        .text_color(p.text2)
                        .child(div().w(px(20.)).flex_none().text_color(p.muted).child(format!("{}.", i + 1)))
                        .child(div().flex_1().min_w_0().child(step.clone()))
                }))
                .children(guide.commands.iter().cloned().enumerate().map(|(i, line)| command_line(p, i, line)))
                .children(guide.detail.clone().map(|d| div().text_size(px(SMALL)).text_color(p.muted).font_family(MONO).child(d)))
                .child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .child(button(p, "retry", t!("connect.retry"), true).on_click(cx.listener(|this, _, _, cx| this.retry(cx))))
                        .child(button(p, "switch", t!("connect.switch_target"), false).on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx)))),
                ),
        )
        .into_any_element()
}

fn row(p: Palette, id: impl Into<ElementId>, label: String, note: &'static str, current: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(36.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .text_size(px(BODY))
        .when(current, |r| r.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
        .when(!current, |r| r.hover(move |s| s.bg(p.wash)).cursor_pointer())
        .child(div().w(px(16.)).text_color(p.accent).child(if current { "✓" } else { "" }))
        .child(div().flex_1().min_w_0().child(label))
        .child(div().text_size(px(SMALL)).text_color(p.muted).child(note))
}

fn modal(p: Palette, card: impl IntoElement) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000066))
        .child(
            v_flex()
                .w(px(500.))
                .max_h(px(620.))
                .p_5()
                .gap_3()
                .rounded(px(12.))
                .bg(p.raised)
                .border_1()
                .border_color(p.hair)
                .text_color(p.text)
                .child(card),
        )
        .into_any_element()
}

/// "Where is the daemon?": this Mac or an SSH host.
pub fn chooser(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.connect;
    let current = &shell.endpoint;
    let remembered: Vec<_> = shell.remembered.recent.iter().filter(|h| !ui.hosts.contains(&h.spec())).map(|h| h.spec()).collect();
    let this_mac = Endpoint::this_mac();

    // The one in use may be neither in the config nor remembered (picked by an env var).
    let in_use = match current {
        Endpoint::Ssh(h) if !ui.hosts.contains(&h.spec()) && !remembered.contains(&h.spec()) => Some((h.spec(), t!("connect.host_current"))),
        _ => None,
    };
    let hosts = in_use.into_iter().chain(ui.hosts.iter().cloned().map(|h| (h, "~/.ssh/config"))).chain(remembered.into_iter().map(|h| (h, t!("connect.host_recent"))));
    let hosts: Vec<_> = hosts
        .enumerate()
        .map(|(ix, (spec, note))| {
            let current = matches!((current, crate::target::SshHost::parse(&spec)), (Endpoint::Ssh(c), Ok(h)) if *c == h);
            let target = spec.clone();
            row(p, ("host", ix), spec, note, current).on_click(cx.listener(move |this, _, window, cx| this.connect_typed(&target, window, cx)))
        })
        .collect();

    modal(
        p,
        v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(t!("connect.title")))
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("connect.intro")))
            .child(row(p, "this-mac", t!("connect.this_mac").into(), t!("connect.this_mac_note"), matches!(current, Endpoint::Local { .. })).on_click(cx.listener(move |this, _, window, cx| this.connect_to(this_mac.clone(), window, cx))))
            .when(!hosts.is_empty(), |c| {
                c.child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("connect.ssh_hosts")))
                    .child(v_flex().id("hosts").max_h(px(190.)).overflow_y_scroll().children(hosts))
            })
            .child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("connect.type_host")))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(field(p, &ui.input)))
                    .child(button(p, "connect", t!("connect.connect"), true).on_click(cx.listener(|this, _, window, cx| {
                        let text = this.connect.input.read(cx).value().to_string();
                        this.connect_typed(&text, window, cx);
                    }))),
            )
            .children(ui.error.clone().map(|e| div().text_size(px(SMALL)).text_color(p.accent).child(e)))
            .child(
                div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(
                    t!("connect.ssh_first_time"),
                ),
            )
            .child(h_flex().justify_end().child(button(p, "close", t!("connect.close"), false).on_click(cx.listener(|this, _, window, cx| this.close_chooser(window, cx))))),
    )
}

/// The question from `ssh` at the front of the queue, if any.
pub fn question(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let prompt = shell.ask.queue.front()?;
    let who = shell.endpoint.label();
    let answer = |a: Answer| {
        cx.listener(move |this, _, window, cx| {
            this.answer_prompt(a.clone(), window, cx);
        })
    };
    let card = match &prompt.ask {
        Ask::Secret { label } => v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(t!("connect.sign_in_to", who = who)))
            .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(label.clone()))
            .child(field(p, &shell.ask.input))
            .child(div().text_size(px(SMALL)).text_color(p.muted).child(t!("connect.secret_note")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(button(p, "ask-cancel", t!("connect.cancel"), false).on_click(answer(Answer::Cancel)))
                    .child(button(p, "ask-ok", t!("connect.ok"), true).on_click(cx.listener(|this, _, window, cx| this.submit_secret(window, cx)))),
            ),
        Ask::HostKey { host, fingerprint, text } => v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(t!("connect.new_machine")))
            .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(t!("connect.host_key_text", host = if host.is_empty() { who.clone() } else { format!("'{host}'") })))
            .when(!fingerprint.is_empty(), |c| {
                c.child(div().px_3().py_2().rounded(px(8.)).bg(p.code_bg).border_1().border_color(p.hair).font_family(MONO).text_size(px(SMALL)).text_color(p.text).child(fingerprint.clone()))
            })
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(text.lines().filter(|l| !l.contains("Are you sure")).collect::<Vec<_>>().join("\n")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(button(p, "ask-no", t!("connect.dont_connect"), false).on_click(answer(Answer::Cancel)))
                    .child(button(p, "ask-yes", t!("connect.fingerprint_ok"), true).on_click(answer(Answer::Yes))),
            ),
        Ask::Confirm { text } => v_flex()
            .gap_3()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(t!("connect.ssh_confirm")))
            .child(div().text_size(px(BODY)).line_height(px(24.)).text_color(p.text2).child(text.clone()))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(button(p, "ask-no", t!("connect.no"), false).on_click(answer(Answer::Cancel)))
                    .child(button(p, "ask-yes", t!("connect.yes"), true).on_click(answer(Answer::Yes))),
            ),
    };
    Some(modal(p, card))
}
