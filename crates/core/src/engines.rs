//! The engines on the daemon's machine: where each stands, and signing in
//! from any screen — a window on another Mac included.
//!
//! Each engine is the person's own command-line tool with its own login,
//! which equipe asks about and drives but never reads, keeps or passes on:
//! Claude Code (`claude auth status`; the person signs in with
//! `claude auth login` themselves) and Codex (`codex login status`,
//! `codex login`, `codex logout`). The sign-in runs here, on the daemon's
//! machine, because that is where the tool keeps the login and where the
//! members use it. With the browser on this Mac, `codex login` opens it and
//! catches its return on `localhost:1455` itself; with the browser on another
//! Mac, `codex login --device-auth` prints an address and a one-time code
//! that the screen shows, so nothing needs to come back to this machine's
//! loopback.

use std::process::Stdio;
use std::time::Duration;

use equipe_protocol::{Engine, EngineState, EngineStatus, SignInFailure, SignInStep};
use equipe_providers::{AuthMethod, CredentialStore, claude_code};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;

/// How long a sign-in may wait for the person (`codex login --device-auth`'s
/// own code lasts as long).
pub const SIGN_IN_WAIT: Duration = Duration::from_secs(15 * 60);
/// How long `codex login status` and `codex logout` may take.
const ASK_WAIT: Duration = Duration::from_secs(20);

/// The `codex` to run: `$EQUIPE_CODEX` (tests, an install off the PATH), else
/// the one on the PATH.
pub fn codex_binary() -> String {
    std::env::var("EQUIPE_CODEX").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| "codex".to_owned())
}

/// Claude Code: what `claude auth status` says of itself.
pub async fn claude(binary: &str) -> EngineStatus {
    match claude_code::auth_status(binary).await {
        Ok(None) => EngineStatus::new(Engine::Claude, EngineState::NotInstalled),
        Ok(Some(status)) if status.logged_in => {
            let plan = if status.uses_subscription() { status.subscription_type.clone() } else { None };
            EngineStatus { plan, per_token: !status.uses_subscription(), ..EngineStatus::new(Engine::Claude, EngineState::SignedIn) }
        }
        Ok(Some(_)) => EngineStatus::new(Engine::Claude, EngineState::SignedOut),
        Err(err) => EngineStatus { detail: Some(err.to_string()), ..EngineStatus::new(Engine::Claude, EngineState::Unknown) },
    }
}

/// Codex: what `codex login status` says — and whether equipe's own older
/// login (`equipe login codex`) is kept too.
pub async fn codex(binary: &str, store: &CredentialStore) -> EngineStatus {
    let legacy = matches!(store.load("openai"), Ok(Some(cred)) if cred.method == AuthMethod::OauthSubscription);
    let asked = Command::new(binary).args(["login", "status"]).stdin(Stdio::null()).kill_on_drop(true).output();
    let mut status = match tokio::time::timeout(ASK_WAIT, asked).await {
        Err(_) => EngineStatus { detail: Some(format!("`{binary} login status` didn't answer in {}s", ASK_WAIT.as_secs())), ..EngineStatus::new(Engine::Codex, EngineState::Unknown) },
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => EngineStatus::new(Engine::Codex, EngineState::NotInstalled),
        Ok(Err(err)) => EngineStatus { detail: Some(err.to_string()), ..EngineStatus::new(Engine::Codex, EngineState::Unknown) },
        Ok(Ok(out)) => {
            let said = format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
            codex_status(out.status.success(), &said)
        }
    };
    status.legacy = legacy;
    // Signed in: who as, and on what plan — `codex login status` says neither.
    if status.state == EngineState::SignedIn {
        let cwd = std::env::temp_dir();
        if let Ok(Ok(Some(account))) = tokio::time::timeout(ASK_WAIT, equipe_providers::codex_app::read_account(binary, &cwd)).await {
            status.account = account.email;
            status.plan = status.plan.or(account.plan);
            status.per_token |= account.api_key;
        }
    }
    status
}

