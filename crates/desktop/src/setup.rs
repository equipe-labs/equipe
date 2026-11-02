//! The first run on this Mac: no squad yet, so the window can't open settings,
//! and the engines (Claude Code, Codex) it needs are not there or not signed
//! in. Everything the terminal would have done is done from here — the
//! install, the sign-ins (each opens the person's browser itself), and the
//! first squad (its form is `views/welcome.rs`) — so nobody has to open a
//! terminal. The rules are here; the screens are `views/setup.rs` (the AI) and
//! `views/welcome.rs` (the squad).
//!
//! Before the AI, git: a new Mac has only Apple's stub until its developer
//! tools are installed, and the squad keeps its work in git. Missing, the
//! screen asks macOS to install them and watches until they're there.
//!
//! No daemon session exists yet (a session needs a squad), so this talks to
//! the tools directly: `claude auth status`, `codex login status`,
//! `equipe login codex`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use equipe_providers::{claude_code, codex_app, credentials::CredentialStore};

/// How often the window looks again while macOS installs its developer tools.
pub const GIT_POLL: std::time::Duration = std::time::Duration::from_secs(3);

/// What Anthropic's own installer line is; the screen shows it before it runs.
pub const CLAUDE_INSTALL: &str = "curl -fsSL https://claude.ai/install.sh | bash";
/// Claude Code's setup guide, for when the install doesn't go through.
pub const CLAUDE_GUIDE: &str = "https://code.claude.com/docs/en/setup";

/// Where Claude Code stands on this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claude {
    Missing,
    SignedOut,
    SignedIn,
}

/// What the window is doing for the person right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Doing {
    #[default]
    Idle,
    Installing,
    ClaudeLogin,
    CodexLogin,
}

#[derive(Default)]
pub struct Setup {
    /// `None` while the first look is under way.
    pub claude: Option<Claude>,
    pub codex: Option<bool>,
    pub probed: bool,
    pub doing: Doing,
    /// "Install" pressed: the line it would run is shown, awaiting a yes.
    pub confirm_install: bool,
    /// What stopped, and the tool's last words.
    pub failed: Option<(Doing, String)>,
    /// The child a sign-in waits on, to cancel it.
    pub pid: Arc<AtomicU32>,
    pub cancelled: bool,
    /// "Next" pressed with an engine ready: the squad form is up.
    pub squad_step: bool,
    /// Whether git runs here; `None` until looked at.
    pub git: Option<bool>,
    /// macOS was asked to install its developer tools: the window watches.
    pub git_asked: bool,
    /// macOS couldn't be asked.
    pub git_failed: bool,
    pub git_watch: Option<gpui_kit::Task<()>>,
}

impl Setup {
    /// Whether an engine is ready, so the first squad can start.
    pub fn ready(&self) -> bool {
        self.claude == Some(Claude::SignedIn) || self.codex == Some(true)
    }

    /// Whether "Next" opens the squad form: git runs and an engine is ready.
    pub fn can_go_on(&self) -> bool {
        self.git == Some(true) && self.ready()
    }

