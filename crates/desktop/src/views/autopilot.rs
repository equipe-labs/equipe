//! Login autopilot in Settings › Accounts (`equipe_core::autopilot`): under
//! each login, the switches for what equipe may do by itself around its
//! sign-in — each with what it does in a line, each the person's alone — and
//! the authenticator seed (pasted, or read off the site's QR code in equipe's
//! browser) and equipe's own passkey; and one card for autopilot as a whole:
//! what it saved this week, the mailbox codes are read from, Messages,
//! keep-alive, and command-line sign-ins. Every secret goes once, masked, to
//! the daemon machine's secret store: no screen gets it back.

use std::collections::HashSet;

use equipe_core::protocol::Command;
use equipe_core::protocol::secrets::SecretValue;
use equipe_core::protocol::vault::{Autopilot, CliKind, CliLoginDraft, Login, MailboxDraft};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::settings::{Line, Section};
use crate::theme::{CONTROL, Palette, R_MD, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// The boxes and what is open.
pub struct AutopilotUi {
    /// Logins whose autopilot switches are open under their row.
    pub open: HashSet<u32>,
    pub seed: Entity<InputState>,
    pub imap_host: Entity<InputState>,
    pub imap_user: Entity<InputState>,
    pub imap_password: Entity<InputState>,
    pub mailbox_open: bool,
    pub cli_name: Entity<InputState>,
    pub cli_host: Entity<InputState>,
    pub cli_public: Entity<InputState>,
    pub cli_secret: Entity<InputState>,
    pub cli_squad: Entity<InputState>,
    /// The command-line form open, for this kind.
    pub cli_kind: Option<CliKind>,
    pub problem: Option<String>,
}

impl AutopilotUi {
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let input = |placeholder: &'static str, masked: bool, window: &mut Window, cx: &mut Context<Shell>| cx.new(|cx| InputState::new(window, cx).masked(masked).placeholder(placeholder));
        Self {
            open: HashSet::new(),
            seed: input(t!("autopilot.seed_placeholder"), true, window, cx),
            imap_host: input(t!("autopilot.imap_host"), false, window, cx),
            imap_user: input(t!("autopilot.imap_user"), false, window, cx),
            imap_password: input(t!("autopilot.imap_password"), true, window, cx),
            mailbox_open: false,
            cli_name: input(t!("autopilot.cli_name"), false, window, cx),
            cli_host: input(t!("autopilot.cli_host"), false, window, cx),
            cli_public: input(t!("autopilot.cli_public"), false, window, cx),
            cli_secret: input(t!("autopilot.cli_secret"), true, window, cx),
            cli_squad: input(t!("autopilot.cli_squad"), false, window, cx),
            cli_kind: None,
            problem: None,
        }
    }
}

fn boxed(p: Palette, input: &Entity<InputState>) -> AnyElement {
    div()
        .w_full()
        .h(px(CONTROL + 4.))
        .px_3()
        .flex()
        .items_center()
        .rounded(px(R_MD))
        .border_1()
        .border_color(p.hair)
        .bg(p.code_bg)
        .text_size(px(13.))
        .child(Input::new(input).appearance(false).bordered(false))
        .into_any_element()
}

fn small(p: Palette, text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(text.into())
}