/// What `codex login status` printed (on either stream) and how it exited,
/// as a status. Only the kind of login is told — never the masked key an
/// API-key login prints.
pub fn codex_status(ok: bool, said: &str) -> EngineStatus {
    let said = plain(said);
    let lines: Vec<&str> = said.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let signed_in = |per_token| EngineStatus { per_token, ..EngineStatus::new(Engine::Codex, EngineState::SignedIn) };
    if let Some(how) = lines.iter().find(|l| l.starts_with("Logged in")) {
        return signed_in(how.contains("API key"));
    }
    if lines.iter().any(|l| l.contains("Not logged in")) {
        return EngineStatus::new(Engine::Codex, EngineState::SignedOut);
    }
    if ok {
        return signed_in(false);
    }
    EngineStatus { detail: lines.last().map(|l| short(l)), ..EngineStatus::new(Engine::Codex, EngineState::Unknown) }
}

/// Every engine, in the order the screens show them.
pub async fn statuses(claude_binary: &str, codex_binary: &str, store: &CredentialStore) -> Vec<EngineStatus> {
    let (claude, codex) = tokio::join!(claude(claude_binary), codex(codex_binary, store));
    vec![claude, codex]
}

/// Without terminal colours: `codex` paints its address and code.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Up to the final byte of the sequence (a letter).
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn short(line: &str) -> String {
    let mut s: String = line.chars().take(240).collect();
    if line.chars().count() > 240 {
        s.push('…');
    }
    s
}

/// What `codex login` prints, read line by line into what the screens are told.
#[derive(Debug, Default)]
pub struct LoginOutput {
    device: bool,
    url: Option<String>,
    awaiting_code: bool,
    told: bool,
    /// Everything it said, for why it failed.
    said: Vec<String>,
}

impl LoginOutput {
    pub fn new(device: bool) -> Self {
        Self { device, ..Self::default() }
    }

    /// One line it printed: what to tell the screens now, if anything.
    pub fn line(&mut self, line: &str) -> Option<SignInStep> {
        let line = plain(line);
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        self.said.push(line.to_owned());
        let url = line.split_whitespace().find(|w| w.starts_with("https://") || w.starts_with("http://")).map(|u| u.trim_end_matches(['.', ',', ')']).to_owned());
        if self.device {
            if line.contains("one-time code") {
                self.awaiting_code = true;
            } else if let Some(url) = url {
                self.url.get_or_insert(url);
            } else if self.awaiting_code {
                self.awaiting_code = false;
                if let (Some(url), false) = (&self.url, self.told) {
                    self.told = true;
                    return Some(SignInStep::DeviceCode { url: url.clone(), code: line.to_owned() });
                }
            }
            return None;
        }
        // The address to open by hand: the sign-in page, not the local server it starts.
        match url {
            Some(url) if !self.told && !url.contains("localhost") && !url.contains("127.0.0.1") => {
                self.told = true;
                Some(SignInStep::Browser { url: Some(url) })
            }
            _ => None,
        }
    }

    /// How it ended, by its exit and what it said.
    pub fn end(&self, ok: bool) -> SignInStep {
        if ok {
            return SignInStep::Done;
        }
        let all = self.said.join("\n");
        let why = if all.contains("not enabled") {
            SignInFailure::DeviceCodeOff
        } else if all.contains("timed out") {
            SignInFailure::Expired
        } else {
            SignInFailure::Failed
        };
        let detail = self.said.iter().rev().find(|l| l.starts_with("Error")).or(self.said.last()).map(|l| short(l)).unwrap_or_default();
        SignInStep::Failed { why, detail }
    }
}

