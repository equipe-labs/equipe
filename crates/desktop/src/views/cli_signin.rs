//! A member's ask for a command-line sign-in (`cli_sign_in`): ONE card in the
//! conversation, over the composer — "Sign in to AWS (profile atinum)", what
//! it is for, a button that opens the sign-in page on the screen the person is
//! looking at, the code to confirm (a device code) or a box for the code the
//! page gives back, and where it stands: waiting for them, checking, signed in,
//! or expired with "Get a new link". The daemon runs the tool on its own
//! Mac; what is pasted here goes straight to the waiting process and is
//! cleared from the box at once — never into a conversation, never to a
//! member.

use std::collections::{HashMap, HashSet};

use equipe_core::protocol::Command;
use equipe_core::protocol::cli_signin::{CliSignIn, PastedCode, Phase};
use equipe_core::t;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::motion;
use crate::theme::{CONTROL, MONO, R_LG, R_MD, SMALL, look};
use crate::ui::{self, Kind};
use crate::{Screen, Shell};

/// The cards' boxes for the code to paste back, by card.
#[derive(Default)]
pub struct CliSignInUi {
    inputs: HashMap<u64, Entity<InputState>>,
    subscriptions: HashMap<u64, Subscription>,
    /// Cards whose link was copied (shown "Copied").
    copied: HashSet<u64>,
}

impl CliSignInUi {
    /// Whether a box has the keyboard: what is typed is the box's.
    pub fn typing(&self, window: &Window, cx: &App) -> bool {
        self.inputs.values().any(|f| f.read(cx).focus_handle(cx).is_focused(window))
    }
}

/// The card's title: "Sign in to AWS", or "… (profile atinum)".
pub fn title(card: &CliSignIn) -> String {
    match card.profile.as_deref() {
        Some(profile) => t!("cli_signin.title_profile", name = card.name, profile = profile).to_string(),
        None => t!("cli_signin.title", name = card.name).to_string(),
    }
}

/// What the card says about where the sign-in runs and what happens to a
/// pasted code. Remote: the daemon is on another machine, so the page opens
/// here — on the Mac in front of the person — not there.
pub fn where_note(remote: bool, machine: &str, wants_code: bool) -> String {
    match (remote, wants_code) {
        (false, false) => t!("cli_signin.note_local").to_string(),
        (false, true) => t!("cli_signin.note_local_code").to_string(),
        (true, false) => t!("cli_signin.note_remote", machine = machine).to_string(),
        (true, true) => t!("cli_signin.note_remote_code", machine = machine).to_string(),
    }
}

/// The line that says where it stands.
pub fn progress(card: &CliSignIn) -> String {
    match card.phase {
        Phase::Starting => t!("cli_signin.starting").to_string(),
        Phase::Waiting if card.wants_code => t!("cli_signin.waiting_code").to_string(),
        Phase::Waiting => t!("cli_signin.waiting").to_string(),
        Phase::Checking => t!("cli_signin.checking").to_string(),
        Phase::SignedIn => match card.who.as_deref() {
            Some(who) => t!("cli_signin.signed_in_as", who = who).to_string(),
            None => t!("cli_signin.signed_in").to_string(),
        },
        Phase::Expired => t!("cli_signin.expired").to_string(),
        Phase::Failed => match card.detail.as_deref() {
            Some(why) => t!("cli_signin.failed_why", why = why).to_string(),
            None => t!("cli_signin.failed").to_string(),
        },
    }
}

