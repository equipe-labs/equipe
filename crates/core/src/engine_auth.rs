//! When a member's turn fails because its engine's sign-in no longer works
//! here: why, looked at by equipe itself, and the fix that is equipe's.
//!
//! Each step is mechanical, so it is code, not a model: the tool's own
//! status (`claude auth status`, `codex login status`), whether the daemon
//! runs inside the Mac's login session (the Security framework's own word
//! for the session, `SessionGetInfo`), and whether the login Keychain is
//! locked there (`SecKeychainGetStatus`: a lock's state, never an item, and
//! never a prompt). Nothing here reads, prints, copies or passes on a
//! credential, types a password or unlocks the Keychain: what only the
//! person may do — approving a sign-in, unlocking — stays theirs, and the
//! card says how.
//!
//! Started over SSH, a daemon runs in that connection's session, which the
//! login Keychain is closed to: Claude Code then finds no sign-in (`claude
//! auth status` says signed out there, while the Mac's own Terminal says
//! signed in) and its turns fail with "OAuth session expired". Signing in
//! again from there would only put a second login in a file; the fix is the
//! daemon back in the login session, which its login service does
//! ([`relaunch_script`]).
//!
//! `claude auth login` needs no terminal: it opens the browser on this
//! machine (whose return it catches on `localhost`) and prints a page to open
//! anywhere instead, whose code it reads on its standard input
//! ([`claude_login`]). So a screen on another Mac signs it in with a link and
//! a paste, and the login lands where the daemon's members read it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use equipe_protocol::{AuthState, Engine, EngineState, EngineStatus, MemberRef, SignInFailure, SignInStep};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// The hint a Claude turn that lost its sign-in leaves ([`equipe_protocol::Event::Error`]'s
/// `next`): what an older screen shows as it is.
pub const CLAUDE_HINT: &str = "claude auth login — in Terminal on this Mac's own screen (Screen Sharing is fine), not over SSH";
/// The one a Codex turn leaves.
pub const CODEX_HINT: &str = "equipe login codex";

/// The engine whose sign-in a failed turn's hint says was lost.
pub fn lost_by(next: Option<&str>) -> Option<Engine> {
    let next = next?.trim();
    if next.starts_with("claude auth login") || next == "equipe login claude" {
        Some(Engine::Claude)
    } else if next == CODEX_HINT {
        Some(Engine::Codex)
    } else {
        None
    }
}

/// The Mac's login Keychain, as the session the daemon runs in sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keychain {
    Unlocked,
    Locked,
    /// Not a Mac, or the Keychain couldn't be asked.
    Unknown,
}

/// Where the daemon runs: inside the Mac's login session (with its screen),
/// or outside it (an SSH connection's). `None`: not a Mac, or it couldn't tell.
/// `EQUIPE_LOGIN_SESSION=inside|outside` says so instead (tests, the fake daemon).
pub fn in_login_session() -> Option<bool> {
    match std::env::var("EQUIPE_LOGIN_SESSION").as_deref() {
        Ok("inside") => return Some(true),
        Ok("outside") => return Some(false),
        _ => {}
    }
    mac::session_attributes().map(|attrs| attrs & mac::SESSION_HAS_GRAPHIC_ACCESS != 0)
}

/// Whether the login Keychain at `home`'s is locked, as this session sees
/// it. `EQUIPE_KEYCHAIN=locked|unlocked` says so instead (tests).
pub fn keychain(home: &Path) -> Keychain {
    match std::env::var("EQUIPE_KEYCHAIN").as_deref() {
        Ok("locked") => return Keychain::Locked,
        Ok("unlocked") => return Keychain::Unlocked,
        _ => {}
    }
    let path = home.join("Library/Keychains/login.keychain-db");
    match mac::keychain_status(&path) {
        Some(status) if status & mac::UNLOCKED != 0 => Keychain::Unlocked,
        Some(_) => Keychain::Locked,
        None => Keychain::Unknown,
    }
}

