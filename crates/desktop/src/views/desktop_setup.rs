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
//! | Locked screen | the Mac is locked and a member's click or typing was refused for it (`DesktopReadiness::lock_card`) | another Mac: "Unlock it" opens Screen Sharing (`vnc://<its address>`; the person types the password there), or, if its port 5900 refuses, how to turn Screen Sharing on there once; this Mac: a note, no button | the daemon sees the screen unlocked; or "Not now" until then (`unlock.rs`) |
//!
//! Never a card, because they put themselves right: installing, a restart,
//! switching hosts, an update in progress, a lock no member waits on, and the old
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
use crate::target::Endpoint;
use crate::theme::{ICON_SM, SMALL};
use crate::unlock;
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
            // Two lines (a title and what to do) need room: at least one
            // row tall, growing with its words, never squeezed.
            .min_h(px(38.))
            .py(px(7.))
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
                ready.lock_card = false;
            }
            this.send_command(Command::DesktopAuto { action: DesktopAutoAction::Dismiss });
            cx.notify();
        })))
    };
    // A locked Mac a member waits on: the daemon says so (`lock_card`), and only
    // while no other card is up. It goes the moment the daemon sees the screen unlocked.
    if matches!(ready.setup, DesktopSetup::Idle)
        && let Some(card) = unlock::card(ready.lock_card, shell.endpoint.is_remote(), shell.unlock.sharing)
    {
        let name = shell.mac_name();
        let words = lock_words(card, &name);
        let url = shell.unlock.url(&shell.mac_address());
        let steps = t!("desktop_setup.sharing_steps", host = name.as_str()).to_owned();
        let unlock_button = words.can_unlock.then(|| {
            div().flex_none().child(ui::button(p, "desktop-setup-unlock", Kind::Secondary, t!("desktop_setup.locked_button")).icon(Icon::Lock).disabled(!connected).on_click(cx.listener(move |_, _, _, cx| {
                // Screen Sharing, on this Mac, to the daemon's: the person types the password there.
                if !cfg!(feature = "snapshot") {
                    cx.open_url(&url);
                }
            })))
        });
        let copy_button = words.can_copy.then(|| {
            div().flex_none().child(ui::button(p, "desktop-setup-copy-steps", Kind::Secondary, t!("desktop_setup.sharing_copy")).icon(Icon::Copy).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(steps.clone()))))
        });
        let check_button = words.can_copy.then(|| {
            div().flex_none().child(ui::button(p, "desktop-setup-check", Kind::Secondary, t!("desktop_setup.sharing_check")).icon(Icon::Retry).disabled(shell.unlock.sharing == unlock::Sharing::Checking).on_click(cx.listener(|this, _, _, cx| this.check_sharing(cx))))
        });
        return Some(
            strip("desktop-setup-locked")
                .child(div().flex_none().child(icon(Icon::Lock, ICON_SM, p.amber)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_color(p.text).font_weight(FontWeight::SEMIBOLD).child(words.title))
                        .child(div().truncate().text_color(p.muted).child(words.body)),
                )
                .children(unlock_button)
                .children(copy_button)
                .children(check_button)
                .child(dismiss("desktop-setup-locked-close", cx))
                .into_any_element(),
        );
    }
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

/// The locked-screen card's words, and which buttons it has.
pub struct LockWords {
    pub title: String,
    pub body: String,
    /// "Unlock it": opens Screen Sharing on this Mac to the daemon's.
    pub can_unlock: bool,
    /// "Copy steps" and "Check again": Screen Sharing is off there.
    pub can_copy: bool,
}

/// What the card says for `card`, naming the daemon's Mac (`name`) when it isn't this one.
pub fn lock_words(card: unlock::Card, name: &str) -> LockWords {
    match card {
        // On this Mac: nothing to open, the person unlocks it where they sit.
        unlock::Card::ThisMac => LockWords { title: t!("desktop_setup.locked_title").to_owned(), body: t!("desktop_setup.locked_body").to_owned(), can_unlock: false, can_copy: false },
        unlock::Card::Unlock => LockWords { title: t!("desktop_setup.locked_title_far", host = name).to_owned(), body: t!("desktop_setup.locked_body_far").to_owned(), can_unlock: true, can_copy: false },
        unlock::Card::TurnOnSharing => LockWords { title: t!("desktop_setup.sharing_title_far", host = name).to_owned(), body: t!("desktop_setup.sharing_body_far", host = name).to_owned(), can_unlock: false, can_copy: true },
    }
}

impl Shell {
    /// What the daemon's Mac is called here: its Tailscale device's name when
    /// known ("Tinum's Mac Mini"), else the address as it was typed.
    pub fn mac_name(&self) -> String {
        match &self.endpoint {
            Endpoint::Ssh(host) => crate::tailnet::peer_for(&self.connect.peers, &host.host).map(|peer| peer.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| host.host.clone()),
            Endpoint::Local { .. } => self.endpoint.label(),
        }
    }

