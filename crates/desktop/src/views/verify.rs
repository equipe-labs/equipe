//! A verification code a site emailed while a member's page waits on it
//! (`equipe_core::verify`): ONE card in the conversation, over the composer —
//! "GitHub sent a verification code to squad@example.com" — with the steps,
//! "Fill code from email" (the daemon reads only that site's recent code
//! email and types the code into the member's page), "Open email" (the
//! mailbox's own web page, in this Mac's browser) and "I'll type it" (the
//! page, in the person's hands, its code field ready). The card never shows
//! or holds the code.
//!
//! Also the "Mailbox for verification codes" row of Logins › Connections:
//! connect Gmail read-only (or use the IMAP mailbox of Login autopilot),
//! what it reads in plain words, and disconnect.

use equipe_core::protocol::Command;
use equipe_core::protocol::vault::GmailClient;
use equipe_core::protocol::verify::{Phase, Verification};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::motion;
use crate::theme::{CONTROL, Palette, R_LG, R_MD, SMALL, look};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// Where Google Cloud makes an OAuth client (for a build without its own).
pub const CLOUD_CREDENTIALS: &str = "https://console.cloud.google.com/apis/credentials";

/// What the cards and the Connections row remember between draws.
#[derive(Default)]
pub struct VerifyUi {
    /// This screen pressed Connect: Google's page is opened here when the daemon gives it.
    pub connecting: bool,
    /// Why connecting didn't go on, as the daemon said.
    pub connect_error: Option<String>,
    /// Disconnect waits for its second click.
    pub confirm_disconnect: bool,
    /// The person's own OAuth client, for a build without one: its two boxes.
    pub client_id: Option<Entity<InputState>>,
    pub client_secret: Option<Entity<InputState>>,
    /// Google's redirect brought back to the daemon's Mac over SSH, while connecting from another Mac.
    pub tunnel: Option<crate::tunnel::Tunnel>,
}

impl VerifyUi {
    /// Whether a box has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        [&self.client_id, &self.client_secret].into_iter().flatten().any(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }
}

/// The card's title: "GitHub sent a verification code to squad@example.com".
pub fn title(card: &Verification) -> String {
    match &card.mailbox {
        Some(m) => t!("verify.title", site = card.name, mailbox = m.address).to_string(),
        None => t!("verify.title_no_mailbox", site = card.name).to_string(),
    }
}

/// The steps the card says, in order.
pub fn steps(card: &Verification) -> Vec<String> {
    if card.mailbox.is_none() {
        return vec![t!("verify.step_connect").to_string(), t!("verify.step_type_yourself", name = card.member).to_string()];
    }
    vec![t!("verify.step_fill", site = card.name).to_string(), t!("verify.step_types", name = card.member).to_string()]
}

/// The line that says where it stands, if anything is to be said.
pub fn progress(card: &Verification) -> Option<String> {
    Some(match card.phase {
        Phase::Waiting => return None,
        Phase::Fetching => t!("verify.fetching", site = card.name).to_string(),
        Phase::Filled => t!("verify.filled", name = card.member).to_string(),
        Phase::NoMail => t!("verify.no_mail", site = card.name).to_string(),
        Phase::Failed => match card.detail.as_deref() {
            Some("not_taken") => t!("verify.not_taken", site = card.name).to_string(),
            Some("host" | "insecure" | "no_field" | "account") => t!("verify.moved").to_string(),
            Some(why) => t!("verify.failed", why = why).to_string(),
            None => t!("verify.failed", why = "—").to_string(),
        },
    })
}

