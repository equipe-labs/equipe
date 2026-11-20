//! Machine credentials: which command-line tools on this machine already have
//! a credential that works without the person, by name and shape only.
//!
//! Members are told in one line ([`summary`]) so they pick a working profile
//! first, and `cli_sign_in` refuses a card for a tool that already has one
//! ([`guard`]) unless the member names the permission those lack.
//!
//! Nothing secret is ever kept: AWS config files are read for section names
//! and the keys that give a profile its shape (a secret key's line only counts
//! as present, its value is dropped as it is read); SSO caches give their start
//! address and expiry only (their token fields are never deserialized);
//! docker's config gives registry names; `gh`, `az` and `npm` are asked only for
//! an exit code, `gcloud` and `kubectl` for an account kind and context names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::cli_signin::Kind;

/// How long the files' reading holds before it is read again.
pub const FILES_TTL: Duration = Duration::from_secs(5 * 60);
/// How long the tools' answers hold before they are asked again.
pub const PROBE_TTL: Duration = Duration::from_secs(10 * 60);
/// How long one tool may take to answer (`npm whoami` asks its registry).
const PROBE_WAIT: Duration = Duration::from_secs(10);
/// The most AWS profiles named in the line.
const MAX_LISTED: usize = 12;
/// The shortest `why_not_machine` that can name a permission.
const MIN_WHY_NOT: usize = 10;
/// The largest SSO cache file read: a real one is a few KiB.
const MAX_CACHE_FILE: u64 = 64 * 1024;

// ── AWS ──

/// How an AWS profile gets its credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwsShape {
    /// `credential_process`: a helper hands credentials out by itself.
    Process,
    /// `role_arn` assumed from a source that needs no one.
    Role,
    /// `role_arn` assumed from a profile that needs a sign-in.
    RoleFromInteractive,
    /// `role_arn` with `web_identity_token_file`.
    WebIdentity,
    /// An access key kept in a file.
    Static,
    /// IAM Identity Center, with a cached sign-in that hasn't run out.
    SsoSignedIn,
    /// IAM Identity Center, its cached sign-in run out or never made.
    SsoExpired,
    /// `aws login`'s console sign-in (`login_session`): it runs out.
    Console,
}

impl AwsShape {
    /// Works now without the person.
    pub fn non_interactive(self) -> bool {
        matches!(self, Self::Process | Self::Role | Self::WebIdentity | Self::Static | Self::SsoSignedIn)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Process | Self::Role | Self::WebIdentity | Self::Static => "non-interactive",
            Self::SsoSignedIn => "SSO, signed in",
            Self::SsoExpired => "interactive, expired",
            Self::RoleFromInteractive | Self::Console => "interactive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsProfile {
    pub name: String,
    pub shape: AwsShape,
}

impl AwsProfile {
    /// "read" / "deploy" / "admin" / "write", where the name plainly says it.
    pub fn access(&self) -> Option<&'static str> {
        access_hint(&self.name)
    }
}

fn access_hint(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    for word in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        match word {
            "read" | "readonly" | "reader" | "ro" | "viewer" | "view" => return Some("read"),
            "deploy" | "deployer" => return Some("deploy"),
            "admin" => return Some("admin"),
            "write" | "writer" | "rw" => return Some("write"),
            _ => {}
        }
    }
    None
}

/// A section's keys; a secret one's value is never kept (an empty string stands for it).
type Section = BTreeMap<String, String>;

fn secret_key(key: &str) -> bool {
    key == "aws_access_key_id" || key.contains("secret") || key.contains("token") || key.contains("password")
}

