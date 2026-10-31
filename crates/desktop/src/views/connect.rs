//! Everything about *which machine*: the chooser, the box that answers `ssh`'s
//! questions (password, code, "do you trust this host?"), and the guide shown
//! when the window can't attach — one per cause, in words for someone who has
//! never used SSH.

use equipe_core::t;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::askpass::{Answer, Ask, Prompt};
use crate::daemon::{Leg, Update};
use crate::failure::Guide;
use crate::tailnet;
use crate::target::{Endpoint, SshHost};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{BODY, FIELD, ICON, ICON_SM, MONO, Palette, R_MD, R_XL, S3, S6, SMALL, UI};
use crate::ui::{self, Kind};
use crate::Shell;

/// The chooser's state.
pub struct ConnectUi {
    pub open: bool,
    /// Its one box: narrows the list, or takes a `user@host` to connect to.
    pub input: Entity<InputState>,
    /// Hosts `~/.ssh/config` names.
    pub hosts: Vec<String>,
    /// Why the last typed destination was refused.
    pub error: Option<String>,
    /// The Tailscale devices this Mac can reach, as last asked.
    pub peers: Vec<tailnet::Peer>,
    /// Whether each host answers, by what it is connected to.
    pub reach: HashMap<String, tailnet::Reach>,
    /// The hidden `~/.ssh/config` hosts are listed too, to show them again.
    pub show_hidden: bool,
    pub _probe: Option<Task<()>>,
}

/// Where a row of the chooser came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// What the person typed in the box, not in the list.
    Typed,
    /// Connected to before.
    Recent,
    /// A Tailscale device, by its name: a user to add before connecting.
    Tailnet,
    /// A host of `~/.ssh/config`.
    Config,
    /// A host of `~/.ssh/config` the person hid.
    Hidden,
}

/// A row of the chooser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What connecting to it is given (a Tailnet row: its host, no user yet).
    pub spec: String,
    /// Its name: the device's, when Tailscale knows the address.
    pub title: String,
    /// What else to say: the address under the name, when last seen.
    pub detail: String,
    pub source: Source,
}

/// The chooser's rows, by section, narrowed to `query`: what was typed (a
/// destination not in the list), the recent hosts, the tailnet's devices not
/// among them, `~/.ssh/config`'s hosts (the hidden ones only with
/// `show_hidden`).
pub fn choices(recent: &[SshHost], config: &[String], hidden: &[String], peers: &[tailnet::Peer], query: &str, show_hidden: bool) -> Vec<Choice> {
    let mut out = Vec::new();
    for host in recent.iter().filter(|h| !config.contains(&h.spec())) {
        let spec = host.spec();
        let (title, detail) = match tailnet::peer_for(peers, &host.host) {
            Some(peer) => (peer.name.clone(), spec.clone()),
            None => (spec.clone(), String::new()),
        };
        out.push(Choice { spec, title, detail, source: Source::Recent });
    }
    let known = |peer: &tailnet::Peer| recent.iter().any(|h| tailnet::peer_for(std::slice::from_ref(peer), &h.host).is_some()) || config.iter().any(|c| tailnet::peer_for(std::slice::from_ref(peer), c).is_some());
    let (q, now) = (query.trim(), now_secs());
    // Devices long unseen only when searched for.
    for peer in peers.iter().filter(|p| !known(p) && (!q.is_empty() || p.fresh(now))) {
        let detail = match peer.last_seen {
            Some(at) => format!("{} · {}", peer.short(), t!("connect.seen", ago = crate::activity::ago(now_secs().saturating_sub(at)))),
            None => peer.short().to_owned(),
        };
        out.push(Choice { spec: peer.host().to_owned(), title: peer.name.clone(), detail, source: Source::Tailnet });
    }
    for host in config {
        let hid = hidden.contains(host);
        if hid && !show_hidden {
            continue;
        }
        out.push(Choice { spec: host.clone(), title: host.clone(), detail: String::new(), source: if hid { Source::Hidden } else { Source::Config } });
    }
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return out;
    }
    let mut shown: Vec<Choice> = out.into_iter().filter(|c| [&c.spec, &c.title, &c.detail].iter().any(|t| t.to_lowercase().contains(&q))).collect();
    // A destination typed whole, not in the list: connect to it as typed.
    if q.contains('@') && SshHost::parse(query).is_ok() && !shown.iter().any(|c| c.spec.eq_ignore_ascii_case(query.trim())) {
        shown.insert(0, Choice { spec: query.trim().to_owned(), title: query.trim().to_owned(), detail: String::new(), source: Source::Typed });
    }
    shown
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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
                        .child(div().pt(px(3.)).flex_none().child(icon(Icon::Info, ICON_SM, p.muted)))
                        .child(div().flex_1().min_w_0().child(why))
                }))
                .child(h_flex().pt_2().child(ui::button(p, "switch", Kind::Secondary, t!("connect.switch_target")).field().on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx))))),
            "connecting-in",
        ))
        .into_any_element()
}

