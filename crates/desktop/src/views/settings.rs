//! Settings (⌘,, or the gear at the sidebar's foot): a page in the window,
//! as Linear's and Codex's. Its own list takes the sidebar's place — 돌아가기,
//! a search that filters every row, the categories — and the page shows the
//! one picked. Rows are a label and a muted line on the left and the control
//! on the right, in rounded cards under short headers. A change applies at
//! once, with no Save button: what the daemon keeps goes to the daemon, what
//! is this window's own goes to `desktop-target`. The rules (categories,
//! search, the keys) are `crate::settings`; why it looks so,
//! docs/SETTINGS-RESEARCH.md.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use equipe_core::i18n::{Front, Lang};
use equipe_core::protocol::{Command, Mode, SessionId};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::{dot, kbd};
use crate::board::{Menu, MergeWay};
use crate::icons::{Icon, icon};
use crate::motion;
use crate::notify::Ping;
use crate::settings::{self as rules, Category};
use crate::state::Conn;
use crate::target::{Endpoint, Start};
use crate::theme::{Appearance, CONTROL, ICON, ICON_SM, MONO, Palette, R_LG, R_MD, R_SM, ROW, SANS, SMALL, UI, look};
use crate::tips::TipExt as _;
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The page's column, at most this wide.
const PAGE: f32 = 720.;

/// A pop-up open over the page: which control opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pop {
    Language,
    Squad,
    Model(SessionId),
    Effort(SessionId),
    Mode(SessionId),
}

/// What a pop-up's row does.
#[derive(Debug, Clone)]
enum Act {
    Language(Option<Lang>),
    Squad(String),
    Model(SessionId, String),
    Effort(SessionId, Option<String>),
    Mode(SessionId, Option<Mode>),
}

pub struct SettingsUi {
    pub category: Category,
    pub search: Entity<InputState>,
    /// Where the window was before settings opened, to go back to.
    pub back_to: Screen,
    /// Bumped whenever the page changes, to fade the new one in.
    pub turn: u64,
    pub pop: Option<Pop>,
    /// Where each control that opens a pop-up was last drawn.
    anchors: Rc<RefCell<HashMap<Pop, Bounds<Pixels>>>>,
    /// The switch just flipped, to slide its knob.
    flipped: Option<(SharedString, Instant)>,
    pub scroll: ScrollHandle,
}

impl SettingsUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            category: Category::default(),
            search: cx.new(|cx| InputState::new(window, cx).placeholder(t!("settings.search_placeholder"))),
            back_to: Screen::Chat,
            turn: 0,
            pop: None,
            anchors: Rc::default(),
            flipped: None,
            scroll: ScrollHandle::new(),
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_owned()
    }
}

// ── the page's pieces ──

/// One row: what it is, a line saying what it does, and its control. `words`
/// are what else search finds it by.
struct Line {
    label: AnyElement,
    text: String,
    desc: Option<(String, bool)>,
    words: String,
    control: Option<AnyElement>,
    /// A row of a list (a shortcut): shorter, no description.
    compact: bool,
}

impl Line {
    fn new(label: impl Into<String>, desc: impl Into<String>) -> Self {
        let label = label.into();
        let desc: String = desc.into();
        Self { label: div().child(label.clone()).into_any_element(), text: label, desc: (!desc.is_empty()).then_some((desc, false)), words: String::new(), control: None, compact: false }
    }

    fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    fn words(mut self, words: impl Into<String>) -> Self {
        self.words = words.into();
        self
    }

    /// The description in the danger colour: a choice that takes care.
    fn warn(mut self, warn: bool) -> Self {
        if let Some((_, w)) = self.desc.as_mut() {
            *w = warn;
        }
        self
    }

    /// The label drawn otherwise (a member's mark, a rule in monospace); `text` stays what search reads.
    fn label_el(mut self, el: impl IntoElement) -> Self {
        self.label = el.into_any_element();
        self
    }

    fn found_by(&self) -> String {
        format!("{} {} {}", self.text, self.desc.as_ref().map(|(d, _)| d.as_str()).unwrap_or_default(), self.words)
    }
}

struct Section {
    title: Option<String>,
    lines: Vec<Line>,
    /// A line under the card: whose setting it is, what follows from it.
    foot: Option<String>,
}

impl Section {
    fn new(title: impl Into<String>, lines: Vec<Line>) -> Self {
        let title: String = title.into();
        Self { title: (!title.is_empty()).then_some(title), lines, foot: None }
    }

    fn foot(mut self, foot: impl Into<String>) -> Self {
        self.foot = Some(foot.into());
        self
    }
}

// ── controls, on the app's tokens ──

/// On or off: a small switch, its knob sliding when flipped.
fn switch(shell: &Shell, id: &str, on: bool, enabled: bool, flip: impl Fn(&mut Shell, bool, &mut Window, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let key = SharedString::from(format!("switch-{id}"));
    let (w, h, knob) = (34., 20., 16.);
    let (from, to) = if on { (2., w - knob - 2.) } else { (w - knob - 2., 2.) };
    let moving = shell.settings.flipped.as_ref().filter(|(k, at)| *k == key && at.elapsed() < motion::BASE).map(|(_, at)| at.elapsed().as_nanos() as u64);
    let knob_color = if p.is_dark() && on { p.on_primary } else { rgb(0xFFFFFF) };
    let knob_el = div().absolute().top(px((h - knob) / 2.)).size(px(knob)).rounded_full().bg(knob_color).shadow(motion::float_shadow(0.25));
    let knob_el: AnyElement = match moving {
        Some(n) => knob_el.left(px(from)).with_animation(ElementId::NamedInteger(key.clone(), n), Animation::new(motion::FAST).with_easing(motion::ease_out), move |k, t| k.left(px(from + (to - from) * t))).into_any_element(),
        None => knob_el.left(px(to)).into_any_element(),
    };
    let track = if on { p.accent } else { p.side_sel };
    div()
        .id(key.clone())
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .rounded_full()
        .bg(track)
        .when(!on, |d| d.border_1().border_color(p.hair))
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| {
            d.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| {
                this.settings.flipped = Some((key.clone(), Instant::now()));
                flip(this, !on, window, cx);
                cx.notify();
            }))
        })
        .child(knob_el)
        .into_any_element()
}

