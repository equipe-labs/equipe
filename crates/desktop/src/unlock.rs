//! Unlocking the daemon's Mac with the person's own Screen Sharing.
//!
//! When the Mac the daemon runs on is locked, members can read apps but
//! their clicks and typing wait. Only the person can unlock it, and equipe
//! never helps with the password: it opens macOS Screen Sharing on the Mac
//! the person is looking at (`vnc://<the daemon's address>`), and the person
//! types the password themselves in that window. Nothing here sees, stores
//! or sends a password, and nothing is ever typed at a lock screen.
//!
//! The daemon decides when the card shows (`DesktopReadiness::lock_card`:
//! the screen is locked, a member waits on it, the person hasn't put the
//! card away) and when it goes (it sees the screen unlocked). This module
//! decides what the card offers, from where the daemon is and whether the
//! Mac there takes Screen Sharing connections:
//!
//! | the daemon is | Screen Sharing (port 5900) | the card |
//! |---|---|---|
//! | on this Mac | n/a | "This Mac is locked", no button: unlock it at the keyboard |
//! | on another Mac | answers, or can't tell | "<Mac> is locked", button "Unlock it" |
//! | on another Mac | refuses the connection | how to turn it on there once, "Copy steps", "Check again" |

use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::target::SshHost;

/// Where macOS Screen Sharing (VNC) listens.
pub const SCREEN_SHARING_PORT: u16 = 5900;

/// How long a look at the port waits for an answer.
const WAIT: Duration = Duration::from_millis(2500);

/// Whether the daemon's Mac takes Screen Sharing connections, as a quick
/// connection (no login, nothing sent) found out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sharing {
    /// Not looked at yet, or it couldn't be told (no answer in time, an
    /// unresolvable name, a jump host): offered as if on, since Screen
    /// Sharing says what is wrong itself.
    #[default]
    Unknown,
    /// Looking now.
    Checking,
    /// The port took the connection.
    On,
    /// The connection was refused: Screen Sharing is off on that Mac.
    Off,
}

/// What the card offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    /// The daemon is on this Mac: unlock it at the keyboard. No button.
    ThisMac,
    /// Another Mac: open Screen Sharing on it.
    Unlock,
    /// Another Mac with Screen Sharing off: how to turn it on there, once.
    TurnOnSharing,
}

/// The card to show for `lock_card` (the daemon's word that a locked screen
/// holds a member up), the daemon being `remote` or not, and what the look at
/// port 5900 found; `None`: no card.
pub fn card(lock_card: bool, remote: bool, sharing: Sharing) -> Option<Card> {
    match (lock_card, remote, sharing) {
        (false, ..) => None,
        (true, false, _) => Some(Card::ThisMac),
        (true, true, Sharing::Off) => Some(Card::TurnOnSharing),
        (true, true, _) => Some(Card::Unlock),
    }
}

/// The address Screen Sharing opens: `vnc://host`, an IPv6 address in brackets.
/// Never a user or a password: the person types those in Screen Sharing.
pub fn vnc_url(address: &str) -> String {
    let address = address.trim();
    if address.contains(':') && !address.starts_with('[') { format!("vnc://[{address}]") } else { format!("vnc://{address}") }
}

/// Where `ssh` would connect for `host` (an alias of `~/.ssh/config` names an
/// address there; `ssh -G` reads the config and connects nowhere), else the
/// host as written. Blocks briefly: call it off the UI thread.
pub fn address(host: &SshHost) -> String {
    let mut args = vec!["-G".to_owned()];
    args.extend(host.args());
    let resolved = Command::new("ssh")
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| crate::ssh::Door::parse(&String::from_utf8_lossy(&out.stdout)))
        .map(|door| door.host);
    resolved.unwrap_or_else(|| host.host.clone())
}

/// Whether something takes a connection at `host:port`: a plain TCP connect,
/// closed again at once, nothing sent. [`Sharing::Off`] only when every
/// address refused it; no answer in time, or a name that doesn't resolve,
/// is [`Sharing::Unknown`]. Blocks up to a few seconds: call it off the UI thread.
pub fn sharing_at(host: &str, port: u16, wait: Duration) -> Sharing {
    let Ok(addrs) = (host, port).to_socket_addrs() else { return Sharing::Unknown };
    let (mut tried, mut refused) = (0, 0);
    for addr in addrs.take(4) {
        tried += 1;
        match TcpStream::connect_timeout(&addr, wait) {
            Ok(_) => return Sharing::On,
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => refused += 1,
            Err(_) => {}
        }
    }
    if tried > 0 && refused == tried { Sharing::Off } else { Sharing::Unknown }
}

