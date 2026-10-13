//! The Mac's screen, shared by the members: one at a time (`lease`), the person
//! first (`presence`), never while locked or logged out (`mac`).
//!
//! Work in progress (T-22): wired into the daemon once the desktop tools (T-35)
//! exist to ask for it.

pub mod lease;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod presence;

/// The person's GUI session, as macOS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Logged in at the console, unlocked: members may use it.
    Open,
    /// The screen is locked; members can't unlock it.
    Locked,
    /// Nobody logged in at the console (or FileVault after a reboot).
    NoSession,
}
