//! Outside MCP servers a squad connects for its members: a design library's
//! official server from the Connections catalog (Mobbin), or any server
//! added by its address. Kept in the squad's settings
//! ([`crate::journal::Journal::mcp_servers`]), given to the members of its
//! roles in their engine's own MCP config, and signed in by the engine's own
//! command-line tool.
//!
//! How a sign-in reaches a member, on each engine:
//!
//! - Claude Code keeps an MCP server's OAuth tokens in its own credential
//!   store under the server's name and a hash of its type, URL and headers
//!   (`<name>|<sha256(type, url, headers)>`, whatever scope the server was
//!   configured in). A member runs with `--mcp-config` and
//!   `--strict-mcp-config`; its entry for the server is `{"type": "http",
//!   "url": …}` with the same name and no headers, so it finds the token the
//!   person's sign-in left. The sign-in itself is `claude mcp login <name>`,
//!   which needs the server configured somewhere: equipe adds it in local
//!   scope to a folder of its own per squad ([`signin_dir`]), so it never
//!   shows in the person's own Claude Code sessions.
//! - Codex keys its tokens the same way (server name and URL) and takes the
//!   server as a config override: `codex mcp login <name> -c
//!   mcp_servers.<name>.url="…"`, and the member's thread config names the
//!   same server and URL.
//!
//! equipe never reads, keeps or passes such a token: no header, no
//! environment variable, no file of its own. Whether a server is connected
//! comes from the engines' own tools (`claude mcp get`, `codex mcp list --json`).
//!
//! Who holds a server's credential is its [`McpAuth`] — the one place that
//! decides how a member's engine is given it ([`credential`]):
//!
//! - [`McpAuth::Engine`]: the engine's own OAuth, as above.
//! - [`McpAuth::Key`]: an API key the person saved in the squad's secret store
//!   through the key card (21st.dev's `x-api-key`). The daemon reads it at a
//!   member's launch and puts it in the engine's config as a header: Claude
//!   Code's `--mcp-config` is then a file in equipe's private folder (which a
//!   fenced member's commands can't read), never an argument; Codex gets it in
//!   the thread's config over its app-server connection. It is a kept key, so
//!   it is masked out of everything a member says.
//! - [`McpAuth::Gateway`]: equipe-managed OAuth for an engine that can't sign
//!   in itself — the data model only for now; the plan is in docs/LOGINS.md.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::connections::{self, Service};
use crate::protocol::Engine;
pub use crate::protocol::mcp_servers::{McpAuth, McpEngineStatus, McpServer, McpServerStatus, McpState, McpTransport};

/// At most this many outside servers a squad.
pub const MOST_SERVERS: usize = 16;

/// Names equipe's own servers have (and Codex's apps server): never an outside one's.
const RESERVED: [&str; 6] = ["equipe", "desktop", "design", "phone", "browser", "codex_apps"];

/// How long an engine's tool may take to say where a server stands.
const CHECK_WAIT: Duration = Duration::from_secs(60);

/// A server's name: a lowercase letter or digit, then letters, digits, `-`
/// and `_`, up to 32 — what members' tools are named after (`mcp__<name>__…`).
pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_ascii_lowercase();
    let ok = (1..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().any(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        return Err(format!("{name:?}: a server's name is letters, digits, `-` and `_`, up to 32, starting with a letter or digit (`mobbin`)."));
    }
    if RESERVED.contains(&name.as_str()) || name.starts_with("browser_") || name.starts_with("browser-") {
        return Err(format!("{name:?} is the name of one of equipe's own servers: pick another."));
    }
    Ok(name)
}

/// A server's address: an https site, as strictly as a key card's page
/// ([`connections::clean_url`]: no credentials, port, IP or localhost), and
/// nothing that would break out of a config value.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = connections::clean_url(url)?;
    if url.chars().any(|c| matches!(c, '"' | '\'' | '\\' | '`' | '<' | '>' | '{' | '}' | '|' | '^')) {
        return Err(format!("{url:?}: not a plain address (an https address like https://api.mobbin.com/mcp)."));
    }
    Ok(url)
}

/// The roles named: lowercase words, each once. None: every member.
pub fn check_roles(roles: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for role in roles.iter().flat_map(|r| r.split(',')).map(|r| r.trim().to_ascii_lowercase()).filter(|r| !r.is_empty()) {
        if role.len() > 40 || !role.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("{role:?} isn't a role (designer, frontend, backend, mobile, qa, data, pm…)."));
        }
        if !out.contains(&role) {
            out.push(role);
        }
    }
    Ok(out)
}

