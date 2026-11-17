//! What the real-Chrome tools need from the Mac beyond the accessibility
//! engine: Chrome's profile list, opening a window in a profile, closing one
//! window. Behind a trait so every rule is tested with a fake, and nothing
//! in the tests touches the person's Chrome.

use std::path::PathBuf;

/// Google Chrome's bundle id.
pub const BUNDLE: &str = "com.google.Chrome";

/// The Mac, as the real-Chrome tools use it.
pub trait ChromeOs: Send + Sync {
    /// The text of Chrome's `Local State` (its profile list).
    fn local_state(&self) -> Result<String, String>;
    /// Open `url` in a new window of the person's Chrome, in profile `dir`.
    fn open(&self, dir: &str, url: &str) -> Result<(), String>;
    /// Close Chrome's window whose window number is `window` (its tabs too).
    fn close(&self, window: u32) -> Result<(), String>;
    fn pause(&self, ms: u64);
}

/// `open`'s arguments for a new window of profile `dir` at `url`.
///
/// `-n` starts a launcher instance even when Chrome is running, so the
/// arguments reach it; Chrome's single-instance check hands them to the
/// running browser (same user data, every sign-in there) and the launcher
/// exits. `-g`: don't bring the launcher forward. No automation switch, no
/// debugging port: this is the person's ordinary Chrome.
pub fn open_args(dir: &str, url: &str) -> Vec<String> {
    vec![
        "-n".into(),
        "-g".into(),
        "-b".into(),
        BUNDLE.into(),
        "--args".into(),
        format!("--profile-directory={dir}"),
        "--new-window".into(),
        url.into(),
    ]
}

/// The real Mac.
pub struct System {
    pub user_home: PathBuf,
}

impl System {
    pub fn new() -> Self {
        Self { user_home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default() }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromeOs for System {
    fn local_state(&self) -> Result<String, String> {
        if !cfg!(target_os = "macos") {
            return Err(UNSUPPORTED.into());
        }
        std::fs::read_to_string(super::profiles::local_state_path(&self.user_home))
            .map_err(|_| "Google Chrome isn't set up on this Mac (no profile list): ask the person to install Chrome and open it once.".to_owned())
    }

    fn open(&self, dir: &str, url: &str) -> Result<(), String> {
        if !cfg!(target_os = "macos") {
            return Err(UNSUPPORTED.into());
        }
        let status = std::process::Command::new("/usr/bin/open")
            .args(open_args(dir, url))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("Couldn't start Chrome: {e}"))?;
        if status.success() { Ok(()) } else { Err("macOS couldn't open Google Chrome: is it installed (in Applications)?".into()) }
    }

    fn close(&self, window: u32) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        return crate::mac::close_window(BUNDLE, window);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            Err(UNSUPPORTED.into())
        }
    }

    fn pause(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

const UNSUPPORTED: &str = "Working the person's own Chrome is macOS-only for now.";

#[cfg(test)]
mod tests {
    #[test]
    fn a_window_opens_in_the_profile_with_no_automation() {
        let args = super::open_args("Profile 1", "https://example.com/a?b=c");
        assert_eq!(args, ["-n", "-g", "-b", "com.google.Chrome", "--args", "--profile-directory=Profile 1", "--new-window", "https://example.com/a?b=c"]);
        assert!(!args.iter().any(|a| a.contains("remote-debugging") || a.contains("enable-automation") || a.contains("user-data-dir")));
    }
}