/// A few fixed choices side by side, the one in use raised.
fn segmented<T: Copy + PartialEq + 'static>(shell: &Shell, id: &str, options: Vec<(T, String)>, now: T, set: impl Fn(&mut Shell, T, &mut Window, &mut Context<Shell>) + 'static, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let set = Rc::new(set);
    let raised = if p.is_dark() { p.side_sel } else { p.raised };
    h_flex()
        .flex_none()
        .h(px(CONTROL))
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(R_MD))
        .bg(if p.is_dark() { p.wash } else { p.code_bg })
        .children(options.into_iter().enumerate().map(|(ix, (value, label))| {
            let on = value == now;
            let set = set.clone();
            let seg = h_flex()
                .id(SharedString::from(format!("{id}-{ix}")))
                .h_full()
                .px_3()
                .items_center()
                .rounded(px(R_SM))
                .text_size(px(SMALL))
                .whitespace_nowrap()
                .text_color(if on { p.text } else { p.text2 })
                .when(on, |s| s.bg(raised).font_weight(FontWeight::MEDIUM).shadow(motion::float_shadow(0.18)))
                .when(!on, |s| s.cursor_pointer())
                .on_click(cx.listener(move |this, _, window, cx| {
                    set(this, value, window, cx);
                    cx.notify();
                }))
                .child(label);
            if on { seg.into_any_element() } else { motion::hover_fill(seg, SharedString::from(format!("{id}-{ix}-fill")), p.wash, false).into_any_element() }
        }))
        .into_any_element()
}

/// A pop-up button: what is chosen and ⌄; a click opens the list under it.
fn picker(shell: &Shell, pop: Pop, label: String, min: f32, cx: &mut Context<Shell>) -> AnyElement {
    use gpui_kit::base::ElementExt as _;
    let p = shell.palette;
    let open = shell.settings.pop == Some(pop);
    let anchors = shell.settings.anchors.clone();
    let id = SharedString::from(format!("picker-{pop:?}"));
    let el = h_flex()
        .id(id.clone())
        .flex_none()
        .h(px(CONTROL))
        .min_w(px(min))
        .max_w(px(220.))
        .pl_3()
        .pr_2()
        .gap_1p5()
        .items_center()
        .justify_between()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .text_color(p.text)
        .cursor_pointer()
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
        .child(icon(Icon::ChevronDown, ICON_SM, p.muted))
        .on_click(cx.listener(move |this, _, _, cx| {
            crate::tips::hide(cx);
            this.settings.pop = if this.settings.pop == Some(pop) { None } else { Some(pop) };
            cx.notify();
        }))
        .on_prepaint(move |bounds, _, _| {
            anchors.borrow_mut().insert(pop, bounds);
        });
    motion::hover_fill(el, SharedString::from(format!("{id}-fill")), p.wash, open).into_any_element()
}

/// A command to type, in monospace, with a button that copies it.
fn command_chip(p: Palette, id: &'static str, command: &'static str) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_1()
        .items_center()
        .child(div().h(px(CONTROL)).px_2().flex().items_center().rounded(px(R_SM)).bg(p.code_bg).border_1().border_color(p.hair).font_family(MONO).text_size(px(12.)).text_color(p.text2).child(command))
        .child(ui::icon_button(p, id, Icon::Copy, t!("main.copy_command"), None).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.to_owned()))))
        .into_any_element()
}

/// Plain words where a control would be: a value to read.
fn value(p: Palette, text: impl Into<SharedString>) -> AnyElement {
    div().flex_none().max_w(px(260.)).truncate().text_size(px(SMALL)).text_color(p.text2).child(text.into()).into_any_element()
}

// ── the words for daemon values ──

pub fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::ReadOnly => t!("settings.mode_read_only"),
        Mode::Ask => t!("settings.mode_ask"),
        Mode::Auto => t!("settings.mode_auto"),
        Mode::Full => t!("settings.mode_full"),
    }
}

pub fn mode_summary(mode: Mode) -> &'static str {
    match mode {
        Mode::ReadOnly => t!("settings.mode_read_only_desc"),
        Mode::Ask => t!("settings.mode_ask_desc"),
        Mode::Auto => t!("settings.mode_auto_desc"),
        Mode::Full => t!("settings.mode_full_desc"),
    }
}

/// An "always allow" rule as a person reads it: `Bash(npm install:*)` is
/// commands starting `npm install`.
pub fn rule_words(rule: &str) -> String {
    if let Some(domain) = rule.strip_prefix("WebFetch(domain:").and_then(|r| r.strip_suffix(')')) {
        return t!("settings.rule_domain", domain = domain);
    }
    match rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(":*)")) {
        Some(prefix) => t!("settings.rule_commands", prefix = prefix),
        None => match rule.split_once('(') {
            Some((tool, rest)) => t!("settings.rule_tool", tool = tool, what = rest.trim_end_matches(')')),
            None => t!("settings.rule_any", tool = rule),
        },
    }
}

fn language_label(shell: &Shell) -> String {
    match shell.board.language {
        Some(lang) => lang.native_name().to_owned(),
        None => t!("settings.language_system", lang = Lang::resolve_auto(Front::App).native_name()),
    }
}

// ── the categories ──