impl Shell {
    /// "Fill code from email": the one click that has the daemon fetch and type the code.
    pub fn fill_verification(&mut self, id: u64, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::FillVerificationCode { squad, id });
        if let Some(card) = self.model.verifications.iter_mut().find(|c| c.id == id) {
            card.phase = Phase::Fetching;
            card.detail = None;
        }
        self.model.revision += 1;
        cx.notify();
    }

    /// "I'll type it": the card goes, and the page opens in the person's hands, its code field ready.
    pub fn type_verification(&mut self, id: u64, cx: &mut Context<Self>) {
        let page = self.model.verifications.iter().find(|c| c.id == id).and_then(|c| c.page);
        let squad = self.model.squad.clone();
        self.send_command(Command::TypeVerificationCode { squad, id });
        self.model.verifications.retain(|c| c.id != id);
        self.model.revision += 1;
        if let Some(page) = page {
            self.reveal_browser(page, cx);
        }
        cx.notify();
    }

    /// "Open email": the mailbox's own web page, in this Mac's browser — never a member's.
    pub fn open_verification_email(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(url) = self.model.verifications.iter().find(|c| c.id == id).and_then(|c| c.open_url.clone()) else { return };
        if url.starts_with("https://") && !cfg!(feature = "snapshot") {
            cx.open_url(&url);
        }
    }

    /// "Connect mailbox" on a card: Logins › Connections, where the row is.
    pub fn open_mailbox_connect(&mut self, cx: &mut Context<Self>) {
        self.open_logins(cx);
    }

    /// Connect Gmail: this build's own client, or the one typed into the row's boxes.
    pub fn connect_gmail(&mut self, cx: &mut Context<Self>) {
        let vu = &self.verify;
        let client = match (&vu.client_id, &vu.client_secret) {
            (Some(id), Some(secret)) => {
                let (id, secret) = (id.read(cx).value().trim().to_owned(), secret.read(cx).value().trim().to_owned());
                (!id.is_empty()).then_some(GmailClient { id, secret })
            }
            _ => None,
        };
        if client.is_none() && !self.model.autopilot.gmail_client_builtin {
            return cx.notify();
        }
        self.send_command(Command::ConnectGmail { client });
        self.verify.connecting = true;
        self.verify.connect_error = None;
        self.verify.client_id = None;
        self.verify.client_secret = None;
        cx.notify();
    }

    /// The boxes for the person's own OAuth client.
    pub fn open_gmail_client_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.verify.client_id = Some(cx.new(|cx| InputState::new(window, cx).placeholder(t!("verify.client_id"))));
        self.verify.client_secret = Some(cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("verify.client_secret"))));
        self.verify.connect_error = None;
        cx.notify();
    }

    pub fn close_gmail_client_form(&mut self, cx: &mut Context<Self>) {
        self.verify.client_id = None;
        self.verify.client_secret = None;
        cx.notify();
    }

    /// Disconnect: Gmail's grant forgotten (and the IMAP mailbox, when that is the one).
    pub fn disconnect_mailbox(&mut self, cx: &mut Context<Self>) {
        if self.model.autopilot.gmail.is_some() {
            self.send_command(Command::DisconnectGmail);
        } else if self.model.autopilot.mailbox.is_some() {
            self.send_command(Command::SaveMailbox { mailbox: None });
        }
        self.verify.confirm_disconnect = false;
        cx.notify();
    }

    /// Connecting Gmail, as the daemon tells it: Google's page opened here
    /// (for the screen that pressed Connect), or why not.
    pub fn on_mailbox_connect(&mut self, url: Option<&str>, error: Option<&str>, cx: &mut Context<Self>) {
        if let Some(why) = error {
            self.verify.connecting = false;
            self.verify.connect_error = Some(why.to_owned());
            if let Some(tunnel) = self.verify.tunnel.take() {
                std::thread::spawn(move || tunnel.close());
            }
        }
        if let Some(url) = url
            && self.verify.connecting
            && url.starts_with("https://accounts.google.com/")
            && !cfg!(feature = "snapshot")
        {
            // Google comes back to a listener on the daemon's Mac: from another
            // Mac, its port is forwarded there first (`crate::tunnel`), for as
            // long as connecting may take.
            let route = equipe_core::cli_signin::route(url, self.endpoint.is_remote(), true, false);
            match (route, &self.endpoint) {
                (equipe_core::cli_signin::Route::Forward(port), crate::target::Endpoint::Ssh(host)) => {
                    let (host, url) = (host.clone(), url.to_owned());
                    let opened = cx.background_spawn(async move { crate::tunnel::open(&host, port) });
                    cx.spawn(async move |this, cx| {
                        let opened = opened.await;
                        let _ = this.update(cx, |this, cx| {
                            if let Ok(tunnel) = opened {
                                this.verify.tunnel = Some(tunnel);
                            }
                            cx.open_url(&url);
                        });
                        cx.background_executor().timer(std::time::Duration::from_secs(6 * 60)).await;
                        let _ = this.update(cx, |this, _| {
                            if let Some(tunnel) = this.verify.tunnel.take() {
                                std::thread::spawn(move || tunnel.close());
                            }
                        });
                    })
                    .detach();
                }
                _ => cx.open_url(url),
            }
        }
        cx.notify();
    }
}