/// The Tailscale device the host that can't be reached was most likely
/// meant to be — an address a digit off, a name a letter off — offered to
/// connect to instead.
fn suggestion(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let Endpoint::Ssh(typed) = &shell.endpoint else { return None };
    let peer = tailnet::near(&shell.connect.peers, &typed.host)?;
    let host = if typed.host.parse::<std::net::IpAddr>().is_ok() { peer.ip.clone() } else { peer.host().to_owned() };
    let meant = SshHost { host, ..typed.clone() };
    let wrong = typed.clone();
    Some(
        h_flex()
            .gap_3()
            .px(px(S3))
            .py_2()
            .items_center()
            .rounded(px(R_MD))
            .bg(p.accent_bg)
            .child(icon(Icon::Lightbulb, ICON, p.accent))
            .child(div().flex_1().min_w_0().text_size(px(BODY)).line_height(px(22.)).text_color(p.text).child(t!("connect.did_you_mean", name = peer.name, ip = meant.spec())))
            .child(ui::button(p, "use-suggestion", Kind::Primary, t!("connect.use_suggestion")).on_click(cx.listener(move |this, _, window, cx| {
                // The one typed wrong goes from the list: it never worked.
                this.remembered.forget(&wrong);
                this.connect_to(Endpoint::Ssh(meant.clone()), window, cx);
            })))
            .into_any_element(),
    )
}

impl ConnectUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>, hosts: Vec<String>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("connect.host_placeholder")));
        Self { open: false, input, hosts, error: None, peers: Vec::new(), reach: HashMap::new(), show_hidden: false, _probe: None }
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
                // The likeliest fix first: the device it was meant to be.
                .children(suggestion(shell, cx))
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

/// "Where is the daemon?": this Mac, a host connected to before, a Tailscale
/// device, a host of `~/.ssh/config` — narrowed by what is typed in the box,
/// which also takes a new `user@host`. Each host says whether it answers; a
/// recent one can be taken off the list, a `~/.ssh/config` one hidden.
pub fn chooser(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let ui = &shell.connect;
    let query = ui.input.read(cx).value().to_string();
    let rows = choices(&shell.remembered.recent, &ui.hosts, &shell.remembered.hidden_hosts, &ui.peers, &query, ui.show_hidden);
    let this_mac = Endpoint::this_mac();
    let on_mac = matches!(shell.endpoint, Endpoint::Local { .. });
    let mac = row(p, "this-mac", Icon::Laptop, t!("connect.this_mac").into(), t!("connect.this_mac_note"), on_mac).on_click(cx.listener(move |this, _, window, cx| this.connect_to(this_mac.clone(), window, cx)));

    let mut list = v_flex().id("hosts").gap(px(2.)).max_h(px(340.)).overflow_y_scroll();
    if query.trim().is_empty() {
        list = list.child(hoverable(p, mac, "this-mac-fill", on_mac));
    }
    let mut last: Option<Source> = None;
    for (ix, choice) in rows.iter().enumerate() {
        let section = match choice.source {
            Source::Typed => None,
            Source::Recent => Some(t!("connect.recent")),
            Source::Tailnet => Some(t!("connect.tailnet")),
            Source::Config | Source::Hidden => Some(t!("connect.config")),
        };
        let same = matches!((last, choice.source), (Some(Source::Config | Source::Hidden), Source::Config | Source::Hidden)) || last == Some(choice.source);
        if let Some(section) = section.filter(|_| !same) {
            list = list.child(label(p, section));
        }
        last = Some(choice.source);
        list = list.child(choice_row(shell, ix, choice, cx));
    }
    let hidden = shell.remembered.hidden_hosts.iter().filter(|h| ui.hosts.contains(h)).count();
    if hidden > 0 && query.trim().is_empty() {
        let words = if ui.show_hidden { t!("connect.hidden_hide").to_owned() } else { t!("connect.hidden_show", count = hidden) };
        list = list.child(
            div()
                .id("hidden-toggle")
                .px(px(S3))
                .py_1()
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(move |s| s.text_color(p.text))
                .child(words)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.connect.show_hidden = !this.connect.show_hidden;
                    cx.notify();
                })),
        );
    }
    if rows.is_empty() && !query.trim().is_empty() {
        list = list.child(div().px(px(S3)).py_2().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("connect.none_match")));
    }

    modal_w(
        p,
        "chooser",
        540.,
        v_flex()
            .gap_3()
            .child(heading(p, t!("connect.title").into(), Some(t!("connect.intro").into())))
            .child(
                v_flex()
                    .gap_1()
                    .child(h_flex().gap_2().items_center().child(div().flex_1().min_w_0().child(search_field(p, &ui.input))))
                    .children(ui.error.clone().map(|e| div().px(px(S3)).pt_1().text_size(px(SMALL)).text_color(p.danger()).child(e))),
            )
            .child(list)
            .child(div().px(px(S3)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("connect.ssh_first_time")))
            .child(h_flex().pt_1().justify_end().child(ui::button(p, "close", Kind::Secondary, t!("connect.close")).field().key("Esc").on_click(cx.listener(|this, _, window, cx| this.close_chooser(window, cx))))),
    )
}