impl Shell {
    /// A box for each card that wants a code pasted back; those of cards gone, dropped.
    pub fn cli_signin_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cards = self.model.cli_sign_ins.clone();
        self.cli_signin.inputs.retain(|id, _| cards.iter().any(|c| c.id == *id));
        self.cli_signin.subscriptions.retain(|id, _| cards.iter().any(|c| c.id == *id));
        self.cli_signin.copied.retain(|id| cards.iter().any(|c| c.id == *id));
        for card in cards.iter().filter(|c| c.wants_code) {
            if self.cli_signin.inputs.contains_key(&card.id) {
                continue;
            }
            // Hidden: nobody beside the screen needs to read it.
            let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("cli_signin.paste_placeholder")));
            let id = card.id;
            let subscription = cx.subscribe_in(&input, window, move |shell, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    shell.submit_cli_signin_code(id, window, cx);
                }
            });
            self.cli_signin.inputs.insert(card.id, input);
            self.cli_signin.subscriptions.insert(card.id, subscription);
        }
    }

    /// The code pasted goes to the daemon — and out of the box at once.
    pub fn submit_cli_signin_code(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.cli_signin.inputs.get(&id).cloned() else { return };
        let code = input.read(cx).value().to_string();
        if code.trim().is_empty() {
            input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        let squad = self.model.squad.clone();
        self.send_command(Command::SubmitCliSignInCode { squad, id, code: PastedCode(code) });
        if let Some(card) = self.model.cli_sign_ins.iter_mut().find(|c| c.id == id) {
            card.phase = Phase::Checking;
        }
        self.model.revision += 1;
        cx.notify();
    }

    pub fn restart_cli_signin(&mut self, id: u64, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::RestartCliSignIn { squad, id });
        if let Some(card) = self.model.cli_sign_ins.iter_mut().find(|c| c.id == id) {
            card.phase = Phase::Starting;
            card.url = None;
            card.user_code = None;
        }
        self.model.revision += 1;
        cx.notify();
    }

    pub fn cancel_cli_signin(&mut self, id: u64, cx: &mut Context<Self>) {
        let squad = self.model.squad.clone();
        self.send_command(Command::CancelCliSignIn { squad, id });
        self.model.cli_sign_ins.retain(|c| c.id != id);
        self.model.revision += 1;
        cx.notify();
    }

    /// The sign-in page, in this Mac's browser: the Mac the person is looking at.
    pub fn open_cli_signin_page(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(url) = self.model.cli_sign_ins.iter().find(|c| c.id == id).and_then(|c| c.url.clone()) else { return };
        if !cfg!(feature = "snapshot") {
            cx.open_url(&url);
        }
        // The code's box is next.
        cx.notify();
    }

    pub fn copy_cli_signin_link(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(url) = self.model.cli_sign_ins.iter().find(|c| c.id == id).and_then(|c| c.url.clone()) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.cli_signin.copied.insert(id);
        cx.notify();
    }
}

/// The card for sign-in `id`.
pub fn card(shell: &Shell, id: u64, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let p = shell.palette;
    let c = shell.model.cli_sign_ins.iter().find(|c| c.id == id)?.clone();
    let role = shell.model.members.iter().find(|m| m.session == c.session).map_or("", |m| m.role.as_str());
    let (_, hue) = look(role, &c.member);
    let color = shell.palette.member(hue);
    let done = c.phase == Phase::SignedIn;
    let remote = shell.endpoint.is_remote();
    let machine = shell.endpoint.label();

    let status_color = match c.phase {
        Phase::SignedIn => p.accent,
        Phase::Expired | Phase::Failed => p.muted,
        _ => p.text2,
    };
    let mut body = v_flex().gap_2();
    // The page and the code to confirm: once the tool printed them.
    if matches!(c.phase, Phase::Waiting | Phase::Checking) {
        if let Some(code) = c.user_code.clone() {
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(div().text_size(px(12.)).text_color(p.muted).child(t!("cli_signin.confirm_code")))
                    .child(div().id(("cli-code", id)).font_family(MONO).text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(code)),
            );
        }
        if c.wants_code
            && let Some(input) = shell.cli_signin.inputs.get(&id).cloned()
        {
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(div().text_size(px(12.)).text_color(p.muted).child(t!("cli_signin.paste_label")))
                    .child(
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
                            .child(Input::new(&input).appearance(false).bordered(false)),
                    ),
            );
        }
    }
    let mut actions = h_flex().gap_2().items_center();
    if matches!(c.phase, Phase::Waiting | Phase::Checking) && c.url.is_some() {
        actions = actions.child(ui::button(p, ("cli-open", id), Kind::Primary, t!("cli_signin.open")).icon(Icon::External).on_click(cx.listener(move |this, _, _, cx| this.open_cli_signin_page(id, cx))));
        let copied = shell.cli_signin.copied.contains(&id);
        actions = actions.child(ui::button(p, ("cli-copy", id), Kind::Ghost, if copied { t!("cli_signin.copied") } else { t!("cli_signin.copy") }).on_click(cx.listener(move |this, _, _, cx| this.copy_cli_signin_link(id, cx))));
        if c.wants_code && c.phase == Phase::Waiting {
            actions = actions.child(ui::button(p, ("cli-submit", id), Kind::Secondary, t!("cli_signin.submit")).key("⏎").on_click(cx.listener(move |this, _, window, cx| this.submit_cli_signin_code(id, window, cx))));
        }
    }
    if matches!(c.phase, Phase::Expired | Phase::Failed) {
        actions = actions.child(ui::button(p, ("cli-new", id), Kind::Primary, t!("cli_signin.new_link")).on_click(cx.listener(move |this, _, _, cx| this.restart_cli_signin(id, cx))));
    }
    if !done {
        actions = actions.child(ui::button(p, ("cli-later", id), Kind::Ghost, t!("cli_signin.not_now")).on_click(cx.listener(move |this, _, _, cx| this.cancel_cli_signin(id, cx))));
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
                    .child(div().text_color(p.accent).child(t!("cli_signin.mark")))
                    .child(div().text_color(color).child(c.member.clone()))
                    .children(c.ticket.map(|t| div().text_color(p.muted).child(format!("T-{t}")))),
            )
            .child(div().id(("cli-title", id)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title(&c)))
            .child(div().text_size(px(13.)).text_color(p.text2).child(t!("cli_signin.for", why = c.why.clone())))
            .child(body)
            .child(div().id(("cli-status", id)).text_size(px(13.)).text_color(status_color).child(progress(&c)))
            .child(actions)
            .child(div().text_size(px(SMALL)).line_height(px(18.)).text_color(p.muted).child(where_note(remote, &machine, c.wants_code)))
            .into_any_element(),
    )
}