/// An AWS config (`[profile x]`, `[default]`, `[sso-session x]`) or credentials
/// file (`[x]`): its profiles and SSO sessions by name.
fn ini(text: &str, credentials: bool) -> (BTreeMap<String, Section>, BTreeMap<String, Section>) {
    let (mut profiles, mut sessions) = (BTreeMap::<String, Section>::new(), BTreeMap::<String, Section>::new());
    // Where the lines go: a profile, a session, or nowhere (`[services x]`).
    let mut at: Option<(bool, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let header = header.trim();
            at = if credentials {
                Some((false, header.to_owned()))
            } else if header == "default" {
                Some((false, "default".to_owned()))
            } else if let Some(name) = header.strip_prefix("profile ") {
                Some((false, name.trim().to_owned()))
            } else {
                header.strip_prefix("sso-session ").map(|name| (true, name.trim().to_owned()))
            };
            if let Some((session, name)) = &at {
                let book = if *session { &mut sessions } else { &mut profiles };
                book.entry(name.clone()).or_default();
            }
            continue;
        }
        // A nested setting (`s3 =` then indented lines) says nothing of credentials.
        if raw.starts_with([' ', '\t']) {
            continue;
        }
        let (Some((session, name)), Some((key, value))) = (&at, line.split_once('=')) else { continue };
        let key = key.trim().to_ascii_lowercase();
        let value = if secret_key(&key) { String::new() } else { value.trim().to_owned() };
        let section = if *session { sessions.get_mut(name) } else { profiles.get_mut(name) };
        if let Some(section) = section {
            section.insert(key, value);
        }
    }
    (profiles, sessions)
}

/// The profiles of an AWS config and credentials file, each with its shape;
/// `sso_valid` says whether the cached sign-in for an SSO start address hasn't
/// run out. Profiles that hold no credentials (a region only) are left out.
pub fn aws_profiles(config: &str, credentials: &str, sso_valid: &dyn Fn(&str) -> bool) -> Vec<AwsProfile> {
    let (mut profiles, sessions) = ini(config, false);
    for (name, keys) in ini(credentials, true).0 {
        profiles.entry(name).or_default().extend(keys);
    }
    profiles.keys().filter_map(|name| shape_of(name, &profiles, &sessions, sso_valid, 0).map(|shape| AwsProfile { name: name.clone(), shape })).collect()
}

fn shape_of(name: &str, profiles: &BTreeMap<String, Section>, sessions: &BTreeMap<String, Section>, sso_valid: &dyn Fn(&str) -> bool, depth: u8) -> Option<AwsShape> {
    let keys = profiles.get(name)?;
    let has = |k: &str| keys.contains_key(k);
    if has("role_arn") {
        if has("web_identity_token_file") {
            return Some(AwsShape::WebIdentity);
        }
        if has("credential_source") {
            return Some(AwsShape::Role);
        }
        let source = keys.get("source_profile")?;
        // A role off the profile's own keys, or a chain: as good as its source.
        let source_shape = if source == name {
            has("aws_access_key_id").then_some(AwsShape::Static)
        } else if depth < 5 {
            shape_of(source, profiles, sessions, sso_valid, depth + 1)
        } else {
            None
        };
        return Some(if source_shape.is_some_and(AwsShape::non_interactive) { AwsShape::Role } else { AwsShape::RoleFromInteractive });
    }
    if has("credential_process") {
        return Some(AwsShape::Process);
    }
    if has("aws_access_key_id") {
        return Some(AwsShape::Static);
    }
    if has("sso_session") || has("sso_start_url") {
        let url = keys.get("sso_start_url").or_else(|| keys.get("sso_session").and_then(|s| sessions.get(s)).and_then(|s| s.get("sso_start_url")));
        return Some(if url.is_some_and(|u| sso_valid(u)) { AwsShape::SsoSignedIn } else { AwsShape::SsoExpired });
    }
    has("login_session").then_some(AwsShape::Console)
}

/// What equipe reads of an SSO cache file: its start address and expiry. The
/// token fields beside them are skipped, never made into a value.
#[derive(Deserialize)]
struct SsoCache {
    #[serde(rename = "startUrl")]
    start_url: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<String>,
}

/// Each SSO start address in the cache folder with its latest expiry (seconds since the epoch).
pub fn sso_expiries(dir: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") || !entry.metadata().is_ok_and(|m| m.is_file() && m.len() <= MAX_CACHE_FILE) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(cache) = serde_json::from_slice::<SsoCache>(&bytes) else { continue };
        if let (Some(url), Some(at)) = (cache.start_url, cache.expires_at.as_deref().and_then(epoch)) {
            let best = out.entry(url).or_insert(at);
            *best = (*best).max(at);
        }
    }
    out
}