#[cfg(feature = "snapshot")]
impl Shell {
    /// A sample card over the composer, no daemon asked: `verify[:waiting|fetching|nomail|filled|failed|nomailbox]`.
    pub fn snapshot_verify(&mut self, what: &str, cx: &mut Context<Self>) {
        use equipe_core::protocol::verify::{Channel, MailboxRef};
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let member = self.model.members.iter().find(|m| m.role != "pm").or(self.model.members.first()).map(|m| (m.name.clone(), m.session));
        let (name, session) = member.map_or(("backend".to_owned(), None), |(n, s)| (n, Some(s)));
        let address = "squad@example.com";
        let (phase, detail, mailbox) = match what {
            "fetching" => (Phase::Fetching, None, true),
            "nomail" => (Phase::NoMail, None, true),
            "filled" => (Phase::Filled, None, true),
            "failed" => (Phase::Failed, Some("not_taken".to_owned()), true),
            "nomailbox" => (Phase::Waiting, None, false),
            _ => (Phase::Waiting, None, true),
        };
        self.model.verifications = vec![Verification {
            id: 900,
            session,
            member: name.clone(),
            ticket: Some(12),
            site: "github.com".into(),
            name: "GitHub".into(),
            channel: Channel::Email,
            mailbox: mailbox.then(|| MailboxRef { address: address.into(), kind: "gmail".into() }),
            open_url: mailbox.then(|| equipe_core::verify::open_url("gmail", address, None, "github.com")).flatten(),
            lane: format!("{}/{name}", self.model.squad),
            page: Some(1),
            phase,
            detail,
            at: now,
        }];
        self.model.revision += 1;
        // In front, as a new card comes.
        self.chat.dock_at.set(usize::MAX);
        self.show(Screen::Chat, cx);
        cx.notify();
    }

    /// The Connections row's mailbox on a sample: `vmailbox:<none|gmail|imap|form|connecting>`.
    pub fn snapshot_mailbox(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        use equipe_core::protocol::vault::{Gmail, Mailbox};
        let status = &mut self.model.autopilot;
        (status.gmail, status.mailbox, status.gmail_client_builtin) = (None, None, true);
        self.verify = VerifyUi::default();
        match what {
            "gmail" => status.gmail = Some(Gmail { address: "squad@example.com".into(), client_id: "x.apps.googleusercontent.com".into() }),
            "imap" => status.mailbox = Some(Mailbox { host: "imap.fastmail.com".into(), port: 993, username: "squad@example.com".into() }),
            "connecting" => self.verify.connecting = true,
            "form" => {
                self.model.autopilot.gmail_client_builtin = false;
                self.open_gmail_client_form(window, cx);
            }
            _ => {}
        }
        cx.notify();
    }
}

fn step_list(p: Palette, steps: Vec<String>) -> Div {
    v_flex().gap_1().children(steps.into_iter().enumerate().map(|(i, s)| {
        h_flex()
            .gap_2()
            .items_start()
            .child(div().flex_none().w(px(16.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(format!("{}.", i + 1)))
            .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(p.text2).child(s))
    }))
}

/// The card for verification `id`.
pub fn card(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let c = shell.model.verifications.iter().find(|c| c.id == id)?.clone();
    let role = c.session.and_then(|s| shell.model.members.iter().find(|m| m.session == s)).map_or("", |m| m.role.as_str());
    let color = shell.palette.member(look(role, &c.member).1);
    let status = progress(&c);
    let status_color = match c.phase {
        Phase::Filled => p.accent,
        Phase::NoMail | Phase::Failed => p.danger(),
        _ => p.text2,
    };
    let mut actions = h_flex().gap_2().items_center().flex_wrap();
    if c.phase.open() {
        if c.mailbox.is_some() {
            actions = actions.child(ui::button(p, ("verify-fill", id), Kind::Primary, t!("verify.fill")).icon(Icon::Check).on_click(cx.listener(move |this, _, _, cx| this.fill_verification(id, cx))));
            if c.open_url.is_some() {
                actions = actions.child(ui::button(p, ("verify-open", id), Kind::Secondary, t!("verify.open_email")).icon(Icon::External).on_click(cx.listener(move |this, _, _, cx| this.open_verification_email(id, cx))));
            }
        } else {
            actions = actions.child(ui::button(p, ("verify-connect", id), Kind::Primary, t!("verify.connect")).icon(Icon::Plug).on_click(cx.listener(|this, _, _, cx| this.open_mailbox_connect(cx))));
        }
    }
    if c.phase != Phase::Filled {
        actions = actions.child(ui::button(p, ("verify-type", id), Kind::Ghost, t!("verify.type_it")).icon(Icon::Keyboard).on_click(cx.listener(move |this, _, _, cx| this.type_verification(id, cx))));
    }
    Some(
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(icon(Icon::Shield, 12., p.accent))
                    .child(div().text_color(p.accent).child(t!("verify.mark")))
                    .child(div().text_color(color).child(c.member.clone()))
                    .children(c.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}")))),
            )
            .child(div().id(("verify-title", id)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title(&c)))
            .when(c.phase != Phase::Filled, |col| col.child(step_list(p, steps(&c))))
            .children(status.map(|s| div().id(("verify-status", id)).text_size(px(13.)).line_height(px(18.)).text_color(status_color).child(s)))
            .child(actions)
            .child(div().text_size(px(SMALL)).line_height(px(18.)).text_color(p.muted).child(t!("verify.never_member", name = c.member)))
            .into_any_element(),
    )
}