fn general(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let mut language = Line::new(t!("sidebar.language"), t!("settings.language_desc"))
        .words(Lang::ALL.iter().map(|l| l.native_name()).collect::<Vec<_>>().join(" ") + " language");
    language.control = Some(picker(shell, Pop::Language, language_label(shell), 0., cx));
    let looks = Appearance::ALL.into_iter().map(|a| (a, appearance_word(a).to_owned())).collect();
    let appearance = Line::new(t!("settings.appearance"), t!("settings.appearance_desc")).words("appearance theme dark light").control(segmented(shell, "appearance", looks, shell.remembered.appearance, |this, a, window, cx| this.set_appearance(a, window, cx), cx));
    let starts = Start::ALL.into_iter().map(|s| (s, start_word(s).to_owned())).collect();
    let start = Line::new(t!("settings.start"), t!("settings.start_desc")).words("launch").control(segmented(shell, "start", starts, shell.remembered.start, |this, s, _, _| this.set_start(s), cx));

    let pings = shell.remembered.pings;
    let ping = |ping: Ping, label: &'static str, desc: &'static str, cx: &mut Context<Shell>| {
        Line::new(label, desc).words("notification").control(switch(shell, ping.code(), pings.get(ping), true, move |this, on, _, _| this.set_ping(ping, on), cx))
    };
    let notifications = vec![
        ping(Ping::Approvals, t!("settings.ping_approvals"), t!("settings.ping_approvals_desc"), cx),
        ping(Ping::Reviews, t!("settings.ping_reviews"), t!("settings.ping_reviews_desc"), cx),
        ping(Ping::Waiting, t!("settings.ping_waiting"), t!("settings.ping_waiting_desc"), cx),
        ping(Ping::Failed, t!("settings.ping_failed"), t!("settings.ping_failed_desc"), cx),
        ping(Ping::Sound, t!("settings.ping_sound"), t!("settings.ping_sound_desc"), cx),
        Line::new(t!("settings.system_notifications"), t!("settings.system_notifications_desc"))
            .words("macos")
            .control(ui::button(p, "open-notifications", Kind::Secondary, t!("settings.open_system_settings")).icon(Icon::External).on_click(|_, _, cx| cx.open_url("x-apple.systempreferences:com.apple.Notifications-Settings.extension"))),
    ];
    vec![
        Section::new(t!("settings.section_look"), vec![language, appearance]).foot(format!("{} {}", t!("settings.language_note"), t!("settings.this_mac_foot"))),
        Section::new(t!("settings.section_start"), vec![start]),
        Section::new(t!("settings.section_notifications"), notifications).foot(t!("settings.notifications_foot")),
    ]
}

fn appearance_word(a: Appearance) -> &'static str {
    match a {
        Appearance::System => t!("settings.appearance_system"),
        Appearance::Light => t!("settings.appearance_light"),
        Appearance::Dark => t!("settings.appearance_dark"),
    }
}

fn start_word(s: Start) -> &'static str {
    match s {
        Start::Pm => t!("sidebar.pm_talk"),
        Start::Inbox => t!("board.menu_inbox"),
        Start::All => t!("board.menu_all"),
    }
}

fn squad(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let model = &shell.model;
    let connected = matches!(model.conn, Conn::Connected { .. });
    let status = match &model.conn {
        Conn::Connecting => t!("sidebar.connecting").to_owned(),
        Conn::Connected { last_event: Some(at) } => t!("sidebar.connected_at", at = at.hm()),
        Conn::Connected { last_event: None } => t!("sidebar.connected").to_owned(),
        Conn::Down { .. } => t!("sidebar.disconnected").to_owned(),
    };
    let squad_name = if model.squad.is_empty() { t!("settings.none").to_owned() } else { model.squad.clone() };
    let squad = Line::new(t!("sidebar.squads"), t!("settings.squad_desc")).words(shell.squads.join(" ")).control(if shell.squads.len() > 1 { picker(shell, Pop::Squad, squad_name, 0., cx) } else { value(p, squad_name) });
    let machine_desc = match &shell.endpoint {
        Endpoint::Local { .. } => t!("settings.machine_local_desc").to_owned(),
        Endpoint::Ssh(host) => t!("settings.machine_ssh_desc", host = host.spec()),
    };
    let machine = Line::new(t!("settings.machine"), machine_desc).words(format!("ssh {}", shell.endpoint.label())).control(
        h_flex()
            .gap_3()
            .items_center()
            .child(h_flex().gap_2().items_center().child(dot(if connected { p.accent } else { p.muted }, 7.)).child(value(p, shell.endpoint.label())))
            .child(ui::button(p, "other-machine", Kind::Secondary, t!("settings.connect_other")).on_click(cx.listener(|this, _, window, cx| this.open_chooser(window, cx)))),
    );
    let connection = Line::new(t!("settings.connection"), status).words("daemon").control(ui::button(p, "reconnect", Kind::Ghost, t!("main.retry")).icon(Icon::Retry).on_click(cx.listener(|this, _, _, cx| this.retry(cx))));
    let version = if model.daemon_version.is_empty() { t!("settings.none").to_owned() } else { model.daemon_version.clone() };
    let stale = crate::notify::stale_daemon(&model.daemon_version, env!("CARGO_PKG_VERSION"));
    let daemon_version = Line::new(t!("settings.daemon_version"), stale.clone().unwrap_or_else(|| t!("settings.daemon_version_desc", app = env!("CARGO_PKG_VERSION")))).warn(stale.is_some()).words("version").control(value(p, version));
    let restart = Line::new(t!("settings.restart_daemon"), t!("settings.restart_daemon_desc")).words("daemon restart update").control(command_chip(p, "copy-restart", "equipe daemon restart --when-idle"));

    let suggestions = Line::new(t!("settings.suggestions"), t!("settings.suggestions_desc")).words("suggest").control(switch(shell, "suggestions", !model.suggestions_off, connected, |this, on, _, _| this.set_suggestions(on), cx));
    let screen = Line::new(t!("settings.mac_screen"), t!("settings.mac_screen_desc")).words("desktop computer use /desktop").control(switch(shell, "desktop", !model.desktop_stopped, connected, |this, on, _, _| this.set_desktop_open(on), cx));
    vec![
        Section::new(t!("settings.section_connection"), vec![squad, machine, connection]).foot(t!("settings.connection_foot")),
        Section::new(t!("settings.section_daemon"), vec![daemon_version, restart]),
        Section::new(t!("settings.section_squad"), vec![suggestions, screen]).foot(t!("settings.squad_foot")),
    ]
}

