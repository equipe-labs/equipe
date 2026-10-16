//! Members working the Mac's desktop apps the way a person does: see the
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
//! without the screen lease (T-22) or the send approval (T-23) yet.

pub mod controller;
pub mod geometry;
pub mod helper;
pub mod install;
pub mod keys;
#[cfg(target_os = "macos")]
mod mac;
pub mod tools;

/// Run the helper in this process until it ends: what the LaunchAgent starts
/// (`equipe desktop serve`).
pub fn serve(home: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    return mac::serve(home);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
        Err(std::io::Error::other("desktop control is macOS only"))
    }
}
