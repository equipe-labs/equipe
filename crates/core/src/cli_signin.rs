//! `cli_sign_in`: a member needs the person signed in to a command-line tool
//! (AWS, GitHub, Google Cloud, Azure) on the daemon's machine. Instead of
//! "open a terminal and run …", equipe runs the tool's own cross-device
//! sign-in itself, in a pseudo-terminal, reads the page and the code its
//! output shows, and puts them on one card. A flow that has the person paste
//! a code back gets it written to the waiting process's standard input — the
//! code is never stored, logged or echoed (the terminal's echo is off).
//!
//! Only the tools' own documented modes run (`aws login --remote`,
//! `aws sso login --no-browser --use-device-code`, `gh auth login --web`,
//! `gcloud auth login --no-launch-browser`, `az login --use-device-code`):
//! equipe never reads a password, never bypasses a provider's sign-in, and
//! the credentials stay in the tool's own files.

use std::io::{Read as _, Write as _};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::protocol::cli_signin::PastedCode;

pub const CLI_SIGN_IN: &str = "cli_sign_in";

/// How long a sign-in waits for the person: device codes last 10 to 15 minutes.
pub const WAIT: Duration = Duration::from_secs(15 * 60);

/// The tools with a cross-device mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `aws login --remote`: a page, and a code to paste back.
    Aws,
    /// `aws sso login --no-browser --use-device-code`: a page and a code to confirm.
    AwsSso,
    /// `gh auth login --web`: the device code to confirm on github.com.
    Gh,
    /// `gcloud auth login --no-launch-browser`: a page, and a code to paste back.
    Gcloud,
    /// `az login --use-device-code`.
    Az,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Self::Aws, Self::AwsSso, Self::Gh, Self::Gcloud, Self::Az];

    /// The word members use for it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::AwsSso => "aws_sso",
            Self::Gh => "gh",
            Self::Gcloud => "gcloud",
            Self::Az => "az",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        let w = word.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        Some(match w.as_str() {
            "aws" | "aws_login" => Self::Aws,
            "aws_sso" | "sso" => Self::AwsSso,
            "gh" | "github" => Self::Gh,
            "gcloud" | "google_cloud" | "gcp" => Self::Gcloud,
            "az" | "azure" => Self::Az,
            _ => return None,
        })
    }

    /// How a card names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Aws | Self::AwsSso => "AWS",
            Self::Gh => "GitHub",
            Self::Gcloud => "Google Cloud",
            Self::Az => "Azure",
        }
    }

    pub fn program(self) -> &'static str {
        match self {
            Self::Aws | Self::AwsSso => "aws",
            Self::Gh => "gh",
            Self::Gcloud => "gcloud",
            Self::Az => "az",
        }
    }

    /// Runs in a terminal of its own. `gh` doesn't: in one it asks about the
    /// person's git credential helper before anything else; over pipes it
    /// only prints the code and the page.
    pub fn tty(self) -> bool {
        !matches!(self, Self::Gh)
    }

    /// The flow has the person paste a code from the page back to the tool.
    pub fn wants_code(self) -> bool {
        matches!(self, Self::Aws | Self::Gcloud)
    }
}

/// What to run: the sign-in, the question that says as whom it ended, and
/// the environment that keeps the tool from opening a browser on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub whoami: (String, Vec<String>),
}

fn word_safe(w: &str) -> bool {
    !w.is_empty() && w.len() <= 80 && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:@".contains(c))
}

/// Arguments that would make equipe handle a secret or change the mode.
fn refused_arg(a: &str) -> bool {
    let l = a.to_ascii_lowercase();
    ["token", "password", "secret", "key", "cred", "stdin", "profile", "remote", "browser", "device-code", "web", "output"].iter().any(|bad| l.contains(bad))
}