/// Why Claude Code can't serve members here, from its own status, the
/// session and the Keychain: the session first, since outside it the
/// Keychain always looks locked and the sign-in always missing.
pub fn claude_verdict(status: &EngineStatus, in_login: Option<bool>, keychain: Keychain, movable: bool) -> (AuthState, String) {
    match status.state {
        EngineState::NotInstalled => (AuthState::NotInstalled, String::new()),
        EngineState::SignedIn => (AuthState::SignedIn { resumed: 0 }, String::new()),
        _ if in_login == Some(false) => (AuthState::OutsideLogin { movable }, String::new()),
        _ if keychain == Keychain::Locked => (AuthState::KeychainLocked, String::new()),
        EngineState::SignedOut => (AuthState::SignedOut, String::new()),
        EngineState::Unknown => (AuthState::Unknown, status.detail.clone().unwrap_or_default()),
    }
}

/// Codex keeps its sign-in in a file of its own: its status is all there is.
pub fn codex_verdict(status: &EngineStatus) -> (AuthState, String) {
    match status.state {
        EngineState::NotInstalled => (AuthState::NotInstalled, String::new()),
        EngineState::SignedIn => (AuthState::SignedIn { resumed: 0 }, String::new()),
        EngineState::SignedOut => (AuthState::SignedOut, String::new()),
        EngineState::Unknown => (AuthState::Unknown, status.detail.clone().unwrap_or_default()),
    }
}

/// Look at why `engine` can't serve members here. Off the core loop: each
/// engine's own tool answers, and the Keychain is asked on the blocking pool.
pub async fn check(engine: Engine, claude_binary: &str, codex_binary: &str, store: &equipe_providers::CredentialStore) -> (AuthState, String) {
    match engine {
        Engine::Codex => codex_verdict(&crate::engines::codex(codex_binary, store).await),
        Engine::Claude => {
            let status = crate::engines::claude(claude_binary).await;
            let home = user_home().unwrap_or_default();
            let (in_login, keychain) = tokio::task::spawn_blocking(move || (in_login_session(), keychain(&home))).await.unwrap_or((None, Keychain::Unknown));
            claude_verdict(&status, in_login, keychain, service_plist().is_some())
        }
    }
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// The daemon's login service, when it is installed (`equipe daemon install`).
pub fn service_plist() -> Option<PathBuf> {
    let file = user_home()?.join("Library/LaunchAgents").join(format!("{SERVICE}.plist"));
    (cfg!(target_os = "macos") && file.is_file()).then_some(file)
}

/// The login service's name (as `equipe daemon install` writes it).
pub const SERVICE: &str = "im.equipe.daemon";

/// What starts the daemon again inside the login session: once this one
/// (`pid`) has stopped as `equipe daemon stop` asks — its work journaled, as
/// at any restart — `equipe daemon start` has its login service run it in
/// the person's graphical session: started again in place, or loaded afresh
/// when launchd refuses it, a few tries each (launchd may still be letting
/// go of an older one). It never starts one on its own beside the service:
/// that one would be outside again, as this one is.
pub fn relaunch_script(exe: &Path, pid: u32) -> String {
    let exe = crate::design::flows::shell_quote(&exe.to_string_lossy());
    format!(
        "sleep 1\n{exe} daemon stop >/dev/null 2>&1\n\
         n=0; while kill -0 {pid} 2>/dev/null && [ $n -lt 150 ]; do sleep 0.2; n=$((n+1)); done\n\
         exec {exe} daemon start >/dev/null 2>&1\n"
    )
}

/// How this daemon runs, as it sees itself ([`equipe_protocol::rpc::Runs`]).
/// The Keychain is asked for its lock's state only — never an item, never a
/// prompt; the caller asks off the async workers.
pub fn runs_here() -> equipe_protocol::rpc::Runs {
    let env = |name: &str| std::env::var(name).ok();
    let home = user_home().unwrap_or_default();
    equipe_protocol::rpc::Runs {
        pid: std::process::id(),
        ssh: ssh_bound(&env),
        service: by_service(&env, std::process::id()),
        login_session: in_login_session(),
        keychain: match keychain(&home) {
            Keychain::Unlocked => Some(true),
            Keychain::Locked => Some(false),
            Keychain::Unknown => None,
        },
    }
}

/// Whether an environment is an SSH connection's: what a process started
/// from one inherits. (`SSH_AUTH_SOCK` alone is launchd's own agent.)
pub fn ssh_bound(env: &dyn Fn(&str) -> Option<String>) -> bool {
    ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"].iter().any(|name| env(name).is_some_and(|v| !v.is_empty()))
}

/// Whether process `pid` was started by its login service: launchd names
/// its job in `XPC_SERVICE_NAME`, systemd its main process in `SYSTEMD_EXEC_PID`.
pub fn by_service(env: &dyn Fn(&str) -> Option<String>, pid: u32) -> bool {
    env("XPC_SERVICE_NAME").as_deref() == Some(SERVICE) || env("SYSTEMD_EXEC_PID").is_some_and(|p| p == pid.to_string())
}

/// Start [`relaunch_script`] on its own, outliving this daemon.
pub fn move_into_login(exe: &Path) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    let script = relaunch_script(exe, std::process::id());
    std::process::Command::new("/bin/sh").arg("-c").arg(script).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn().map(drop)
}

