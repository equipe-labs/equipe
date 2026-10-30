//! Settings → AI 플랫폼: a card for each platform in `crate::engines::PLATFORMS`
//! — its mark and name, who signs it in, where it stands, the members on it,
//! its usage — and signing it in or out from the window. The rules are
//! `crate::engines`; the rows and cards are settings' own (`super::settings`).

use std::time::Instant;

use equipe_core::protocol::{Command, Engine, EngineState, NebiusKey};
use equipe_core::t;
use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::board::dot;
use super::settings::{Line, Section, command_chip, value};
use crate::engines::{self, Card, PLATFORMS, Platform, SignsIn, Tone, Usage};
use crate::icons::{Icon, icon};
use crate::state::Conn;
use crate::theme::{CONTROL, ICON_SM, MONO, Palette, R_MD, R_SM};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// Focus slots per card: its buttons, in the order Tab reaches them.
pub const KEYS: usize = 4;

/// Where the daemon is, as a card says it: "이 Mac", or the host.
fn host(shell: &Shell) -> String {
    shell.endpoint.label()
}

/// The platform's mark: its icon on its colour, in a rounded tile.
fn mark(platform: &Platform) -> AnyElement {
    div()
        .flex_none()
        .size(px(30.))
        .rounded(px(R_MD))
        .bg(rgb(platform.tint))
        // A dark tile (Codex's) still reads on a dark page.
        .border_1()
        .border_color(rgba(0xffffff1f))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(platform.mark, 18., rgb(0xffffff)))
        .into_any_element()
}

/// Where it stands, as a dot and a few words beside the name.
fn pill(p: Palette, tone: Option<Tone>, words: String) -> AnyElement {
    let color = match tone {
        Some(Tone::Ready) => p.accent,
        Some(Tone::Needs) => p.amber,
        _ => p.muted,
    };
    h_flex()
        .flex_none()
        .gap_1p5()
        .items_center()
        .h(px(22.))
        .px_2()
        .rounded(px(11.))
        .bg(p.wash)
        .text_size(px(12.))
        .text_color(p.text2)
        .when_some(tone, |pill, _| pill.child(dot(color, 6.)))
        .when(tone.is_none(), |pill| pill.child(crate::motion::spinner(11., p.muted)))
        .child(words)
        .into_any_element()
}

/// A card's button, reached by Tab.
fn button(shell: &Shell, at: usize, slot: usize, id: &'static str, kind: Kind, label: impl Into<SharedString>) -> ui::Button {
    let mut b = ui::button(shell.palette, (id, at), kind, label);
    if let Some(focus) = shell.settings.engine_keys.get(at * KEYS + slot) {
        b = b.focus(focus);
    }
    b
}

pub(super) fn sections(shell: &Shell, cx: &mut Context<Shell>) -> Vec<Section> {
    let now = Instant::now();
    let mut sections: Vec<Section> = PLATFORMS.iter().enumerate().map(|(at, platform)| card(shell, at, platform, now, cx)).collect();
    sections.push(nebius(shell, cx));
    if let Some(last) = sections.pop() {
        sections.push(last.foot(t!("engines.foot", host = host(shell))));
    }
    sections
}