/// A server as asked for, checked: `name` (or a catalog id), its address
/// (left out for a catalog service: the catalog's), the roles (left out: the
/// catalog's, else every member) and the transport.
pub fn server(name: &str, url: Option<&str>, roles: Option<&[String]>, transport: Option<&str>) -> Result<McpServer, String> {
    let name = check_name(name)?;
    let catalog = Service::by_id(&name).and_then(|s| s.mcp.map(|m| (s.id, m)));
    let url = match (url.map(str::trim).filter(|u| !u.is_empty()), catalog) {
        (Some(url), _) => check_url(url)?,
        (None, Some((_, mcp))) => mcp.url.to_owned(),
        (None, None) => return Err(format!("Give {name}'s address: its https URL (`https://example.com/mcp`).")),
    };
    let roles = match (roles, catalog) {
        (Some(roles), _) => check_roles(roles)?,
        (None, Some((_, mcp))) => mcp.roles.iter().map(|r| (*r).to_owned()).collect(),
        (None, None) => Vec::new(),
    };
    let transport = match transport {
        Some(word) => McpTransport::parse(word).ok_or_else(|| format!("{word:?}: a transport is http (streamable HTTP) or sse."))?,
        None => McpTransport::Http,
    };
    // The catalog's own server, at its own address: remembered as the catalog's, its key with it.
    let catalog = catalog.filter(|(_, mcp)| mcp.url == url);
    let auth = match catalog.and_then(|(id, mcp)| Some((Service::by_id(id)?.key_name?, mcp.key_header?))) {
        Some((secret, header)) => McpAuth::Key { secret: secret.to_owned(), header: header.to_owned() },
        None => McpAuth::Engine,
    };
    Ok(McpServer { name, url, transport, roles, enabled: true, catalog: catalog.map(|(id, _)| id.to_owned()), auth })
}

/// How a member's engine is given a server's credential at launch.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// The engine signs in itself and keeps its own token: nothing to give.
    Engine,
    /// A header the daemon puts in the engine's config (its value from the secret store).
    Header { name: String, value: String },
    /// Its key isn't saved yet (`secret`): the server is left out, and the key card asked for.
    MissingKey { secret: String },
    /// equipe-managed OAuth: not available yet, so the server is left out.
    Unavailable,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine => f.write_str("Engine"),
            Self::Header { name, .. } => write!(f, "Header({name}: •••)"),
            Self::MissingKey { secret } => write!(f, "MissingKey({secret})"),
            Self::Unavailable => f.write_str("Unavailable"),
        }
    }
}

/// How `server`'s credential reaches a member: `key` reads a kept key's
/// value by name (the daemon's secret store), asked only for a keyed server.
pub fn credential(server: &McpServer, key: impl FnOnce(&str) -> Option<String>) -> Credential {
    match &server.auth {
        McpAuth::Engine => Credential::Engine,
        McpAuth::Key { secret, header } => match key(secret).filter(|v| !v.trim().is_empty()) {
            Some(value) => Credential::Header { name: header.clone(), value },
            None => Credential::MissingKey { secret: secret.clone() },
        },
        McpAuth::Gateway { .. } => Credential::Unavailable,
    }
}

/// The catalog service a server is, for its words and links.
pub fn service_of(server: &McpServer) -> Option<&'static Service> {
    server.catalog.as_deref().and_then(Service::by_id).filter(|s| s.is_mcp())
}

/// How a server is named for the person: the catalog's label, else its name.
pub fn label(server: &McpServer) -> String {
    service_of(server).map_or_else(|| server.name.clone(), |s| s.label.to_owned())
}

/// `servers` with `server` in it: one of the same name replaced in place,
/// else added at the end (at most [`MOST_SERVERS`]).
pub fn with(servers: &[McpServer], server: McpServer) -> Result<Vec<McpServer>, String> {
    let mut out = servers.to_vec();
    match out.iter().position(|s| s.name == server.name) {
        Some(at) => out[at] = server,
        None if out.len() >= MOST_SERVERS => return Err(format!("A squad connects at most {MOST_SERVERS} outside servers: remove one first.")),
        None => out.push(server),
    }
    Ok(out)
}

/// The servers a member of `role` on `engine` gets: on, for its role, over a
/// transport its engine speaks.
pub fn for_member<'a>(servers: &'a [McpServer], role: &str, engine: Engine) -> Vec<&'a McpServer> {
    servers.iter().filter(|s| s.enabled && s.serves(role) && s.transport.on(engine)).collect()
}

/// A server's entry in a Claude Code member's `--mcp-config`: its type and
/// URL — for the engine's own sign-in nothing else, the same name, type and
/// URL (and no headers) the sign-in was made for, so Claude Code finds the
/// token in its own store; for a kept key, its header too.
pub fn claude_entry(server: &McpServer, credential: &Credential) -> Value {
    let mut entry = json!({ "type": server.transport.code(), "url": server.url });
    if let Credential::Header { name, value } = credential {
        entry["headers"] = json!({ name.as_str(): value });
    }
    entry
}

