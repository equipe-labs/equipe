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
pub mod engine;
pub mod geometry;
pub mod helper;
pub mod install;
pub mod keys;
pub mod linux;
#[cfg(target_os = "macos")]
mod mac;
pub mod prompts;
pub mod system_guard;
#[cfg(windows)]
mod windows;
pub mod tools;

/// Set (to anything) in the environment of an old helper bundle the app
/// handed over to ([`serve_as_app`]).
pub const FALLBACK_ENV: &str = "EQUIPE_HELPER_HANDED_OVER";

/// The prompt-budget identity of the app's own Accessibility prompt.
pub const APP_PROMPT: &str = "equipe.app";

/// Run the helper as equipe.app — the app's executable started by the
/// LaunchAgent as `desktop serve`, so the one "equipe" permission covers it.
///
/// Moving from the old helper bundle: while the app isn't allowed
/// Accessibility yet and the old bundle is there, the helper hands over
/// to the old bundle (`exec`, in the same process launchd started, so macOS
/// judges it by the old bundle's grant) — members keep working. The daemon
/// starts the app again now and then to see whether it is allowed by then.
pub fn serve_as_app(home: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        if mac::trusted() {
            prompts::observe(home, prompts::Kind::Accessibility, APP_PROMPT, true, now);
        } else if let Some(old) = install::bundle_executable() {
            // No prompt from here: this runs in the background (the daemon
            // starts the app again now and then). Only the person's click on
            // the Accessibility card asks macOS anything.
            use std::os::unix::process::CommandExt as _;
            let err = std::process::Command::new(&old).args(["desktop", "serve", "--home"]).arg(home).env(FALLBACK_ENV, "1").exec();
            eprintln!("equipe desktop helper: couldn't hand over to {}: {err}", old.display());
        }
    }
    serve(home)
}

/// Run the helper in this process until it ends: what the LaunchAgent starts
/// (`equipe desktop serve`).
pub fn serve(home: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    return mac::serve(home);
    #[cfg(windows)]
    return windows::serve(home);
    #[cfg(target_os = "linux")]
    return linux::serve(home);
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = home;
        Err(std::io::Error::other("desktop control runs on macOS, Windows and Linux only"))
    }
}