fn team(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let model = &shell.model;
    let connected = matches!(model.conn, Conn::Connected { .. });
    let lines: Vec<Line> = model
        .ordered()
        .into_iter()
        .map(|m| {
            let (mark, hue) = look(&m.role, &m.name);
            let engine = match m.engine {
                Some(equipe_core::protocol::Engine::Codex) => "Codex",
                Some(_) => "Claude Code",
                None => "",
            };
            let desc = [Some(m.role.clone()).filter(|r| !r.is_empty() && *r != m.name), Some(engine.to_owned()).filter(|e| !e.is_empty())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            let label = h_flex().gap_2().items_center().child(div().text_size(px(12.)).text_color(p.member(hue)).child(mark)).child(m.name.clone());
            let model_name = m.model.as_deref().map(super::composer::short_model).unwrap_or_else(|| t!("settings.engine_default").to_owned());
            let effort = model.efforts.get(&m.session).cloned().unwrap_or_else(|| t!("settings.engine_default").to_owned());
            let mode = model.member_modes.get(&m.session).map(|m| mode_label(*m).to_owned()).unwrap_or_else(|| t!("settings.mode_follows").to_owned());
            let controls = h_flex()
                .gap_2()
                .when(connected, |c| {
                    let s = m.session;
                    c.child(with_tip(picker(shell, Pop::Model(s), model_name.clone(), 112., cx), format!("tip-model-{s}"), p, t!("composer.tip_model")))
                        .child(with_tip(picker(shell, Pop::Effort(s), effort.clone(), 96., cx), format!("tip-effort-{s}"), p, t!("composer.tip_effort")))
                        .child(with_tip(picker(shell, Pop::Mode(s), mode.clone(), 124., cx), format!("tip-mode-{s}"), p, t!("composer.tip_mode")))
                });
            Line::new(m.name.clone(), desc).label_el(label).words(format!("{} {model_name} {effort} {mode}", m.role)).control(controls)
        })
        .collect();
    let lines = if lines.is_empty() { vec![Line::new(t!("settings.no_members"), t!("settings.no_members_desc"))] } else { lines };
    vec![Section::new(t!("settings.section_members"), lines).foot(t!("settings.team_foot"))]
}

/// `el` with a tooltip, in a box of its own that can be hovered.
fn with_tip(el: AnyElement, id: String, p: Palette, label: &'static str) -> AnyElement {
    div().id(SharedString::from(id)).child(el).tip(p, label, None).into_any_element()
}

fn permissions(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let model = &shell.model;
    let connected = matches!(model.conn, Conn::Connected { .. });
    let modes = Mode::ALL.into_iter().map(|m| (m, mode_label(m).to_owned())).collect();
    let mode = Line::new(t!("settings.squad_mode"), mode_summary(model.mode))
        .warn(model.mode == Mode::Full)
        .words(Mode::ALL.iter().map(|m| format!("{} {}", mode_label(*m), mode_summary(*m))).collect::<Vec<_>>().join(" ") + " permission mode")
        .control(if connected { segmented(shell, "squad-mode", modes, model.mode, |this, m, _, _| this.set_squad_mode(m), cx) } else { value(p, mode_label(model.mode)) });
    let overrides: Vec<String> = model.ordered().iter().filter_map(|m| model.member_modes.get(&m.session).map(|mode| format!("{} · {}", m.name, mode_label(*mode)))).collect();
    let mut first = vec![mode];
    if !overrides.is_empty() {
        first.push(
            Line::new(t!("settings.member_modes"), overrides.join(", "))
                .control(ui::button(p, "to-team", Kind::Ghost, t!("settings.cat_team")).icon(Icon::ArrowRight).on_click(cx.listener(|this, _, _, cx| this.settings_go(Category::Team, cx)))),
        );
    }

    let rules: Vec<Line> = model
        .allowed
        .iter()
        .enumerate()
        .map(|(ix, rule)| {
            let taken = rule.clone();
            Line::new(rule.clone(), rule_words(rule))
                .label_el(div().font_family(MONO).text_size(px(12.5)).child(rule.clone()))
                .words("always allow")
                .control(ui::button(p, ("revoke", ix), Kind::Secondary, t!("settings.revoke")).disabled(!connected).on_click(cx.listener(move |this, _, _, cx| this.disallow(&taken, cx))))
        })
        .collect();
    let rules = if rules.is_empty() { vec![Line::new(t!("settings.no_rules"), t!("settings.no_rules_desc")).words("always allow")] } else { rules };

    let blocked = Line::new(t!("settings.blocked"), t!("settings.blocked_desc")).words("sudo rm force push");
    let sign_in = Line::new(t!("settings.shared_sign_in"), t!("settings.shared_sign_in_desc"))
        .words("browser login account")
        .control(ui::button(p, "to-browser", Kind::Ghost, t!("settings.cat_browser")).icon(Icon::ArrowRight).on_click(cx.listener(|this, _, _, cx| this.settings_go(Category::Browser, cx))));
    vec![
        Section::new(t!("settings.section_mode"), first).foot(t!("settings.mode_foot")),
        Section::new(t!("settings.section_rules"), rules).foot(t!("settings.rules_foot")),
        Section::new(t!("settings.section_never"), vec![blocked, sign_in]),
    ]
}

fn projects(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let model = &shell.model;
    let lines: Vec<Line> = model
        .projects
        .iter()
        .map(|path| {
            let name = equipe_core::board::project_name(path);
            let now = shell.merge_way_of(&name);
            let mut ways: Vec<(Option<MergeWay>, String)> = vec![(None, t!("settings.way_ask").to_owned())];
            ways.extend([MergeWay::Branch, MergeWay::Merge, MergeWay::Push].map(|w| (Some(w), w.label().to_owned())));
            let project = name.clone();
            let id = format!("way-{name}");
            Line::new(name.clone(), short_path(path))
                .words("merge push branch")
                .control(segmented(shell, &id, ways, now, move |this, way, _, _| this.set_project_way(&project, way), cx))
        })
        .collect();
    let lines = if lines.is_empty() { vec![Line::new(t!("settings.no_projects"), t!("settings.no_projects_desc"))] } else { lines };
    vec![Section::new(t!("settings.section_projects"), lines).foot(t!("settings.projects_foot"))]
}

/// A project's folder, short enough for one line: `~` for home, and only
/// its last two folders when it is long.
pub fn short_path(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let shown = match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    };
    if shown.chars().count() <= 48 {
        return shown;
    }
    let parts: Vec<&str> = shown.split('/').filter(|p| !p.is_empty()).collect();
    format!("…/{}", parts[parts.len().saturating_sub(2)..].join("/"))
}