fn card(shell: &Shell, at: usize, platform: &'static Platform, now: Instant, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let engine = platform.engine;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let status = shell.engines.status(engine);
    let card = shell.engines.card(engine).clone();
    let words = format!("{} {} ai platform engine login {}", platform.name, platform.tool, (platform.who)());

    let (tone, said) = match status {
        Some(status) => {
            let (tone, said) = engines::headline(status);
            (Some(tone), said)
        }
        None if shell.engines.no_answer(now) || !connected => (Some(Tone::Unknown), t!("engines.unknown").to_owned()),
        None => (None, t!("engines.checking").to_owned()),
    };
    let signed_in = status.is_some_and(|s| s.state == EngineState::SignedIn);
    let installed = status.is_some_and(|s| s.state != EngineState::NotInstalled);

    // The head: mark, name and where it stands; who signs it in; the main action.
    let head = h_flex().gap_3().items_center().child(mark(platform)).child(
        h_flex().gap_2().items_center().child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(platform.name)).child(pill(p, tone, said.clone())),
    );
    let action: Option<AnyElement> = match (&card, status.map(|s| s.state), platform.signs_in) {
        (Card::Idle, Some(EngineState::NotInstalled), _) => Some(button(shell, at, 0, "engine-guide", Kind::Secondary, t!("engines.install_guide")).icon(Icon::External).on_click(move |_, _, cx| cx.open_url(platform.guide)).into_any_element()),
        (Card::Idle, Some(EngineState::SignedIn), SignsIn::FromApp) => Some(
            button(shell, at, 0, "engine-sign-out", Kind::Secondary, t!("engines.sign_out"))
                .disabled(!connected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.engines.ask_sign_out(engine);
                    cx.notify();
                }))
                .into_any_element(),
        ),
        (Card::Idle, Some(_), SignsIn::FromApp) => Some(
            button(shell, at, 0, "engine-sign-in", Kind::Primary, t!("engines.sign_in"))
                .disabled(!connected)
                .on_click(cx.listener(move |this, _, _, cx| this.engine_sign_in(engine, cx)))
                .into_any_element(),
        ),
        (Card::Idle, Some(EngineState::SignedOut | EngineState::Unknown), SignsIn::InTerminal { .. }) if !shell.endpoint.is_remote() => Some(
            button(shell, at, 0, "engine-terminal", Kind::Primary, t!("engines.terminal_sign_in"))
                .icon(Icon::Terminal)
                .on_click(cx.listener(move |this, _, _, cx| this.engine_terminal(engine, cx)))
                .into_any_element(),
        ),
        _ => None,
    };
    let mut lines = vec![Line::new(platform.name, (platform.who)()).label_el(head).words(words.clone()).control_opt(action)];

    // Signed in from here: who as and on what plan, and the way to another account.
    if card == Card::Idle && signed_in && platform.signs_in == SignsIn::FromApp {
        let who = status.and_then(|s| s.account.clone());
        let plan = status.and_then(|s| s.plan.clone()).map(|plan| engines::title_case(&plan));
        let said = match (who, plan) {
            (Some(who), Some(plan)) => t!("engines.account_on", account = who, plan = plan),
            (Some(who), None) => who,
            (None, Some(plan)) => t!("engines.account_plan_only", plan = plan),
            (None, None) => t!("engines.account_unknown").to_owned(),
        };
        let switch = button(shell, at, 1, "engine-switch-account", Kind::Ghost, t!("engines.switch_account"))
            .tip(t!("engines.switch_account_tip"), None)
            .disabled(!connected)
            .on_click(cx.listener(move |this, _, _, cx| this.engine_sign_in(engine, cx)));
        lines.push(
            Line::new(t!("engines.row_account"), "")
                .words(format!("{words} account plan email switch"))
                .control(h_flex().gap_3().items_center().child(value(p, said)).child(switch).into_any_element()),
        );
    }

    // A sign-in on its way, or how it ended.
    if let Some(line) = progress(shell, at, platform, &card, cx) {
        lines.push(line.words(words.clone()));
    }

    // Not connected yet: what it adds, and how to get there.
    if card == Card::Idle && status.is_some() && !signed_in {
        let mut pitch = Line::new((platform.pitch)(), "").words(words.clone());
        if !installed {
            pitch = Line::new((platform.pitch)(), t!("engines.install_desc", host = host(shell))).words(words.clone()).below(h_flex().child(command_chip(p, SharedString::from(format!("engine-install-{at}")), platform.install)));
        } else if let SignsIn::InTerminal { command } = platform.signs_in {
            let desc = if shell.endpoint.is_remote() { t!("engines.terminal_remote", host = host(shell), command = command) } else { t!("engines.terminal_desc", tool = platform.tool) };
            pitch = Line::new((platform.pitch)(), desc).words(words.clone()).when_remote(shell.endpoint.is_remote(), |l| l.control(command_chip(p, SharedString::from(format!("engine-login-{at}")), command)));
            if shell.engines.terminal.is_some() && !shell.endpoint.is_remote() {
                pitch = pitch.control(h_flex().gap_2().items_center().child(crate::motion::spinner(12., p.muted)).child(value(p, t!("engines.terminal_waiting"))));
            }
        }
        let said = h_flex().gap_2().items_center().text_color(p.text2).child(icon(Icon::Lightbulb, ICON_SM, p.amber)).child((platform.pitch)());
        lines.push(pitch.label_el(said));
    }

    if let Some(status) = status {
        if let Some(detail) = status.detail.as_ref().filter(|_| status.state == EngineState::Unknown) {
            lines.push(Line::new(t!("engines.unknown"), detail.clone()).warn(true).words(words.clone()));
        }
        let members = engines::members_line(status, &shell.model.squad).unwrap_or_else(|| t!("engines.members_none").to_owned());
        lines.push(Line::new(t!("engines.row_members"), "").words(format!("{words} member")).control(value(p, members)));
        lines.push(usage(shell, platform, signed_in).words(format!("{words} usage limit")));
        if status.legacy {
            lines.push(Line::new(t!("engines.legacy"), t!("engines.legacy_desc")).words(format!("{words} legacy equipe login")).control(command_chip(p, SharedString::from(format!("engine-legacy-{at}")), "equipe logout")));
        }
    } else if shell.engines.no_answer(now) {
        lines.push(Line::new(t!("engines.no_answer"), t!("engines.no_answer_desc")).warn(true).words(words).control(command_chip(p, SharedString::from(format!("engine-restart-{at}")), "equipe daemon restart --when-idle")));
    }
    Section::new("", lines)
}