    /// The daemon's Mac's address as typed (Screen Sharing opens the one `ssh` resolved it to, once known).
    pub fn mac_address(&self) -> String {
        match &self.endpoint {
            Endpoint::Ssh(host) => host.host.clone(),
            Endpoint::Local { .. } => String::new(),
        }
    }

    /// The daemon's word about the locked-screen card, as it came in: while it
    /// stands, look once at whether Screen Sharing answers on the daemon's Mac
    /// (another Mac only: a plain connection to port 5900, no login); once it
    /// is gone, forget what was found, so the next lock looks afresh.
    pub fn on_lock_card(&mut self, lock_card: bool, cx: &mut Context<Self>) {
        if !lock_card {
            self.unlock.reset();
            return;
        }
        if self.endpoint.is_remote() && self.unlock.address.is_none() && self.unlock.task.is_none() {
            self.check_sharing(cx);
        }
    }

    /// Look at the daemon's Mac: its address as `ssh` resolves it, and whether
    /// it takes Screen Sharing connections. Off the UI thread; the card is
    /// redrawn with the answer.
    pub fn check_sharing(&mut self, cx: &mut Context<Self>) {
        let Endpoint::Ssh(host) = self.endpoint.clone() else { return };
        if cfg!(feature = "snapshot") {
            return;
        }
        self.unlock.sharing = unlock::Sharing::Checking;
        let need_peers = self.connect.peers.is_empty();
        self.unlock.task = Some(cx.spawn(async move |this, cx| {
            let (address, sharing, peers) = cx
                .background_executor()
                .spawn(async move {
                    let address = unlock::address(&host);
                    let sharing = unlock::sharing(&address);
                    // The device's own name for the card, when Tailscale knows it.
                    let peers = if need_peers { crate::tailnet::load() } else { Vec::new() };
                    (address, sharing, peers)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.unlock.address = Some(address);
                this.unlock.sharing = sharing;
                this.unlock.task = None;
                if !peers.is_empty() {
                    this.connect.peers = peers;
                }
                cx.notify();
            });
        }));
        cx.notify();
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

    #[core::prelude::v1::test]
    fn the_lock_card_says_whose_mac_and_has_only_the_button_that_does_something_there() {
        use unlock::Card;
        let here = lock_words(Card::ThisMac, "");
        assert!(!here.can_unlock && !here.can_copy, "nothing to open for the Mac in front of the person");
        // Whatever language the test runs in: the Mac's name where there is one, no placeholder left, no key shown.
        let said = |w: &LockWords| format!("{} {}", w.title, w.body);
        assert!(!said(&here).contains('{') && !said(&here).contains("desktop_setup."), "{}", said(&here));
        let far = lock_words(Card::Unlock, "Tinum's Mac Mini");
        assert!(far.can_unlock && !far.can_copy);
        assert!(far.title.contains("Tinum's Mac Mini") && !said(&far).contains('{'), "{}", said(&far));
        assert_ne!(far.title, here.title, "another Mac is named; this one isn't");
        let off = lock_words(Card::TurnOnSharing, "Tinum's Mac Mini");
        assert!(!off.can_unlock && off.can_copy, "no button that would open a refused connection: it says how to turn it on there");
        assert!(off.title.contains("Tinum's Mac Mini") && off.body.contains("Tinum's Mac Mini") && !said(&off).contains('{'), "{}", said(&off));
        // The steps that "Copy steps" puts on the clipboard name the Mac.
        let steps = t!("desktop_setup.sharing_steps", host = "Tinum's Mac Mini");
        assert!(steps.contains("Tinum's Mac Mini") && steps.lines().count() >= 4 && !steps.contains('{'), "{steps}");
    }

    #[core::prelude::v1::test]
    fn the_lock_card_is_up_only_while_the_daemon_says_so_and_goes_with_its_word() {
        let mut ready = equipe_core::protocol::DesktopReadiness { supported: true, locked: true, lock_card: true, ..Default::default() };
        // Remote daemon: the card with its button, whatever the port said until it refuses.
        assert_eq!(unlock::card(ready.lock_card, true, unlock::Sharing::Unknown), Some(unlock::Card::Unlock));
        assert_eq!(unlock::card(ready.lock_card, true, unlock::Sharing::Off), Some(unlock::Card::TurnOnSharing));
        // The daemon on this Mac: the note only.
        assert_eq!(unlock::card(ready.lock_card, false, unlock::Sharing::Off), Some(unlock::Card::ThisMac));
        // The daemon sees the screen unlocked, or a member no longer waits, or the person said not now: its word goes and the card with it.
        ready.lock_card = false;
        for remote in [false, true] {
            assert_eq!(unlock::card(ready.lock_card, remote, unlock::Sharing::On), None);
        }
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