/// Run `codex login` (`--device-auth` for a code) on this machine, telling
/// each step as it comes; the last step is how it ended. Dropped, the
/// process goes with it.
pub async fn codex_login(binary: &str, device: bool, tell: impl Fn(SignInStep)) -> SignInStep {
    let mut command = Command::new(binary);
    command.arg("login");
    if device {
        command.arg("--device-auth");
    }
    let child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return SignInStep::Failed { why: SignInFailure::NotInstalled, detail: String::new() },
        Err(err) => return SignInStep::Failed { why: SignInFailure::Failed, detail: err.to_string() },
    };
    if !device {
        tell(SignInStep::Browser { url: None });
    }
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return SignInStep::Failed { why: SignInFailure::Failed, detail: String::new() };
    };
    let mut output = LoginOutput::new(device);
    let run = async {
        let (mut out, mut err) = (BufReader::new(out).lines(), BufReader::new(err).lines());
        let (mut out_open, mut err_open) = (true, true);
        while out_open || err_open {
            let line = tokio::select! {
                line = out.next_line(), if out_open => line.ok().flatten().or_else(|| { out_open = false; None }),
                line = err.next_line(), if err_open => line.ok().flatten().or_else(|| { err_open = false; None }),
            };
            if let Some(step) = line.and_then(|l| output.line(&l)) {
                tell(step);
            }
        }
        child.wait().await.map(|s| s.success()).unwrap_or(false)
    };
    match tokio::time::timeout(SIGN_IN_WAIT, run).await {
        Ok(ok) => output.end(ok),
        Err(_) => SignInStep::Failed { why: SignInFailure::Expired, detail: String::new() },
    }
}