/// An ISO 8601 time as AWS writes it (`2026-11-19T10:00:00Z`, `…UTC`,
/// `….123Z`, `…+09:00`), in seconds since the epoch.
pub fn epoch(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut ymd = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let mut hms = rest.get(..8)?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let tail = rest[8..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match tail {
        "" | "Z" | "z" | "UTC" => 0,
        t if t.starts_with(['+', '-']) => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            let digits: String = t[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() != 4 {
                return None;
            }
            sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    let secs = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + s - offset;
    u64::try_from(secs).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ── the other tools ──

/// What one other tool said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolState {
    /// The program (`gh`, `gcloud`, `az`, `kubectl`, `docker`, `npm`).
    pub tool: &'static str,
    pub signed_in: bool,
    /// A kind or names, never an account or a secret: "service account", "contexts dev, prod".
    pub detail: Option<String>,
}

/// Docker's config: the registries it holds a login or a helper for, by name.
#[derive(Deserialize, Default)]
struct DockerConfig {
    #[serde(default)]
    auths: BTreeMap<String, IgnoredAny>,
    #[serde(default, rename = "credHelpers")]
    cred_helpers: BTreeMap<String, IgnoredAny>,
}

/// The registries docker's config names: hosts only.
pub fn docker_registries(text: &str) -> Vec<String> {
    let Ok(config) = serde_json::from_str::<DockerConfig>(text) else { return Vec::new() };
    let mut names: Vec<String> = config
        .auths
        .into_keys()
        .chain(config.cred_helpers.into_keys())
        .map(|k| {
            let k = k.trim_start_matches("https://").trim_start_matches("http://");
            k.split('/').next().unwrap_or(k).to_owned()
        })
        .filter(|k| !k.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The program's exit, `None` when it isn't installed or didn't answer in time.
async fn run(program: &str, args: &[&str], want_stdout: bool) -> Option<(bool, String)> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(if want_stdout { std::process::Stdio::piped() } else { std::process::Stdio::null() })
        .stderr(std::process::Stdio::null())
        .env("NO_COLOR", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1")
        .kill_on_drop(true);
    let out = tokio::time::timeout(PROBE_WAIT, cmd.output()).await.ok()?.ok()?;
    let stdout = if want_stdout { String::from_utf8_lossy(&out.stdout).into_owned() } else { String::new() };
    Some((out.status.success(), stdout))
}

/// `gh`, `gcloud`, `az`, `kubectl` and `npm`, asked whether they are signed in.
pub async fn probe_tools() -> Vec<ToolState> {
    let (gh, gcloud, az, kubectl, npm) = tokio::join!(
        run("gh", &["auth", "status"], false),
        run("gcloud", &["auth", "list", "--filter=status:ACTIVE", "--format=value(account)"], true),
        run("az", &["account", "show", "--output", "none"], false),
        run("kubectl", &["config", "get-contexts", "-o", "name"], true),
        run("npm", &["whoami"], false),
    );
    let mut out = Vec::new();
    if let Some((ok, _)) = gh {
        out.push(ToolState { tool: "gh", signed_in: ok, detail: None });
    }
    if let Some((ok, accounts)) = gcloud {
        // The account's kind only, never its address.
        let first = accounts.lines().map(str::trim).find(|l| !l.is_empty());
        let detail = first.map(|a| if a.ends_with(".gserviceaccount.com") { "service account" } else { "user account" }.to_owned());
        out.push(ToolState { tool: "gcloud", signed_in: ok && first.is_some(), detail });
    }
    if let Some((ok, _)) = az {
        out.push(ToolState { tool: "az", signed_in: ok, detail: None });
    }
    if let Some((true, names)) = kubectl {
        let names: Vec<&str> = names.lines().map(str::trim).filter(|l| !l.is_empty()).take(MAX_LISTED).collect();
        if !names.is_empty() {
            out.push(ToolState { tool: "kubectl", signed_in: true, detail: Some(format!("contexts {}", names.join(", "))) });
        }
    }
    if let Some((true, _)) = npm {
        out.push(ToolState { tool: "npm", signed_in: true, detail: None });
    }
    out
}

// ── the inventory ──

/// What this machine has, by name and shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub aws: Vec<AwsProfile>,
    /// AWS keys in the daemon's environment (`AWS_ACCESS_KEY_ID`): names only.
    pub aws_env: bool,
    /// A GitHub token in the daemon's environment (`GH_TOKEN`, `GITHUB_TOKEN`).
    pub gh_env: bool,
    pub docker: Vec<String>,
    /// The other tools' answers; empty until they have been asked.
    pub tools: Vec<ToolState>,
}

impl Inventory {
    fn tool(&self, name: &str) -> Option<&ToolState> {
        self.tools.iter().find(|t| t.tool == name)
    }

    /// The AWS profiles that work without the person.
    pub fn usable_aws(&self) -> Vec<&AwsProfile> {
        self.aws.iter().filter(|p| p.shape.non_interactive()).collect()
    }
}

/// An environment variable's value, by name.
pub type Env = dyn Fn(&str) -> Option<String>;

/// Where to look, and the environment to read (`None`: none, as in a test).
pub struct Where<'a> {
    pub home: &'a Path,
    pub env: Option<&'a Env>,
}

impl Where<'_> {
    fn var(&self, name: &str) -> Option<String> {
        self.env.and_then(|env| env(name)).filter(|v| !v.is_empty())
    }

    fn file(&self, var: &str, default: &[&str]) -> PathBuf {
        self.var(var).map(PathBuf::from).unwrap_or_else(|| default.iter().fold(self.home.to_owned(), |p, part| p.join(part)))
    }
}

/// The files' part: AWS profiles, environment names, docker registries.
pub fn scan_files(at: &Where<'_>, now: u64) -> Inventory {
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let config = read(at.file("AWS_CONFIG_FILE", &[".aws", "config"]));
    let credentials = read(at.file("AWS_SHARED_CREDENTIALS_FILE", &[".aws", "credentials"]));
    let expiries = sso_expiries(&at.home.join(".aws").join("sso").join("cache"));
    let aws = aws_profiles(&config, &credentials, &|url| expiries.get(url).is_some_and(|&at| at > now));
    let docker_config = at.var("DOCKER_CONFIG").map(|d| PathBuf::from(d).join("config.json")).unwrap_or_else(|| at.home.join(".docker").join("config.json"));
    Inventory {
        aws,
        aws_env: at.var("AWS_ACCESS_KEY_ID").is_some(),
        gh_env: at.var("GH_TOKEN").is_some() || at.var("GITHUB_TOKEN").is_some(),
        docker: docker_registries(&read(docker_config)),
        tools: Vec::new(),
    }
}

#[derive(Default)]
struct Probed {
    at: Option<Instant>,
    tools: Vec<ToolState>,
    running: bool,
}

/// The inventory, kept a while: the files read again after [`FILES_TTL`], the
/// tools asked again in the background after [`PROBE_TTL`].
#[derive(Default)]
pub struct Cache {
    files: Option<(Instant, Inventory)>,
    probed: Arc<Mutex<Probed>>,
}

impl Cache {
    /// What the machine has now. `probe`: ask the tools too (in the
    /// background; their answers come with a later call).
    pub fn inventory(&mut self, at: &Where<'_>, probe: bool) -> Inventory {
        let fresh = self.files.as_ref().is_some_and(|(when, _)| when.elapsed() < FILES_TTL);
        if !fresh {
            self.files = Some((Instant::now(), scan_files(at, now_secs())));
        }
        let mut inv = self.files.as_ref().map(|(_, inv)| inv.clone()).unwrap_or_default();
        if probe {
            self.probe_if_stale();
            if let Ok(probed) = self.probed.lock() {
                inv.tools = probed.tools.clone();
            }
        }
        inv
    }

    fn probe_if_stale(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        {
            let Ok(mut probed) = self.probed.lock() else { return };
            if probed.running || probed.at.is_some_and(|at| at.elapsed() < PROBE_TTL) {
                return;
            }
            probed.running = true;
        }
        let probed = self.probed.clone();
        handle.spawn(async move {
            let tools = probe_tools().await;
            if let Ok(mut p) = probed.lock() {
                *p = Probed { at: Some(Instant::now()), tools, running: false };
            }
        });
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ── what members are told, and the guard ──

fn aws_item(p: &AwsProfile) -> String {
    match p.access() {
        Some(access) => format!("{} ({access}, {})", p.name, p.shape.label()),
        None => format!("{} ({})", p.name, p.shape.label()),
    }
}

/// The machine's credentials in one line, `None` when it has none to speak of:
/// "AWS: ops-read (read, non-interactive), console (interactive, expired); gh: signed in".
pub fn summary(inv: &Inventory) -> Option<String> {
    let mut parts = Vec::new();
    let mut aws: Vec<&AwsProfile> = inv.aws.iter().collect();
    // Those that work first; within each, by name.
    aws.sort_by_key(|p| !p.shape.non_interactive());
    let mut items: Vec<String> = aws.iter().take(MAX_LISTED).map(|p| aws_item(p)).collect();
    if aws.len() > MAX_LISTED {
        items.push(format!("{} more", aws.len() - MAX_LISTED));
    }
    if inv.aws_env {
        items.insert(0, "keys in the environment (non-interactive)".to_owned());
    }
    if !items.is_empty() {
        parts.push(format!("AWS: {}", items.join(", ")));
    }
    for tool in ["gh", "gcloud", "az"] {
        let env = tool == "gh" && inv.gh_env;
        match inv.tool(tool) {
            _ if env => parts.push("gh: a token in the environment (non-interactive)".to_owned()),
            Some(t) if t.signed_in => parts.push(match &t.detail {
                Some(d) => format!("{tool}: signed in ({d})"),
                None => format!("{tool}: signed in"),
            }),
            Some(_) => parts.push(format!("{tool}: not signed in")),
            None => {}
        }
    }
    for t in inv.tools.iter().filter(|t| matches!(t.tool, "kubectl" | "npm") && t.signed_in) {
        parts.push(match &t.detail {
            Some(d) => format!("{}: {d}", t.tool),
            None => format!("{}: signed in", t.tool),
        });
    }
    if !inv.docker.is_empty() {
        parts.push(format!("docker: logged in to {}", inv.docker.iter().take(MAX_LISTED).cloned().collect::<Vec<_>>().join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The note a member's briefing carries, `None` when the machine has nothing to tell.
pub fn briefing(inv: &Inventory) -> Option<String> {
    let line = summary(inv)?;
    Some(format!(
        "Credentials already on this machine (names only, checked by equipe): {line}. Use a non-interactive one first — \
`AWS_PROFILE=<name>` for AWS, a read one for read-only checks — and don't ask the person to sign in while one works: \
`cli_sign_in` and AWS's console sign-in in the browser refuse then."
    ))
}

/// Whether `host` is an AWS console sign-in address: the sign-in pages
/// (`signin.aws.amazon.com`, `<region>.signin.aws.amazon.com`), the console they
/// lead to, and an IAM Identity Center portal (`<name>.awsapps.com`).
pub fn aws_sign_in_host(host: &str) -> bool {
    let host = crate::vault::site_of(host);
    let under = |root: &str| host == root || host.ends_with(&format!(".{root}"));
    under("signin.aws.amazon.com") || under("console.aws.amazon.com") || under("awsapps.com") || under("signin.amazonaws.cn") || under("console.amazonaws.cn")
}

/// Whether a sign-in to AWS in the browser (the console, a portal) may go to the
/// person: refused while a profile that works without them exists, naming it,
/// unless `why_not_machine` names the permission those lack. `tool` is the tool
/// the refusal tells the member to call again.
pub fn browser_guard(inv: &Inventory, why_not_machine: Option<&str>, tool: &str) -> Result<(), String> {
    guard(Kind::Aws, inv, why_not_machine).map_err(|text| {
        text.replace("`cli_sign_in`", &format!("`{tool}`"))
            .replace("No card was shown.", "The person was not asked, and nothing was handed over: the console sign-in in the browser is the last resort, not the first.")
    })
}

/// Whether `cli_sign_in` may put a card up for `kind`: refused while a
/// credential for that tool already works here, unless `why_not_machine`
/// names the permission the working ones lack.
pub fn guard(kind: Kind, inv: &Inventory, why_not_machine: Option<&str>) -> Result<(), String> {
    if why_not_machine.is_some_and(|w| w.trim().chars().count() >= MIN_WHY_NOT) {
        return Ok(());
    }
    let again = "If it fails with each of them for lack of a permission, call `cli_sign_in` again with `why_not_machine` \
naming that permission and the error you got (\"s3:PutObject AccessDenied with every profile\"). No card was shown.";
    match kind {
        Kind::Aws | Kind::AwsSso => {
            let usable = inv.usable_aws();
            if usable.is_empty() && !inv.aws_env {
                return Ok(());
            }
            let mut names: Vec<String> = usable.iter().map(|p| match p.access() {
                Some(a) => format!("`{}` ({a})", p.name),
                None => format!("`{}`", p.name),
            }).collect();
            if inv.aws_env {
                names.insert(0, "the keys in the environment (no AWS_PROFILE needed)".to_owned());
            }
            let example = usable.iter().find(|p| p.access() == Some("read")).or(usable.first()).map(|p| format!(" — `AWS_PROFILE={} aws sts get-caller-identity`", p.name)).unwrap_or_default();
            Err(format!(
                "This machine already has AWS credentials that work without the person: {}. Use AWS_PROFILE=<name>{example}; \
a read one for read-only checks. {again}",
                names.join(", ")
            ))
        }
        Kind::Gh | Kind::Gcloud | Kind::Az => {
            let tool = kind.program();
            let env = kind == Kind::Gh && inv.gh_env;
            if !env && !inv.tool(tool).is_some_and(|t| t.signed_in) {
                return Ok(());
            }
            let how = match kind {
                _ if env => "a GitHub token is in the environment",
                Kind::Gh => "`gh auth status` passes",
                Kind::Gcloud => "`gcloud auth list` has an active account",
                _ => "`az account show` passes",
            };
            Err(format!("{} is already signed in on this machine ({how}): use it as it is. {again}", kind.name()))
        }
        Kind::ClaudeMcp | Kind::CodexMcp | Kind::EquipeMcp => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
[default]
region = us-east-1

[profile acme-ops-read]
credential_process = /usr/local/bin/signing-helper credential-process --certificate /x/cert.pem
region = us-east-1

[profile acme-ops-deploy]
credential_process = /usr/local/bin/signing-helper credential-process --role deploy

[profile acme]
login_session = arn:aws:iam::123456789012:user/someone

[profile sso-dev]
sso_session = corp
sso_account_id = 111111111111
sso_role_name = ReadOnly

[profile sso-old]
sso_start_url = https://old.awsapps.com/start
sso_region = us-east-1

[profile assumed]
role_arn = arn:aws:iam::111111111111:role/Thing
source_profile = acme-ops-read

[profile assumed-from-console]
role_arn = arn:aws:iam::111111111111:role/Thing
source_profile = acme

[profile on-ec2]
role_arn = arn:aws:iam::111111111111:role/Thing
credential_source = Ec2InstanceMetadata

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = us-east-1
sso_registration_scopes = sso:account:access

[services local]
s3 =
  endpoint_url = http://localhost:4566
";

    const CREDENTIALS: &str = "\
[static-ci]
aws_access_key_id = AKIAFAKEFAKEFAKEFAKE
aws_secret_access_key = fake/fake/fake
";

    fn shapes(valid: &[&str]) -> Vec<(String, AwsShape)> {
        aws_profiles(CONFIG, CREDENTIALS, &|u| valid.contains(&u)).into_iter().map(|p| (p.name, p.shape)).collect()
    }

    #[test]
    fn aws_profiles_get_their_shape_from_the_config() {
        let got = shapes(&["https://corp.awsapps.com/start"]);
        let want = [
            ("acme", AwsShape::Console),
            ("acme-ops-deploy", AwsShape::Process),
            ("acme-ops-read", AwsShape::Process),
            ("assumed", AwsShape::Role),
            ("assumed-from-console", AwsShape::RoleFromInteractive),
            ("on-ec2", AwsShape::Role),
            ("sso-dev", AwsShape::SsoSignedIn),
            ("sso-old", AwsShape::SsoExpired),
            ("static-ci", AwsShape::Static),
        ];
        assert_eq!(got, want.iter().map(|(n, s)| ((*n).to_owned(), *s)).collect::<Vec<_>>());
        // `default` holds a region only: not a credential.
        assert!(!got.iter().any(|(n, _)| n == "default"));
        // The SSO session's sign-in ran out: interactive again.
        assert!(shapes(&[]).contains(&("sso-dev".to_owned(), AwsShape::SsoExpired)));
    }

    #[test]
    fn a_secret_value_is_never_kept() {
        let (profiles, _) = ini(CREDENTIALS, true);
        let keys = &profiles["static-ci"];
        assert_eq!(keys["aws_access_key_id"], "");
        assert_eq!(keys["aws_secret_access_key"], "");
        let all: String = profiles.values().flat_map(|s| s.values().cloned()).collect();
        assert!(!all.contains("FAKE") && !all.contains("fake"), "{all}");
    }

    #[test]
    fn the_sso_cache_gives_its_expiry_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("equipe-sso-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, dir) = (root.join("home"), root.join("home/.aws/sso/cache"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("aaa.json"),
            r#"{"startUrl":"https://corp.awsapps.com/start","region":"us-east-1","accessToken":"FAKE-TOKEN","refreshToken":"FAKE-REFRESH","expiresAt":"2030-01-02T03:04:05Z"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("bbb.json"), r#"{"startUrl":"https://corp.awsapps.com/start","accessToken":"FAKE","expiresAt":"2020-01-01T00:00:00UTC"}"#).unwrap();
        std::fs::write(dir.join("botocore-client-id.json"), r#"{"clientId":"FAKE","clientSecret":"FAKE","expiresAt":"2031-01-01T00:00:00Z"}"#).unwrap();
        std::fs::write(dir.join("not-json.txt"), "x").unwrap();
        let got = sso_expiries(&dir);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got["https://corp.awsapps.com/start"], epoch("2030-01-02T03:04:05Z").unwrap());
        // Read through a home folder: signed in before it runs out, expired after.
        std::fs::write(home.join(".aws/config"), CONFIG).unwrap();
        let at = Where { home: &home, env: None };
        let before = scan_files(&at, epoch("2029-12-31T00:00:00Z").unwrap());
        assert!(before.aws.contains(&AwsProfile { name: "sso-dev".into(), shape: AwsShape::SsoSignedIn }));
        let after = scan_files(&at, epoch("2030-01-03T00:00:00Z").unwrap());
        assert!(after.aws.contains(&AwsProfile { name: "sso-dev".into(), shape: AwsShape::SsoExpired }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn times_are_read_as_aws_writes_them() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(epoch("2026-11-19T10:00:00UTC"), Some(1_795_082_400));
        assert_eq!(epoch("2026-11-19T10:00:00.123Z"), Some(1_795_082_400));
        assert_eq!(epoch("2026-11-19T19:00:00+09:00"), Some(1_795_082_400));
        assert_eq!(epoch("2026-11-19T10:00:00+0000"), Some(1_795_082_400));
        assert_eq!(epoch("yesterday"), None);
        assert_eq!(epoch("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn docker_gives_registry_names_only() {
        let text = r#"{"auths":{"https://index.docker.io/v1/":{"auth":"ZmFrZTpmYWtl"},"ghcr.io":{}},"credsStore":"desktop","credHelpers":{"123.dkr.ecr.us-east-1.amazonaws.com":"ecr-login"}}"#;
        assert_eq!(docker_registries(text), ["123.dkr.ecr.us-east-1.amazonaws.com", "ghcr.io", "index.docker.io"]);
        assert!(docker_registries("not json").is_empty());
    }

    fn inv() -> Inventory {
        Inventory { aws: aws_profiles(CONFIG, "", &|_| false), ..Inventory::default() }
    }

    #[test]
    fn the_summary_names_working_profiles_first_with_what_their_names_say() {
        let mut inv = inv();
        inv.tools = vec![
            ToolState { tool: "gh", signed_in: true, detail: None },
            ToolState { tool: "gcloud", signed_in: false, detail: None },
            ToolState { tool: "kubectl", signed_in: true, detail: Some("contexts dev, prod".into()) },
        ];
        inv.docker = vec!["ghcr.io".into()];
        let line = summary(&inv).unwrap();
        assert!(
            line.starts_with("AWS: acme-ops-deploy (deploy, non-interactive), acme-ops-read (read, non-interactive), assumed (non-interactive), on-ec2 (non-interactive), acme (interactive)"),
            "{line}"
        );
        assert!(line.contains("sso-dev (interactive, expired)"), "{line}");
        assert!(line.contains("; gh: signed in; gcloud: not signed in; kubectl: contexts dev, prod; docker: logged in to ghcr.io"), "{line}");
        assert!(!line.contains("credential-process") && !line.contains("arn:"), "no values: {line}");
        assert_eq!(summary(&Inventory::default()), None);
        assert!(briefing(&inv).unwrap().contains("AWS_PROFILE=<name>"));
    }

    #[test]
    fn access_is_read_from_the_name_only_when_plain() {
        assert_eq!(access_hint("acme-ops-read"), Some("read"));
        assert_eq!(access_hint("ReadOnly_prod"), Some("read"));
        assert_eq!(access_hint("acme-ops-deploy"), Some("deploy"));
        assert_eq!(access_hint("prod-admin"), Some("admin"));
        assert_eq!(access_hint("acme"), None);
        assert_eq!(access_hint("thread-runner"), None);
    }

    #[test]
    fn a_sign_in_card_is_refused_while_a_machine_credential_works() {
        let inv = inv();
        let err = guard(Kind::Aws, &inv, None).unwrap_err();
        assert!(err.contains("`acme-ops-read` (read)") && err.contains("`acme-ops-deploy` (deploy)") && err.contains("AWS_PROFILE=acme-ops-read"), "{err}");
        assert!(err.contains("why_not_machine") && !err.contains("`acme`"), "{err}");
        assert!(guard(Kind::AwsSso, &inv, None).is_err());
        // A vague reason isn't one; a named permission is.
        assert!(guard(Kind::Aws, &inv, Some("need it")).is_err());
        assert!(guard(Kind::Aws, &inv, Some("iam:CreateRole AccessDenied with acme-ops-deploy")).is_ok());
        // Nothing that works without the person: the card goes up.
        let console_only = Inventory { aws: aws_profiles("[profile acme]\nlogin_session = x\n", "", &|_| false), ..Inventory::default() };
        assert!(guard(Kind::Aws, &console_only, None).is_ok());
        assert!(guard(Kind::Aws, &Inventory { aws_env: true, ..Inventory::default() }, None).unwrap_err().contains("environment"));
        // The other tools: by what they said; not yet asked, the card goes up.
        assert!(guard(Kind::Gh, &inv, None).is_ok());
        let signed = Inventory { tools: vec![ToolState { tool: "gh", signed_in: true, detail: None }, ToolState { tool: "gcloud", signed_in: false, detail: None }], ..Inventory::default() };
        assert!(guard(Kind::Gh, &signed, None).unwrap_err().contains("gh auth status"));
        assert!(guard(Kind::Gcloud, &signed, None).is_ok());
        assert!(guard(Kind::Gh, &Inventory { gh_env: true, ..Inventory::default() }, None).is_err());
        // An outside MCP server's sign-in is equipe's own: never held back here.
        assert!(guard(Kind::EquipeMcp, &signed, None).is_ok());
    }
}
