//! The desktop's own setup, as the person sees it: a strip above the screen
//! while the daemon installs the accessibility engine, when it couldn't, and —
//! the one thing macOS keeps for the person — when "equipe Desktop Helper"
//! needs its Accessibility switch. The daemon does everything else by itself
//! (`equipe_core::desktop::auto`); this only says what it says, once, with the
//! button that opens System Settings at the right pane (the daemon's machine,
//! even over SSH) and the one that puts the card away.

use equipe_core::protocol::{Command, DesktopAutoAction, DesktopFailure, DesktopSetup};
use equipe_core::t;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
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
        DesktopSetup::Installing => Some(
            strip("desktop-setup-installing")
                .child(div().flex_none().child(Spinner::new().with_size(px(14.)).color(Hsla::from(p.muted))))
                .child(div().flex_1().min_w_0().truncate().text_color(p.text).child(t!("desktop_setup.installing")))
                .into_any_element(),
        ),
        DesktopSetup::Failed { reason, .. } => Some(
            strip("desktop-setup-failed")
                .child(div().flex_none().child(icon(Icon::Warning, ICON_SM, p.amber)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(t!("desktop_setup.failed_title")))
                        .child(div().truncate().text_color(p.muted).child(failure_words(*reason))),
                )
                .child(div().flex_none().child(ui::button(p, "desktop-setup-retry", Kind::Secondary, t!("desktop_setup.retry")).icon(Icon::Retry).disabled(!connected).on_click(cx.listener(|this, _, _, cx| {
                    this.send_command(Command::DesktopAuto { action: DesktopAutoAction::Retry });
                    cx.notify();
                }))))
                .child(dismiss("desktop-setup-failed-close", cx))
                .into_any_element(),
        ),
        DesktopSetup::Grant => Some(
            strip("desktop-setup-grant")
                .child(div().flex_none().child(icon(Icon::Shield, ICON_SM, p.amber)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(t!("desktop_setup.grant_title")))
                        .child(div().truncate().text_color(p.muted).child(t!("desktop_setup.grant_body"))),
                )
                .child(div().flex_none().child(ui::button(p, "desktop-setup-open", Kind::Secondary, t!("desktop_setup.grant_button")).icon(Icon::Shield).disabled(!connected).on_click(cx.listener(|this, _, _, cx| {
                    // macOS's prompt again if it wants asking, and System Settings
                    // at Accessibility, on the daemon's machine.
                    this.send_command(Command::DesktopCheck { guide: true });
                    cx.notify();
                }))))
                .child(dismiss("desktop-setup-grant-close", cx))
                .into_any_element(),
        ),
        DesktopSetup::Idle | DesktopSetup::JustReady => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