/// A server's entry in a Codex member's thread config: its URL (Codex finds
/// its own sign-in by name and URL), and a kept key's header, sent over the
/// app-server connection — never an argument, a variable or a file.
/// `approved`: its tools run without asking.
pub fn codex_entry(server: &McpServer, approved: bool, credential: &Credential) -> Value {
    let mut entry = equipe_providers::codex_app::mcp_http_server(&server.url, approved);
    if let Credential::Header { name, value } = credential {
        entry["http_headers"] = json!({ name.as_str(): value });
    }
    entry
}

/// Whether a Claude Code member's MCP config carries a secret, and so goes
/// in a private file rather than on its command line.
pub fn carries_secret(credentials: &[Credential]) -> bool {
    credentials.iter().any(|c| matches!(c, Credential::Header { .. }))
}

/// Where a member's Claude Code MCP config is written when it carries a key:
/// equipe's private folder, which fenced commands can't read.
pub fn private_config_path(home: &Path, session: u64) -> PathBuf {
    home.join("credentials").join("mcp").join(format!("{session}.json"))
}

/// Write `config` there, readable by the person only.
pub fn write_private_config(path: &Path, config: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(config.as_bytes())
}

/// The rule that lets a Claude Code member use every tool of `server` without asking.
pub fn allowed_rule(server: &McpServer) -> String {
    format!("mcp__{}", server.name)
}

/// What a member is told of the outside servers it has, in a line or two;
/// `None` without any — nothing is added to anyone else's charter.
pub fn charter_note(servers: &[&McpServer]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    let each: Vec<String> = servers
        .iter()
        .map(|s| match service_of(s) {
            Some(service) => format!("`{}` ({}: {}; tools `mcp__{}__*`)", s.name, service.label, purpose(service.id), s.name),
            None => format!("`{}` (tools `mcp__{}__*`)", s.name, s.name),
        })
        .collect();
    Some(format!(
        "Outside MCP servers the squad connected for you: {}. Use them where they fit — a design library's server for \
references instead of browsing its site. When one says it needs a sign-in, say so in a line and carry on without \
it: equipe has already put one card for it in front of the person.",
        each.join("; ")
    ))
}

/// What a catalog MCP service is for, in English, for a member's charter.
fn purpose(id: &str) -> &'static str {
    match id {
        "mobbin" => "search real product screens and flows for design references",
        "21st" => "proven React, Tailwind and shadcn/ui components to adapt",
        _ => "the service's own tools",
    }
}

// ───────────────────────── the engines' own tools ─────────────────────────

/// The folder equipe signs `squad`'s servers in from with Claude Code: a
/// local-scope server there is the squad's alone, out of the person's own
/// sessions.
pub fn signin_dir(home: &Path, squad: &str) -> PathBuf {
    crate::journal::squad_dir(home, squad).join("mcp-signin")
}

fn strs(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

/// `claude mcp add`: the server in local scope, as members name it.
pub fn claude_add_args(server: &McpServer) -> Vec<String> {
    let mut args = strs(&["mcp", "add", "--scope", "local", "--transport", server.transport.code()]);
    args.extend([server.name.clone(), server.url.clone()]);
    args
}

pub fn claude_get_args(name: &str) -> Vec<String> {
    strs(&["mcp", "get", name])
}

pub fn claude_remove_args(name: &str) -> Vec<String> {
    strs(&["mcp", "remove", "--scope", "local", name])
}

/// `claude mcp login`: the page printed rather than opened (`no_browser`) —
/// for a card, whose button opens it on the screen the person looks at; a
/// terminal of the person's own lets it open the browser itself.
pub fn claude_login_args(name: &str, no_browser: bool) -> Vec<String> {
    let mut args = strs(&["mcp", "login", name]);
    if no_browser {
        args.push("--no-browser".into());
    }
    args
}

/// Codex's config override naming the server: `mcp_servers.<name>.url="<url>"`
/// (both checked: no quote or backslash can be in either).
pub fn codex_override(server: &McpServer) -> String {
    format!("mcp_servers.{}.url=\"{}\"", server.name, server.url)
}

pub fn codex_list_args(server: &McpServer) -> Vec<String> {
    let mut args = strs(&["mcp", "list", "--json", "-c"]);
    args.push(codex_override(server));
    args
}

pub fn codex_login_args(server: &McpServer) -> Vec<String> {
    let mut args = strs(&["mcp", "login", &server.name, "-c"]);
    args.push(codex_override(server));
    args
}

pub fn codex_logout_args(server: &McpServer) -> Vec<String> {
    let mut args = strs(&["mcp", "logout", &server.name, "-c"]);
    args.push(codex_override(server));
    args
}

/// What `claude mcp get <name>` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeGet {
    /// No server of that name where it ran.
    Missing,
    /// One of that name, at another address or over another transport.
    Other,
    /// It, and where it stands.
    Status(McpState, Option<String>),
}