/// The commands for `kind`, for `profile` (AWS profile, or a gcloud configuration) and `extra` flags.
pub fn plan(kind: Kind, profile: Option<&str>, extra: &[String]) -> Result<Plan, String> {
    if let Some(p) = profile
        && (!word_safe(p) || p.contains(['/', ':', '=']))
    {
        return Err("The profile is a plain name: letters, digits, `-`, `_`, `.`, `@`.".into());
    }
    if profile.is_some() && matches!(kind, Kind::Gh | Kind::Az) {
        return Err(format!("{} has no profiles: leave `profile` out.", kind.name()));
    }
    for a in extra {
        if !word_safe(a) {
            return Err(format!("`{a}` isn't a plain flag or value."));
        }
        if refused_arg(a) {
            return Err(format!("`{a}` isn't allowed here: equipe runs only the tool's cross-device sign-in, and never handles tokens, passwords or keys."));
        }
    }
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    let mut args = Vec::new();
    let mut whoami_args = Vec::new();
    let mut env: Vec<(String, String)> = [("BROWSER", "echo"), ("GH_BROWSER", "echo"), ("NO_COLOR", "1"), ("TERM", "dumb"), ("AWS_PAGER", "")]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    match kind {
        Kind::Aws => {
            args.extend(s(&["login", "--remote"]));
            whoami_args.extend(s(&["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]));
        }
        Kind::AwsSso => {
            args.extend(s(&["sso", "login", "--no-browser", "--use-device-code"]));
            whoami_args.extend(s(&["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]));
        }
        Kind::Gh => {
            args.extend(s(&["auth", "login", "--web", "--git-protocol", "https", "--skip-ssh-key"]));
            whoami_args.extend(s(&["api", "user", "--jq", ".login"]));
        }
        Kind::Gcloud => {
            args.extend(s(&["auth", "login", "--no-launch-browser", "--brief"]));
            whoami_args.extend(s(&["config", "get-value", "account"]));
            if let Some(p) = profile {
                env.push(("CLOUDSDK_ACTIVE_CONFIG_NAME".into(), p.to_owned()));
            }
        }
        Kind::Az => {
            args.extend(s(&["login", "--use-device-code"]));
            whoami_args.extend(s(&["account", "show", "--query", "user.name", "--output", "tsv"]));
        }
    }
    if let (Some(p), Kind::Aws | Kind::AwsSso) = (profile, kind) {
        args.extend(["--profile".to_owned(), p.to_owned()]);
        whoami_args.extend(["--profile".to_owned(), p.to_owned()]);
    }
    // GitHub's host, when it isn't github.com, is an ordinary flag.
    args.extend(extra.iter().cloned());
    if kind == Kind::Gh && !extra.iter().any(|a| a == "--hostname" || a == "-h") {
        args.extend(s(&["--hostname", "github.com"]));
    }
    Ok(Plan { program: kind.program().to_owned(), args, env, whoami: (kind.program().to_owned(), whoami_args) })
}

/// A call, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub kind: Kind,
    pub profile: Option<String>,
    pub extra: Vec<String>,
    pub why: String,
}

pub fn check(args: &Value) -> Result<Asked, String> {
    let tool = args.get("tool").and_then(Value::as_str).unwrap_or_default();
    let kind = Kind::parse(tool).ok_or_else(|| format!("`{tool}` has no cross-device sign-in equipe runs. Use one of: aws, aws_sso, gh, gcloud, az — or, for anything else, `block_on_person` with the exact steps."))?;
    let why = args.get("why").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if why.is_empty() {
        return Err("Say in `why` what the sign-in is for, in a line in the person's language.".into());
    }
    let profile = args.get("profile").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned);
    let extra: Vec<String> = args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    plan(kind, profile.as_deref(), &extra)?;
    Ok(Asked { kind, profile, extra, why: why.to_owned() })
}

/// The tool, as an MCP tool definition.
pub fn tool() -> Value {
    json!({
        "name": CLI_SIGN_IN,
        "description": "Have the person sign in to a command-line tool on this machine — AWS, GitHub, Google Cloud, Azure — \
without opening a terminal. equipe runs the tool's own cross-device sign-in itself and shows the person ONE card: a button \
that opens the sign-in page on the screen they are looking at, the code to confirm (or a box to paste the code the page \
gives back), and its progress. You never see a code, a password or a token. When they have signed in you are told as whom \
(the tool's own whoami, e.g. the AWS ARN) and carry on; if the link expires they get a new one from the card. Use this \
instead of asking the person to run `aws login`, `gh auth login`, `gcloud auth login` or `az login` in a terminal, and \
only when no machine credential exists (a GitHub App token, an access key already set up, an environment variable). Never ask for \
a password in chat. tool: aws (`aws login --remote`, the console sign-in; needs `profile` when the profile isn't the default), aws_sso \
(`aws sso login --use-device-code`, for an SSO profile), gh (`gh auth login --web`), gcloud (`gcloud auth login --no-launch-browser`; \
`profile` names the gcloud configuration), az (`az login --use-device-code`). Other tools have no flow equipe runs: use \
`block_on_person` with the exact steps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool": { "type": "string", "enum": ["aws", "aws_sso", "gh", "gcloud", "az"], "description": "Which tool to sign in." },
                "why": { "type": "string", "description": "What it is for, in a line in the person's language: \"deploy the staging stack\"." },
                "profile": { "type": "string", "description": "The AWS profile to sign in (`atinum`), or the gcloud configuration. Leave out for the default, and for gh and az." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Rare extra flags, one per item (`--region`, `us-east-1`; `--hostname`, `github.example.com`). Never tokens, passwords or keys." },
            },
            "required": ["tool", "why"],
        },
    })
}

// ── reading what the tool prints ──

/// Escape sequences and carriage returns taken out of terminal output.
pub fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: up to a letter.
                Some('[') => {
                    for n in chars.by_ref() {
                        if n.is_ascii_alphabetic() || n == '~' {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST.
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// `ABCD-1234`, `WDJB-MJHT`: what device-code flows show.
fn device_code_shaped(word: &str) -> bool {
    let Some((a, b)) = word.split_once('-') else { return false };
    let part = |p: &str| (4..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    part(a) && part(b)
}

/// Azure's code: nine capitals and digits after "enter the code".
fn azure_code(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("enter the code")? + "enter the code".len();
    let word = text[at..].split_whitespace().next()?;
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    ((6..=12).contains(&word.len()) && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then(|| word.to_owned())
}

/// The page addresses in `text` whose end has been seen (white space after
/// them): a half-printed one isn't taken.
fn urls(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("https://") {
        let start = from + at;
        let rest = &text[start..];
        let Some(end) = rest.find(char::is_whitespace) else { break };
        let url = rest[..end].trim_end_matches(['.', ',', ')', '"', '\'', '>', ';']);
        if url.len() > "https://".len() + 3 {
            found.push(url.to_owned());
        }
        from = start + end;
    }
    found
}

/// The page to open: the one that fills the code in when the tool prints
/// both, else the first.
fn page_in(text: &str) -> Option<String> {
    let all = urls(text);
    all.iter().find(|u| u.contains("user_code=")).or(all.first()).cloned()
}

/// What a sign-in's output has shown so far, read as it arrives.
#[derive(Debug, Clone, Default)]
pub struct Reader {
    kind: Option<Kind>,
    text: String,
    /// How much of the output was dropped from the front of `text`.
    dropped: usize,
    /// Where the last prompt answered with Enter was.
    answered: Option<usize>,
    pub url: Option<String>,
    pub code: Option<String>,
}

const KEPT: usize = 24 * 1024;

impl Reader {
    pub fn new(kind: Kind) -> Self {
        Reader { kind: Some(kind), ..Self::default() }
    }

    /// More output. Whether the page or the code became known.
    pub fn feed(&mut self, chunk: &str) -> bool {
        self.text.push_str(&clean(chunk));
        if self.text.len() > KEPT {
            let mut cut = self.text.len() - KEPT;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
            self.dropped += cut;
        }
        let before = (self.url.clone(), self.code.clone());
        if let Some(page) = page_in(&self.text) {
            self.url = Some(page);
        }
        if self.code.is_none() && !self.kind.is_some_and(Kind::wants_code) {
            let from_url = self.url.as_deref().and_then(|u| u.split_once("user_code=")).map(|(_, c)| c.split('&').next().unwrap_or_default().to_owned()).filter(|c| device_code_shaped(c));
            self.code = from_url
                .or_else(|| self.text.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')).find(|w| device_code_shaped(w)).map(str::to_owned))
                .or_else(|| (self.kind == Some(Kind::Az)).then(|| azure_code(&self.text)).flatten());
        }
        before != (self.url.clone(), self.code.clone())
    }

    /// The output says a code or link ran out.
    pub fn says_expired(&self) -> bool {
        let l = self.text.to_ascii_lowercase();
        ["expired", "timed out", "timeout", "expire"].iter().any(|w| l.contains(w))
    }

    /// The last line it printed, for "why it ended" — a short one, no addresses.
    pub fn last_line(&self) -> String {
        let line = self.text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
        let line = if line.contains("://") { "" } else { line };
        line.chars().take(160).collect()
    }

    /// Whether a prompt that Enter answers ("Press Enter to open … in your
    /// browser") is newly shown: once for each.
    fn take_enter(&mut self) -> bool {
        let lower = self.text.to_ascii_lowercase();
        let Some(at) = lower.rfind("press enter") else { return false };
        let at = self.dropped + at;
        if self.answered.is_some_and(|a| a >= at) {
            return false;
        }
        self.answered = Some(at);
        true
    }
}

// ── the process, in a pseudo-terminal (or pipes) ──

enum Input {
    Pty(std::fs::File),
    Pipe(tokio::process::ChildStdin),
}

impl Input {
    async fn send(&mut self, bytes: &[u8]) {
        use tokio::io::AsyncWriteExt as _;
        match self {
            Input::Pty(f) => {
                let _ = f.write_all(bytes);
                let _ = f.flush();
            }
            Input::Pipe(p) => {
                let _ = p.write_all(bytes).await;
                let _ = p.flush().await;
            }
        }
    }
}

struct Proc {
    input: Input,
    chunks: UnboundedReceiver<String>,
    child: tokio::process::Child,
}

/// Start `program` with a terminal of its own (its echo off), or — for a
/// tool that asks questions only a terminal answers — with plain pipes.
fn start(tty: bool, program: &str, args: &[String], env: &[(String, String)]) -> std::io::Result<Proc> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).kill_on_drop(true);
    let (tx, chunks) = unbounded_channel::<String>();
    if !tty {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| std::io::Error::new(e.kind(), format!("{program}: {e}")))?;
        let input = Input::Pipe(child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?);
        for out in [child.stdout.take().map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)].into_iter().flatten() {
            let tx = tx.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt as _;
                let mut out = out;
                let mut buf = [0u8; 4096];
                while let Ok(n) = out.read(&mut buf).await {
                    if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                        break;
                    }
                }
            });
        }
        return Ok(Proc { input, chunks, child });
    }
    let (mut m, mut s): (libc::c_int, libc::c_int) = (0, 0);
    // Wide, so that a long page address is never broken over lines.
    let mut size = libc::winsize { ws_row: 50, ws_col: 1000, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: plain libc calls on descriptors this function just made and owns.
    unsafe {
        if libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    // SAFETY: both descriptors are fresh from `openpty`.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    // SAFETY: as above. Closed on exec, so the child holds only its own copies;
    // no echo (what the person pastes is never printed back) and no line limit
    // (a pasted code may be long).
    unsafe {
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut t) == 0 {
            t.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON);
            t.c_cc[libc::VMIN] = 1;
            t.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &t);
        }
    }
    cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    // SAFETY: only async-signal-safe calls between fork and exec: its own session, the pty as its terminal.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| std::io::Error::new(e.kind(), format!("{program}: {e}")))?;
    // The parent's copies of the terminal's far end go with the command.
    drop(cmd);
    let mut reader = std::fs::File::from(master.try_clone()?);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(Proc { input: Input::Pty(std::fs::File::from(master)), chunks, child })
}

/// How a sign-in ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The link or the code ran out (the tool said so, or nobody came in time).
    Expired,
    Failed(String),
}