fn browser(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let shown = Line::new(t!("settings.browsers_shown"), t!("settings.browsers_shown_desc")).words("headless window").control(switch(shell, "browsers", shell.model.browsers_shown, connected, |this, on, _, _| this.set_browsers_shown(on), cx));
    let panel = Line::new(t!("settings.browser_panel"), t!("settings.browser_panel_desc"))
        .words("panel ⌘⇧B")
        .control(ui::button(p, "open-panel", Kind::Secondary, t!("settings.open_panel")).key("⌘⇧B").on_click(cx.listener(|this, _, window, cx| this.toggle_browser(window, cx))));
    let sign_in = Line::new(t!("settings.sign_in"), t!("settings.sign_in_desc")).words("login account equipe browser").control(command_chip(p, "copy-browser", "equipe browser"));
    let pop_out = Line::new(t!("settings.pop_out"), t!("settings.pop_out_desc")).words("chrome passkey");
    vec![
        Section::new(t!("settings.section_members_browsers"), vec![shown, panel]).foot(t!("settings.squad_foot")),
        Section::new(t!("settings.section_sign_in"), vec![sign_in, pop_out]).foot(t!("settings.sign_in_foot")),
    ]
}

fn keys(shell: &Shell) -> Vec<Section> {
    let p = shell.palette;
    let mut sections: Vec<Section> = Vec::new();
    for key in rules::shortcuts() {
        let chips = h_flex().flex_none().gap_1().children(key.keys.split(" · ").map(|k| kbd(p, k).h(px(22.)).px(px(7.)).text_size(px(12.)).text_color(p.text2)));
        let mut line = Line::new(key.what, "").words(key.keys).control(chips);
        line.compact = true;
        match sections.last_mut().filter(|s| s.title.as_deref() == Some(key.area)) {
            Some(section) => section.lines.push(line),
            None => sections.push(Section::new(key.area, vec![line])),
        }
    }
    if let Some(last) = sections.last_mut() {
        last.foot = Some(t!("settings.keys_foot").to_owned());
    }
    sections
}

fn about(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let p = shell.palette;
    let version = env!("CARGO_PKG_VERSION");
    let can = crate::relaunch::can_update(cx);
    let this_version = Line::new(t!("settings.app_version"), t!("settings.app_version_desc"))
        .words("version")
        .control(h_flex().gap_1().items_center().child(value(p, version)).child(ui::icon_button(p, "copy-version", Icon::Copy, t!("settings.copy_version"), None).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(version.to_owned())))));
    let mut updates = vec![this_version];
    if let Some(ready) = shell.update_ready.clone() {
        updates.push(Line::new(crate::relaunch::label(&ready), t!("settings.update_ready_desc")).words("update").control(ui::button(p, "relaunch-now", Kind::Primary, t!("settings.relaunch")).on_click(cx.listener(|this, _, _, cx| this.relaunch_to_update(cx)))));
    }
    updates.push(
        Line::new(t!("settings.check_updates"), if can { t!("settings.check_updates_desc") } else { t!("settings.no_updater") })
            .words("update sparkle")
            .control(ui::button(p, "check-now", Kind::Secondary, t!("settings.check_now")).disabled(!can).on_click(|_, _, cx| crate::relaunch::check_now(cx))),
    );
    updates.push(
        Line::new(t!("settings.auto_update"), t!("settings.auto_update_desc"))
            .words("update automatic")
            .control(switch(shell, "auto-update", can && crate::relaunch::automatic(cx), can, |_, on, _, cx| crate::relaunch::set_automatic(on, cx), cx)),
    );
    let link = |id: &'static str, label: &'static str, desc: &'static str, url: &'static str| {
        Line::new(label, desc).words(url).control(ui::button(p, id, Kind::Ghost, t!("settings.open")).icon(Icon::External).on_click(move |_, _, cx| cx.open_url(url)))
    };
    let links = vec![
        link("site", t!("settings.link_site"), "equipe.im", crate::install::SITE),
        link("github", "GitHub", "github.com/equipe-labs/equipe", "https://github.com/equipe-labs/equipe"),
        link("docs", t!("settings.link_docs"), t!("settings.link_docs_desc"), "https://github.com/equipe-labs/equipe/tree/main/docs"),
        link("licence", t!("settings.link_licence"), t!("settings.link_licence_desc"), "https://github.com/equipe-labs/equipe/blob/main/LICENSE"),
    ];
    vec![Section::new(t!("settings.section_updates"), updates), Section::new(t!("settings.section_links"), links)]
}

fn sections(shell: &Shell, category: Category, cx: &mut Context<Shell>) -> Vec<Section> {
    match category {
        Category::General => general(shell, cx),
        Category::Squad => squad(shell, cx),
        Category::Team => team(shell, cx),
        Category::Permissions => permissions(shell, cx),
        Category::Projects => projects(shell, cx),
        Category::Browser => browser(shell, cx),
        Category::Keys => keys(shell),
        Category::About => about(shell, cx),
    }
}

fn intro(category: Category) -> &'static str {
    match category {
        Category::General => t!("settings.intro_general"),
        Category::Squad => t!("settings.intro_squad"),
        Category::Team => t!("settings.intro_team"),
        Category::Permissions => t!("settings.intro_permissions"),
        Category::Projects => t!("settings.intro_projects"),
        Category::Browser => t!("settings.intro_browser"),
        Category::Keys => t!("settings.intro_keys"),
        Category::About => t!("settings.intro_about"),
    }
}

// ── drawing ──

fn draw_line(p: Palette, line: Line, first: bool) -> AnyElement {
    let compact = line.compact;
    h_flex()
        .w_full()
        .min_h(px(if compact { 40. } else if line.desc.is_some() { 60. } else { 48. }))
        .px_4()
        .py(px(if compact { 6. } else { 10. }))
        .gap_4()
        .items_center()
        .when(!first, |r| r.border_t_1().border_color(p.hair))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(if compact { SMALL } else { UI })).text_color(p.text).child(line.label))
                .children(line.desc.map(|(desc, warn)| div().text_size(px(12.5)).line_height(px(18.)).text_color(if warn { p.danger() } else { p.muted }).child(desc))),
        )
        .children(line.control.map(|c| div().flex_none().flex().items_center().child(c)))
        .into_any_element()
}