/// A turn its engine's lost sign-in stopped, kept to be carried on once it is
/// signed in again: whose, and what it was asked (each, in order, when the
/// person asked again while it was out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stopped {
    pub engine: Engine,
    #[serde(skip)]
    pub session: equipe_protocol::SessionId,
    pub member: Option<MemberRef>,
    pub prompts: Vec<String>,
    /// When it stopped (unix seconds).
    pub at: u64,
}

/// How long a stopped turn waits to be carried on; older, it's the person's to pick up.
pub const KEPT_FOR: u64 = 12 * 60 * 60;

/// Where the stopped turns wait across a restart (moving into the login session is one).
pub fn stopped_file(home: &Path) -> PathBuf {
    home.join("sign-in-stopped.json")
}

/// The members' stopped turns, kept for the daemon that comes next; none: the file goes.
pub fn save_stopped(home: &Path, stopped: &[Stopped]) {
    let kept: Vec<&Stopped> = stopped.iter().filter(|s| s.member.is_some()).collect();
    let file = stopped_file(home);
    if kept.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&kept) {
        let _ = std::fs::write(file, text);
    }
}

/// The stopped turns an earlier daemon kept, not too old to carry on.
pub fn load_stopped(home: &Path, now: u64) -> Vec<Stopped> {
    let Ok(text) = std::fs::read_to_string(stopped_file(home)) else { return Vec::new() };
    let all: Vec<Stopped> = serde_json::from_str(&text).unwrap_or_default();
    all.into_iter().filter(|s| now.saturating_sub(s.at) < KEPT_FOR).collect()
}

/// What a stopped member is told when it carries on: why it stopped, and
/// what it had been asked — which never reached it, or did in part.
pub fn carry_on_prompt(engine: Engine, prompts: &[String]) -> String {
    let tool = match engine {
        Engine::Claude => "Claude Code",
        Engine::Codex => "Codex",
    };
    let asked = prompts.join("\n\n---\n\n");
    format!(
        "[equipe: your last turn stopped because {tool}'s sign-in on this machine wasn't working; it is signed in again. \
What you were asked is below — if you had already started on it, carry on from where you were rather than starting over.]\n\n{asked}"
    )
}

/// How long `claude auth login` waits for the person.
pub const SIGN_IN_WAIT: Duration = crate::engines::SIGN_IN_WAIT;

/// What `claude auth login` prints, read line by line.
#[derive(Debug, Default)]
pub struct ClaudeLoginOutput {
    told: bool,
    said: Vec<String>,
}