/// NVIDIA's green, for Nebius Token Factory's tile.
const NVIDIA_GREEN: u32 = 0x76b900;

/// Nebius Token Factory: the person's own API key, pasted here once and kept
/// on the daemon's machine — members run on Nemotron with it, and every
/// handed-in ticket gets a Nemotron review. The key never comes back to a screen.
fn nebius(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let ui = &shell.settings;
    let connected = matches!(shell.model.conn, Conn::Connected { .. });
    let status = shell.engines.nebius.clone();
    let words = "nebius token factory nvidia nemotron api key open model byok".to_owned();
    let key = status.as_ref().map_or(NebiusKey::None, |s| s.key);
    let (tone, said) = match (&status, key) {
        (None, _) if connected => (None, t!("engines.checking").to_owned()),
        (None, _) => (Some(Tone::Unknown), t!("engines.unknown").to_owned()),
        (Some(_), NebiusKey::None) => (Some(Tone::Needs), t!("engines.nebius_none").to_owned()),
        (Some(s), _) if !s.codex => (Some(Tone::Needs), t!("engines.nebius_needs_codex_short").to_owned()),
        (Some(_), NebiusKey::Environment) => (Some(Tone::Ready), t!("engines.nebius_ready_env").to_owned()),
        (Some(_), NebiusKey::Saved) => (Some(Tone::Ready), t!("engines.nebius_ready_saved", host = host(shell))),
    };
    let tile = div()
        .flex_none()
        .size(px(30.))
        .rounded(px(R_MD))
        .bg(rgb(NVIDIA_GREEN))
        .border_1()
        .border_color(rgba(0xffffff1f))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(Icon::Brain, 18., rgb(0xffffff)));
    let head = h_flex()
        .gap_3()
        .items_center()
        .child(tile)
        .child(h_flex().gap_2().items_center().child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Nebius Token Factory")).child(pill(p, tone, said)));
    let get_key = ui::button(p, "nebius-get-key", Kind::Secondary, t!("engines.nebius_get_key")).icon(Icon::External).on_click(|_, _, cx| cx.open_url("https://tokenfactory.nebius.com"));
    let mut lines = vec![Line::new("Nebius Token Factory", t!("engines.nebius_who")).label_el(head).words(words.clone()).control(get_key)];

    // The key: pasted, saved on the daemon's machine, never shown again.
    let field = div()
        .flex_1()
        .h(px(CONTROL + 4.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .bg(p.code_bg)
        .text_size(px(13.))
        .child(Input::new(&ui.nebius_key).appearance(false).bordered(false));
    let save = ui::button(p, "nebius-save", Kind::Primary, if key == NebiusKey::Saved { t!("engines.nebius_replace") } else { t!("engines.nebius_save") })
        .disabled(!connected)
        .on_click(cx.listener(|this, _, window, cx| this.save_nebius_key(window, cx)));
    let mut row = h_flex().w_full().gap_2().items_center().child(field).child(save);
    if key == NebiusKey::Saved {
        let forgetting = ui.nebius_forgetting;
        row = row.child(
            ui::button(p, "nebius-forget", if forgetting { Kind::Danger } else { Kind::Ghost }, if forgetting { t!("engines.nebius_forget_sure") } else { t!("engines.nebius_forget") })
                .disabled(!connected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.settings.nebius_forgetting {
                        this.settings.nebius_forgetting = false;
                        this.send_command(Command::SetNebiusKey { key: None });
                    } else {
                        this.settings.nebius_forgetting = true;
                    }
                    cx.notify();
                })),
        );
    }
    let below = v_flex()
        .w_full()
        .gap_1p5()
        .child(row)
        .children(ui.nebius_problem.clone().map(|why| div().text_size(px(12.)).text_color(p.danger()).child(why)))
        .child(div().text_size(px(12.)).text_color(p.muted).child(t!("engines.nebius_desc", host = host(shell))))
        .when(key == NebiusKey::Environment, |v| v.child(div().text_size(px(12.)).text_color(p.muted).child(t!("engines.nebius_env_note"))));
    lines.push(Line::new(t!("engines.nebius_key_label"), "").words(format!("{words} key paste save")).below(below));

    // Its agent: the Codex CLI, free and with no sign-in.
    if status.as_ref().is_some_and(|s| !s.codex) {
        lines.push(
            Line::new(t!("engines.nebius_needs_codex"), t!("engines.install_desc", host = host(shell)))
                .words(format!("{words} codex install"))
                .below(h_flex().child(command_chip(p, SharedString::from("nebius-codex-install"), "npm install -g @openai/codex"))),
        );
    }
    if let Some(model) = status.as_ref().and_then(|s| s.model.clone()) {
        lines.push(Line::new(t!("engines.nebius_model"), "").words(format!("{words} model")).control(value(p, model)));
    }
    Section::new("", lines)
}