    pub fn cancel(&mut self) {
        let pid = self.pid.swap(0, Ordering::SeqCst);
        self.cancelled = true;
        #[cfg(unix)]
        if pid != 0 {
            // SAFETY: a plain signal to the child this window started.
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
}

/// `claude` on `path` (a PATH value) or where its installer and Homebrew put
/// it: opened from Finder the app has no shell PATH.
pub fn find_claude(home: &Path, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let on_path = path.into_iter().flat_map(std::env::split_paths);
    let usual = [home.join(".local/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), home.join(".claude/local")];
    on_path.chain(usual).map(|dir| dir.join("claude")).find(|bin| bin.is_file())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// A PATH a child of this app can use: the usual folders in front.
fn child_path() -> String {
    let home = home();
    format!("{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{}", home.display(), std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()))
}

/// Whether git runs here. Snapshots draw a new Mac's screen on any Mac
/// (`FAKE_NO_GIT=1`).
pub fn git_ready() -> bool {
    let no_git = cfg!(feature = "snapshot") && std::env::var("FAKE_NO_GIT").is_ok_and(|v| v == "1");
    !no_git && crate::install::git_ready(std::env::var_os("PATH").as_deref())
}

/// Where each engine stands. Blocks: call it off the UI thread.
pub fn probe() -> (Claude, bool) {
    let claude = match find_claude(&home(), std::env::var_os("PATH").as_deref()) {
        None => Claude::Missing,
        Some(bin) => {
            let asked = tokio::runtime::Builder::new_current_thread().enable_all().build().ok().and_then(|rt| rt.block_on(claude_code::auth_status(&bin.to_string_lossy())).ok().flatten());
            match asked {
                Some(status) if status.logged_in => Claude::SignedIn,
                _ => Claude::SignedOut,
            }
        }
    };
    // equipe's own ChatGPT login, or the person's Codex CLI signed in.
    let own = matches!(CredentialStore::default_location().load("openai"), Ok(Some(_)));
    let cli = codex_app::find(codex_app::DEFAULT_BINARY).or_else(|| Some(home().join(".local/bin/codex")).filter(|p| p.is_file())).and_then(|bin| {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        rt.block_on(codex_app::login_status(&bin.to_string_lossy())).ok().flatten()
    });
    (claude, own || cli.is_some_and(|s| s.signed_in()))
}

/// The tool's last words when it fails, trimmed to something a card can hold.
pub fn last_words(output: &str) -> String {
    let lines: Vec<&str> = output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(3)..];
    let text = tail.join("\n");
    if text.chars().count() > 300 { text.chars().take(300).collect::<String>() + "…" } else { text }
}

/// Run `command` to its end, off the UI thread's business: `pid` holds the
/// child while it runs, so "취소" can end it. `Err` carries its last words.
fn run(mut command: Command, pid: &AtomicU32) -> Result<(), String> {
    command.env("PATH", child_path()).env("EQUIPE_NO_OPEN", "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().map_err(|e| e.to_string())?;
    pid.store(child.id(), Ordering::SeqCst);
    let out = child.wait_with_output().map_err(|e| e.to_string());
    pid.store(0, Ordering::SeqCst);
    let out = out?;
    if out.status.success() {
        return Ok(());
    }
    let said = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Err(last_words(&said))
}

/// What `doing` runs.
pub fn job(doing: Doing, pid: &AtomicU32) -> Result<(), String> {
    let equipe = || crate::install::find_cli(&home(), std::env::var_os("PATH").as_deref()).ok_or_else(|| "equipe".to_owned());
    match doing {
        Doing::Installing => {
            let mut c = Command::new("sh");
            c.arg("-c").arg(CLAUDE_INSTALL);
            run(c, pid)
        }
        Doing::ClaudeLogin => {
            let bin = find_claude(&home(), std::env::var_os("PATH").as_deref()).ok_or_else(|| "claude".to_owned())?;
            let mut c = Command::new(bin);
            c.args(["auth", "login"]);
            run(c, pid)
        }
        Doing::CodexLogin => {
            let mut c = Command::new(equipe()?);
            // The browser opens by itself; a login nobody finishes ends after ten minutes.
            c.args(["login", "codex"]).env_remove("EQUIPE_NO_OPEN");
            run(c, pid)
        }
        Doing::Idle => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_is_found_where_its_installer_puts_it() {
        let home = std::env::temp_dir().join(format!("equipe-setup-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        assert_eq!(find_claude(&home, Some("/nonexistent".as_ref())), None);
        std::fs::write(home.join(".local/bin/claude"), "").unwrap();
        assert_eq!(find_claude(&home, Some("/nonexistent".as_ref())), Some(home.join(".local/bin/claude")));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn last_words_keeps_the_end() {
        assert_eq!(last_words("a\n\nb\nc\nd\n"), "b\nc\nd");
        assert_eq!(last_words(""), "");
    }

    #[test]
    fn next_waits_for_git_and_an_engine() {
        let mut s = Setup { codex: Some(true), ..Setup::default() };
        assert!(!s.can_go_on(), "git not looked at yet");
        s.git = Some(false);
        assert!(!s.can_go_on(), "a new Mac's stub");
        s.git = Some(true);
        assert!(s.can_go_on());
        s.codex = Some(false);
        assert!(!s.can_go_on());
    }

    #[test]
    fn engines_ready_when_either_is() {
        let mut s = Setup::default();
        assert!(!s.ready());
        s.codex = Some(true);
        assert!(s.ready());
        s.codex = Some(false);
        s.claude = Some(Claude::SignedOut);
        assert!(!s.ready());
    }
}