/// Read `claude mcp get`'s answer for `server`: its `Status:` line (✓
/// Connected, ! Needs authentication, ✘ Failed to connect …), its `Issue:`
/// line, and whether its `Type:` and `URL:` are the server's.
pub fn parse_claude_get(output: &str, server: &McpServer) -> ClaudeGet {
    let text = crate::cli_signin::clean(output);
    if text.contains("No MCP server named") || text.contains("No MCP server found") {
        return ClaudeGet::Missing;
    }
    let field = |key: &str| text.lines().map(str::trim).find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_owned());
    let (Some(status), Some(url)) = (field("Status:"), field("URL:")) else {
        return ClaudeGet::Status(McpState::Failing, Some(short(text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no answer"))));
    };
    if url != server.url || field("Type:").is_some_and(|t| !t.eq_ignore_ascii_case(server.transport.code())) {
        return ClaudeGet::Other;
    }
    let issue = field("Issue:").map(|i| short(&i));
    let lower = status.to_ascii_lowercase();
    let state = if lower.contains("needs authentication") || lower.contains("needs auth") {
        McpState::NeedsSignIn
    } else if lower.contains("connected") && !lower.contains("failed") {
        McpState::Connected
    } else {
        McpState::Failing
    };
    let detail = match state {
        McpState::Connected => None,
        McpState::Failing => issue.or_else(|| Some(short(status.trim_start_matches(|c: char| !c.is_alphanumeric())))),
        _ => issue,
    };
    ClaudeGet::Status(state, detail)
}

/// Read `codex mcp list --json` for the server named `name`: signed in
/// (`o_auth`, or a bearer token of the person's own), not signed in, or no
/// sign-in needed (`unsupported`: the server asks for none).
pub fn parse_codex_list(output: &str, name: &str) -> (McpState, Option<String>) {
    let Ok(Value::Array(servers)) = serde_json::from_str::<Value>(output.trim()) else {
        return (McpState::Failing, Some(short(output.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no answer"))));
    };
    let Some(found) = servers.iter().find(|s| s["name"] == name) else {
        return (McpState::Failing, Some("Codex didn't list it".to_owned()));
    };
    if found["enabled"] == false {
        return (McpState::Failing, Some("turned off in Codex's own settings".to_owned()));
    }
    match found["auth_status"].as_str().unwrap_or_default() {
        "not_logged_in" => (McpState::NeedsSignIn, None),
        "o_auth" | "oauth" | "bearer_token" | "unsupported" => (McpState::Connected, None),
        other => (McpState::Failing, Some(short(&format!("Codex says {other:?}")))),
    }
}

/// A line of an engine's answer, short, with no address in it.
fn short(line: &str) -> String {
    let line: String = line.split_whitespace().filter(|w| !w.contains("://")).collect::<Vec<_>>().join(" ");
    line.chars().take(200).collect()
}

/// Run `program args` in `cwd`, no input, at most [`CHECK_WAIT`]: its exit
/// and what it printed (both streams).
async fn run(program: &str, args: &[String], cwd: Option<&Path>) -> Result<(bool, String), String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true).env("NO_COLOR", "1");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = match tokio::time::timeout(CHECK_WAIT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("{program}: {e}")),
        Err(_) => return Err(format!("{program} didn't answer in {} s", CHECK_WAIT.as_secs())),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Make sure `server` is configured, as members name it, in `dir` for
/// Claude Code (`claude mcp add`, in local scope): what `claude mcp login`
/// and `claude mcp get` need. Where it stands, as `claude mcp get` says.
pub async fn prepare_claude(binary: &str, dir: &Path, server: &McpServer) -> Result<(McpState, Option<String>), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (_, said) = run(binary, &claude_get_args(&server.name), Some(dir)).await?;
    match parse_claude_get(&said, server) {
        ClaudeGet::Status(state, detail) => return Ok((state, detail)),
        // Another address under the name: the old one goes (with its sign-in, which was for that address).
        ClaudeGet::Other => {
            let _ = run(binary, &claude_remove_args(&server.name), Some(dir)).await?;
        }
        ClaudeGet::Missing => {}
    }
    let (added, said) = run(binary, &claude_add_args(server), Some(dir)).await?;
    if !added {
        return Err(short(said.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("claude mcp add failed")));
    }
    let (_, said) = run(binary, &claude_get_args(&server.name), Some(dir)).await?;
    match parse_claude_get(&said, server) {
        ClaudeGet::Status(state, detail) => Ok((state, detail)),
        _ => Err("Claude Code didn't keep the server".to_owned()),
    }
}

/// Where `server` stands for Codex members, as `codex mcp list --json` says
/// with the server named by a config override (nothing written to Codex's config).
pub async fn check_codex(binary: &str, server: &McpServer) -> Result<(McpState, Option<String>), String> {
    let (_, said) = run(binary, &codex_list_args(server), None).await?;
    // The JSON is on standard output; anything Codex logs comes after it.
    let json = said.split("\n\n").next().unwrap_or_default();
    Ok(match parse_codex_list(json, &server.name) {
        (McpState::Failing, _) => parse_codex_list(&said, &server.name),
        found => found,
    })
}

/// Where `server` stands on `engine`, as that engine's tool says now.
pub async fn check(engine: Engine, binary: &str, dir: &Path, server: &McpServer, now: u64) -> McpEngineStatus {
    let said = match engine {
        Engine::Claude => prepare_claude(binary, dir, server).await,
        Engine::Codex if !server.transport.on(Engine::Codex) => Ok((McpState::Failing, Some("Codex reaches servers over streamable HTTP only".to_owned()))),
        Engine::Codex => check_codex(binary, server).await,
    };
    let (state, detail) = said.unwrap_or_else(|why| (McpState::Failing, Some(short(&why))));
    McpEngineStatus { engine, state, detail, checked_at: now }
}

/// Clear what an engine kept for `server` (Claude Code: its local-scope
/// entry, which takes the server's sign-in with it; Codex: its sign-in).
/// Only when no other squad uses the same name and address: the sign-in is
/// the engine's for that pair, whichever squad made it.
pub async fn forget(engine: Engine, binary: &str, dir: &Path, server: &McpServer) {
    let _ = match engine {
        Engine::Claude if dir.is_dir() => run(binary, &claude_remove_args(&server.name), Some(dir)).await,
        Engine::Claude => return,
        Engine::Codex => run(binary, &codex_logout_args(server), None).await,
    };
}

/// The sign-in a card runs for `server` on `engine`: the engine's own
/// interactive login, in a terminal of its own, its page printed for the card.
pub fn signin_plan(engine: Engine, binary: &str, dir: &Path, server: &McpServer) -> crate::cli_signin::Plan {
    let env = vec![("NO_COLOR".to_owned(), "1".to_owned()), ("BROWSER".to_owned(), "echo".to_owned()), ("TERM".to_owned(), "xterm".to_owned())];
    let (args, cwd) = match engine {
        Engine::Claude => (claude_login_args(&server.name, true), Some(dir.to_path_buf())),
        Engine::Codex => (codex_login_args(server), None),
    };
    crate::cli_signin::Plan { program: binary.to_owned(), args, env, whoami: (String::new(), Vec::new()), cwd }
}

/// What a sign-in card for `server` is for: the catalog id (a screen shows
/// that service's own words for it), else the server's name.
pub fn signin_why(server: &McpServer) -> String {
    service_of(server).map_or_else(|| server.name.clone(), |s| s.id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mobbin() -> McpServer {
        server("mobbin", None, None, None).unwrap()
    }

    #[test]
    fn a_catalog_name_alone_is_its_official_server_for_its_roles() {
        let m = mobbin();
        assert_eq!((m.url.as_str(), m.roles.clone(), m.catalog.as_deref(), m.transport, m.enabled), ("https://api.mobbin.com/mcp", vec!["designer".to_owned(), "frontend".to_owned()], Some("mobbin"), McpTransport::Http, true));
        assert_eq!(label(&m), "Mobbin");
        // At another address it is someone's own server, named the same.
        let own = server("mobbin", Some("https://mcp.example.com/mobbin"), Some(&["qa".to_owned()]), None).unwrap();
        assert_eq!((own.catalog, own.roles), (None, vec!["qa".to_owned()]));
        // Any server by its address; no roles: everyone.
        let any = server("Acme-Docs", Some("https://docs.acme.example.com/mcp"), None, Some("sse")).unwrap();
        assert_eq!((any.name.as_str(), any.roles.is_empty(), any.transport), ("acme-docs", true, McpTransport::Sse));
        assert!(server("acme", None, None, None).unwrap_err().contains("address"), "outside the catalog the address is needed");
    }

    #[test]
    fn an_address_is_checked_as_strictly_as_a_key_cards_page() {
        assert_eq!(check_url(" https://api.mobbin.com/mcp ").as_deref(), Ok("https://api.mobbin.com/mcp"));
        for bad in [
            "http://api.mobbin.com/mcp",
            "https://user:pw@api.mobbin.com/mcp",
            "https://api.mobbin.com:8443/mcp",
            "https://localhost/mcp",
            "https://127.0.0.1/mcp",
            "https://api.mobbin.com/m cp",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://api.mobbin.com/mcp\"; evil=\"1",
            "https://api.mobbin.com/mcp?x={y}",
            "api.mobbin.com/mcp",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_are_plain_and_never_equipes_own() {
        assert_eq!(check_name("Mobbin").as_deref(), Ok("mobbin"));
        assert_eq!(check_name("21st").as_deref(), Ok("21st"));
        for bad in ["", "123", "-abc", "a b", "a.b", "equipe", "desktop", "design", "phone", "browser", "browser_jh", "codex_apps", &"x".repeat(33), "a\"b"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(check_roles(&["Designer, frontend".to_owned(), "designer".to_owned()]).unwrap(), ["designer", "frontend"]);
        assert!(check_roles(&["a b".to_owned()]).is_err());
    }

    #[test]
    fn a_server_reaches_only_its_roles_when_on_and_sse_only_claude_code() {
        let mut servers = vec![mobbin(), server("docs", Some("https://docs.example.com/mcp"), None, Some("sse")).unwrap()];
        let names = |role: &str, engine: Engine, s: &[McpServer]| for_member(s, role, engine).iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("designer", Engine::Claude, &servers), ["mobbin", "docs"]);
        assert_eq!(names("designer", Engine::Codex, &servers), ["mobbin"], "Codex speaks streamable HTTP only");
        assert_eq!(names("backend", Engine::Claude, &servers), ["docs"], "no roles named: everyone");
        servers[0].enabled = false;
        assert_eq!(names("frontend", Engine::Claude, &servers), ["docs"], "off: no one");
        // Replaced in place by name; at most MOST_SERVERS.
        let replaced = with(&servers, mobbin()).unwrap();
        assert_eq!((replaced.len(), replaced[0].enabled), (2, true));
        let full: Vec<McpServer> = (0..MOST_SERVERS).map(|i| server(&format!("s{i}"), Some("https://x.example.com/mcp"), None, None).unwrap()).collect();
        assert!(with(&full, mobbin()).is_err());
    }

    #[test]
    fn a_members_entry_is_the_address_and_never_a_token() {
        let m = mobbin();
        let none = credential(&m, |_| panic!("no key is read for the engine's own sign-in"));
        assert_eq!(none, Credential::Engine);
        assert_eq!(claude_entry(&m, &none), json!({ "type": "http", "url": "https://api.mobbin.com/mcp" }));
        let codex = codex_entry(&m, false, &none);
        assert_eq!((codex["url"].as_str(), codex["default_tools_approval_mode"].as_str()), (Some("https://api.mobbin.com/mcp"), Some("prompt")));
        assert!(!carries_secret(&[none.clone()]));
        for entry in [claude_entry(&m, &none), codex_entry(&m, true, &none)] {
            let text = entry.to_string().to_ascii_lowercase();
            for word in ["header", "token", "authorization", "bearer", "env"] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
        assert_eq!(allowed_rule(&m), "mcp__mobbin");
    }

    #[test]
    fn a_codex_members_thread_has_the_server_by_address_over_the_persons_own_of_that_name() {
        use equipe_providers::codex_app::{others_off, thread_config};
        let m = mobbin();
        let ours = json!({ "equipe": { "command": "/opt/equipe" }, "mobbin": codex_entry(&m, true, &Credential::Engine) });
        // The person's own Codex servers: one named the same (theirs is replaced, not switched off), one other (off).
        let listing = json!({ "data": [{ "name": "mobbin" }, { "name": "shadcn" }] });
        let config = thread_config(Some(&ours), Some(&others_off(&listing, &ours)));
        assert_eq!(config["mcp_servers"]["mobbin"]["url"], "https://api.mobbin.com/mcp");
        assert!(config["mcp_servers"]["mobbin"].get("enabled").is_none(), "ours, not switched off");
        assert_eq!(config["mcp_servers"]["shadcn"], json!({ "enabled": false }));
        assert!(!config.to_string().to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn a_keyed_server_gets_its_header_from_the_store_only_at_launch_and_is_left_out_without_it() {
        let ui = server("21st", None, None, None).unwrap();
        assert_eq!(ui.auth, McpAuth::Key { secret: "21st-api-key".into(), header: "x-api-key".into() });
        assert_eq!((ui.url.as_str(), ui.roles.clone()), ("https://21st.dev/api/mcp", vec!["designer".to_owned(), "frontend".to_owned()]));
        let missing = credential(&ui, |_| None);
        assert_eq!(missing, Credential::MissingKey { secret: "21st-api-key".into() });
        let given = credential(&ui, |name| (name == "21st-api-key").then(|| "21st_sk_test_value".to_owned()));
        assert!(!format!("{given:?}").contains("21st_sk"), "never in a log line");
        assert_eq!(claude_entry(&ui, &given)["headers"], json!({ "x-api-key": "21st_sk_test_value" }));
        assert_eq!(codex_entry(&ui, true, &given)["http_headers"], json!({ "x-api-key": "21st_sk_test_value" }));
        assert!(carries_secret(&[Credential::Engine, given]));
        // At another address, someone's own server: no key of the catalog's goes to it.
        assert_eq!(server("21st", Some("https://mcp.example.com/x"), None, None).unwrap().auth, McpAuth::Engine);
        // equipe-managed OAuth: modelled, not given yet.
        assert_eq!(credential(&McpServer { auth: McpAuth::Gateway { secret: "x".into() }, ..ui }, |_| None), Credential::Unavailable);
    }

    #[test]
    fn a_private_config_is_the_persons_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = std::env::temp_dir().join(format!("equipe-mcp-private-{}", std::process::id()));
        let path = private_config_path(&home, 7);
        assert!(path.starts_with(home.join("credentials")), "under the folder fenced commands can't read");
        write_private_config(&path, "{}").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_engines_commands_name_the_server_as_members_do() {
        let m = mobbin();
        assert_eq!(claude_add_args(&m).join(" "), "mcp add --scope local --transport http mobbin https://api.mobbin.com/mcp");
        assert_eq!(claude_login_args("mobbin", true).join(" "), "mcp login mobbin --no-browser");
        assert_eq!(codex_login_args(&m), ["mcp", "login", "mobbin", "-c", "mcp_servers.mobbin.url=\"https://api.mobbin.com/mcp\""]);
        assert_eq!(codex_list_args(&m)[..3], ["mcp", "list", "--json"]);
        let plan = signin_plan(Engine::Claude, "/bin/claude", Path::new("/h/squads/acme/mcp-signin"), &m);
        assert_eq!((plan.program.as_str(), plan.args.join(" "), plan.cwd.as_deref()), ("/bin/claude", "mcp login mobbin --no-browser".to_owned(), Some(Path::new("/h/squads/acme/mcp-signin"))));
        assert!(signin_plan(Engine::Codex, "codex", Path::new("/d"), &m).cwd.is_none());
    }

    // Recorded from Claude Code 2.1.308 (`claude mcp get`), the address a stand-in's.
    const GET_NEEDS: &str = "mobbin:\n  Scope: Local config (private to you in this project)\n  Status: ! Needs authentication\n  Type: http\n  URL: https://api.mobbin.com/mcp\n\nTo remove this server, run: claude mcp remove mobbin -s local\n";
    const GET_OK: &str = "mobbin:\n  Scope: Local config (private to you in this project)\n  Status: \u{2714} Connected\n  Type: http\n  URL: https://api.mobbin.com/mcp\n";
    const GET_LOCKED: &str = "mobbin:\n  Scope: Local config (private to you in this project)\n  Status: \u{2718} Failed to connect\n  Issue: Couldn't read the stored credentials for MCP server \"mobbin\", so the request didn't run. If your Mac's keychain is locked, unlock it, then try again.\n  Type: http\n  URL: https://api.mobbin.com/mcp\n";
    const GET_MISSING: &str = "No MCP server named \"mobbin\". Configured servers: other\n";

    #[test]
    fn claude_codes_answer_reads_connected_needs_sign_in_or_failing() {
        let m = mobbin();
        assert_eq!(parse_claude_get(GET_NEEDS, &m), ClaudeGet::Status(McpState::NeedsSignIn, None));
        assert_eq!(parse_claude_get(GET_OK, &m), ClaudeGet::Status(McpState::Connected, None));
        match parse_claude_get(GET_LOCKED, &m) {
            ClaudeGet::Status(McpState::Failing, Some(why)) => assert!(why.contains("keychain is locked"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_claude_get(GET_MISSING, &m), ClaudeGet::Missing);
        assert_eq!(parse_claude_get(&GET_OK.replace("api.mobbin.com/mcp", "other.example.com/mcp"), &m), ClaudeGet::Other, "another address under the name");
        assert_eq!(parse_claude_get(&GET_OK.replace("Type: http", "Type: sse"), &m), ClaudeGet::Other);
        let tools_failed = GET_OK.replace("\u{2714} Connected", "! Connected \u{b7} tools fetch failed");
        assert!(matches!(parse_claude_get(&tools_failed, &m), ClaudeGet::Status(McpState::Failing, _)));
    }

    #[test]
    fn codexs_answer_reads_its_auth_status() {
        let listing = |auth: &str| format!(r#"[{{"name":"shadcn","enabled":true,"auth_status":"unsupported"}},{{"name":"mobbin","enabled":true,"transport":{{"type":"streamable_http","url":"https://api.mobbin.com/mcp"}},"auth_status":"{auth}"}}]"#);
        assert_eq!(parse_codex_list(&listing("not_logged_in"), "mobbin"), (McpState::NeedsSignIn, None));
        assert_eq!(parse_codex_list(&listing("o_auth"), "mobbin"), (McpState::Connected, None));
        assert_eq!(parse_codex_list(&listing("unsupported"), "mobbin").0, McpState::Connected, "a server that asks for no sign-in");
        assert_eq!(parse_codex_list(&listing("o_auth"), "other").0, McpState::Failing);
        assert_eq!(parse_codex_list("error: unknown flag", "mobbin").0, McpState::Failing);
    }

    #[test]
    fn a_note_goes_only_to_a_member_with_servers_and_says_what_each_is_for() {
        assert_eq!(charter_note(&[]), None);
        let m = mobbin();
        let note = charter_note(&[&m]).unwrap();
        assert!(note.contains("`mobbin` (Mobbin: search real product screens and flows for design references; tools `mcp__mobbin__*`)"), "{note}");
        assert!(note.contains("needs a sign-in") && note.contains("one card"), "{note}");
    }

    fn fake(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("equipe-squad-mcp-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[tokio::test]
    async fn claude_code_gets_the_server_added_in_its_folder_once_then_says_where_it_stands() {
        // A stand-in `claude`: `get` answers from a file `add` writes, in the folder it runs in.
        let script = fake(
            "claude",
            r#"case "$2" in
  get) if [ -f added ]; then printf 'mobbin:\n  Status: ! Needs authentication\n  Type: http\n  URL: %s\n' "$(cat added)"; else echo 'No MCP server named "mobbin".'; exit 1; fi ;;
  add) echo "$8" > added; echo "$@" >> calls ;;
  remove) rm -f added; echo "$@" >> calls ;;
esac"#,
        );
        let dir = script.parent().unwrap().join("signin");
        let _ = std::fs::remove_dir_all(&dir);
        let m = mobbin();
        let first = check(Engine::Claude, script.to_str().unwrap(), &dir, &m, 7).await;
        assert_eq!((first.state, first.checked_at), (McpState::NeedsSignIn, 7));
        let again = check(Engine::Claude, script.to_str().unwrap(), &dir, &m, 8).await;
        assert_eq!(again.state, McpState::NeedsSignIn);
        let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
        assert_eq!(calls.lines().count(), 1, "added once: {calls}");
        assert!(calls.contains("--scope local --transport http mobbin https://api.mobbin.com/mcp"));
        // Moved to another address: the old entry goes, the new one comes.
        let moved = McpServer { url: "https://mcp.example.com/m".into(), catalog: None, ..m.clone() };
        assert_eq!(check(Engine::Claude, script.to_str().unwrap(), &dir, &moved, 9).await.state, McpState::NeedsSignIn);
        let calls = std::fs::read_to_string(dir.join("calls")).unwrap();
        assert!(calls.lines().nth(1).unwrap().starts_with("mcp remove --scope local mobbin") && calls.lines().nth(2).unwrap().contains("https://mcp.example.com/m"), "{calls}");
    }

    #[tokio::test]
    async fn codex_is_asked_with_the_server_as_an_override_and_a_missing_tool_is_failing() {
        let script = fake("codex", r#"echo "$@" > "$(dirname "$0")/asked"; printf '[{"name":"mobbin","enabled":true,"auth_status":"o_auth"}]\n'"#);
        let m = mobbin();
        let status = check(Engine::Codex, script.to_str().unwrap(), Path::new("/nowhere"), &m, 1).await;
        assert_eq!(status.state, McpState::Connected);
        let asked = std::fs::read_to_string(script.parent().unwrap().join("asked")).unwrap();
        assert_eq!(asked.trim(), "mcp list --json -c mcp_servers.mobbin.url=\"https://api.mobbin.com/mcp\"");
        let gone = check(Engine::Codex, "/no/such/codex", Path::new("/nowhere"), &m, 1).await;
        assert_eq!(gone.state, McpState::Failing);
        assert!(gone.detail.unwrap().contains("/no/such/codex"));
        let sse = McpServer { transport: McpTransport::Sse, ..m };
        assert_eq!(check(Engine::Codex, script.to_str().unwrap(), Path::new("/nowhere"), &sse, 1).await.state, McpState::Failing);
    }
}