/// Run the sign-in `program args` until it ends. `on_seen` hears the page and
/// the code as they become known; `codes` is what the person pastes, written
/// to the tool's standard input as one line and dropped — never kept, never
/// in a log.
pub async fn drive(kind: Kind, program: &str, args: &[String], env: &[(String, String)], wait: Duration, mut codes: UnboundedReceiver<PastedCode>, mut on_seen: impl FnMut(Option<&str>, Option<&str>)) -> Outcome {
    let mut proc = match start(kind.tty(), program, args, env) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let mut reader = Reader::new(kind);
    let deadline = tokio::time::Instant::now() + wait;
    let mut codes_open = true;
    loop {
        tokio::select! {
            chunk = proc.chunks.recv() => {
                let Some(chunk) = chunk else { break };
                // A terminal is asked where its cursor is; a prompt library waits for the answer.
                if chunk.contains("\x1b[6n") {
                    proc.input.send(b"\x1b[24;80R").await;
                }
                if reader.feed(&chunk) {
                    on_seen(reader.url.as_deref(), reader.code.as_deref());
                }
                // "Press Enter to open … in your browser": Enter (its browser is `echo`).
                if reader.take_enter() {
                    proc.input.send(b"\n").await;
                }
            }
            code = codes.recv(), if codes_open => {
                match code {
                    Some(code) => {
                        let line = code.0.trim().replace(['\r', '\n'], "");
                        proc.input.send(line.as_bytes()).await;
                        proc.input.send(b"\n").await;
                    }
                    None => codes_open = false,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = proc.child.kill().await;
                return Outcome::Expired;
            }
        }
    }
    let status = match tokio::time::timeout_at(deadline, proc.child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Outcome::Failed(e.to_string()),
        Err(_) => {
            let _ = proc.child.kill().await;
            return Outcome::Expired;
        }
    };
    if status.success() {
        Outcome::Done
    } else if reader.says_expired() {
        Outcome::Expired
    } else {
        Outcome::Failed(reader.last_line())
    }
}

/// As whom the tool is signed in now: its own whoami's first line, short.
pub async fn whoami(plan: &Plan) -> Option<String> {
    let mut cmd = tokio::process::Command::new(&plan.whoami.0);
    cmd.args(&plan.whoami.1).envs(plan.env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = clean(&text).lines().map(str::trim).find(|l| !l.is_empty() && *l != "None" && *l != "(unset)")?.to_owned();
    Some(line.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded from the real tools: aws_login_remote (aws-cli 2.34), gh_web (gh 2.86, over pipes),
    // gcloud_no_launch_browser (gcloud 576); their one-time codes and state values are long expired.
    // Written from the tools' documentation, not recorded (no SSO session or `az` at hand):
    // aws_sso_device and az_device_code.
    const AWS_REMOTE: &str = include_str!("../tests/fixtures/cli_signin/aws_login_remote.txt");
    const AWS_SSO: &str = include_str!("../tests/fixtures/cli_signin/aws_sso_device.txt");
    const GH: &str = include_str!("../tests/fixtures/cli_signin/gh_web.txt");
    const GCLOUD: &str = include_str!("../tests/fixtures/cli_signin/gcloud_no_launch_browser.txt");
    const AZ: &str = include_str!("../tests/fixtures/cli_signin/az_device_code.txt");

    fn read(kind: Kind, sample: &str) -> Reader {
        // Fed in uneven pieces, as a terminal delivers it.
        let mut r = Reader::new(kind);
        for piece in sample.as_bytes().chunks(37) {
            r.feed(&String::from_utf8_lossy(piece));
        }
        r
    }

    #[test]
    fn aws_remote_gives_a_page_and_wants_a_code_back() {
        let r = read(Kind::Aws, AWS_REMOTE);
        let url = r.url.unwrap();
        assert!(url.starts_with("https://") && url.contains("signin.aws.amazon.com") && url.len() > 200, "{url}");
        assert!(!url.ends_with(['.', ' ']));
        assert_eq!(r.code, None, "nothing to confirm: the code is pasted back");
        assert!(Kind::Aws.wants_code());
    }

    #[test]
    fn aws_sso_device_flow_gives_a_page_and_a_code_to_confirm() {
        let r = read(Kind::AwsSso, AWS_SSO);
        assert_eq!(r.url.as_deref(), Some("https://device.sso.us-east-1.amazonaws.com/?user_code=KQRD-HMFB"));
        assert_eq!(r.code.as_deref(), Some("KQRD-HMFB"));
        assert!(!Kind::AwsSso.wants_code());
    }

    #[test]
    fn github_gives_its_device_page_and_code() {
        let r = read(Kind::Gh, GH);
        assert_eq!(r.url.as_deref(), Some("https://github.com/login/device"));
        assert_eq!(r.code.as_deref(), Some("2D53-6413"));
    }

    #[test]
    fn google_cloud_gives_a_long_page_and_wants_a_code_back() {
        let r = read(Kind::Gcloud, GCLOUD);
        let url = r.url.unwrap();
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/auth?") && url.contains("redirect_uri=") && url.len() > 300, "{url}");
        assert_eq!(r.code, None);
    }

    #[test]
    fn azure_gives_its_device_page_and_code() {
        let r = read(Kind::Az, AZ);
        assert_eq!(r.url.as_deref(), Some("https://login.microsoft.com/device"));
        assert_eq!(r.code.as_deref(), Some("HQ4X7ZB2K"));
    }

    #[test]
    fn a_half_printed_address_is_not_taken() {
        let mut r = Reader::new(Kind::Aws);
        assert!(!r.feed("Open https://signin.aws.amazon.com/v1/auth"));
        assert_eq!(r.url, None);
        assert!(r.feed("orize?x=1\nPaste the code: "));
        assert_eq!(r.url.as_deref(), Some("https://signin.aws.amazon.com/v1/authorize?x=1"));
    }

    #[test]
    fn colours_and_carriage_returns_are_taken_out() {
        assert_eq!(clean("\x1b[1mhello\x1b[0m\r\nthere\r\x1b]0;title\x07done"), "hello\nthere\ndone");
    }

    #[test]
    fn the_commands_are_each_tools_own_cross_device_mode() {
        let aws = plan(Kind::Aws, Some("atinum"), &[]).unwrap();
        assert_eq!((aws.program.as_str(), aws.args.join(" ").as_str()), ("aws", "login --remote --profile atinum"));
        assert_eq!(aws.whoami.1.join(" "), "sts get-caller-identity --query Arn --output text --profile atinum");
        let sso = plan(Kind::AwsSso, Some("atinum"), &[]).unwrap();
        assert_eq!(sso.args.join(" "), "sso login --no-browser --use-device-code --profile atinum");
        assert!(plan(Kind::Gh, None, &[]).unwrap().args.join(" ").starts_with("auth login --web"));
        assert!(plan(Kind::Gcloud, None, &[]).unwrap().args.contains(&"--no-launch-browser".to_owned()));
        assert_eq!(plan(Kind::Az, None, &[]).unwrap().args.join(" "), "login --use-device-code");
        assert!(plan(Kind::Gh, Some("x"), &[]).is_err(), "gh has no profiles");
        // Never a token, a password, a key; never a different mode.
        for bad in ["--with-token", "--password", "--service-account-key", "--cred-file", "--profile"] {
            assert!(plan(Kind::Gcloud, None, &[bad.to_owned()]).is_err(), "{bad}");
        }
        assert!(plan(Kind::Aws, Some("a b"), &[]).is_err());
        assert!(plan(Kind::Aws, None, &["$(x)".into()]).is_err());
    }

    #[test]
    fn a_call_is_checked() {
        let ok = check(&json!({ "tool": "AWS", "why": "deploy", "profile": "atinum" })).unwrap();
        assert_eq!((ok.kind, ok.profile.as_deref()), (Kind::Aws, Some("atinum")));
        assert!(check(&json!({ "tool": "vercel", "why": "x" })).unwrap_err().contains("block_on_person"));
        assert!(check(&json!({ "tool": "gh" })).unwrap_err().contains("why"));
    }

    #[test]
    fn the_tool_names_its_modes_and_never_a_password() {
        let d = tool()["description"].as_str().unwrap().to_owned();
        assert!(d.contains("--remote") && d.contains("never see") && d.contains("Never ask for a password"), "{d}");
    }

    // ── a fake CLI, in a real pseudo-terminal ──

    fn fake(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("equipe-cli-signin-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fakecli");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    const SECRET: &str = "4/0AX4XfWh-very-secret-code";

    #[tokio::test]
    async fn a_code_pasted_goes_to_the_waiting_process_and_nowhere_else() {
        let script = fake("code", r#"echo 'Open this page: https://signin.example.com/authorize?x=1'; printf 'Enter code: '; read code; if [ "$code" = '4/0AX4XfWh-very-secret-code' ]; then echo "welcome"; exit 0; fi; echo "wrong"; exit 3"#);
        let (tx, rx) = unbounded_channel();
        let mut seen: Vec<(Option<String>, Option<String>)> = Vec::new();
        let sent = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            tx.send(PastedCode(SECRET.into())).unwrap();
            tx
        });
        let outcome = drive(Kind::Aws, script.to_str().unwrap(), &[], &[], Duration::from_secs(20), rx, |u, c| seen.push((u.map(str::to_owned), c.map(str::to_owned)))).await;
        let _keep = sent.await.unwrap();
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(seen[0].0.as_deref(), Some("https://signin.example.com/authorize?x=1"));
        // The terminal's echo is off: the code isn't in anything equipe read.
        assert!(!format!("{seen:?}{outcome:?}").contains("very-secret"));
        // And nothing was written for it: the script's folder holds only the script.
        let left: Vec<_> = std::fs::read_dir(script.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 1);
    }

    #[tokio::test]
    async fn a_wrong_code_fails_and_the_tools_last_line_is_the_reason() {
        let script = fake("wrong", "echo 'https://signin.example.com/a '; printf 'Enter code: '; read code; echo; echo 'Invalid code'; exit 3");
        let (tx, rx) = unbounded_channel();
        tx.send(PastedCode("nope".into())).unwrap();
        let outcome = drive(Kind::Gcloud, script.to_str().unwrap(), &[], &[], Duration::from_secs(20), rx, |_, _| {}).await;
        drop(tx);
        assert_eq!(outcome, Outcome::Failed("Invalid code".into()));
    }

    #[tokio::test]
    async fn waiting_too_long_is_expiry() {
        let script = fake("slow", "echo 'https://signin.example.com/a '; exec sleep 30");
        let (_tx, rx) = unbounded_channel();
        let outcome = drive(Kind::Gh, script.to_str().unwrap(), &[], &[], Duration::from_millis(700), rx, |_, _| {}).await;
        assert_eq!(outcome, Outcome::Expired);
    }

    #[tokio::test]
    async fn a_tool_saying_the_code_expired_is_expiry() {
        let script = fake("expired", "echo 'https://github.com/login/device'; echo 'X-Error: device_code expired'; exit 1");
        let (_tx, rx) = unbounded_channel();
        let outcome = drive(Kind::Gh, script.to_str().unwrap(), &[], &[], Duration::from_secs(20), rx, |_, _| {}).await;
        assert_eq!(outcome, Outcome::Expired);
    }

    #[tokio::test]
    async fn a_device_flow_reports_its_code_and_presses_enter_for_the_tool() {
        let script = fake("gh", "echo '! First copy your one-time code: 8A3F-21C9'; printf 'Press Enter to open https://github.com/login/device in your browser... '; read x; echo 'Logged in as abel'");
        let (_tx, rx) = unbounded_channel();
        let mut last = (None, None);
        let outcome = drive(Kind::Gh, script.to_str().unwrap(), &[], &[], Duration::from_secs(20), rx, |u, c| last = (u.map(str::to_owned), c.map(str::to_owned))).await;
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(last, (Some("https://github.com/login/device".into()), Some("8A3F-21C9".into())));
    }

    #[tokio::test]
    async fn whoami_is_the_first_line_of_the_tools_own_answer() {
        let script = fake("who", "echo 'arn:aws:iam::123456789012:user/atinum-admin'");
        let plan = Plan { program: String::new(), args: Vec::new(), env: Vec::new(), whoami: (script.to_string_lossy().into_owned(), Vec::new()) };
        assert_eq!(whoami(&plan).await.as_deref(), Some("arn:aws:iam::123456789012:user/atinum-admin"));
    }
}