impl ClaudeLoginOutput {
    /// One line it printed: what to tell the screens now, if anything. The
    /// page to open anywhere is the one that isn't this machine's own
    /// `localhost`: it ends on a code to paste.
    pub fn line(&mut self, line: &str) -> Option<SignInStep> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let url = line.split_whitespace().find(|w| w.starts_with("https://")).map(|u| u.trim_end_matches(['.', ',', ')']).to_owned());
        // Never kept whole: the address signs this machine in to whoever finishes it.
        self.said.push(if url.is_some() { "(the sign-in page)".to_owned() } else { line.to_owned() });
        match url {
            Some(url) if !self.told && !url.contains("localhost") && !url.contains("127.0.0.1") => {
                self.told = true;
                Some(SignInStep::Paste { url })
            }
            _ => None,
        }
    }

    /// How it ended, by its exit and what it said.
    pub fn end(&self, ok: bool) -> SignInStep {
        if ok {
            return SignInStep::Done;
        }
        let detail = self.said.iter().rev().find(|l| l.contains("failed") || l.contains("Error") || l.contains("error")).or(self.said.last()).cloned().unwrap_or_default();
        let why = if detail.contains("timed out") || detail.contains("expired") { SignInFailure::Expired } else { SignInFailure::Failed };
        SignInStep::Failed { why, detail: detail.chars().take(240).collect() }
    }
}

/// A code as the person pasted it, fit for the tool's prompt: one line,
/// not empty, not absurdly long.
pub fn pasted(code: &str) -> Option<String> {
    let code = code.trim();
    (!code.is_empty() && code.len() <= 2048 && !code.contains(['\n', '\r'])).then(|| code.to_owned())
}