/// The chooser's box: a magnifier, then the text — it narrows the list and
/// takes a destination.
fn search_field(p: Palette, state: &Entity<InputState>) -> impl IntoElement {
    h_flex()
        .w_full()
        .h(px(FIELD))
        .pl(px(S3))
        .gap(px(6.))
        .items_center()
        .rounded(px(R_MD))
        .bg(p.code_bg)
        .border_1()
        .border_color(p.hair)
        .text_size(px(UI))
        .text_color(p.text)
        .child(icon(Icon::Search, 14., p.muted))
        .child(div().flex_1().min_w_0().child(Input::new(state).appearance(false).bordered(false)))
}

/// A host's row: its name and what else to say, whether it answers, and on
/// the pointer what can be done to it besides connecting.
fn choice_row(shell: &Shell, ix: usize, choice: &Choice, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let current = matches!((&shell.endpoint, SshHost::parse(&choice.spec)), (Endpoint::Ssh(c), Ok(h)) if *c == h) && choice.source != Source::Tailnet;
    let group: SharedString = format!("host-row-{ix}").into();
    let glyph = match choice.source {
        Source::Typed => Icon::Plus,
        Source::Tailnet => Icon::Globe,
        _ => Icon::Server,
    };
    let reach = shell.connect.reach.get(&choice.spec).copied();
    let title = match choice.source {
        Source::Typed => t!("connect.typed", host = choice.title),
        _ => choice.title.clone(),
    };
    let action: Option<AnyElement> = match choice.source {
        Source::Recent => {
            let host = choice.spec.clone();
            Some(row_action(p, ("forget", ix), Icon::X, t!("connect.forget"), &group).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.forget_host(&host, cx);
            })).into_any_element())
        }
        Source::Config => {
            let host = choice.spec.clone();
            Some(row_action(p, ("hide", ix), Icon::X, t!("connect.hide"), &group).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.hide_host(&host, true, cx);
            })).into_any_element())
        }
        Source::Hidden => {
            let host = choice.spec.clone();
            Some(
                div()
                    .id(("unhide", ix))
                    .px_2()
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .cursor_pointer()
                    .child(t!("connect.unhide"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.hide_host(&host, false, cx);
                    }))
                    .into_any_element(),
            )
        }
        Source::Typed | Source::Tailnet => None,
    };
    let answer = match reach {
        Some(tailnet::Reach::Answers) => Some(crate::tips::tip(div().id(("answers", ix)).size(px(16.)).flex().items_center().justify_center().child(crate::views::board::dot(p.accent, 7.)), p, t!("connect.answers"), None).into_any_element()),
        Some(tailnet::Reach::Silent) => Some(div().flex_none().text_size(px(12.)).text_color(p.muted).child(t!("connect.silent")).into_any_element()),
        _ => None,
    };
    let row = h_flex()
        .id(("host", ix))
        .group(group.clone())
        .h(px(FIELD))
        .px(px(S3))
        .gap_2()
        .items_center()
        .rounded(px(R_MD))
        .text_size(px(UI))
        .text_color(if choice.source == Source::Hidden { p.muted } else { p.text })
        .when(current, |r| r.bg(p.wash))
        .cursor_pointer()
        .child(div().w(px(ICON)).flex_none().flex().justify_center().when(current, |d| d.child(icon(Icon::Check, ICON, p.accent))))
        .child(icon(glyph, ICON, if choice.source == Source::Typed { p.accent } else { p.muted }))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .items_baseline()
                .overflow_hidden()
                .child(div().flex_none().max_w(px(260.)).overflow_hidden().whitespace_nowrap().text_ellipsis().when(current, |d| d.font_weight(FontWeight::SEMIBOLD)).child(title))
                .when(!choice.detail.is_empty(), |r| r.child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(p.muted).child(choice.detail.clone()))),
        )
        .children(answer)
        // The same slot on every row, so the answers line up.
        .child(div().min_w(px(22.)).flex_none().flex().justify_end().children(action));
    let chosen = choice.clone();
    let row = row.on_click(cx.listener(move |this, _, window, cx| this.choose(&chosen, window, cx)));
    hoverable(p, row, ("host-fill", ix), current)
}