/// The first sign-in card, over the composer, in the conversation.
pub fn docked(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if shell.screen != Screen::Chat {
        return None;
    }
    let id = shell.model.cli_sign_ins.first()?.id;
    let p = shell.palette;
    let body = card(shell, id, cx)?;
    Some(motion::rise(div().w_full().mb_3().p_3().rounded(px(R_LG)).bg(p.raised).border_1().border_color(p.hair).shadow_sm().child(body), ("cli-signin-in", id)).into_any_element())
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring gpui's own `test` attribute along.
    use super::{progress, title, where_note};
    use equipe_core::protocol::cli_signin::{CliSignIn, Phase};

    fn card(phase: Phase) -> CliSignIn {
        CliSignIn {
            id: 1,
            session: 3,
            member: "devops".into(),
            ticket: Some(12),
            tool: "aws".into(),
            name: "AWS".into(),
            profile: Some("atinum".into()),
            why: "deploy the staging stack".into(),
            url: Some("https://signin.aws.amazon.com/v1/authorize?x=1".into()),
            user_code: None,
            wants_code: true,
            phase,
            who: None,
            detail: None,
            at: 0,
        }
    }

    // The words are read in Korean, as every test in this app reads them.
    #[test]
    fn the_title_names_the_tool_and_its_profile() {
        assert_eq!(title(&card(Phase::Waiting)), "AWS 로그인 (프로필 atinum)");
        let mut plain = card(Phase::Waiting);
        plain.profile = None;
        assert_eq!(title(&plain), "AWS 로그인");
    }

    #[test]
    fn a_remote_daemons_card_says_the_page_opens_here_and_a_local_ones_does_not_name_a_machine() {
        let local = where_note(false, "mac-mini", true);
        let remote = where_note(true, "mac-mini", true);
        assert!(!local.contains("mac-mini") && local.contains("이 Mac에서 도구의"), "{local}");
        assert!(remote.contains("mac-mini") && remote.contains("지금 쓰고 있는 이 Mac에서 열려요"), "{remote}");
        assert_ne!(where_note(false, "m", false), where_note(false, "m", true), "a pasted code is explained where the flow has one");
        assert_ne!(where_note(true, "m", false), where_note(false, "m", false));
    }

    #[test]
    fn progress_reads_waiting_checking_signed_in_and_expired() {
        assert!(progress(&card(Phase::Waiting)).contains("기다리는 중"));
        assert!(progress(&card(Phase::Checking)).contains("확인하는 중"));
        let mut done = card(Phase::SignedIn);
        done.who = Some("arn:aws:iam::1:user/atinum-admin".into());
        assert!(progress(&done).contains("✓") && progress(&done).contains("atinum-admin"));
        assert!(progress(&card(Phase::Expired)).contains("만료"));
        let mut failed = card(Phase::Failed);
        failed.detail = Some("Invalid code".into());
        assert!(progress(&failed).contains("Invalid code"));
    }
}
