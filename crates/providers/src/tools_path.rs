//! Where the tools members run are found. An app opened from Finder, and the
//! daemon it starts, get launchd's bare PATH (`/usr/bin:/bin:/usr/sbin:/sbin`):
//! `claude` in `~/.local/bin` or `codex` from Homebrew would never be found,
//! even once installed. So equipe widens its own PATH once, before anything
//! runs: what it was given first, then what the person's login shell sets,
//! then the folders installers put tools in — those even before they exist, so
//! a tool installed while the daemon runs is found without a restart.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Set once widened, so a child equipe doesn't ask the shell again.
const WIDENED: &str = "EQUIPE_PATH_WIDENED";
/// How long the login shell may take to say its PATH.
const SHELL_WAIT: Duration = Duration::from_secs(3);

/// The folders installers put tools in: Claude Code's and Codex's own
/// installers (`~/.local/bin`), Homebrew's, cargo's.
pub fn usual_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".cargo/bin"),
    ]
}

/// `path` with `more`'s folders after it, each folder once, in order.
pub fn widened(path: Option<&OsStr>, more: &[PathBuf]) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in path.into_iter().flat_map(std::env::split_paths).chain(more.iter().cloned()) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| path.map(OsStr::to_os_string).unwrap_or_default())
}

/// The PATH the person's login shell sets, when it says within `SHELL_WAIT`.
fn login_shell_path() -> Option<OsString> {
    use std::io::Read as _;
    let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "printf '%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + SHELL_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let out = out.trim();
    (!out.is_empty()).then(|| out.into())
}

/// Widen this process's PATH (see the module). `ask_shell`: also take the
/// login shell's, for what runs long — the daemon, its service. Call it first
/// thing in `main`, before any thread starts: it sets the environment.
pub fn widen(ask_shell: bool) {
    let Some(home) = dirs::home_dir() else { return };
    let mut more = Vec::new();
    if ask_shell && std::env::var_os(WIDENED).is_none() {
        if let Some(shell) = login_shell_path() {
            more.extend(std::env::split_paths(&shell));
        }
    }
    more.extend(usual_dirs(&home));
    let path = widened(std::env::var_os("PATH").as_deref(), &more);
    // SAFETY: called from `main` before any other thread exists, so nothing
    // reads the environment while it changes.
    unsafe {
        std::env::set_var("PATH", path);
        if ask_shell {
            std::env::set_var(WIDENED, "1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_finder_path_gains_where_installers_put_tools() {
        let home = Path::new("/Users/someone");
        let path = widened(Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")), &usual_dirs(home));
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[..4], [PathBuf::from("/usr/bin"), "/bin".into(), "/usr/sbin".into(), "/sbin".into()], "what it was given comes first");
        assert!(dirs.contains(&home.join(".local/bin")), "Claude Code's and Codex's installers: {dirs:?}");
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")), "Homebrew: {dirs:?}");
    }

    #[test]
    fn each_folder_is_there_once_and_empty_ones_not_at_all() {
        let more = [PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/x/bin"), PathBuf::from("/x/bin")];
        let path = widened(Some(OsStr::new("/opt/homebrew/bin::/usr/bin")), &more);
        assert_eq!(path, OsString::from("/opt/homebrew/bin:/usr/bin:/x/bin"));
        assert_eq!(widened(None, &[PathBuf::from("/a")]), OsString::from("/a"));
    }
}
