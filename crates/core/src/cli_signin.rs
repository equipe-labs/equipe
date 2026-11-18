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
//!
//! The same card signs the person in to an outside MCP server the squad
//! connected (`crate::squad_mcp`): the engine's own `claude mcp login
//! --no-browser` or `codex mcp login`. Those are equipe's to start, never a
//! member's to ask for.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
    /// `claude mcp login <server> --no-browser`: an outside MCP server, for
    /// Claude Code members. The page's redirect finishes it by itself on the
    /// daemon's machine; elsewhere the person pastes the address the page
    /// ended on.
    ClaudeMcp,
    /// `codex mcp login <server>`: an outside MCP server, for Codex members.
    CodexMcp,
    /// equipe's own sign-in to an outside MCP server ([`crate::mcp_oauth`]),
    /// for every engine: no tool runs; its page's redirect comes back to the
    /// daemon's loopback address, or the person pastes where it ended.
    EquipeMcp,
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
            Self::ClaudeMcp => "claude_mcp",
            Self::CodexMcp => "codex_mcp",
            Self::EquipeMcp => "equipe_mcp",
        }
    }

    /// An outside MCP server's sign-in: started by equipe, never asked for by a member.
    pub fn is_mcp(self) -> bool {
        matches!(self, Self::ClaudeMcp | Self::CodexMcp | Self::EquipeMcp)
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
            Self::ClaudeMcp => "Claude Code",
            Self::CodexMcp => "Codex",
            Self::EquipeMcp => "equipe",
        }
    }

    pub fn program(self) -> &'static str {
        match self {
            Self::Aws | Self::AwsSso => "aws",
            Self::Gh => "gh",
            Self::Gcloud => "gcloud",
            Self::Az => "az",
            Self::ClaudeMcp => "claude",
            Self::CodexMcp => "codex",
            Self::EquipeMcp => "equipe",
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
        matches!(self, Self::Aws | Self::Gcloud | Self::ClaudeMcp | Self::EquipeMcp)
    }

    /// What ends a pasted line: Claude Code reads its terminal raw, where
    /// Enter is a carriage return.
    fn enter(self) -> &'static [u8] {
        if self == Self::ClaudeMcp { b"\r" } else { b"\n" }
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
    /// Where it runs, when that matters (Claude Code finds a local-scope server by its folder).
    pub cwd: Option<std::path::PathBuf>,
}

fn word_safe(w: &str) -> bool {
    !w.is_empty() && w.len() <= 80 && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:@".contains(c))
}

/// The extra flags a member may add for `kind`, each with whether it takes a
/// value: only what an ordinary interactive sign-in needs. Everything else —
/// a token, a scope, another host, a storage or mode switch — is refused.
/// `gh` takes none: its host is always github.com, its scopes gh's own.
fn allowed_flags(kind: Kind) -> &'static [(&'static str, bool)] {
    match kind {
        Kind::Aws => &[("--region", true)],
        Kind::AwsSso => &[("--region", true), ("--sso-session", true)],
        Kind::Gcloud => &[("--project", true), ("--update-adc", false), ("--force", false)],
        Kind::Az => &[("--tenant", true), ("--allow-no-subscriptions", false)],
        Kind::Gh | Kind::ClaudeMcp | Kind::CodexMcp | Kind::EquipeMcp => &[],
    }
}