/// [`sharing_at`] for Screen Sharing's own port.
pub fn sharing(host: &str) -> Sharing {
    sharing_at(host, SCREEN_SHARING_PORT, WAIT)
}

/// What the unlock card knows about the daemon's Mac, kept while it is up.
#[derive(Default)]
pub struct Ui {
    /// The Mac's address, once resolved (an alias of the SSH config, a name, an address).
    pub address: Option<String>,
    pub sharing: Sharing,
    /// The look in progress; dropped with the card.
    pub task: Option<gpui_kit::Task<()>>,
}

impl Ui {
    /// The card is gone (the screen was unlocked): the next lock looks afresh.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Where Screen Sharing should open: the resolved address, else `fallback` as written.
    pub fn url(&self, fallback: &str) -> String {
        vnc_url(self.address.as_deref().unwrap_or(fallback))
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[core::prelude::v1::test]
    fn the_card_follows_the_daemons_word_where_it_runs_and_whether_screen_sharing_answers() {
        for remote in [false, true] {
            for sharing in [Sharing::Unknown, Sharing::Checking, Sharing::On, Sharing::Off] {
                assert_eq!(card(false, remote, sharing), None, "the daemon didn't ask for it: no card, {remote} {sharing:?}");
            }
        }
        // The daemon is on this Mac: say so, no button (nothing to open for a Mac in front of the person).
        for sharing in [Sharing::Unknown, Sharing::On, Sharing::Off] {
            assert_eq!(card(true, false, sharing), Some(Card::ThisMac));
        }
        // Another Mac: the button, until the port is known to refuse.
        assert_eq!(card(true, true, Sharing::Unknown), Some(Card::Unlock));
        assert_eq!(card(true, true, Sharing::Checking), Some(Card::Unlock));
        assert_eq!(card(true, true, Sharing::On), Some(Card::Unlock));
        assert_eq!(card(true, true, Sharing::Off), Some(Card::TurnOnSharing));
    }

    #[core::prelude::v1::test]
    fn the_url_is_the_hosts_address_and_never_carries_a_user_or_a_password() {
        assert_eq!(vnc_url("tinums-mac-mini.tail3383cf.ts.net"), "vnc://tinums-mac-mini.tail3383cf.ts.net");
        assert_eq!(vnc_url("100.66.18.3"), "vnc://100.66.18.3");
        assert_eq!(vnc_url("fd7a::2"), "vnc://[fd7a::2]");
        assert_eq!(vnc_url("[fd7a::2]"), "vnc://[fd7a::2]");
        assert_eq!(vnc_url(" mini.local "), "vnc://mini.local");
        let ui = Ui { address: Some("100.66.18.3".into()), ..Ui::default() };
        assert_eq!(ui.url("alias"), "vnc://100.66.18.3", "the address ssh resolved");
        assert_eq!(Ui::default().url("alias"), "vnc://alias", "until it is resolved, as written");
        // An `ssh` destination with a user keeps only the host.
        let host = SshHost::parse("tinum@tinums-mac-mini:2222").unwrap();
        assert_eq!(vnc_url(&host.host), "vnc://tinums-mac-mini");
    }

    #[core::prelude::v1::test]
    fn a_port_that_takes_the_connection_is_on_and_one_that_refuses_it_is_off() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap().port();
        assert_eq!(sharing_at("127.0.0.1", open, Duration::from_secs(2)), Sharing::On);
        drop(listener);
        // Nothing listens there any more: the connection is refused at once.
        assert_eq!(sharing_at("127.0.0.1", open, Duration::from_secs(2)), Sharing::Off);
    }

    #[core::prelude::v1::test]
    fn a_name_that_doesnt_resolve_is_not_taken_for_screen_sharing_being_off() {
        assert_eq!(sharing_at("no-such-host.invalid", SCREEN_SHARING_PORT, Duration::from_millis(500)), Sharing::Unknown);
    }
}
