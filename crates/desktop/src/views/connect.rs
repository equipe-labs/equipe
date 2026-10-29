//! Everything about *which machine*: the chooser, the box that answers `ssh`'s
//! questions (password, code, "do you trust this host?"), and the guide shown
//! when the window can't attach — one per cause, in words for someone who has
//! never used SSH.

use equipe_core::t;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::askpass::{Answer, Ask, Prompt};
use crate::daemon::{Leg, Update};
use crate::failure::Guide;
use crate::target::Endpoint;
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, FIELD, ICON, MONO, Palette, R_MD, R_XL, S3, S6, SMALL, UI};
use crate::ui::{self, Kind};
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
    /// "Don't ask again": once the password worked, the app's key goes onto
    /// that machine (`sshkey.rs`). On unless the person takes it off.
    pub remember: bool,
}

/// How far the connection the window waits on has got.
#[derive(Debug, Clone)]
pub struct Connecting {
    pub leg: Leg,
    /// Where it goes, once `ssh` has said.
    pub door: Option<String>,
    /// Since when on this leg.
    pub since: Instant,
    /// The person pressed Try Again on a guide: this try is shown, not the guide.
    pub retrying: bool,
}

/// After this long on a leg, the screen says what may be keeping it.
pub const SLOW: [(Leg, Duration); 3] = [(Leg::Reaching, Duration::from_secs(6)), (Leg::SigningIn, Duration::from_secs(20)), (Leg::Opening, Duration::from_secs(8))];

impl Connecting {
    pub fn new(now: Instant) -> Self {
        Self { leg: Leg::Reaching, door: None, since: now, retrying: false }
    }

    /// One update from the link; true when the screen changes. A new try
    /// starts over; a leg only moves forward.
    pub fn hear(&mut self, update: &Update, now: Instant) -> bool {
        match update {
            Update::Leg(Leg::Reaching) if self.leg != Leg::Reaching => {
                (self.leg, self.since) = (Leg::Reaching, now);
                true
            }
            Update::Leg(leg) => self.reached(*leg, now),
            Update::Door(door) if self.door.as_ref() != Some(door) => {
                self.door = Some(door.clone());
                true
            }
            Update::Down(_) => std::mem::take(&mut self.retrying),
            _ => false,
        }
    }

    /// At `leg` now, if that is further.
    pub fn reached(&mut self, leg: Leg, now: Instant) -> bool {
        if leg <= self.leg {
            return false;
        }
        (self.leg, self.since) = (leg, now);
        true
    }

    /// What may be keeping it, once it has been a while on this leg.
    pub fn slow(&self, now: Instant) -> Option<&'static str> {
        let (_, after) = SLOW.iter().find(|(leg, _)| *leg == self.leg)?;
        (now.saturating_duration_since(self.since) >= *after).then(|| match self.leg {
            Leg::Reaching => t!("connect.slow_reach"),
            Leg::SigningIn => t!("connect.slow_sign_in"),
            Leg::Opening => t!("connect.slow_open"),
        })
    }
}

/// The body while a connection over SSH is being made: each leg, how long the
/// current one has taken, and what may be keeping it.
pub fn connecting_screen(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let c = &shell.connecting;
    let now = Instant::now();
    let waited = now.saturating_duration_since(c.since).as_secs();
    let legs = [(Leg::Reaching, t!("connect.leg_reach")), (Leg::SigningIn, t!("connect.leg_sign_in")), (Leg::Opening, t!("connect.leg_open"))];
    let rows = legs.into_iter().map(|(leg, label)| {
        let at = leg.cmp(&c.leg);
        let mark = match at {
            std::cmp::Ordering::Less => icon(Icon::Check, 14., p.accent).into_any_element(),
            std::cmp::Ordering::Equal => motion::spinner(14., p.text2),
            std::cmp::Ordering::Greater => div().size(px(6.)).rounded_full().bg(p.hair).into_any_element(),
        };
        h_flex()
            .gap_2()
            .items_center()
            .min_h(px(24.))
            .text_size(px(BODY))
            .child(div().size(px(16.)).flex_none().flex().items_center().justify_center().child(mark))
            .child(div().min_w_0().text_color(if at == std::cmp::Ordering::Greater { p.muted } else { p.text }).child(label))
            .when(at == std::cmp::Ordering::Equal && waited >= 3, |row| row.child(div().flex_none().text_size(px(SMALL)).text_color(p.muted).child(t!("connect.seconds", n = waited))))
    });
    v_flex()
        .id("connecting")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .items_center()
        .justify_center()
        .py_8()
        .child(motion::rise(
            v_flex()
                .w_full()
                .max_w(px(440.))
                .px_4()
                .gap_3()
                .child(div().text_size(px(18.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(t!("connect.connecting_title", name = shell.endpoint.label())))
                .children(c.door.clone().map(|door| div().text_size(px(SMALL)).text_color(p.muted).font_family(MONO).child(t!("connect.door", door = door))))
                .child(v_flex().gap_1().pt_1().children(rows))
                .children(c.slow(now).map(|why| {
                    h_flex()
                        .gap_2()
                        .items_start()
                        .p(px(S3))
                        .rounded(px(R_MD))
                        .bg(p.wash)
                        .text_size(px(SMALL))
                        .line_height(px(20.))
                        .text_color(p.text2)
                        .child(div().pt(px(2.)).flex_none().child(icon(Icon::Info, 14., p.muted)))
                        .child(div().flex_1().min_w_0().child(why))
                }))
                .child(h_flex().pt_2().child(ui::button(p, "switch", Kind::Secondary, t!("connect.switch_target")).field().on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx))))),
            "connecting-in",
        ))
        .into_any_element()
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
        Self { queue: VecDeque::new(), input, remember: true }
    }
}