/// The sign-in's row while it is on its way, failed, or asks to sign out.
fn progress(shell: &Shell, at: usize, platform: &'static Platform, card: &Card, cx: &mut Context<Shell>) -> Option<Line> {
    let p = shell.palette;
    let engine = platform.engine;
    let waiting = |label: String| h_flex().gap_2().items_center().child(crate::motion::spinner(14., p.text2)).child(label);
    let cancel = |cx: &mut Context<Shell>| button(shell, at, 1, "engine-cancel", Kind::Ghost, t!("engines.cancel")).on_click(cx.listener(move |this, _, _, cx| this.engine_cancel(engine, cx)));
    let copied = shell.engines.copied.is_some_and(|(e, at)| e == engine && at.elapsed() < std::time::Duration::from_millis(1600));
    let line = match card {
        Card::Idle => return None,
        Card::Starting { .. } => Line::new(t!("engines.starting"), "").label_el(waiting(t!("engines.starting").to_owned())).control(cancel(cx)),
        Card::Elsewhere => Line::new(t!("engines.elsewhere"), "").label_el(waiting(t!("engines.elsewhere").to_owned())).control(cancel(cx)),
        Card::Browser { url } => {
            let mut controls = h_flex().gap_2().items_center();
            if let Some(url) = url.clone() {
                let label = if copied { t!("engines.copied") } else { t!("engines.copy_link") };
                controls = controls.child(button(shell, at, 2, "engine-copy-link", Kind::Ghost, label).icon(Icon::Copy).on_click(cx.listener(move |this, _, _, cx| this.engine_copy(engine, &url, cx))));
            }
            Line::new(t!("engines.browser_wait"), t!("engines.browser_desc", host = host(shell))).label_el(waiting(t!("engines.browser_wait").to_owned())).control(controls.child(cancel(cx)))
        }
        Card::Code { url, code } => {
            let (open, copy) = (url.clone(), code.clone());
            let shown = url.trim_start_matches("https://").to_owned();
            let code_el = h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .px_3()
                        .py(px(6.))
                        .rounded(px(R_SM))
                        .bg(p.code_bg)
                        .border_1()
                        .border_color(p.hair)
                        .font_family(MONO)
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text)
                        .child(code.clone()),
                )
                .child(button(shell, at, 2, "engine-copy-code", Kind::Secondary, if copied { t!("engines.copied") } else { t!("engines.copy_code") }).icon(Icon::Copy).on_click(cx.listener(move |this, _, _, cx| this.engine_copy(engine, &copy, cx))))
                .child(button(shell, at, 3, "engine-open-page", Kind::Ghost, t!("engines.open_page")).icon(Icon::External).on_click(move |_, _, cx| cx.open_url(&open)));
            Line::new(t!("engines.code_title"), t!("engines.code_desc", url = shown)).label_el(waiting(t!("engines.code_title").to_owned())).control(cancel(cx)).below(code_el)
        }
        Card::Failed { problem, detail } => {
            let desc = engines::problem_words(*problem, &host(shell));
            // The tool's own words, quieter, under the advice.
            let said = (!detail.is_empty()).then(|| div().font_family(MONO).text_size(px(11.5)).line_height(px(17.)).text_color(p.muted).child(detail.clone()));
            let mut controls = h_flex().gap_2().items_center();
            if matches!(problem, engines::Problem::Daemon(equipe_core::protocol::SignInFailure::DeviceCodeOff)) {
                controls = controls.child(button(shell, at, 2, "engine-chatgpt", Kind::Ghost, t!("engines.open_chatgpt_settings")).icon(Icon::External).on_click(|_, _, cx| cx.open_url("https://chatgpt.com/#settings/Security")));
            }
            let dismiss = button(shell, at, 3, "engine-dismiss", Kind::Ghost, t!("engines.dismiss")).on_click(cx.listener(move |this, _, _, cx| {
                this.engines.keep(engine);
                cx.notify();
            }));
            let retry = button(shell, at, 1, "engine-retry", Kind::Secondary, t!("engines.retry")).icon(Icon::Retry).on_click(cx.listener(move |this, _, _, cx| this.engine_sign_in(engine, cx)));
            let label = h_flex().gap_2().items_center().child(icon(Icon::Warning, ICON_SM, p.danger())).child(t!("engines.failed_title"));
            let line = Line::new(t!("engines.failed_title"), desc).label_el(label).warn(true).control(controls.child(dismiss).child(retry));
            match said {
                Some(said) => line.below(said),
                None => line,
            }
        }
        Card::ConfirmSignOut => {
            let keep = button(shell, at, 1, "engine-keep", Kind::Ghost, t!("engines.keep")).on_click(cx.listener(move |this, _, _, cx| {
                this.engines.keep(engine);
                cx.notify();
            }));
            let out = button(shell, at, 2, "engine-sign-out-sure", Kind::Danger, t!("engines.sign_out")).on_click(cx.listener(move |this, _, _, cx| this.engine_sign_out(engine, cx)));
            Line::new(t!("engines.confirm_sign_out", name = platform.name), t!("engines.confirm_sign_out_desc", host = host(shell), tool = platform.tool, name = platform.name)).warn(true).control(h_flex().gap_2().items_center().child(keep).child(out))
        }
        Card::SigningOut => Line::new(t!("engines.signing_out"), "").label_el(waiting(t!("engines.signing_out").to_owned())),
    };
    Some(line)
}