/// `extra`, checked against [`allowed_flags`]: each a flag of the list, its
/// value (when it takes one) the next item or after `=`, a plain word.
fn check_extra(kind: Kind, extra: &[String]) -> Result<(), String> {
    let allowed = allowed_flags(kind);
    let list = if allowed.is_empty() { "none".to_owned() } else { allowed.iter().map(|(f, v)| if *v { format!("`{f} <value>`") } else { format!("`{f}`") }).collect::<Vec<_>>().join(", ") };
    let refused = |a: &str| format!("`{a}` isn't allowed for {}: equipe runs only the tool's own cross-device sign-in, and never handles tokens, passwords or keys. Extra flags allowed: {list}.", kind.program());
    let value_ok = |v: &str| word_safe(v) && !v.starts_with('-');
    let mut items = extra.iter();
    while let Some(a) = items.next() {
        if !word_safe(a) {
            return Err(format!("`{a}` isn't a plain flag or value."));
        }
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (a.as_str(), None),
        };
        let Some(&(_, takes_value)) = allowed.iter().find(|(f, _)| *f == flag) else { return Err(refused(a)) };
        match (takes_value, inline) {
            (false, None) => {}
            (false, Some(_)) => return Err(format!("`{flag}` takes no value.")),
            (true, Some(v)) if value_ok(v) => {}
            (true, Some(_)) => return Err(format!("`{a}`: `{flag}` takes a plain value.")),
            (true, None) => match items.next() {
                Some(v) if value_ok(v) => {}
                _ => return Err(format!("`{flag}` needs a plain value after it.")),
            },
        }
    }
    Ok(())
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
    if !kind.is_mcp() {
        check_extra(kind, extra)?;
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
        Kind::ClaudeMcp | Kind::CodexMcp | Kind::EquipeMcp => return Err("An MCP server's sign-in is equipe's to start (`crate::squad_mcp`).".into()),
    }
    if let (Some(p), Kind::Aws | Kind::AwsSso) = (profile, kind) {
        args.extend(["--profile".to_owned(), p.to_owned()]);
        whoami_args.extend(["--profile".to_owned(), p.to_owned()]);
    }
    args.extend(extra.iter().cloned());
    // Always github.com: another host isn't a member's to pick.
    if kind == Kind::Gh {
        args.extend(s(&["--hostname", "github.com"]));
    }
    Ok(Plan { program: kind.program().to_owned(), args, env, whoami: (kind.program().to_owned(), whoami_args), cwd: None })
}