/// A small button at a row's end, seen on the pointer.
fn row_action(p: Palette, id: impl Into<ElementId>, glyph: Icon, name: &'static str, group: &SharedString) -> Stateful<Div> {
    crate::tips::tip(
        div()
            .id(id.into())
            .size(px(22.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .cursor_pointer()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .hover(move |s| s.bg(p.code_bg))
            .child(icon(glyph, 13., p.muted)),
        p,
        name,
        None,
    )
}

impl Shell {
    /// Ask what is around (the tailnet) and whether each host answers, as
    /// the chooser opens; rows fill in as answers come.
    pub fn probe_hosts(&mut self, cx: &mut Context<Self>) {
        self.connect.reach.clear();
        self.connect._probe = Some(cx.spawn(async move |this, cx| {
            let peers = cx.background_executor().spawn(async { tailnet::load() }).await;
            let Some(specs) = this
                .update(cx, |this, cx| {
                    this.connect.peers = peers;
                    let rows = choices(&this.remembered.recent, &this.connect.hosts, &this.remembered.hidden_hosts, &this.connect.peers, "", true);
                    let specs: Vec<String> = rows.into_iter().map(|c| c.spec).collect();
                    for spec in &specs {
                        this.connect.reach.insert(spec.clone(), tailnet::Reach::Checking);
                    }
                    cx.notify();
                    specs
                })
                .ok()
            else {
                return;
            };
            let checks: Vec<_> = specs
                .into_iter()
                .filter_map(|spec| {
                    let host = SshHost::parse(&spec).ok()?;
                    Some((spec, cx.background_executor().spawn(async move { tailnet::reach(&host) })))
                })
                .collect();
            for (spec, check) in checks {
                let reach = check.await;
                if this
                    .update(cx, |this, cx| {
                        this.connect.reach.insert(spec, reach);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    /// What the tailnet holds, once at launch: a host that can't be reached
    /// may be one of its devices typed a digit wrong.
    pub fn load_peers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let peers = cx.background_executor().spawn(async { tailnet::load() }).await;
            let _ = this.update(cx, |this, cx| {
                this.connect.peers = peers;
                cx.notify();
            });
        })
        .detach();
    }

    /// A row of the chooser, clicked (or the first one, on Enter): connect —
    /// or, a Tailscale device, put it in the box for the user to go before it.
    pub fn choose(&mut self, choice: &Choice, window: &mut Window, cx: &mut Context<Self>) {
        if choice.source != Source::Tailnet {
            return self.connect_typed(&choice.spec, window, cx);
        }
        let text = format!("@{}", choice.spec);
        self.connect.error = None;
        self.connect.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_cursor_position(gpui_kit::component::input::Position::new(0, 0), window, cx);
        });
        cx.notify();
    }

    /// Enter in the chooser's box: its first row — what was typed, or the
    /// one host it narrowed to — else what was typed, as typed.
    pub fn choose_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.connect.input.read(cx).value().to_string();
        if query.trim().is_empty() {
            return;
        }
        // "@host", once a user is typed before it, is a destination.
        if query.trim().starts_with('@') {
            self.connect.error = Some(t!("connect.need_user").into());
            return cx.notify();
        }
        let rows = choices(&self.remembered.recent, &self.connect.hosts, &self.remembered.hidden_hosts, &self.connect.peers, &query, self.connect.show_hidden);
        match rows.first() {
            Some(first) if first.source != Source::Tailnet || !query.contains('@') => {
                let first = first.clone();
                self.choose(&first, window, cx);
            }
            _ => self.connect_typed(&query, window, cx),
        }
    }

    /// Take a host typed before off the chooser's list.
    pub fn forget_host(&mut self, spec: &str, cx: &mut Context<Self>) {
        if let Ok(host) = SshHost::parse(spec) {
            self.remembered.forget(&host);
            self.remembered.save(&equipe_providers::credentials::equipe_home());
        }
        cx.notify();
    }

    /// Hide a host of `~/.ssh/config` from the chooser, or show it again.
    pub fn hide_host(&mut self, name: &str, hidden: bool, cx: &mut Context<Self>) {
        self.remembered.hide_host(name, hidden);
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        cx.notify();
    }
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


#[cfg(test)]
mod tests {
    use super::{Source, choices};
    use crate::tailnet::{Peer, parse};
    use crate::target::SshHost;

    fn peers() -> Vec<Peer> {
        parse(
            r#"{"Peer": {
                "b": {"HostName": "tinums-mac-mini", "DNSName": "tinums-mac-mini.tail3383cf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.66.18.3"], "Online": true},
                "d": {"HostName": "Abel의 MacBook Air", "DNSName": "abels-macbook-air.tail5abbbf.ts.net.", "OS": "macOS", "TailscaleIPs": ["100.93.184.56"], "Online": true}
            }}"#,
        )
    }

    fn host(text: &str) -> SshHost {
        SshHost::parse(text).unwrap()
    }

    #[test]
    fn the_list_is_recent_then_tailnet_then_config_each_named_once() {
        let recent = [host("atinumai@100.66.18.3"), host("atinumai@110.66.18.3"), host("macmini")];
        let config = ["macmini".to_owned(), "vision".to_owned(), "ironact-prod".to_owned()];
        let rows = choices(&recent, &config, &["ironact-prod".to_owned()], &peers(), "", false);
        let seen: Vec<(Source, &str, &str)> = rows.iter().map(|c| (c.source, c.title.as_str(), c.detail.as_str())).collect();
        assert_eq!(
            seen,
            [
                (Source::Recent, "tinums-mac-mini", "atinumai@100.66.18.3"),
                (Source::Recent, "atinumai@110.66.18.3", ""),
                (Source::Tailnet, "Abel의 MacBook Air", "abels-macbook-air"),
                (Source::Config, "macmini", ""),
                (Source::Config, "vision", ""),
            ],
            "a recent address is named by its device; that device isn't offered again; a hidden host is left out; a config host isn't listed twice"
        );
        let all = choices(&recent, &config, &["ironact-prod".to_owned()], &peers(), "", true);
        assert_eq!(all.last().map(|c| (c.source, c.spec.as_str())), Some((Source::Hidden, "ironact-prod")), "shown again on asking");
    }

    #[test]
    fn typing_narrows_the_list_and_a_whole_destination_comes_first() {
        let recent = [host("atinumai@100.66.18.3")];
        let config = ["macmini".to_owned(), "vision".to_owned()];
        let rows = choices(&recent, &config, &[], &peers(), "tinums", false);
        assert_eq!(rows.iter().map(|c| c.spec.as_str()).collect::<Vec<_>>(), ["atinumai@100.66.18.3"], "found by its device's name");
        let rows = choices(&recent, &config, &[], &peers(), "me@new-box:2222", false);
        assert_eq!(rows.first().map(|c| (c.source, c.spec.as_str())), Some((Source::Typed, "me@new-box:2222")));
        let rows = choices(&recent, &config, &[], &peers(), "atinumai@100.66.18.3", false);
        assert_eq!(rows.iter().map(|c| c.source).collect::<Vec<_>>(), [Source::Recent], "one already listed isn't offered twice");
        assert!(choices(&recent, &config, &[], &peers(), "zzz", false).is_empty());
    }
}