fn draw_section(p: Palette, section: Section) -> AnyElement {
    let card_bg = p.raised;
    v_flex()
        .w_full()
        .gap_2()
        .children(section.title.map(|t| div().px_1().text_size(px(SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(p.text2).child(t)))
        .child(v_flex().w_full().rounded(px(R_LG)).border_1().border_color(p.hair).bg(card_bg).overflow_hidden().children(section.lines.into_iter().enumerate().map(|(ix, line)| draw_line(p, line, ix == 0))))
        .children(section.foot.map(|f| div().px_1().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(f)))
        .into_any_element()
}

/// The list on the left: back, search, the categories.
fn nav(shell: &Shell, found: &[(Category, usize)], searching: bool, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let at = shell.settings.category;
    let needs = shell.model.needs_you_count();
    let row = |id: SharedString, on: bool| {
        h_flex()
            .id(id)
            .h(px(32.))
            .flex_none()
            .mx_2()
            .px_2()
            .gap(px(8.))
            .items_center()
            .rounded(px(R_MD))
            .text_size(px(UI))
            .cursor_pointer()
            .when(on, |r| r.bg(p.side_sel))
            .when(!on, |r| r.hover(move |s| s.bg(p.side_hover)))
    };
    let back = row("settings-back".into(), false)
        .tip_at(p, t!("settings.back_tip"), Some("Esc"), gpui_kit::base::Placement::Right)
        .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
        .child(icon(Icon::ArrowLeft, ICON, p.text2))
        .child(div().text_color(p.text2).child(t!("settings.back")));
    let search = h_flex()
        .mx_2()
        .mt_2()
        .mb_2()
        .h(px(30.))
        .pl_2()
        .gap_0p5()
        .items_center()
        .rounded(px(R_MD))
        .bg(p.wash)
        .border_1()
        .border_color(p.hair)
        .text_size(px(SMALL))
        .child(icon(Icon::Search, ICON_SM, p.muted))
        .child(div().flex_1().min_w_0().child(Input::new(&shell.settings.search).appearance(false).bordered(false)));
    let items = Category::ALL.into_iter().map(|c| {
        let count = found.iter().find(|(f, _)| *f == c).map(|(_, n)| *n);
        let on = !searching && c == at;
        let dim = searching && count.is_none();
        row(SharedString::from(format!("settings-cat-{}", c.code())), on)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.settings.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                this.settings_go(c, cx);
            }))
            .child(icon(c.icon(), ICON, if on { p.text } else if dim { p.muted } else { p.text2 }))
            .child(div().flex_1().min_w_0().truncate().text_color(if on { p.text } else if dim { p.muted } else { p.text2 }).child(c.label()))
            .children(count.filter(|_| searching).map(|n| div().text_size(px(11.)).text_color(p.muted).child(n.to_string())))
    });
    v_flex()
        .w(px(shell.board.sidebar_width))
        .h_full()
        .flex_none()
        .bg(p.side)
        .font_family(SANS)
        .border_r_1()
        .border_color(p.hair)
        .child(div().h(px(super::sidebar::STRIP)).flex_none().w_full().window_control_area(WindowControlArea::Drag))
        .child(back)
        .child(search)
        .child(v_flex().flex_1().min_h_0().gap(px(1.)).children(items))
        // What waits meanwhile: one row back to it, so settings never hides a request.
        .children((needs > 0).then(|| {
            row("settings-needs".into(), false)
                .mb_2()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_settings(window, cx);
                    this.open_side_menu(Menu::Inbox, cx);
                }))
                .child(div().w(px(ICON)).flex_none().flex().items_center().justify_center().child(dot(p.orange, 7.)))
                .child(div().flex_1().min_w_0().truncate().text_color(p.text2).child(t!("board.menu_inbox")))
                .child(div().flex_none().min_w(px(18.)).h(px(18.)).px(px(5.)).flex().items_center().justify_center().rounded(px(9.)).bg(p.accent).text_color(p.on_primary).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(needs.to_string()))
        }))
        .into_any_element()
}

pub fn render(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let query = shell.settings.query(cx);
    let searching = !query.is_empty();
    // While searching, every category's rows; found ones under their category.
    let (found, page): (Vec<(Category, usize)>, Vec<AnyElement>) = if searching {
        let mut all: Vec<(Category, Vec<Section>)> = Category::ALL.into_iter().map(|c| (c, sections(shell, c, cx))).collect();
        let index: Vec<(Category, String, (usize, usize, usize))> = all
            .iter()
            .enumerate()
            .flat_map(|(ci, (c, secs))| {
                secs.iter().enumerate().flat_map(move |(si, s)| s.lines.iter().enumerate().map(move |(li, l)| (*c, format!("{} {}", s.title.clone().unwrap_or_default(), l.found_by()), (ci, si, li))))
            })
            .collect();
        let hits = rules::search(&query, &index);
        let counts = hits.iter().map(|(c, rows)| (*c, rows.len())).collect();
        let mut page = Vec::new();
        page.push(div().text_size(px(SMALL)).text_color(p.muted).child(if hits.is_empty() { t!("settings.search_none", query = query.clone()) } else { t!("settings.search_found", query = query.clone(), n = hits.iter().map(|(_, r)| r.len()).sum::<usize>()) }).into_any_element());
        for (category, rows) in hits {
            let ci = Category::ALL.iter().position(|c| *c == category).unwrap_or(0);
            let secs = std::mem::take(&mut all[ci].1);
            let mut kept: Vec<Section> = Vec::new();
            for (si, mut section) in secs.into_iter().enumerate() {
                let lines: Vec<Line> = std::mem::take(&mut section.lines).into_iter().enumerate().filter(|(li, _)| rows.contains(&(ci, si, *li))).map(|(_, l)| l).collect();
                if !lines.is_empty() {
                    kept.push(Section { title: section.title, lines, foot: None });
                }
            }
            page.push(
                h_flex()
                    .id(SharedString::from(format!("found-{}", category.code())))
                    .mt_2()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text)
                    .hover(move |s| s.text_color(p.accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.settings.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                        this.settings_go(category, cx);
                    }))
                    .child(icon(category.icon(), ICON, p.text2))
                    .child(category.label())
                    .child(icon(Icon::ChevronRight, ICON_SM, p.muted))
                    .into_any_element(),
            );
            page.extend(kept.into_iter().map(|s| draw_section(p, s)));
        }
        (counts, page)
    } else {
        let category = shell.settings.category;
        let mut page = vec![v_flex()
            .gap_1()
            .child(div().text_size(px(22.)).line_height(px(30.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(category.label()))
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(intro(category)))
            .into_any_element()];
        page.extend(sections(shell, category, cx).into_iter().map(|s| draw_section(p, s)));
        (Vec::new(), page)
    };
    let column = v_flex().w_full().max_w(px(PAGE)).gap_6().children(page);
    let column = motion::rise(column, ElementId::NamedInteger("settings-page".into(), shell.settings.turn));
    let main = v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.bg)
        .child(div().h(px(super::sidebar::STRIP)).flex_none().w_full().window_control_area(WindowControlArea::Drag))
        .child(div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&shell.settings.scroll).child(div().w_full().flex().justify_center().px_8().pt_2().pb(px(48.)).child(column)));
    h_flex().size_full().font_family(SANS).text_color(p.text).child(nav(shell, &found, searching, cx)).child(main).children(pop_menu(shell, cx)).into_any_element()
}