/// The switches a login's autopilot has, with their words.
fn switches() -> [(&'static str, &'static str, &'static str, fn(&Autopilot) -> bool, fn(&mut Autopilot, bool)); 7] {
    [
        ("totp", t!("autopilot.totp"), t!("autopilot.totp_tip"), |a| a.totp, |a, on| a.totp = on),
        ("email", t!("autopilot.email"), t!("autopilot.email_tip"), |a| a.email, |a, on| a.email = on),
        ("sms", t!("autopilot.sms"), t!("autopilot.sms_tip"), |a| a.sms, |a, on| a.sms = on),
        ("passkey", t!("autopilot.passkey"), t!("autopilot.passkey_tip"), |a| a.passkey, |a, on| a.passkey = on),
        ("remember", t!("autopilot.remember"), t!("autopilot.remember_tip"), |a| a.remember, |a, on| a.remember = on),
        ("keepalive", t!("autopilot.keepalive"), t!("autopilot.keepalive_tip"), |a| a.keepalive, |a, on| a.keepalive = on),
        ("device", t!("autopilot.device_code"), t!("autopilot.device_code_tip"), |a| a.device_code, |a, on| a.device_code = on),
    ]
}

/// The button on a login's row that opens its autopilot switches.
pub fn toggle(shell: &Shell, login: &Login, cx: &mut Context<Shell>) -> AnyElement {
    let id = login.id;
    let open = shell.settings.accounts.autopilot.open.contains(&id);
    ui::button(shell.palette, SharedString::from(format!("login-autopilot-{id}")), Kind::Ghost, t!("autopilot.open"))
        .selected(open)
        .on_click(cx.listener(move |this, _, _, cx| {
            let open = &mut this.settings.accounts.autopilot.open;
            if !open.remove(&id) {
                open.insert(id);
            }
            cx.notify();
        }))
        .into_any_element()
}

/// Under a login's row, when open: its autopilot switches, each with what it
/// does, and its seed and passkey.
pub fn below(shell: &Shell, login: &Login, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let ui = &shell.settings.accounts.autopilot;
    if !ui.open.contains(&login.id) {
        return None;
    }
    let p = shell.palette;
    let id = login.id;
    let a = login.autopilot.clone();
    let rows = switches().into_iter().map(|(key, name, tip, get, set)| {
        let handle = cx.entity().downgrade();
        let now = a.clone();
        // A seed or a passkey first: the switch waits for it.
        let enabled = match key {
            "totp" => a.has_totp,
            "passkey" => a.has_passkey,
            "email" => shell.model.autopilot.mailbox.is_some(),
            "sms" => shell.model.autopilot.sms,
            _ => true,
        };
        let switch = ui::switch(p, SharedString::from(format!("login-ap-{key}-{id}")), get(&a)).name(name).disabled(!enabled).on_flip(move |on, _, cx| {
            let mut changed = now.clone();
            set(&mut changed, on);
            let _ = handle.update(cx, |this, cx| {
                this.send_command(Command::SetLoginAutopilot { id, autopilot: changed.clone() });
                cx.notify();
            });
        });
        h_flex().w_full().gap_3().items_start().child(switch).child(v_flex().flex_1().gap_0p5().child(div().text_size(px(SMALL)).text_color(p.text2).child(name)).child(small(p, tip)))
    });
    let seed = if a.has_totp {
        h_flex().gap_2().items_center().child(small(p, t!("autopilot.seed_kept"))).child(ui::button(p, SharedString::from(format!("login-seed-forget-{id}")), Kind::Ghost, t!("autopilot.seed_forget")).on_click(cx.listener(move |this, _, _, cx| {
            this.send_command(Command::SetLoginTotp { id, secret: None });
            cx.notify();
        })))
    } else {
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(div().flex_1().child(boxed(p, &ui.seed)))
            .child(ui::button(p, SharedString::from(format!("login-seed-save-{id}")), Kind::Secondary, t!("autopilot.seed_save")).on_click(cx.listener(move |this, _, window, cx| {
                let input = this.settings.accounts.autopilot.seed.clone();
                let secret = input.read(cx).value().trim().to_owned();
                input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                if !secret.is_empty() {
                    this.send_command(Command::SetLoginTotp { id, secret: Some(SecretValue(secret)) });
                }
                cx.notify();
            })))
            .child(ui::button(p, SharedString::from(format!("login-seed-scan-{id}")), Kind::Ghost, t!("autopilot.seed_scan")).on_click(cx.listener(move |this, _, _, cx| {
                this.send_command(Command::ScanLoginTotp { id });
                cx.notify();
            })))
    };
    let passkey = if a.has_passkey {
        ui::button(p, SharedString::from(format!("login-passkey-forget-{id}")), Kind::Ghost, t!("autopilot.passkey_forget")).on_click(cx.listener(move |this, _, _, cx| {
            this.send_command(Command::ForgetLoginPasskey { id });
            cx.notify();
        }))
    } else {
        ui::button(p, SharedString::from(format!("login-passkey-add-{id}")), Kind::Ghost, t!("autopilot.passkey_add")).on_click(cx.listener(move |this, _, _, cx| {
            this.send_command(Command::RegisterLoginPasskey { id });
            cx.notify();
        }))
    };
    let checked = a.checked.as_ref().map(|c| small(p, t!("autopilot.last_check", what = format!("{} · {}", crate::activity::ago(now_secs().saturating_sub(c.at)), c.outcome.replace('_', " ")))));
    Some(
        v_flex()
            .w_full()
            .gap_2()
            .pt_1()
            .children(rows)
            .child(seed)
            .child(h_flex().gap_2().items_center().child(passkey).when(!a.has_passkey, |row| row.child(small(p, t!("autopilot.passkey_add_note")))))
            .children(checked)
            .into_any_element(),
    )
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The kinds of command-line sign-ins, as the form offers them.
fn kinds() -> [(CliKind, &'static str); 5] {
    [
        (CliKind::GhToken, t!("autopilot.kind_gh_token")),
        (CliKind::GhApp, t!("autopilot.kind_gh_app")),
        (CliKind::AwsKeys, t!("autopilot.kind_aws")),
        (CliKind::Env, t!("autopilot.kind_env")),
        (CliKind::DeviceLogin, t!("autopilot.kind_device")),
    ]
}

fn kind_word(kind: CliKind) -> &'static str {
    kinds().into_iter().find(|(k, _)| *k == kind).map_or("", |(_, w)| w)
}

/// The "Login autopilot" card: this week, the mailbox, Messages,
/// keep-alive, command-line sign-ins.
pub(super) fn section(shell: &Shell, cx: &mut Context<Shell>) -> Section {
    let p = shell.palette;
    let status = &shell.model.autopilot;
    let ui = &shell.settings.accounts.autopilot;
    let resolved: u64 = status.resolved.iter().map(|(_, n)| n).sum();
    let summary = if resolved == 0 && status.asked == 0 { t!("autopilot.summary_none").to_owned() } else { t!("autopilot.summary", resolved = resolved, minutes = status.minutes_saved, asked = status.asked) };
    let mut lines = vec![Line::new(t!("autopilot.title"), summary).words(t!("autopilot.words"))];
    // The mailbox codes are read from.
    let mailbox_desc = status.mailbox.as_ref().map_or_else(|| t!("autopilot.mailbox_none").to_owned(), |m| t!("autopilot.mailbox_desc", user = m.username, host = m.host));
    let mut mailbox = Line::new(t!("autopilot.mailbox"), mailbox_desc);
    if ui.mailbox_open {
        let field = |input: &Entity<InputState>| div().flex_1().child(boxed(p, input));
        mailbox = mailbox.below(
            v_flex()
                .w_full()
                .gap_2()
                .pt_1()
                .child(h_flex().w_full().gap_2().child(field(&ui.imap_host)).child(field(&ui.imap_user)).child(field(&ui.imap_password)))
                .child(h_flex().w_full().gap_2().items_center().child(small(p, t!("autopilot.mailbox_note")).flex_1()).child(ui::button(p, "ap-mailbox-cancel", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.settings.accounts.autopilot.mailbox_open = false;
                    cx.notify();
                }))).child(ui::button(p, "ap-mailbox-save", Kind::Primary, t!("accounts.save")).on_click(cx.listener(|this, _, window, cx| this.save_mailbox_form(window, cx))))),
        );
    } else {
        let connected = status.mailbox.is_some();
        mailbox = mailbox.control(
            h_flex()
                .gap_2()
                .child(ui::button(p, "ap-mailbox-open", Kind::Secondary, t!("autopilot.mailbox_connect")).on_click(cx.listener(|this, _, _, cx| {
                    this.settings.accounts.autopilot.mailbox_open = true;
                    cx.notify();
                })))
                .when(connected, |row| {
                    row.child(ui::button(p, "ap-mailbox-forget", Kind::Ghost, t!("autopilot.mailbox_forget")).on_click(cx.listener(|this, _, _, cx| {
                        this.send_command(Command::SaveMailbox { mailbox: None });
                        cx.notify();
                    })))
                }),
        );
    }
    lines.push(mailbox);
    // Messages: the person's grant, said plainly where it is missing.
    let sms_desc = match status.sms_access {
        None => t!("autopilot.sms_unsupported"),
        Some(false) if status.sms => t!("autopilot.sms_no_access"),
        _ => t!("autopilot.sms_desc"),
    };
    let sms_on = status.sms;
    lines.push(Line::new(t!("autopilot.sms_line"), sms_desc).warn(status.sms && status.sms_access == Some(false)).switch(shell, "ap-sms", sms_on, status.sms_access.is_some(), |this, on, _, _| this.send_command(Command::SetSmsCodes { on }), cx));
    // Keep-alive: how often.
    let hours = status.keepalive_hours;
    let options: Vec<(u32, String)> = [0u32, 3, 6, 12, 24].into_iter().map(|h| (h, if h == 0 { t!("autopilot.keepalive_off").to_owned() } else { t!("autopilot.hours", n = h) })).collect();
    let picker = super::settings::segmented(shell, "ap-keepalive", options, hours, |this, h, _, _| this.send_command(Command::SetKeepAlive { hours: h }), cx);
    lines.push(Line::new(t!("autopilot.keepalive_line"), t!("autopilot.keepalive_desc")).control(picker).wrap_control());
    // Command-line sign-ins.
    for c in &status.cli {
        let id = c.id;
        let mut desc = format!("{} · {}", kind_word(c.kind), if c.host.is_empty() { c.squad.clone().unwrap_or_else(|| t!("accounts.every_squad").to_owned()) } else { c.host.clone() });
        if let Some(last) = &c.last {
            desc = format!("{desc} · {last}");
        }
        lines.push(Line::new(c.name.clone(), desc).control(ui::button(p, SharedString::from(format!("ap-cli-del-{id}")), Kind::Ghost, t!("accounts.delete")).on_click(cx.listener(move |this, _, _, cx| {
            this.send_command(Command::DeleteCliLogin { id });
            cx.notify();
        }))));
    }
    let add = Line::new(t!("autopilot.cli"), t!("autopilot.cli_desc"));
    let add = match ui.cli_kind {
        Some(kind) => {
            let field = |input: &Entity<InputState>| div().flex_1().child(boxed(p, input));
            let picks = h_flex().gap_1().flex_wrap().children(kinds().into_iter().map(|(k, word)| {
                ui::button(p, SharedString::from(format!("ap-cli-kind-{k:?}")), Kind::Ghost, word).selected(k == kind).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.accounts.autopilot.cli_kind = Some(k);
                    cx.notify();
                }))
            }));
            add.below(
                v_flex()
                    .w_full()
                    .gap_2()
                    .pt_1()
                    .child(picks)
                    .child(h_flex().w_full().gap_2().child(field(&ui.cli_name)).child(field(&ui.cli_host)))
                    .child(h_flex().w_full().gap_2().child(field(&ui.cli_public)).child(field(&ui.cli_squad)))
                    .when(kind != CliKind::DeviceLogin, |form| form.child(field(&ui.cli_secret)))
                    .children(ui.problem.clone().map(|why| div().text_size(px(SMALL)).text_color(p.danger()).child(why)))
                    .child(h_flex().w_full().gap_2().items_center().child(small(p, t!("autopilot.cli_note")).flex_1()).child(ui::button(p, "ap-cli-cancel", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.settings.accounts.autopilot.cli_kind = None;
                        cx.notify();
                    }))).child(ui::button(p, "ap-cli-save", Kind::Primary, t!("accounts.save")).on_click(cx.listener(|this, _, window, cx| this.save_cli_form(window, cx))))),
            )
        }
        None => add.control(ui::button(p, "ap-cli-add", Kind::Secondary, t!("accounts.add_button")).on_click(cx.listener(|this, _, _, cx| {
            this.settings.accounts.autopilot.cli_kind = Some(CliKind::GhToken);
            this.settings.accounts.autopilot.problem = None;
            cx.notify();
        }))),
    };
    lines.push(add);
    Section::new(t!("autopilot.title"), lines).foot(t!("autopilot.foot", machine = shell.endpoint.label()))
}