/// `codex logout`: the tool forgets its own login.
pub async fn codex_logout(binary: &str) -> Result<(), String> {
    let run = Command::new(binary).arg("logout").stdin(Stdio::null()).kill_on_drop(true).output();
    match tokio::time::timeout(ASK_WAIT, run).await {
        Err(_) => Err(format!("`{binary} logout` didn't answer in {}s", ASK_WAIT.as_secs())),
        Ok(Err(err)) => Err(err.to_string()),
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => Err(short(plain(&String::from_utf8_lossy(&out.stderr)).trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `codex login --device-auth` 0.154 prints, colours and all.
    const DEVICE: &str = "\nWelcome to Codex [v\u{1b}[90m0.154.0\u{1b}[0m]\n\u{1b}[90mOpenAI's command-line coding agent\u{1b}[0m\n\nFollow these steps to sign in with ChatGPT using device code authorization:\n\n1. Open this link in your browser and sign in to your account\n   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m\n\n2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m\n   \u{1b}[94mABCD-EFGH2\u{1b}[0m\n\n\u{1b}[90mContinue only if you started this login in Codex. If a website or another person gave you this code, cancel.\u{1b}[0m\n";

    #[test]
    fn a_device_sign_in_tells_the_address_and_the_code_once() {
        let mut output = LoginOutput::new(true);
        let told: Vec<SignInStep> = DEVICE.lines().filter_map(|l| output.line(l)).collect();
        assert_eq!(told, vec![SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "ABCD-EFGH2".into() }]);
        assert_eq!(output.end(true), SignInStep::Done);
    }

    #[test]
    fn a_browser_sign_in_tells_the_page_to_open_by_hand_not_the_local_server() {
        let mut output = LoginOutput::new(false);
        let said = "Starting local login server on http://localhost:1455.\nIf your browser did not open, navigate to this URL to authenticate:\n\nhttps://auth.openai.com/oauth/authorize?response_type=code&state=s\n";
        let told: Vec<SignInStep> = said.lines().filter_map(|l| output.line(l)).collect();
        assert_eq!(told, vec![SignInStep::Browser { url: Some("https://auth.openai.com/oauth/authorize?response_type=code&state=s".into()) }]);
    }

    #[test]
    fn a_failed_sign_in_says_why_in_the_tools_words() {
        let mut off = LoginOutput::new(true);
        off.line("Error logging in with device code: device code login is not enabled for this Codex server. Use the browser login or verify the server URL.");
        let SignInStep::Failed { why, detail } = off.end(false) else { panic!() };
        assert_eq!(why, SignInFailure::DeviceCodeOff);
        assert!(detail.starts_with("Error logging in with device code"));
        let mut late = LoginOutput::new(true);
        late.line("Error logging in with device code: device auth timed out after 15 minutes");
        assert!(matches!(late.end(false), SignInStep::Failed { why: SignInFailure::Expired, .. }));
        assert!(matches!(LoginOutput::new(false).end(false), SignInStep::Failed { why: SignInFailure::Failed, .. }));
    }

    #[test]
    fn codex_stands_as_its_status_says_and_never_tells_the_key() {
        assert_eq!(codex_status(true, "Logged in using ChatGPT\n").state, EngineState::SignedIn);
        let key = codex_status(true, "Logged in using an API key - sk-proj-***ABCD\n");
        assert_eq!((key.state, key.per_token), (EngineState::SignedIn, true));
        assert!(!serde_json::to_string(&key).unwrap().contains("ABCD"));
        assert_eq!(codex_status(false, "Not logged in\n").state, EngineState::SignedOut);
        let broken = codex_status(false, "Error checking login status: bad config\n");
        assert_eq!((broken.state, broken.detail.as_deref()), (EngineState::Unknown, Some("Error checking login status: bad config")));
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("equipe-engines-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn claude_code_stands_as_it_says_of_itself() {
        let dir = scratch("claude");
        assert_eq!(claude(dir.join("no-such-claude").to_str().unwrap()).await.state, EngineState::NotInstalled);
        let max = claude(&script(&dir, "max", r#"echo '{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max"}'"#)).await;
        assert_eq!((max.state, max.plan.as_deref(), max.per_token), (EngineState::SignedIn, Some("max"), false));
        let key = claude(&script(&dir, "key", r#"echo '{"loggedIn":true,"authMethod":"api_key"}'"#)).await;
        assert_eq!((key.state, key.plan, key.per_token), (EngineState::SignedIn, None, true));
        assert_eq!(claude(&script(&dir, "out", r#"echo '{"loggedIn":false}'"#)).await.state, EngineState::SignedOut);
    }

    #[tokio::test]
    async fn codex_missing_says_so_and_equipes_older_login_shows_beside_it() {
        let dir = scratch("codex");
        let store = CredentialStore::new(&dir);
        let missing = codex(dir.join("no-such-codex").to_str().unwrap(), &store).await;
        assert_eq!((missing.state, missing.legacy), (EngineState::NotInstalled, false));
        let cred = equipe_providers::Credentials { provider: "openai".into(), method: AuthMethod::OauthSubscription, access: "a.b.c".into(), refresh: None, expires: None, account_id: None, residency: None };
        store.save(&cred).unwrap();
        let out = codex(&script(&dir, "codex", "echo 'Not logged in' >&2; exit 1"), &store).await;
        assert_eq!((out.state, out.legacy), (EngineState::SignedOut, true));
    }

    #[tokio::test]
    async fn a_device_sign_in_runs_the_tool_and_tells_the_code_before_it_ends() {
        let dir = scratch("login");
        let body = format!("[ \"$1 $2\" = \"login --device-auth\" ] || exit 9\nprintf '{}'\nsleep 0.2\necho 'Successfully logged in' >&2", DEVICE.replace('\u{1b}', "\\033").replace('\'', "'\\''").replace('\n', "\\n"));
        let bin = script(&dir, "codex", &body);
        let told = std::sync::Mutex::new(Vec::new());
        let end = codex_login(&bin, true, |step| told.lock().unwrap().push(step)).await;
        assert_eq!(end, SignInStep::Done);
        assert_eq!(*told.lock().unwrap(), vec![SignInStep::DeviceCode { url: "https://auth.openai.com/codex/device".into(), code: "ABCD-EFGH2".into() }]);
        let none = codex_login(dir.join("nope").to_str().unwrap(), true, |_| {}).await;
        assert!(matches!(none, SignInStep::Failed { why: SignInFailure::NotInstalled, .. }));
        let bye = script(&dir, "bye", "[ \"$1\" = logout ] && echo 'Successfully logged out' >&2");
        assert_eq!(codex_logout(&bye).await, Ok(()));
    }
}