/// Run `claude auth login` here with no terminal, telling each step as it
/// comes: its browser opens on this machine, and the page to open anywhere
/// goes to the screens; a code from `codes` goes to its prompt. The last
/// step is how it ended. Dropped, the process goes with it.
pub async fn claude_login(binary: &str, mut codes: mpsc::UnboundedReceiver<String>, tell: impl Fn(SignInStep)) -> SignInStep {
    let child = Command::new(binary).args(["auth", "login"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return SignInStep::Failed { why: SignInFailure::NotInstalled, detail: String::new() },
        Err(err) => return SignInStep::Failed { why: SignInFailure::Failed, detail: err.to_string() },
    };
    tell(SignInStep::Browser { url: None });
    let (Some(mut input), Some(out), Some(err)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        return SignInStep::Failed { why: SignInFailure::Failed, detail: String::new() };
    };
    let mut output = ClaudeLoginOutput::default();
    let run = async {
        let (mut out, mut err) = (BufReader::new(out).lines(), BufReader::new(err).lines());
        let (mut out_open, mut err_open, mut codes_open) = (true, true, true);
        while out_open || err_open {
            tokio::select! {
                line = out.next_line(), if out_open => match line.ok().flatten() {
                    Some(line) => if let Some(step) = output.line(&line) { tell(step) },
                    None => out_open = false,
                },
                line = err.next_line(), if err_open => match line.ok().flatten() {
                    Some(line) => if let Some(step) = output.line(&line) { tell(step) },
                    None => err_open = false,
                },
                code = codes.recv(), if codes_open => match code {
                    Some(code) => {
                        let _ = input.write_all(format!("{code}\n").as_bytes()).await;
                        let _ = input.flush().await;
                    }
                    None => codes_open = false,
                },
            }
        }
        child.wait().await.map(|s| s.success()).unwrap_or(false)
    };
    match tokio::time::timeout(SIGN_IN_WAIT, run).await {
        Ok(ok) => output.end(ok),
        Err(_) => SignInStep::Failed { why: SignInFailure::Expired, detail: String::new() },
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_void};
    use std::path::Path;

    /// `sessionHasGraphicAccess` (Security/AuthSession.h).
    pub const SESSION_HAS_GRAPHIC_ACCESS: u32 = 0x0010;
    /// `kSecUnlockStateStatus` (Security/SecKeychain.h).
    pub const UNLOCKED: u32 = 1;
    /// `callerSecuritySession`.
    const CALLER: u32 = u32::MAX;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SessionGetInfo(session: u32, id: *mut u32, attributes: *mut u32) -> i32;
        fn SecKeychainOpen(path: *const std::ffi::c_char, keychain: *mut *mut c_void) -> i32;
        fn SecKeychainGetStatus(keychain: *mut c_void, status: *mut u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    /// This process's security session's attributes.
    pub fn session_attributes() -> Option<u32> {
        let (mut id, mut attributes) = (0u32, 0u32);
        // SAFETY: both out-pointers are valid for the call; the caller's own session is always there to ask.
        let status = unsafe { SessionGetInfo(CALLER, &mut id, &mut attributes) };
        (status == 0).then_some(attributes)
    }

    /// The keychain at `path`'s status bits: whether it is unlocked, readable,
    /// writable. Asks for no item and shows no prompt.
    pub fn keychain_status(path: &Path) -> Option<u32> {
        if !path.is_file() {
            return None;
        }
        let path = CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let mut keychain: *mut c_void = std::ptr::null_mut();
        // SAFETY: `path` is a NUL-terminated string that outlives the call; `keychain` is a valid out-pointer.
        if unsafe { SecKeychainOpen(path.as_ptr(), &mut keychain) } != 0 || keychain.is_null() {
            return None;
        }
        let mut status = 0u32;
        // SAFETY: `keychain` was just opened and is released right after.
        let asked = unsafe { SecKeychainGetStatus(keychain, &mut status) };
        // SAFETY: SecKeychainOpen's reference is ours to release, once.
        unsafe { CFRelease(keychain) };
        (asked == 0).then_some(status)
    }
}

#[cfg(not(target_os = "macos"))]
mod mac {
    use std::path::Path;

    pub const SESSION_HAS_GRAPHIC_ACCESS: u32 = 0x0010;
    pub const UNLOCKED: u32 = 1;

    pub fn session_attributes() -> Option<u32> {
        None
    }

    pub fn keychain_status(_: &Path) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_turns_hint_names_the_engine_whose_sign_in_was_lost() {
        assert_eq!(lost_by(Some(CLAUDE_HINT)), Some(Engine::Claude));
        assert_eq!(lost_by(Some("claude auth login — in Terminal on this Mac's own screen")), Some(Engine::Claude), "an older daemon's words");
        assert_eq!(lost_by(Some("equipe login claude")), Some(Engine::Claude));
        assert_eq!(lost_by(Some(CODEX_HINT)), Some(Engine::Codex));
        assert_eq!(lost_by(Some("see https://code.claude.com/docs/en/setup")), None);
        assert_eq!(lost_by(None), None);
    }

    fn claude(state: EngineState) -> EngineStatus {
        EngineStatus::new(Engine::Claude, state)
    }

    #[test]
    fn outside_the_login_session_the_daemon_is_moved_not_signed_in_again() {
        // As this Mac said over SSH: signed out, the Keychain closed — while its own screen said signed in.
        let (state, _) = claude_verdict(&claude(EngineState::SignedOut), Some(false), Keychain::Locked, true);
        assert_eq!(state, AuthState::OutsideLogin { movable: true });
        let (state, _) = claude_verdict(&claude(EngineState::SignedOut), Some(false), Keychain::Locked, false);
        assert_eq!(state, AuthState::OutsideLogin { movable: false }, "no login service: installed at the Mac's screen");
    }

    #[test]
    fn inside_it_a_locked_keychain_comes_before_signing_in_again() {
        assert_eq!(claude_verdict(&claude(EngineState::SignedOut), Some(true), Keychain::Locked, true).0, AuthState::KeychainLocked);
        assert_eq!(claude_verdict(&claude(EngineState::SignedOut), Some(true), Keychain::Unlocked, true).0, AuthState::SignedOut);
        assert_eq!(claude_verdict(&claude(EngineState::SignedOut), None, Keychain::Unknown, false).0, AuthState::SignedOut, "not a Mac: signing in is all there is");
        assert_eq!(claude_verdict(&claude(EngineState::SignedIn), Some(false), Keychain::Locked, true).0, AuthState::SignedIn { resumed: 0 }, "it works: nothing to fix");
        assert_eq!(claude_verdict(&claude(EngineState::NotInstalled), Some(true), Keychain::Unlocked, true).0, AuthState::NotInstalled);
        let broken = EngineStatus { detail: Some("`claude auth status` didn't answer in 20s".into()), ..claude(EngineState::Unknown) };
        assert_eq!(claude_verdict(&broken, Some(true), Keychain::Unlocked, true), (AuthState::Unknown, "`claude auth status` didn't answer in 20s".into()));
    }

    #[test]
    fn codex_stands_as_its_status_says() {
        assert_eq!(codex_verdict(&EngineStatus::new(Engine::Codex, EngineState::SignedOut)).0, AuthState::SignedOut);
        assert_eq!(codex_verdict(&EngineStatus::new(Engine::Codex, EngineState::SignedIn)).0, AuthState::SignedIn { resumed: 0 });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_session_and_the_keychain_are_asked_without_a_prompt() {
        // Whatever this machine's session is, it answers, and at once.
        let began = std::time::Instant::now();
        assert!(in_login_session().is_some());
        let home = user_home().unwrap();
        let _ = keychain(&home);
        assert_eq!(keychain(Path::new("/nonexistent")), Keychain::Unknown);
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    /// What `claude auth login` 2.1.308 printed with no terminal (its page's codes cut).
    const SAID: &str = "Opening browser to sign in…\nIf the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&state=s\nPaste code here if prompted > Login failed: Request failed with status code 400\n";

    #[test]
    fn a_claude_sign_in_tells_the_page_to_open_anywhere_and_says_why_it_failed() {
        let mut output = ClaudeLoginOutput::default();
        let told: Vec<SignInStep> = SAID.lines().filter_map(|l| output.line(l)).collect();
        assert_eq!(told.len(), 1);
        let SignInStep::Paste { url } = &told[0] else { panic!("{told:?}") };
        assert!(url.starts_with("https://claude.com/cai/oauth/authorize") && url.contains("platform.claude.com"), "{url}");
        let SignInStep::Failed { why, detail } = output.end(false) else { panic!() };
        assert_eq!(why, SignInFailure::Failed);
        assert!(detail.contains("Login failed"), "{detail}");
        assert!(!detail.contains("https://"), "the page is never kept: {detail}");
        assert_eq!(ClaudeLoginOutput::default().end(true), SignInStep::Done);
    }

    #[test]
    fn a_pasted_code_is_one_line() {
        assert_eq!(pasted("  abc#def \n").as_deref(), Some("abc#def"));
        assert_eq!(pasted("   "), None);
        assert_eq!(pasted("a\nb"), None);
    }

    fn script(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("claude");
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_string_lossy().into_owned()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-auth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn the_code_the_person_pastes_reaches_the_tools_prompt_and_signs_it_in() {
        let dir = scratch("login");
        // A stand-in that prints the page, reads a line and signs in only with the right code.
        let bin = script(&dir, "[ \"$1 $2\" = \"auth login\" ] || exit 9\necho 'If the browser didn'\"'\"'t open, visit: https://claude.com/cai/oauth/authorize?code=true'\nprintf 'Paste code here if prompted > '\nread code\n[ \"$code\" = 'abc#def' ] && echo 'Login successful.' && exit 0\necho 'Login failed: bad code'; exit 1");
        let (tx, rx) = mpsc::unbounded_channel();
        let told = std::sync::Mutex::new(Vec::new());
        let end = claude_login(&bin, rx, |step| {
            if matches!(step, SignInStep::Paste { .. }) {
                let _ = tx.send("abc#def".into());
            }
            told.lock().unwrap().push(step);
        })
        .await;
        assert_eq!(end, SignInStep::Done);
        let told = told.into_inner().unwrap();
        assert_eq!(told[0], SignInStep::Browser { url: None });
        assert!(matches!(&told[1], SignInStep::Paste { url } if url.starts_with("https://claude.com/")));
        let (_, rx) = mpsc::unbounded_channel();
        let none = claude_login(dir.join("nope").to_str().unwrap(), rx, |_| {}).await;
        assert!(matches!(none, SignInStep::Failed { why: SignInFailure::NotInstalled, .. }));
    }

    #[test]
    fn the_daemon_starts_again_inside_once_it_has_stopped_and_never_alone_twice() {
        let script = relaunch_script(Path::new("/Users/a b/.local/bin/equipe"), 4242);
        let stop = script.find("daemon stop").unwrap();
        let wait = script.find("kill -0 4242").unwrap();
        let start = script.find("exec '/Users/a b/.local/bin/equipe' daemon start").unwrap();
        assert!(stop < wait && wait < start, "{script}");
        assert!(script.contains("'/Users/a b/.local/bin/equipe' daemon stop"), "{script}");
        // `daemon start` goes through the login service, and never starts one beside it.
        assert!(!script.contains("bootout") && !script.contains("daemon run"), "{script}");
        assert!(!script.contains("unlock") && !script.contains("security"), "never the Keychain");
    }

    #[test]
    fn a_daemon_started_over_ssh_knows_it_and_so_does_one_its_service_started() {
        fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_owned())
        }
        assert!(ssh_bound(&env(&[("SSH_CONNECTION", "100.93.184.56 54171 100.106.17.55 22")])));
        assert!(ssh_bound(&env(&[("SSH_TTY", "/dev/ttys004")])));
        assert!(!ssh_bound(&env(&[("SSH_AUTH_SOCK", "/private/tmp/com.apple.launchd.mM2GR5ssrV/Listeners")])), "launchd's own agent socket");
        assert!(!ssh_bound(&env(&[("SSH_CONNECTION", "")])));
        assert!(by_service(&env(&[("XPC_SERVICE_NAME", "im.equipe.daemon")]), 7));
        assert!(!by_service(&env(&[("XPC_SERVICE_NAME", "application.com.apple.Terminal.123")]), 7), "a Terminal's");
        assert!(by_service(&env(&[("SYSTEMD_EXEC_PID", "7")]), 7));
        assert!(!by_service(&env(&[("SYSTEMD_EXEC_PID", "6")]), 7), "a shell its service started");
        assert_eq!(runs_here().pid, std::process::id());
    }

    #[test]
    fn stopped_turns_wait_across_a_restart_for_a_while() {
        let home = scratch("stopped");
        let member = Some(MemberRef { squad: "demo".into(), member: "pm".into() });
        let stopped = vec![
            Stopped { engine: Engine::Claude, session: 3, member: member.clone(), prompts: vec!["fix the header".into()], at: 1000 },
            Stopped { engine: Engine::Claude, session: 9, member: None, prompts: vec!["hi".into()], at: 1000 },
        ];
        save_stopped(&home, &stopped);
        let back = load_stopped(&home, 1000 + 60);
        assert_eq!(back.len(), 1, "a member's, not a passing conversation's");
        assert_eq!((back[0].member.clone(), back[0].prompts.clone()), (member, vec!["fix the header".to_owned()]));
        assert!(load_stopped(&home, 1000 + KEPT_FOR + 1).is_empty(), "too old: the person's to pick up");
        save_stopped(&home, &[]);
        assert!(!stopped_file(&home).exists());
    }

    #[test]
    fn a_member_carrying_on_hears_why_it_stopped_and_what_it_was_asked() {
        let prompt = carry_on_prompt(Engine::Claude, &["fix the header".into(), "and the footer".into()]);
        assert!(prompt.contains("Claude Code's sign-in") && prompt.contains("fix the header") && prompt.contains("and the footer"), "{prompt}");
        assert!(prompt.find("fix the header") < prompt.find("and the footer"));
    }
}