/// 사용량: the plan's windows where they are known; room for the rest.
fn usage(shell: &Shell, platform: &Platform, signed_in: bool) -> Line {
    let p = shell.palette;
    let line = Line::new(t!("engines.row_usage"), "");
    // Claude's as before; another engine's from what the daemon read of it.
    let read = match platform.usage {
        Usage::ClaudePlan => shell.model.plan,
        Usage::Later => shell.model.engines.plan(platform.engine).map(|p| p.usage),
    };
    let kind = shell.model.engines.plan(platform.engine).and_then(|p| p.plan_type.clone());
    match (platform.usage, read) {
        (_, Some(plan)) if signed_in && plan.five_hour.is_none() && plan.seven_day.is_none() => {
            // Read, but a plan without five-hour or weekly windows (a free one keeps a monthly one).
            line.control(value(p, t!("engines.usage_no_windows", plan = kind.as_deref().map(engines::title_case).unwrap_or_default())))
        }
        (_, Some(plan)) if signed_in => {
            let rows = engines::plan_usage(&plan, crate::state::now_unix() as i64);
            if rows.is_empty() {
                return line.control(value(p, t!("engines.usage_unknown")));
            }
            let bars = v_flex().w_full().gap_2().children(rows.into_iter().map(|(words, percent)| {
                let fill = if percent >= 90 { p.danger() } else if percent >= 70 { p.amber } else { p.accent };
                v_flex()
                    .gap_1()
                    .child(div().text_size(px(12.5)).text_color(p.text2).child(words))
                    .child(div().w_full().h(px(5.)).rounded(px(3.)).bg(p.wash).child(div().h_full().w(relative(percent as f32 / 100.)).rounded(px(3.)).bg(fill)))
            }));
            line.below(bars)
        }
        (_, None) if signed_in => line.control(value(p, t!("engines.usage_unknown"))),
        _ => line.control(value(p, "—")),
    }
}

/// Small conveniences on settings' rows.
trait LineExt: Sized {
    fn control_opt(self, control: Option<AnyElement>) -> Self;
    fn when_remote(self, remote: bool, f: impl FnOnce(Self) -> Self) -> Self;
}

