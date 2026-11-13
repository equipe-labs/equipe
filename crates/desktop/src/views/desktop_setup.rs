//! The desktop's own setup, as the person sees it: a strip above the screen
//! only when work is blocked and the person can unblock it with one action.
//!
//! What can show here, and nothing else (the daemon decides, in
//! `equipe_core::desktop::auto`; this only says what it says):
//!
//! | card | shows when | the person's one action | goes away for good |
//! |---|---|---|---|
//! | Grant | a member's desktop call was refused for want of Accessibility (or the first set-up, never skipped), and the switch is still off | on the daemon's Mac: "Open System Settings" (a click, so macOS may prompt once); on another Mac: do it there (no button here) | the switch is seen on; or "Not now": kept per Mac and code identity in `<home>/desktop/setup.json`, back only after the switch was on and was taken back while a member needs it |
//! | Failed | the engine or helper couldn't be set up and a member needs it (or it has failed for `FAILURE_GRACE`) | "Try again" | the next try works; or "Not now" for that reason, kept until a different reason |
//!
//! Never a card, because they put themselves right: installing, a restart,
//! switching hosts, an update in progress, a locked screen, and the old
//! "equipe Desktop Helper" (its stale Accessibility row is harmless).
//! System prompts follow the same rule (docs/PROMPTS.md): only the person's
//! click asks macOS anything.
//!
//! A card about the daemon's Mac says which Mac when that isn't this one.

use equipe_core::protocol::{Command, DesktopAutoAction, DesktopFailure, DesktopSetup};
use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::icons::{Icon, icon};
use crate::theme::{ICON_SM, SMALL};
use crate::ui::{self, Kind};
use crate::Shell;

/// What a failure says, in the person's language: what happened and what
/// equipe does about it.
pub fn failure_words(reason: DesktopFailure) -> &'static str {
    match reason {
        DesktopFailure::Download => t!("desktop_setup.failed_download"),
        DesktopFailure::Checksum => t!("desktop_setup.failed_checksum"),
        DesktopFailure::Helper => t!("desktop_setup.failed_helper"),
        DesktopFailure::Session => t!("desktop_setup.failed_session"),
        DesktopFailure::Other => t!("desktop_setup.failed_other"),
    }
}

pub fn banner(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let ready = shell.model.desktop_ready.as_ref()?;
    let p = shell.palette;
    let connected = matches!(shell.model.conn, crate::state::Conn::Connected { .. });
    let (remote, host) = (shell.model.remote, shell.model.host.clone());
    let strip = |id: &'static str| {
        h_flex()
            .id(id)
            .h(px(38.))
            .flex_none()
            .pl(px(shell.header_inset(16., cx)))
            .pr_3()
            .gap_2p5()
            .items_center()
            .bg(Rgba { a: if p.is_dark() { 0.13 } else { 0.10 }, ..p.amber })
            .border_b_1()
            .border_color(p.hair)
            .text_size(px(SMALL))
            .window_control_area(WindowControlArea::Drag)
    };
    let dismiss = |id: &'static str, cx: &mut Context<Shell>| {
        div().flex_none().child(ui::icon_button(p, id, Icon::X, t!("desktop_setup.dismiss"), None).on_click(cx.listener(|this, _, _, cx| {
            // Gone at once; the daemon keeps it gone until a member needs the desktop.
            if let Some(ready) = this.model.desktop_ready.as_mut() {
                ready.setup = DesktopSetup::Idle;
            }
            this.send_command(Command::DesktopAuto { action: DesktopAutoAction::Dismiss });
            cx.notify();
        })))
    };
    match &ready.setup {
        // Installing resolves by itself: never a card.
        DesktopSetup::Installing => None,
        DesktopSetup::Failed { reason, .. } => Some(
            strip("desktop-setup-failed")
                .child(div().flex_none().child(icon(Icon::Warning, ICON_SM, p.amber)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(if remote { t!("desktop_setup.failed_title_far", host = host.as_str()).to_owned() } else { t!("desktop_setup.failed_title").to_owned() }))
                        .child(div().truncate().text_color(p.muted).child(failure_words(*reason))),
                )
                .child(div().flex_none().child(ui::button(p, "desktop-setup-retry", Kind::Secondary, t!("desktop_setup.retry")).icon(Icon::Retry).disabled(!connected).on_click(cx.listener(|this, _, _, cx| {
                    this.send_command(Command::DesktopAuto { action: DesktopAutoAction::Retry });
                    cx.notify();
                }))))
                .child(dismiss("desktop-setup-failed-close", cx))
                .into_any_element(),
        ),
        DesktopSetup::Grant => {
            let words = grant_words(helper_name(ready), remote, &host);
            let open = words.can_open.then(|| {
                div().flex_none().child(ui::button(p, "desktop-setup-open", Kind::Secondary, t!("desktop_setup.grant_button")).icon(Icon::Shield).disabled(!connected).on_click(cx.listener(|this, _, _, cx| {
                    // macOS's prompt again if it wants asking, and System Settings
                    // at Accessibility, on the daemon's machine (which is this one).
                    this.send_command(Command::DesktopCheck { guide: true });
                    cx.notify();
                })))
            });
            Some(
                strip("desktop-setup-grant")
                    .child(div().flex_none().child(icon(Icon::Shield, ICON_SM, p.amber)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(words.title))
                            .child(div().truncate().text_color(p.muted).child(words.body)),
                    )
                    .children(open)
                    .child(dismiss("desktop-setup-grant-close", cx))
                    .into_any_element(),
            )
        }
        DesktopSetup::Idle | DesktopSetup::JustReady => None,
    }
}

/// The grant card's words, and whether its button can do anything on this screen.
pub struct GrantWords {
    pub title: String,
    pub body: String,
    /// The daemon's Mac is the one in front of the person: "Open System
    /// Settings" opens it here. Else the card says where to do it and has no button.
    pub can_open: bool,
}

pub fn grant_words(name: &str, remote: bool, host: &str) -> GrantWords {
    if remote {
        GrantWords { title: t!("desktop_setup.grant_title_far", name = name, host = host).to_owned(), body: t!("desktop_setup.grant_body_far", host = host).to_owned(), can_open: false }
    } else {
        GrantWords { title: t!("desktop_setup.grant_title", name = name).to_owned(), body: t!("desktop_setup.grant_body").to_owned(), can_open: true }
    }
}

/// What System Settings lists the helper as.
fn helper_name(ready: &equipe_core::protocol::DesktopReadiness) -> &str {
    if ready.helper_name.is_empty() { "equipe" } else { &ready.helper_name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[core::prelude::v1::test]
    fn a_card_about_another_mac_names_it_and_has_no_button_that_opens_nothing_here() {
        let far = grant_words("equipe", true, "Tinum's Mac Mini");
        assert!(far.title.contains("Tinum's Mac Mini") && far.body.contains("Tinum's Mac Mini"), "{} / {}", far.title, far.body);
        assert!(!far.can_open, "System Settings would open on the other Mac");
        let near = grant_words("equipe", false, "");
        assert!(near.can_open && !near.title.contains("{host}") && near.title != far.title);
    }

    // `gpui_kit::*` brings a `test` macro of its own into scope.
    #[core::prelude::v1::test]
    fn every_failure_has_its_own_words_in_the_persons_language() {
        let words = [DesktopFailure::Download, DesktopFailure::Checksum, DesktopFailure::Helper, DesktopFailure::Session, DesktopFailure::Other].map(failure_words);
        for (i, a) in words.iter().enumerate() {
            assert!(!a.is_empty() && !a.starts_with("desktop_setup."));
            for b in &words[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