/// The open pop-up's rows: what each says, whether it is the one in use, what it does.
fn choices(shell: &Shell, pop: Pop) -> (f32, Vec<(String, String, bool, Act)>) {
    let model = &shell.model;
    let member = |session: SessionId| model.members.iter().find(|m| m.session == session);
    match pop {
        Pop::Language => {
            let now = shell.board.language;
            let mut rows = vec![(t!("settings.language_system", lang = Lang::resolve_auto(Front::App).native_name()), String::new(), now.is_none(), Act::Language(None))];
            rows.extend(Lang::ALL.into_iter().map(|l| (l.native_name().to_owned(), String::new(), now == Some(l), Act::Language(Some(l)))));
            (240., rows)
        }
        Pop::Squad => (240., shell.squads.iter().map(|s| (s.clone(), String::new(), *s == model.squad, Act::Squad(s.clone()))).collect()),
        Pop::Model(session) => {
            let Some(m) = member(session) else { return (0., Vec::new()) };
            let mut models = crate::compose::engine_models(m.engine, m.model.as_deref());
            if models.is_empty() {
                models.extend(m.model.clone());
            }
            let now = m.model.clone().unwrap_or_default();
            let exact = models.contains(&now);
            (260., models.into_iter().map(|x| (super::composer::short_model(&x), x.clone(), now == x || (!exact && now.contains(&x)), Act::Model(session, x))).collect())
        }
        Pop::Effort(session) => {
            let Some(m) = member(session) else { return (0., Vec::new()) };
            let now = model.efforts.get(&session).cloned();
            let mut rows = vec![(t!("settings.engine_default").to_owned(), String::new(), now.is_none(), Act::Effort(session, None))];
            rows.extend(super::composer::efforts(m.engine).iter().map(|e| ((*e).to_owned(), String::new(), now.as_deref() == Some(*e), Act::Effort(session, Some((*e).to_owned())))));
            (200., rows)
        }
        Pop::Mode(session) => {
            let now = model.member_modes.get(&session).copied();
            let mut rows = vec![(t!("settings.mode_follows").to_owned(), mode_label(model.mode).to_owned(), now.is_none(), Act::Mode(session, None))];
            rows.extend(Mode::ALL.into_iter().map(|m| (mode_label(m).to_owned(), String::new(), now == Some(m), Act::Mode(session, Some(m)))));
            (240., rows)
        }
    }
}

fn pop_menu(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let pop = shell.settings.pop?;
    let anchor = *shell.settings.anchors.borrow().get(&pop)?;
    let (width, rows) = choices(shell, pop);
    if rows.is_empty() {
        return None;
    }
    let rows = rows.into_iter().enumerate().map(|(ix, (label, hint, on, act))| {
        let row = h_flex()
            .id(("settings-pop-row", ix))
            .min_h(px(ROW))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(R_SM))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.run_pop(act.clone(), window, cx)))
            .child(div().w(px(ICON)).flex_none().flex().justify_center().children(on.then(|| icon(Icon::Check, ICON, p.accent))))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).text_color(p.text).child(label))
            .when(!hint.is_empty(), |r| r.child(div().flex_none().max_w(px(140.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(p.muted).child(hint)));
        motion::hover_fill(row, ("settings-pop-fill", ix), p.wash, false).into_any_element()
    });
    let surface = v_flex()
        .id("settings-pop")
        .w(px(width))
        .max_h(px(360.))
        .overflow_y_scroll()
        .p_1()
        .rounded(px(R_MD))
        .bg(p.popover())
        .border_1()
        .border_color(p.hair)
        .text_color(p.text)
        .children(rows)
        .on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if !anchor.contains(&e.position) {
                this.settings.pop = None;
                cx.notify();
            }
        }));
    let surface = motion::pop(surface, ElementId::NamedInteger("settings-pop-in".into(), pop_number(pop)), false);
    let positioner = gpui_kit::base::Positioner::side(anchor).placement(gpui_kit::base::Placement::Bottom).align(gpui_kit::base::Align::End).offset(px(crate::theme::GAP)).margin(px(8.)).occlude();
    Some(deferred(positioner.child(surface)).with_priority(2).into_any_element())
}

fn pop_number(pop: Pop) -> u64 {
    match pop {
        Pop::Language => 1,
        Pop::Squad => 2,
        Pop::Model(s) => 3 + s * 8,
        Pop::Effort(s) => 4 + s * 8,
        Pop::Mode(s) => 5 + s * 8,
    }
}

// ── what the page does ──

impl Shell {
    /// ⌘,, the gear: settings, at `category` (or where it was left).
    pub fn open_settings(&mut self, category: Option<Category>, window: &mut Window, cx: &mut Context<Self>) {
        crate::tips::hide(cx);
        if self.screen != Screen::Settings {
            self.settings.back_to = self.screen;
            self.screen = Screen::Settings;
            self.settings.turn += 1;
        }
        self.board.popup = None;
        if let Some(category) = category {
            self.settings_go(category, cx);
        }
        // Keys go to the page (↑↓, Esc), not to the box underneath.
        window.blur(cx);
        cx.notify();
    }

