//! Where the tools members run are found. An app opened from Finder, and the
//! daemon it starts, get launchd's bare PATH (`/usr/bin:/bin:/usr/sbin:/sbin`):
//! `claude` in `~/.local/bin`, an npm global `codex.cmd` on Windows, or Codex
//! from Homebrew would never be found,
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
    #[cfg(not(windows))]
    return vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".cargo/bin"),
    ];
    #[cfg(windows)]
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".cargo/bin"),
    ];
    #[cfg(windows)]
    {
        // npm's per-user global prefix. Node's installer normally adds this
        // to PATH, but the daemon may have inherited an older service PATH.
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|p| !p.is_empty()) {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    }
    #[cfg(windows)]
    dirs
}

/// Candidate names Windows resolves for a command. Keep this explicit rather
/// than relying on `CreateProcess`'s `.exe`-only lookup: npm installs `.cmd`
/// launchers for both Claude Code and Codex.
fn command_names(binary: &OsStr) -> Vec<OsString> {
    let path = Path::new(binary);
    if path.extension().is_some() || !cfg!(windows) {
        return vec![binary.to_os_string()];
    }
    #[cfg(windows)]
    return ["exe", "cmd", "bat", "com"].into_iter().map(|ext| path.with_extension(ext).into_os_string()).collect();
    #[cfg(not(windows))]
    unreachable!()
}

/// Where a command is: an explicit path, or the first executable name on
/// PATH. On Windows this includes npm's `.cmd` launcher and native `.exe`.
pub fn find(binary: impl AsRef<OsStr>) -> Option<PathBuf> {
    find_in(binary.as_ref(), std::env::var_os("PATH").as_deref())
}

/// Resolve when possible, otherwise preserve the original name so spawning it
/// produces the operating system's ordinary NotFound error.
pub fn resolved(binary: impl AsRef<OsStr>) -> PathBuf {
    let binary = binary.as_ref();
    find(binary).unwrap_or_else(|| PathBuf::from(binary))
}

/// Start an installed tool without asking Rust to execute an npm `.cmd`
/// launcher directly. Recent Rust versions reject some perfectly valid CLI
/// arguments for batch files; npm's launcher is only a shim around Node, so
/// invoke the package entry point instead.
pub fn command(binary: impl AsRef<OsStr>) -> tokio::process::Command {
    let binary = resolved(binary);
    #[cfg(windows)]
    if binary.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd")) {
        let package = match binary.file_stem().and_then(OsStr::to_str) {
            Some("claude") => Some(Path::new("node_modules/@anthropic-ai/claude-code/cli.js")),
            Some("codex") => Some(Path::new("node_modules/@openai/codex/bin/codex.js")),
            _ => None,
        };
        if let Some(entry) = package.map(|entry| binary.parent().unwrap_or_else(|| Path::new("")).join(entry)).filter(|entry| entry.is_file())
            && let Some(node) = find("node")
        {
            let mut command = tokio::process::Command::new(node);
            command.arg(entry);
            return command;
        }
    }
    tokio::process::Command::new(binary)
}

fn find_in(binary: &OsStr, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path = Path::new(binary);
    let explicit = path.components().count() > 1 || path.is_absolute();
    let dirs: Vec<PathBuf> = if explicit {
        vec![path.parent().unwrap_or_else(|| Path::new("")).to_owned()]
    } else {
        path_env.map(|p| std::env::split_paths(p).collect()).unwrap_or_default()
    };
    let name = if explicit { path.file_name()? } else { binary };
    dirs.into_iter().flat_map(|dir| command_names(name).into_iter().map(move |name| dir.join(name))).find(|p| p.is_file())
}

/// Git for Windows' Bash, used by features whose command language is POSIX
/// shell rather than PowerShell. `CLAUDE_CODE_GIT_BASH_PATH` also supports a
/// portable Git installation, matching Claude Code itself.
pub fn git_bash() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(given) = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").filter(|p| !p.is_empty()) {
            let path = PathBuf::from(given);
            if path.is_file() {
                return Some(path);
            }
        }
        let mut roots = Vec::new();
        for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(key).filter(|p| !p.is_empty()) {
                roots.push(PathBuf::from(root));
            }
        }
        roots
            .into_iter()
            .flat_map(|root| [root.join("Git/bin/bash.exe"), root.join("Programs/Git/bin/bash.exe")])
            .find(|p| p.is_file())
            .or_else(|| find("bash"))
    }
    #[cfg(not(windows))]
    {
        find("bash")
    }
}

/// Require a POSIX shell on Windows without quietly feeding Bash syntax to
/// PowerShell. Callers that can express their work in PowerShell should do so
/// instead; this is only for commands whose language is already Bash.
pub fn require_git_bash() -> Result<PathBuf, String> {
    git_bash().ok_or_else(|| {
        "Git Bash is required for this command on Windows. Install Git for Windows from https://git-scm.com/download/win, then restart equipe.".into()
    })
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
    if ask_shell
        && std::env::var_os(WIDENED).is_none()
        && let Some(shell) = login_shell_path()
    {
        more.extend(std::env::split_paths(&shell));
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
    #[cfg(unix)]
    fn a_bare_finder_path_gains_where_installers_put_tools() {
        let home = Path::new("/Users/someone");
        let path = widened(Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")), &usual_dirs(home));
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[..4], [PathBuf::from("/usr/bin"), "/bin".into(), "/usr/sbin".into(), "/sbin".into()], "what it was given comes first");
        assert!(dirs.contains(&home.join(".local/bin")), "Claude Code's and Codex's installers: {dirs:?}");
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")), "Homebrew: {dirs:?}");
    }

    #[test]
    #[cfg(unix)]
    fn each_folder_is_there_once_and_empty_ones_not_at_all() {
        let more = [PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/x/bin"), PathBuf::from("/x/bin")];
        let path = widened(Some(OsStr::new("/opt/homebrew/bin::/usr/bin")), &more);
        assert_eq!(path, OsString::from("/opt/homebrew/bin:/usr/bin:/x/bin"));
        assert_eq!(widened(None, &[PathBuf::from("/a")]), OsString::from("/a"));
    }

    #[test]
    fn a_command_is_found_on_path() {
        let here = std::env::current_exe().unwrap();
        assert_eq!(find(&here), Some(here));
    }

    #[test]
    fn a_missing_bash_has_an_install_instruction() {
        if git_bash().is_none() {
            let why = require_git_bash().unwrap_err();
            assert!(why.contains("Git for Windows") && why.contains("git-scm.com"), "{why}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn npm_cmd_launchers_are_found_on_windows() {
        let dir = std::env::temp_dir().join(format!("equipe-tools-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("equipe-fake-engine.cmd");
        std::fs::write(&tool, "@echo off\r\nexit /b 0\r\n").unwrap();
        let path = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(find_in(OsStr::new("equipe-fake-engine"), Some(&path)), Some(tool));
        let _ = std::fs::remove_dir_all(dir);
    }
}
