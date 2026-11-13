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
#[cfg(target_os = "macos")]
mod mac;
pub mod prompts;
#[cfg(windows)]
mod windows;
pub mod tools;

/// Set (to anything) in the environment of an old helper bundle the app
/// handed over to ([`serve_as_app`]).
pub const FALLBACK_ENV: &str = "EQUIPE_HELPER_HANDED_OVER";

/// The prompt-budget identity of the app's own Accessibility prompt, asked
/// once when it takes over from an old helper bundle.
pub const APP_PROMPT: &str = "equipe.app";

/// Run the helper as equipe.app — the app's executable started by the
/// LaunchAgent as `desktop serve`, so the one "equipe" permission covers it.
///
/// Moving from the old helper bundle: while the app isn't allowed
/// Accessibility yet and the old bundle is there, macOS's prompt for the app
/// is shown once (so the person sees the right row) and the helper hands over
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
            if prompts::recorded(home, prompts::Kind::Accessibility).is_none_or(|r| r.identity != APP_PROMPT)
                && let Some(_asking) = prompts::begin(prompts::Kind::Accessibility)
            {
                prompts::record(home, prompts::Kind::Accessibility, APP_PROMPT, prompts::Answer::Asked, now);
                mac::ask_accessibility();
            }
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
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = home;
        Err(std::io::Error::other("desktop control is macOS only"))
    }
}