    /// Back to where the window was.
    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen != Screen::Settings {
            return;
        }
        self.settings.pop = None;
        self.screen = self.settings.back_to;
        if self.screen == Screen::Chat {
            self.composer.focus(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.screen == Screen::Settings {
            self.close_settings(window, cx);
        } else {
            self.open_settings(None, window, cx);
        }
    }

    pub fn settings_go(&mut self, category: Category, cx: &mut Context<Self>) {
        if self.settings.category != category {
            self.settings.category = category;
            self.settings.turn += 1;
            self.settings.scroll.set_offset(point(px(0.), px(0.)));
        }
        self.settings.pop = None;
        cx.notify();
    }

    /// The page's keys: Esc closes a pop-up, then clears the search, then
    /// goes back; ↑↓ walk the categories; ⌘F finds. True when taken.
    pub fn on_settings_key(&mut self, key: &str, modifiers: &Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.screen != Screen::Settings {
            return false;
        }
        let typing = self.settings.search.read(cx).focus_handle(cx).is_focused(window);
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;
        match key {
            "escape" if self.settings.pop.is_some() => self.settings.pop = None,
            "escape" if !self.settings.query(cx).is_empty() => self.settings.search.update(cx, |s, cx| s.set_value(String::new(), window, cx)),
            "escape" => self.close_settings(window, cx),
            "up" | "down" if plain && !typing => {
                let next = self.settings.category.step(key == "down");
                self.settings_go(next, cx);
            }
            "f" if modifiers.platform => self.settings.search.update(cx, |s, cx| s.focus(window, cx)),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn run_pop(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.pop = None;
        match act {
            Act::Language(lang) => self.set_language(lang, cx),
            Act::Squad(name) => self.select_squad(&name, window, cx),
            Act::Model(session, model) => {
                if let Some(m) = self.model.members.iter_mut().find(|m| m.session == session) {
                    m.model = Some(model.clone());
                }
                self.send_command(Command::SetModel { session, model });
            }
            Act::Effort(session, effort) => {
                self.send_command(Command::SetEffort { session, effort: effort.clone() });
                match effort {
                    Some(effort) => self.model.efforts.insert(session, effort),
                    None => self.model.efforts.remove(&session),
                };
            }
            Act::Mode(session, mode) => {
                if let Some(name) = self.model.members.iter().find(|m| m.session == session).map(|m| m.name.clone()) {
                    self.send_command(Command::SetMode { squad: self.model.squad.clone(), member: Some(name), mode });
                }
            }
        }
        cx.notify();
    }

    /// The language picked in settings (`None`: System, the Mac's — the
    /// language set for equipe alone in System Settings included): kept for
    /// every screen, the console's too, and shown at once, menus included.
    /// What equipe says in the conversations follows.
    pub fn set_language(&mut self, choice: Option<Lang>, cx: &mut Context<Self>) {
        let home = equipe_providers::credentials::equipe_home();
        if let Err(why) = equipe_core::i18n::save(&home, choice) {
            eprintln!("equipe: the language wasn't kept: {why}");
        }
        self.board.language = choice;
        let lang = Lang::resolve_in(&home, Front::App);
        if lang != crate::i18n::lang() {
            crate::i18n::set(lang);
            crate::set_menus(cx);
            self.send_command(equipe_core::protocol::Command::Language { lang: lang.code().to_owned() });
        }
        cx.refresh_windows();
        cx.notify();
    }

    pub fn set_appearance(&mut self, appearance: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        self.remembered.appearance = appearance;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        appearance.apply();
        self.palette = Palette::new(appearance.resolve(window.appearance()));
        self.sync_browser(window, cx);
        cx.notify();
    }

    pub fn set_start(&mut self, start: Start) {
        self.remembered.start = start;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
    }

    pub fn set_ping(&mut self, ping: Ping, on: bool) {
        self.remembered.pings.set(ping, on);
        self.remembered.save(&equipe_providers::credentials::equipe_home());
    }

    pub fn set_project_way(&mut self, project: &str, way: Option<MergeWay>) {
        match way {
            Some(way) => self.remembered.set_merge_way(project, way.code()),
            None => self.remembered.clear_merge_way(project),
        }
        self.remembered.save(&equipe_providers::credentials::equipe_home());
    }

    pub fn set_squad_mode(&mut self, mode: Mode) {
        self.send_command(Command::SetMode { squad: self.model.squad.clone(), member: None, mode: Some(mode) });
    }

    /// Take an "always allow" rule back: the daemon answers with the rules left.
    pub fn disallow(&mut self, rule: &str, cx: &mut Context<Self>) {
        self.send_command(Command::Disallow { squad: self.model.squad.clone(), rule: rule.to_owned() });
        cx.notify();
    }

    pub fn set_suggestions(&mut self, on: bool) {
        self.send_command(Command::Suggestions { squad: self.model.squad.clone(), on });
        self.model.suggestions_off = !on;
    }

    pub fn set_browsers_shown(&mut self, show: bool) {
        self.send_command(Command::ShowBrowser { squad: self.model.squad.clone(), member: None, show });
        self.model.browsers_shown = show;
    }

    pub fn set_desktop_open(&mut self, open: bool) {
        self.send_command(Command::Desktop { open });
        self.model.desktop_stopped = !open;
    }

    /// What the window opens on, as the person said in settings.
    pub fn open_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        match self.remembered.start {
            Start::Pm => {}
            Start::Inbox => self.open_side_menu(Menu::Inbox, cx),
            Start::All => self.open_side_menu(Menu::All, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, mode_label, mode_summary, rule_words, short_path};

    #[test]
    fn a_rule_reads_as_what_it_lets_through() {
        assert!(rule_words("Bash(npm install:*)").contains("npm install"));
        assert!(rule_words("WebFetch(domain:vercel.com)").contains("vercel.com"));
        assert!(!rule_words("WebFetch(domain:vercel.com)").contains("WebFetch"), "said as a person reads it");
        assert!(rule_words("Edit").contains("Edit"));
    }

    #[test]
    fn a_long_project_path_keeps_its_last_folders() {
        assert_eq!(short_path(std::path::Path::new("/srv/app")), "/srv/app");
        assert_eq!(short_path(std::path::Path::new("/var/folders/hy/cgxhvswn5pqdwkkfxxbztjj00000gn/T/equipe-fake/web")), "…/equipe-fake/web");
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(short_path(&std::path::PathBuf::from(home).join("code/site")), "~/code/site");
        }
    }

    #[test]
    fn every_mode_has_words_of_its_own() {
        let labels: std::collections::HashSet<&str> = Mode::ALL.iter().map(|m| mode_label(*m)).collect();
        assert_eq!(labels.len(), 4);
        assert!(Mode::ALL.iter().all(|m| !mode_summary(*m).is_empty()));
    }
}
