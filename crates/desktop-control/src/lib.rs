//! Members working desktop apps the way a person does: see the
//! screen, click, type, open apps (docs/DESKTOP-CONTROL.md).
//!
//! The permissions this takes — Screen Recording, Accessibility — belong to a
//! helper of its own, never to the daemon or the members: a grant attaches to
//! the process launchd started, and every child would inherit it. The helper is
//! an app bundle with a fixed bundle id and signature ([`install`]), run as a
//! LaunchAgent in the person's GUI session, listening on a socket only the
//! daemon can use ([`helper`]). A member calls `mcp__desktop__…`; the daemon
//! checks the member may, and hands the call on.
//!
//! Experimental: off for every member until the person turns it on, and
//! behind the screen lease (T-22) and the send approval (T-23) in the daemon.

pub mod controller;
pub mod geometry;
pub mod helper;
pub mod install;
pub mod keys;
#[cfg(target_os = "macos")]
mod mac;
pub mod tools;

/// Windows builds expose the feature without pretending it worked. The UI
/// Automation implementation is tracked separately; callers can show this
/// exact, actionable limitation instead of a macOS setup instruction.
pub const WINDOWS_UNAVAILABLE: &str =
    "Desktop control isn't available on Windows yet. Windows UI Automation support is planned; browser tools still work.";

/// Run the helper in this process until it ends: what the LaunchAgent starts
/// (`equipe desktop serve`).
pub fn serve(home: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    return mac::serve(home);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
        #[cfg(windows)]
        return Err(std::io::Error::other(WINDOWS_UNAVAILABLE));
        #[cfg(not(windows))]
        return Err(std::io::Error::other("desktop control is macOS only"));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    #[test]
    fn windows_explains_the_missing_uia_backend() {
        let error = super::serve(std::path::Path::new("C:\\Users\\tester\\.equipe")).unwrap_err();
        assert_eq!(error.to_string(), super::WINDOWS_UNAVAILABLE);
    }
}