/// The first verification card, over the composer, in the conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let id = shell.model.verifications.first()?.id;
    let p = shell.palette;
    let body = card(shell, id, cx)?;
    Some(motion::rise(div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(body), ("verify-in", id)).into_any_element())
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

/// How the connected mailbox is named in the row.
pub fn mailbox_words(status: &equipe_core::protocol::vault::AutopilotStatus) -> String {
    match (&status.gmail, &status.mailbox) {
        (Some(g), _) => t!("verify.mailbox_gmail", address = g.address).to_string(),
        (None, Some(m)) => t!("verify.mailbox_imap", user = m.username, host = m.host).to_string(),
        (None, None) => t!("verify.mailbox_none").to_string(),
    }
}

/// Logins › Connections: "Mailbox for verification codes".
pub fn mailbox_row(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let p = shell.palette;
    let status = &shell.model.autopilot;
    let vu = &shell.verify;
    let connected = status.gmail.is_some() || status.mailbox.is_some();
    let mut actions = h_flex().flex_none().gap_2().items_center();
    if !connected && vu.client_id.is_none() {
        let builtin = status.gmail_client_builtin;
        actions = actions
            .child(ui::button(p, "verify-connect-gmail", Kind::Primary, t!("verify.connect_gmail")).icon(Icon::Plug).on_click(cx.listener(move |this, _, window, cx| {
                if builtin {
                    this.connect_gmail(cx);
                } else {
                    this.open_gmail_client_form(window, cx);
                }
            })))
            .child(ui::button(p, "verify-use-imap", Kind::Ghost, t!("verify.use_imap")).on_click(cx.listener(|this, _, _, cx| {
                this.logins_ui.autopilot.mailbox_open = true;
                cx.notify();
            })));
    }
    if connected {
        if vu.confirm_disconnect {
            actions = actions
                .child(ui::button(p, "verify-disconnect-no", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.verify.confirm_disconnect = false;
                    cx.notify();
                })))
                .child(ui::button(p, "verify-disconnect-yes", Kind::Danger, t!("verify.disconnect_yes")).on_click(cx.listener(|this, _, _, cx| this.disconnect_mailbox(cx))));
        } else {
            actions = actions.child(ui::button(p, "verify-disconnect", Kind::Ghost, t!("verify.disconnect")).icon(Icon::Trash).on_click(cx.listener(|this, _, _, cx| {
                this.verify.confirm_disconnect = true;
                cx.notify();
            })));
        }
    }
    let chip = |text: String, tone: Rgba| div().flex_none().h(px(22.)).px_2().flex().items_center().rounded_full().bg(p.wash).text_size(px(12.)).text_color(tone).whitespace_nowrap().child(text);
    let mut chips = h_flex().gap_2().flex_wrap().items_center().child(chip(mailbox_words(status), if connected { p.text2 } else { p.muted }));
    if connected {
        chips = chips.child(chip(t!("verify.readonly").to_string(), p.text2));
    }
    if vu.connecting && !connected {
        chips = chips.child(chip(t!("verify.connecting").to_string(), p.text2));
    }
    // A build without its own Google client: the person's own, in two boxes.
    let form = match (&vu.client_id, &vu.client_secret) {
        (Some(id_box), Some(secret_box)) if !connected => Some(
            v_flex()
                .w_full()
                .gap_2()
                .px_4()
                .pb_3()
                .pl(px(16. + 32. + 12.))
                .child(div().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("verify.client_steps")))
                .child(h_flex().child(ui::button(p, "verify-open-cloud", Kind::Secondary, t!("verify.open_cloud")).icon(Icon::External).on_click(cx.listener(|_, _, _, cx| {
                    if !cfg!(feature = "snapshot") {
                        cx.open_url(CLOUD_CREDENTIALS);
                    }
                }))))
                .child(h_flex().w_full().gap_2().child(div().flex_1().min_w_0().child(boxed(p, id_box))).child(div().flex_1().min_w_0().child(boxed(p, secret_box))))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(ui::button(p, "verify-client-cancel", Kind::Ghost, t!("accounts.cancel")).on_click(cx.listener(|this, _, _, cx| this.close_gmail_client_form(cx))))
                        .child(ui::button(p, "verify-client-connect", Kind::Primary, t!("verify.connect_gmail")).on_click(cx.listener(|this, _, _, cx| this.connect_gmail(cx)))),
                )
                .into_any_element(),
        ),
        _ => None,
    };
    v_flex()
        .id("conn-row-mailbox")
        .w_full()
        .child(
            h_flex()
                .w_full()
                .px_4()
                .py_3()
                .gap_3()
                .items_center()
                .flex_wrap()
                .child(div().flex_none().size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).bg(p.wash).child(icon(Icon::Inbox, 14., p.text2)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(ui::WORDS_AT_LEAST))
                        .gap_0p5()
                        .child(div().text_size(px(SMALL)).font_weight(FontWeight::MEDIUM).text_color(p.text).child(t!("verify.mailbox_title")))
                        .child(div().text_size(px(12.)).line_height(px(17.)).text_color(p.muted).child(t!("verify.mailbox_reads")))
                        .child(h_flex().w_full().pt_1().child(chips)),
                )
                .child(actions),
        )
        .children(vu.confirm_disconnect.then(|| div().w_full().px_4().pb_3().pl(px(16. + 32. + 12.)).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("verify.disconnect_note"))))
        .children(vu.connect_error.clone().filter(|_| !connected).map(|why| div().id("verify-connect-error").w_full().px_4().pb_3().pl(px(16. + 32. + 12.)).text_size(px(12.)).line_height(px(18.)).text_color(p.danger()).child(why)))
        .children(form)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring gpui's own `test` attribute along.
    use super::{progress, steps, title};
    use equipe_core::protocol::verify::{Channel, MailboxRef, Phase, Verification};

    fn card(phase: Phase, mailbox: bool) -> Verification {
        Verification {
            id: 1,
            session: Some(3),
            member: "backend".into(),
            ticket: Some(12),
            site: "github.com".into(),
            name: "GitHub".into(),
            channel: Channel::Email,
            mailbox: mailbox.then(|| MailboxRef { address: "squad@example.com".into(), kind: "gmail".into() }),
            open_url: None,
            lane: "acme/backend".into(),
            page: Some(4),
            phase,
            detail: None,
            at: 0,
        }
    }

    // The words are read in Korean, as every test in this app reads them.
    #[test]
    fn the_title_names_the_site_and_the_mailbox_the_code_went_to() {
        let t = title(&card(Phase::Waiting, true));
        assert!(t.contains("GitHub") && t.contains("squad@example.com"), "{t}");
        let bare = title(&card(Phase::Waiting, false));
        assert!(bare.contains("GitHub") && !bare.contains('@'), "{bare}");
    }

    #[test]
    fn the_steps_say_what_happens_and_without_a_mailbox_how_to_connect_one() {
        let with = steps(&card(Phase::Waiting, true));
        assert_eq!(with.len(), 2);
        assert!(with[1].contains("backend"), "{with:?}");
        assert_ne!(steps(&card(Phase::Waiting, false)), with);
    }

    #[test]
    fn where_it_stands_reads_each_phase_and_never_a_code() {
        assert_eq!(progress(&card(Phase::Waiting, true)), None);
        for phase in [Phase::Fetching, Phase::Filled, Phase::NoMail, Phase::Failed] {
            let line = progress(&card(phase, true)).unwrap();
            // A card has no code to show; no line reads like one either.
            let longest_digits = line.split(|c: char| !c.is_ascii_digit()).map(str::len).max().unwrap_or(0);
            assert!(!line.is_empty() && longest_digits < 4, "{phase:?}: {line}");
        }
        let mut refused = card(Phase::Failed, true);
        refused.detail = Some("not_taken".into());
        assert!(progress(&refused).unwrap().contains("GitHub"));
    }
}