pub fn button(p: Palette, id: &'static str, label: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
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

/// A one-line text box in the window's own colours, `FIELD` tall, its text
/// `S3` in from its edge (the box pads its text itself; the rest is ours).
fn field(p: Palette, state: &Entity<InputState>) -> impl IntoElement {
    h_flex()
        .w_full()
        .h(px(FIELD))
        .pl(px(S3) - gpui_kit::component::Size::Medium.input_px())
        .items_center()
        .rounded(px(R_MD))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .text_size(px(UI))
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
                .h(px(FIELD))
                .px(px(S3))
                .flex()
                .items_center()
                .rounded(px(R_MD))
                .bg(p.code_bg)
                .border_1()
                .border_color(p.hair)
                .font_family(MONO)
                .text_size(px(SMALL))
                .text_color(p.text2)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(text),
        )
        .child(ui::button(p, ("copy", ix), Kind::Secondary, t!("connect.copy")).icon(Icon::Copy).field().on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))))
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
        .child(motion::rise(
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
                        .child(ui::button(p, "retry", Kind::Primary, t!("connect.retry")).field().on_click(cx.listener(|this, _, _, cx| this.retry(cx))))
                        .child(ui::button(p, "switch", Kind::Secondary, t!("connect.switch_target")).field().on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx)))),
                ),
            "guide-in",
        ))
        .into_any_element()
}

/// The chooser's grid: every row is [S3 | ✓ | S2 | name … detail | S3], inside
/// a highlight with the same S3 padding; the section labels and the field's
/// text start at that same S3.
fn row(p: Palette, id: impl Into<ElementId>, glyph: Icon, label: String, note: &'static str, current: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(FIELD))
        .px(px(S3))
        .gap_2()
        .items_center()
        .rounded(px(R_MD))
        .text_size(px(UI))
        .text_color(p.text)
        .when(current, |r| r.bg(p.wash).font_weight(FontWeight::SEMIBOLD))
        .when(!current, |r| r.cursor_pointer())
        .child(div().w(px(ICON)).flex_none().flex().justify_center().when(current, |d| d.child(icon(Icon::Check, ICON, p.accent))))
        .child(icon(glyph, ICON, p.muted))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
        .child(div().flex_none().font_weight(FontWeight::NORMAL).text_size(px(12.)).text_color(p.muted).child(note))
}

/// A row that eases its fill in under the pointer (the current one is filled already).
fn hoverable(p: Palette, row: Stateful<Div>, id: impl Into<ElementId>, current: bool) -> AnyElement {
    if current { row.into_any_element() } else { motion::hover_fill(row, id, p.wash, false).into_any_element() }
}

/// A section's name, at the rows' inner padding.
fn label(p: Palette, text: &'static str) -> impl IntoElement {
    div().px(px(S3)).pt_2().pb_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(p.muted).child(text)
}

/// A box over the window: the window dims, the card settles in.
fn modal(p: Palette, id: &'static str, card: impl IntoElement) -> AnyElement {
    modal_w(p, id, 480., card)
}

/// [`modal`], `width` wide.
pub fn modal_w(p: Palette, id: &'static str, width: f32, card: impl IntoElement) -> AnyElement {
    motion::fade(
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000073))
            // A press on the dimmed window around the box does what Esc does.
            .on_mouse_down(MouseButton::Left, |_, window, cx| window.dispatch_action(Box::new(crate::DismissOverlay), cx))
            .child(motion::dialog(
                v_flex()
                    // A press inside the box is the box's.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .w(px(width))
                    .max_h(px(640.))
                    .p(px(S6))
                    .gap_3()
                    .rounded(px(R_XL))
                    .bg(p.popover())
                    .border_1()
                    .border_color(p.hair)
                    .text_color(p.text)
                    .child(card),
                (id, 1usize),
            )),
        (id, 0usize),
        motion::FAST,
    )
    .into_any_element()
}

