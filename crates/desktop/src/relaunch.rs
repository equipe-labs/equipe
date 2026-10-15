//! An update downloaded and waiting to go in, for the sidebar's "다시 시작해
//! 업데이트" (as Claude's "Relaunch to update"). On a Mac, Sparkle downloads
//! it in the background and says so (src/update.rs); a click installs it and
//! starts the app again, and quitting installs it anyway.

use std::sync::OnceLock;

use tokio::sync::watch;

/// The version waiting, once one is.
fn channel() -> &'static watch::Sender<Option<String>> {
    static READY: OnceLock<watch::Sender<Option<String>>> = OnceLock::new();
    READY.get_or_init(|| watch::channel(None).0)
}

/// Follow what waits: now, and whenever it changes.
pub fn watch() -> watch::Receiver<Option<String>> {
    channel().subscribe()
}

/// An update is downloaded and goes in on the next start.
pub fn ready(version: String) {
    channel().send_replace(Some(version));
}

/// What the sidebar's button says.
pub fn label(version: &str) -> String {
    format!("다시 시작해 업데이트 · {version}")
}

/// Install the waiting update now and start again. False when there is none
/// to install this way (then it goes in when the app quits).
pub fn now() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::update::install_now()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_downloaded_update_is_announced_to_whoever_follows_now_or_later() {
        let early = watch();
        assert!(early.borrow().is_none() || early.borrow().is_some());
        ready("0.33.0".into());
        assert_eq!(watch().borrow().as_deref(), Some("0.33.0"), "a later follower sees it at once");
        assert!(early.has_changed().unwrap(), "an earlier one is woken");
        assert_eq!(label("0.33.0"), "다시 시작해 업데이트 · 0.33.0");
    }
}