impl Shell {
    fn save_mailbox_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.settings.accounts.autopilot;
        let (h, u, pw) = (ui.imap_host.clone(), ui.imap_user.clone(), ui.imap_password.clone());
        let host = h.read(cx).value().trim().to_owned();
        let username = u.read(cx).value().trim().to_owned();
        let password = pw.read(cx).value().to_string();
        pw.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        if host.is_empty() || username.is_empty() {
            return cx.notify();
        }
        self.send_command(Command::SaveMailbox { mailbox: Some(MailboxDraft { host, port: 993, username, password: (!password.is_empty()).then_some(password) }) });
        self.settings.accounts.autopilot.mailbox_open = false;
        cx.notify();
    }

    fn save_cli_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &self.settings.accounts.autopilot;
        let Some(kind) = ui.cli_kind else { return };
        let read = |input: &Entity<InputState>, cx: &Context<Self>| input.read(cx).value().trim().to_owned();
        let (name, host, public, squad, secret) = (read(&ui.cli_name, cx), read(&ui.cli_host, cx), read(&ui.cli_public, cx), read(&ui.cli_squad, cx), read(&ui.cli_secret, cx));
        let inputs = [ui.cli_name.clone(), ui.cli_host.clone(), ui.cli_public.clone(), ui.cli_squad.clone(), ui.cli_secret.clone()];
        if name.is_empty() || (kind != CliKind::DeviceLogin && secret.is_empty()) {
            self.settings.accounts.autopilot.problem = Some(t!("autopilot.cli_missing").to_owned());
            return cx.notify();
        }
        for input in inputs {
            input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        }
        let draft = CliLoginDraft { id: None, kind, name, host, public, squad: (!squad.is_empty()).then_some(squad), secret: (!secret.is_empty()).then_some(secret) };
        self.send_command(Command::SaveCliLogin { draft });
        self.settings.accounts.autopilot.cli_kind = None;
        self.settings.accounts.autopilot.problem = None;
        cx.notify();
    }
}