/// The title and the words under it, as every box here has them.
fn heading(p: Palette, title: String, text: Option<String>) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .children(text.map(|t| div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t)))
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
    let hosts: Vec<AnyElement> = hosts
        .enumerate()
        .map(|(ix, (spec, note))| {
            let current = matches!((current, crate::target::SshHost::parse(&spec)), (Endpoint::Ssh(c), Ok(h)) if *c == h);
            let target = spec.clone();
            let row = row(p, ("host", ix), Icon::Server, spec, note, current).on_click(cx.listener(move |this, _, window, cx| this.connect_typed(&target, window, cx)));
            hoverable(p, row, ("host-fill", ix), current)
        })
        .collect();
    let on_mac = matches!(current, Endpoint::Local { .. });
    let mac = row(p, "this-mac", Icon::Laptop, t!("connect.this_mac").into(), t!("connect.this_mac_note"), on_mac).on_click(cx.listener(move |this, _, window, cx| this.connect_to(this_mac.clone(), window, cx)));

    modal(
        p,
        "chooser",
        v_flex()
            .gap_3()
            .child(heading(p, t!("connect.title").into(), Some(t!("connect.intro").into())))
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(hoverable(p, mac, "this-mac-fill", on_mac))
                    .when(!hosts.is_empty(), |c| c.child(label(p, t!("connect.ssh_hosts"))).child(v_flex().id("hosts").gap(px(2.)).max_h(px(196.)).overflow_y_scroll().children(hosts))),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(label(p, t!("connect.type_host")))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(field(p, &ui.input)))
                            .child(ui::button(p, "connect", Kind::Primary, t!("connect.connect")).field().on_click(cx.listener(|this, _, window, cx| {
                                let text = this.connect.input.read(cx).value().to_string();
                                this.connect_typed(&text, window, cx);
                            }))),
                    )
                    .children(ui.error.clone().map(|e| div().px(px(S3)).pt_1().text_size(px(SMALL)).text_color(p.danger()).child(e))),
            )
            .child(div().px(px(S3)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connect.ssh_first_time")))
            .child(h_flex().pt_1().justify_end().child(ui::button(p, "close", Kind::Secondary, t!("connect.close")).field().key("Esc").on_click(cx.listener(|this, _, window, cx| this.close_chooser(window, cx))))),
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
    let buttons = |no: (&'static str, &'static str), yes: (&'static str, &'static str)| (no, yes);
    let (card, (no, yes)) = match &prompt.ask {
        Ask::Secret { label } => {
            // A password over SSH (not a key's passphrase): asked once, if the person wants.
            let password = shell.endpoint.is_remote() && label.to_lowercase().contains("password");
            let remember = shell.ask.remember;
            let check = div()
                .flex_none()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if remember { p.accent } else { p.muted })
                .when(remember, |c| c.bg(p.accent).child(icon(Icon::Check, 11., p.on_accent)));
            let keep = h_flex()
                .id("ask-remember")
                .px(px(S3))
                .gap_2()
                .items_start()
                .cursor_pointer()
                .child(div().pt(px(2.)).child(check))
                .child(
                    v_flex()
                        .gap(px(2.))
                        .child(div().text_size(px(SMALL)).text_color(p.text).child(t!("connect.remember")))
                        .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connect.remember_note"))),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ask.remember = !this.ask.remember;
                    cx.notify();
                }));
            (
                v_flex()
                    .gap_3()
                    .child(heading(p, t!("connect.sign_in_to", who = who), Some(label.clone())))
                    .child(field(p, &shell.ask.input))
                    .when(password, |c| c.child(keep))
                    .when(!password, |c| c.child(div().px(px(S3)).text_size(px(12.)).text_color(p.muted).child(t!("connect.secret_note")))),
                buttons(("ask-cancel", t!("connect.cancel")), ("ask-ok", t!("connect.ok"))),
            )
        }
        Ask::HostKey { host, fingerprint, text } => (
            v_flex()
                .gap_3()
                .child(heading(p, t!("connect.new_machine").into(), Some(t!("connect.host_key_text", host = if host.is_empty() { who.clone() } else { format!("'{host}'") }))))
                .when(!fingerprint.is_empty(), |c| c.child(div().px(px(S3)).py_2().rounded(px(R_MD)).bg(p.code_bg).border_1().border_color(p.hair).font_family(MONO).text_size(px(SMALL)).text_color(p.text).child(fingerprint.clone())))
                .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(text.lines().filter(|l| !l.contains("Are you sure")).collect::<Vec<_>>().join("\n"))),
            buttons(("ask-no", t!("connect.dont_connect")), ("ask-yes", t!("connect.fingerprint_ok"))),
        ),
        Ask::Confirm { text } => (
            v_flex().gap_3().child(heading(p, t!("connect.ssh_confirm").into(), Some(text.clone()))),
            buttons(("ask-no", t!("connect.no")), ("ask-yes", t!("connect.yes"))),
        ),
    };
    let secret = matches!(prompt.ask, Ask::Secret { .. });
    let yes_button = ui::button(p, yes.0, Kind::Primary, yes.1).field();
    let yes_button = if secret { yes_button.on_click(cx.listener(|this, _, window, cx| this.submit_secret(window, cx))) } else { yes_button.on_click(answer(Answer::Yes)) };
    let card = card.child(h_flex().pt_1().gap_2().justify_end().child(ui::button(p, no.0, Kind::Secondary, no.1).field().on_click(answer(Answer::Cancel))).child(yes_button));
    Some(modal(p, "question", card))
}