impl LineExt for Line {
    fn control_opt(self, control: Option<AnyElement>) -> Self {
        match control {
            Some(c) => self.control(c),
            None => self,
        }
    }

    fn when_remote(self, remote: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if remote { f(self) } else { self }
    }
}

// ── what the cards do ──

impl Shell {
    /// Where the engines stand, from the daemon; the page says so if it never answers.
    /// The pasted key to the daemon, once it reads like one; the box is emptied at once.
    pub fn save_nebius_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.settings.nebius_key.clone();
        let key = input.read(cx).value().trim().to_owned();
        if !equipe_core::nemotron::key_is_plausible(&key) {
            self.settings.nebius_problem = Some(t!("engines.nebius_bad_key").to_owned());
            return cx.notify();
        }
        self.settings.nebius_problem = None;
        self.settings.nebius_forgetting = false;
        input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        self.send_command(Command::SetNebiusKey { key: Some(key) });
        cx.notify();
    }

    pub fn ask_engines(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.model.conn, Conn::Connected { .. }) {
            return;
        }
        let waiting = self.engines.statuses.is_none() && self.engines.asked.is_none();
        for command in self.engines.open(Instant::now()) {
            self.send_command(command);
        }
        if waiting {
            self.engines_later(engines::NO_ANSWER, cx);
        }
    }

    /// Look again after `wait`: a silent daemon, a start nobody answered.
    fn engines_later(&self, wait: std::time::Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.engines.tick(Instant::now());
                cx.notify();
            });
        })
        .detach();
    }

    /// "로그인": on the daemon's machine; from another Mac, with a code.
    pub fn engine_sign_in(&mut self, engine: Engine, cx: &mut Context<Self>) {
        for command in self.engines.sign_in(engine, self.endpoint.is_remote(), Instant::now()) {
            self.send_command(command);
        }
        self.engines_later(engines::NO_ANSWER, cx);
        cx.notify();
    }

    pub fn engine_cancel(&mut self, engine: Engine, cx: &mut Context<Self>) {
        for command in self.engines.cancel(engine) {
            self.send_command(command);
        }
        cx.notify();
    }

    pub fn engine_sign_out(&mut self, engine: Engine, cx: &mut Context<Self>) {
        for command in self.engines.sign_out(engine) {
            self.send_command(command);
        }
        cx.notify();
    }

    /// The code, or the page to open by hand, to the clipboard: "복사됨" a moment.
    pub fn engine_copy(&mut self, engine: Engine, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        self.engines.copied = Some((engine, Instant::now()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(1700)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// "터미널에서 로그인": Terminal opens on this Mac (the daemon's) running
    /// the tool's own sign-in, for the person to finish themselves; the card
    /// looks again now and then until it is signed in.
    pub fn engine_terminal(&mut self, engine: Engine, cx: &mut Context<Self>) {
        let SignsIn::InTerminal { command } = engines::platform(engine).signs_in else { return };
        if let Err(why) = open_terminal(command) {
            self.composer.flash = Some(t!("engines.terminal_failed", why = why));
            cx.notify();
            return;
        }
        let started = self.engines.terminal.is_none();
        self.engines.terminal = Some(Instant::now());
        cx.notify();
        if !started {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(engines::TERMINAL_POLL).await;
                let going = this
                    .update(cx, |this, _| {
                        this.engines.tick(Instant::now());
                        let watching = this.engines.terminal.is_some();
                        if watching && this.screen == Screen::Settings && this.settings.category == crate::settings::Category::Engines {
                            this.send_command(equipe_core::protocol::Command::Engines);
                        }
                        watching
                    })
                    .unwrap_or(false);
                if !going {
                    break;
                }
            }
        })
        .detach();
    }
}

/// A Terminal window on this Mac running `command` in the person's own login
/// shell (their PATH, their environment), through a `.command` file: no
/// Automation permission is asked for, and nothing is read or passed on.
fn open_terminal(command: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join("equipe-sign-in");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{}.command", command.split_whitespace().next().unwrap_or("sign-in")));
    let script = format!("#!/bin/sh\nclear\nexec \"${{SHELL:-/bin/zsh}}\" -lc '{command}; exec \"${{SHELL:-/bin/zsh}}\" -l'\n");
    std::fs::write(&file, script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let opened = std::process::Command::new("open").arg(&file).status().map_err(|e| e.to_string())?;
    if opened.success() { Ok(()) } else { Err(format!("open {}", opened)) }
}