/// A call, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub kind: Kind,
    pub profile: Option<String>,
    pub extra: Vec<String>,
    pub why: String,
    /// The permission the machine's own credentials lack, when the member says
    /// one (`crate::machine_creds::guard`).
    pub why_not_machine: Option<String>,
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
    let why_not_machine = args.get("why_not_machine").and_then(Value::as_str).map(str::trim).filter(|w| !w.is_empty()).map(str::to_owned);
    Ok(Asked { kind, profile, extra, why: why.to_owned(), why_not_machine })
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
only when no machine credential exists (a GitHub App token, an access key already set up, an environment variable). It is \
refused while a credential for that tool already works here without the person (your briefing lists them: an AWS profile \
with `credential_process` or a role, an unexpired SSO sign-in, `gh` signed in): use it, and only if it lacks a permission \
you need, call again with `why_not_machine` naming that permission. Never ask for \
a password in chat. tool: aws (`aws login --remote`, the console sign-in; needs `profile` when the profile isn't the default), aws_sso \
(`aws sso login --use-device-code`, for an SSO profile), gh (`gh auth login --web`), gcloud (`gcloud auth login --no-launch-browser`; \
`profile` names the gcloud configuration), az (`az login --use-device-code`). Other tools have no flow equipe runs: use \
`block_on_person` with the exact steps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool": { "type": "string", "enum": ["aws", "aws_sso", "gh", "gcloud", "az"], "description": "Which tool to sign in." },
                "why": { "type": "string", "description": "What it is for, in a line in the person's language: \"deploy the staging stack\"." },
                "profile": { "type": "string", "description": "The AWS profile to sign in (`my-console`), or the gcloud configuration. Leave out for the default, and for gh and az." },
                "why_not_machine": { "type": "string", "description": "Only when this machine already has a working credential for the tool: the exact permission it lacks and the error you got with it (\"iam:CreateRole AccessDenied with every listed profile\"). Without it, the call is refused while one works." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Rare extra flags, one per item. Only these: aws `--region`; aws_sso `--region`, `--sso-session`; gcloud `--project`, `--update-adc`, `--force`; az `--tenant`, `--allow-no-subscriptions`; gh none. A flag's value is the next item (`--region`, `us-east-1`). Never tokens, passwords or keys." },
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

// ── where a sign-in page's last step lands ──

/// `%xx` decoded (and `+` as a space), for one query value.
fn unpct(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() => {
                let hex = |b: u8| (b as char).to_digit(16).unwrap_or_default() as u8;
                out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
                i += 3;
                continue;
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The port a sign-in page sends its last step back to on the machine the
/// tool runs on: its `redirect_uri` when that is a loopback address
/// (`http://localhost:<port>/…`, `http://127.0.0.1:<port>/…`, `http://[::1]:<port>/…`),
/// percent-encoded or not. None: the page finishes elsewhere (a device code,
/// a code to paste back), so it opens on any machine as it is.
pub fn loopback_port(url: &str) -> Option<u16> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or_default();
    let value = query.split('&').find_map(|pair| pair.strip_prefix("redirect_uri="))?;
    let redirect = unpct(value);
    let rest = redirect.strip_prefix("http://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (host, port) = authority.rsplit_once(':')?;
    if !matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return None;
    }
    port.parse::<u16>().ok().filter(|p| *p > 0)
}

/// How a screen opens a sign-in page so that its last step reaches the tool
/// waiting on the daemon's machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// In this Mac's browser, as it is: the daemon is here, or the page
    /// doesn't come back to a loopback address.
    Here,
    /// The loopback port forwarded over the screen's SSH connection to the
    /// daemon's machine for the sign-in, then the page opened here: its
    /// redirect lands on the daemon's machine through the tunnel.
    Forward(u16),
    /// In the daemon machine's own browser, the one the browser panel shows:
    /// its redirect lands there by itself.
    Panel,
    /// Opened here; the address the page ends on is pasted back on the card.
    Paste,
}

/// The way to open a sign-in page `url`: `remote`, the daemon is on another
/// machine; `forward`, the screen can forward a port to it (an SSH
/// connection, the port free here); `panel`, the daemon's browser can be shown.
pub fn route(url: &str, remote: bool, forward: bool, panel: bool) -> Route {
    match (remote, loopback_port(url)) {
        (false, _) | (true, None) => Route::Here,
        (true, Some(port)) if forward => Route::Forward(port),
        (true, Some(_)) if panel => Route::Panel,
        (true, Some(_)) => Route::Paste,
    }
}

// ── the process, in a pseudo-terminal (or pipes) ──

enum Input {
    #[cfg(unix)]
    Pty(std::fs::File),
    Pipe(tokio::process::ChildStdin),
}

impl Input {
    async fn send(&mut self, bytes: &[u8]) {
        use tokio::io::AsyncWriteExt as _;
        match self {
            #[cfg(unix)]
            Input::Pty(f) => {
                use std::io::Write as _;
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
fn start(tty: bool, program: &str, args: &[String], env: &[(String, String)], cwd: Option<&std::path::Path>) -> std::io::Result<Proc> {
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
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
    in_pty(cmd, program, tx, chunks)
}

/// Start `cmd` with a pseudo-terminal as its terminal: its own session, the
/// terminal's echo off.
#[cfg(unix)]
fn in_pty(mut cmd: tokio::process::Command, program: &str, tx: UnboundedSender<String>, chunks: UnboundedReceiver<String>) -> std::io::Result<Proc> {
    use std::io::Read as _;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::process::Stdio;
    let (mut m, mut s): (libc::c_int, libc::c_int) = (0, 0);
    // Wide, so that a long page address is never broken over lines.
    let mut size = libc::winsize { ws_row: 50, ws_col: 1000, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: plain libc calls on descriptors this function just made and owns.
    // The size goes as a raw pointer: macOS's `openpty` takes it `*mut`, Linux's `*const`.
    unsafe {
        if libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut size) != 0 {
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

/// No pseudo-terminals here: the tools that need one to sign in can't be
/// driven, and say so.
#[cfg(not(unix))]
fn in_pty(_cmd: tokio::process::Command, program: &str, _tx: UnboundedSender<String>, _chunks: UnboundedReceiver<String>) -> std::io::Result<Proc> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("{program}: signing in from a terminal of its own works on macOS and Linux only")))
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
pub async fn drive(kind: Kind, program: &str, args: &[String], env: &[(String, String)], wait: Duration, codes: UnboundedReceiver<PastedCode>, on_seen: impl FnMut(Option<&str>, Option<&str>)) -> Outcome {
    drive_in(kind, None, program, args, env, wait, codes, on_seen).await
}

/// [`drive`], run in `cwd`.
#[allow(clippy::too_many_arguments)]
pub async fn drive_in(kind: Kind, cwd: Option<&std::path::Path>, program: &str, args: &[String], env: &[(String, String)], wait: Duration, mut codes: UnboundedReceiver<PastedCode>, mut on_seen: impl FnMut(Option<&str>, Option<&str>)) -> Outcome {
    let mut proc = match start(kind.tty(), program, args, env, cwd) {
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
                        proc.input.send(kind.enter()).await;
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

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn gh_refuses_scopes_insecure_storage_another_host_and_a_token() {
        for bad in [&["--scopes", "admin:org"][..], &["--scopes=repo"], &["--insecure-storage"], &["--hostname", "evil.example.com"], &["--hostname=evil.example.com"], &["-h", "evil.example.com"], &["--with-token"], &["--git-protocol", "ssh"], &["github.com"]] {
            let err = plan(Kind::Gh, None, &v(bad)).unwrap_err();
            assert!(err.contains("isn't allowed for gh") && err.contains("allowed: none"), "{bad:?}: {err}");
        }
        assert!(check(&json!({ "tool": "gh", "why": "push", "args": ["--insecure-storage"] })).is_err());
        // With nothing extra: github.com, the web flow, as always.
        assert_eq!(plan(Kind::Gh, None, &[]).unwrap().args.join(" "), "auth login --web --git-protocol https --skip-ssh-key --hostname github.com");
    }

    #[test]
    fn each_tools_own_allowed_flags_still_pass() {
        assert_eq!(plan(Kind::Aws, Some("atinum"), &v(&["--region", "us-east-1"])).unwrap().args.join(" "), "login --remote --profile atinum --region us-east-1");
        assert!(plan(Kind::AwsSso, None, &v(&["--sso-session", "corp", "--region=eu-west-1"])).is_ok());
        assert!(plan(Kind::Gcloud, Some("work"), &v(&["--project", "my-proj-123", "--update-adc", "--force"])).is_ok());
        assert_eq!(plan(Kind::Az, None, &v(&["--tenant", "contoso.onmicrosoft.com", "--allow-no-subscriptions"])).unwrap().args.join(" "), "login --use-device-code --tenant contoso.onmicrosoft.com --allow-no-subscriptions");
        assert!(check(&json!({ "tool": "az", "why": "deploy", "args": ["--tenant", "0000-1111"] })).is_ok());
        // Another tool's flag, a missing or flag-shaped value, a value on a switch: refused.
        assert!(plan(Kind::Aws, None, &v(&["--tenant", "x"])).is_err());
        assert!(plan(Kind::Aws, None, &v(&["--region"])).is_err());
        assert!(plan(Kind::Aws, None, &v(&["--region", "--profile"])).is_err());
        assert!(plan(Kind::Gcloud, None, &v(&["--force=yes"])).is_err());
        assert!(plan(Kind::Gcloud, None, &v(&["--enable-gdrive-access"])).is_err());
        assert!(plan(Kind::Az, None, &v(&["--service-principal"])).is_err());
    }

    // Recorded from Claude Code 2.1.308 (`claude mcp login mobbin --no-browser`, in a terminal);
    // its state and challenge long expired.
    const CLAUDE_MCP: &str = include_str!("../tests/fixtures/cli_signin/claude_mcp_login.txt");

    #[test]
    fn claude_codes_mcp_login_gives_its_authorization_page_and_wants_the_redirect_back() {
        let r = read(Kind::ClaudeMcp, CLAUDE_MCP);
        let url = r.url.unwrap();
        assert!(url.starts_with("https://") && url.contains("/oauth/authorize?") && url.contains("redirect_uri=http%3A%2F%2Flocalhost"), "{url}");
        assert_eq!(r.code, None);
        assert!(Kind::ClaudeMcp.wants_code() && !Kind::CodexMcp.wants_code());
        assert!(Kind::ClaudeMcp.is_mcp() && !Kind::Gh.is_mcp());
        // Members never start one: not a tool word, and no plan of `cli_sign_in`'s.
        assert!(Kind::parse("claude_mcp").is_none() && Kind::parse("codex_mcp").is_none());
        assert!(plan(Kind::ClaudeMcp, None, &[]).is_err());
    }

    #[tokio::test]
    async fn a_redirect_pasted_into_claude_code_ends_with_a_carriage_return_in_its_folder() {
        // Claude Code reads raw: the line is taken at a carriage return. And it runs where its server is configured.
        // `dd bs=1` reads a byte at a time, as Claude Code takes the line: a buffered reader
        // (macOS's `head -c`) would swallow the carriage return with the address when both are waiting.
        let script = fake("claude-mcp", r#"stty -icrnl; pwd > where; echo 'Visit this URL to authorize:'; echo '  https://auth.example.com/oauth/authorize?state=x '; printf 'Or paste the redirect URL here: '; url=$(dd bs=1 count=47 2>/dev/null); end=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' '); case "$end $url" in "0d http://localhost:"*code=*) echo 'Authenticated with "mobbin".'; exit 0 ;; esac; echo "bad: $end"; exit 2"#);
        let dir = script.parent().unwrap().to_path_buf();
        let (tx, rx) = unbounded_channel();
        // Pasted once the page is on the card, as the person would: the tool printed it after setting its terminal up.
        let mut tx = Some(tx);
        let outcome = drive_in(Kind::ClaudeMcp, Some(&dir), "/bin/bash", &[script.to_string_lossy().into_owned()], &[], Duration::from_secs(20), rx, |url, _| {
            if url.is_some()
                && let Some(tx) = tx.take()
            {
                tx.send(PastedCode("http://localhost:5555/callback?code=abc&state=x".into())).unwrap();
            }
        })
        .await;
        assert_eq!(outcome, Outcome::Done);
        let ran_in = std::fs::read_to_string(dir.join("where")).unwrap();
        assert_eq!(std::path::Path::new(ran_in.trim()).canonicalize().unwrap(), dir.canonicalize().unwrap());
    }

    #[test]
    fn a_pages_loopback_redirect_port_is_read_encoded_or_not() {
        let r = read(Kind::ClaudeMcp, CLAUDE_MCP);
        assert!(loopback_port(&r.url.unwrap()).is_some_and(|p| p > 1024), "Claude Code's own page comes back to localhost");
        assert_eq!(loopback_port("https://auth.example.com/authorize?client_id=x&redirect_uri=http%3A%2F%2Flocalhost%3A55531%2Fcallback&state=s"), Some(55531));
        assert_eq!(loopback_port("https://accounts.google.com/o/oauth2/v2/auth?client_id=x&redirect_uri=http%3A%2F%2F127.0.0.1%3A53682&scope=y"), Some(53682));
        assert_eq!(loopback_port("https://a.example.com/authorize?redirect_uri=http://[::1]:4100/cb"), Some(4100));
        // Elsewhere: no tunnel, the page opens as it is.
        for none in [
            "https://github.com/login/device",
            "https://device.sso.us-east-1.amazonaws.com/?user_code=KQRD-HMFB",
            "https://accounts.google.com/o/oauth2/auth?redirect_uri=https%3A%2F%2Fsdk.cloud.google.com%2Fauthcode.html",
            "https://a.example.com/authorize?redirect_uri=http%3A%2F%2Fevil.example.com%3A80%2Fcb",
            "https://a.example.com/authorize?redirect_uri=http%3A%2F%2Flocalhost%2Fcb",
            "https://a.example.com/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A0%2Fcb",
            "https://a.example.com/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A99999%2Fcb",
        ] {
            assert_eq!(loopback_port(none), None, "{none}");
        }
    }

    #[test]
    fn a_page_opens_here_through_a_tunnel_in_the_panel_or_with_a_paste_box() {
        let mcp = "https://auth.example.com/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A55531%2Fcallback";
        let device = "https://github.com/login/device";
        // The daemon on this Mac: always here.
        assert_eq!(route(mcp, false, true, true), Route::Here);
        // On another Mac: a device page opens here as it is.
        assert_eq!(route(device, true, true, true), Route::Here);
        // A loopback redirect: through the SSH tunnel where it can be made...
        assert_eq!(route(mcp, true, true, true), Route::Forward(55531));
        // ...else in the daemon's own browser, in the panel...
        assert_eq!(route(mcp, true, false, true), Route::Panel);
        // ...else here, the address it ends on pasted back.
        assert_eq!(route(mcp, true, false, false), Route::Paste);
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
        // A folder of its own per call: tests run side by side, and two may fake the same tool.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("equipe-cli-signin-{}-{name}-{n}", std::process::id()));
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
        let plan = Plan { program: String::new(), args: Vec::new(), env: Vec::new(), whoami: (script.to_string_lossy().into_owned(), Vec::new()), cwd: None };
        assert_eq!(whoami(&plan).await.as_deref(), Some("arn:aws:iam::123456789012:user/atinum-admin"));
    }
}
