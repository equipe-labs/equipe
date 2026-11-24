//! Outside MCP servers a squad connects for its members: a design library's
//! official server from the Connections catalog (Mobbin, 21st.dev), or any
//! server added by its address. Kept in the squad's settings
//! ([`crate::journal::Journal::mcp_servers`]) and given to the members of its
//! roles from their next turn.
//!
//! The person connects a server to equipe once, and it works for every
//! engine a member runs on — Claude Code, Codex, and the model providers that
//! run on Codex's agent. Who holds the credential is the server's [`McpAuth`]:
//!
//! - [`McpAuth::Oauth`] (the default): equipe's own connection. A server that
//!   asks for no sign-in needs nothing; one that does is signed in to once,
//!   with equipe as the OAuth client ([`crate::mcp_oauth`]), the tokens kept
//!   in the squad's secret store and refreshed there.
//! - [`McpAuth::Key`]: an API key the person saved through the key card
//!   (21st.dev's `x-api-key`), in the same store.
//!
//! Either way each member's engine gets a stdio entry for equipe's proxy
//! (`equipe mcp proxy --squad … --server … --member … --home …`): it holds
//! nothing, and the daemon forwards each message to the server with the
//! credential added ([`crate::mcp_gateway`]). No token, key or header is in
//! a member's config, environment, files or process. Where the server
//! stands is equipe's own connection's ([`probe`]), the same for every engine.
//!
//! - [`McpAuth::Engine`]: the fallback for a server that won't let other
//!   apps sign in (no dynamic client registration). Each engine's own CLI
//!   signs in (`claude mcp login`, in local scope in a folder of equipe's per
//!   squad, [`signin_dir`]; `codex mcp login` with a config override) and
//!   members get the server's address; status comes from those tools
//!   (`claude mcp get`, `codex mcp list --json`). equipe never reads, keeps
//!   or passes such a token.
//!
//! - [`McpAuth::Local`]: a local server, a command on this machine speaking
//!   MCP over stdio (`equipe mcp add <name> --stdio -- <command> [args]`).
//!   Each member's engine runs the command itself, with no `env` of equipe's:
//!   it inherits what the engine gives its MCP servers (Claude Code: the
//!   member's own environment; Codex: its safe default set). equipe never
//!   adds a credential to it and never takes one out. Only the person adds
//!   one (the CLI or a screen of theirs); a member's settings change can't
//!   ([`server`] takes addresses only). Where it stands is equipe's own look:
//!   the command started and asked to `initialize` ([`probe_local`]).

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
    if loopback_allowed()
        && let Some(url) = loopback_url(url)
    {
        return Ok(url);
    }
    let url = connections::clean_url(url)?;
    if url.chars().any(|c| matches!(c, '"' | '\'' | '\\' | '`' | '<' | '>' | '{' | '}' | '|' | '^')) {
        return Err(format!("{url:?}: not a plain address (an https address like https://api.mobbin.com/mcp)."));
    }
    Ok(url)
}

/// Whether a server may be on this machine's own loopback address
/// (`http://127.0.0.1:<port>/…`): only in a debug build run with
/// `EQUIPE_MCP_ALLOW_LOOPBACK=1`, to try the flows end to end against a
/// stand-in server. A release build never takes such an address.
fn loopback_allowed() -> bool {
    cfg!(debug_assertions) && std::env::var("EQUIPE_MCP_ALLOW_LOOPBACK").is_ok_and(|v| v == "1")
}

/// `url` when it is `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…`, plain.
fn loopback_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let (host, port) = authority.rsplit_once(':')?;
    let plain = !url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '\\' | '`' | '<' | '>' | '{' | '}' | '|' | '^' | '@'));
    (plain && matches!(host, "127.0.0.1" | "localhost") && port.parse::<u16>().is_ok_and(|p| p > 0)).then(|| url.to_owned())
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
    // Said first: a local server is the person's own command, never an address.
    let transport = match transport {
        Some(word) => McpTransport::parse(word).filter(|t| *t != McpTransport::Stdio).ok_or_else(|| format!("{word:?}: a transport is http (streamable HTTP) or sse (a local server is added by the person with `equipe mcp add <name> --stdio -- <command>`)."))?,
        None => McpTransport::Http,
    };
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
    // The catalog's own server, at its own address: remembered as the catalog's, its key with it.
    let catalog = catalog.filter(|(_, mcp)| mcp.url == url);
    let auth = match catalog.and_then(|(id, mcp)| Some((Service::by_id(id)?.key_name?, mcp.key_header?))) {
        Some((secret, header)) => McpAuth::Key { secret: secret.to_owned(), header: header.to_owned() },
        None => McpAuth::Oauth,
    };
    Ok(McpServer { name, url, transport, command: Vec::new(), roles, enabled: true, catalog: catalog.map(|(id, _)| id.to_owned()), auth })
}

/// At most this many words in a local server's command line, each at most [`LONGEST_WORD`] long.
const MOST_WORDS: usize = 64;
const LONGEST_WORD: usize = 4096;

/// A local server's command line: a program, then its arguments — plain
/// words (no control character), the program not an option.
pub fn check_command(command: &[String]) -> Result<Vec<String>, String> {
    let Some(program) = command.first().map(|p| p.trim()).filter(|p| !p.is_empty()) else {
        return Err("Give the server's command after `--` (`equipe mcp add tools --stdio -- node ~/tools/server.mjs`).".to_owned());
    };
    if program.starts_with('-') {
        return Err(format!("{program:?}: the command's first word is the program to run."));
    }
    if command.len() > MOST_WORDS || command.iter().any(|w| w.len() > LONGEST_WORD || w.chars().any(char::is_control)) {
        return Err(format!("A server's command is at most {MOST_WORDS} plain words."));
    }
    let mut out = command.to_vec();
    out[0] = program.to_owned();
    Ok(out)
}

/// A local server as asked for, checked: `name`, its command line and the
/// roles (left out: every member). It holds no credential ([`McpAuth::Local`]).
pub fn stdio_server(name: &str, command: &[String], roles: Option<&[String]>) -> Result<McpServer, String> {
    let name = check_name(name)?;
    let command = check_command(command)?;
    let roles = roles.map(check_roles).transpose()?.unwrap_or_default();
    Ok(McpServer { name, url: String::new(), transport: McpTransport::Stdio, command, roles, enabled: true, catalog: None, auth: McpAuth::Local })
}

/// A server a screen or the CLI asked for, checked again in the daemon: a
/// local one by its command, any other by its address.
pub fn checked(asked: &McpServer) -> Result<McpServer, String> {
    if asked.is_local() || asked.auth == McpAuth::Local {
        let mut server = stdio_server(&asked.name, &asked.command, Some(&asked.roles))?;
        server.enabled = asked.enabled;
        return Ok(server);
    }
    let mut server = server(&asked.name, Some(&asked.url), Some(&asked.roles), Some(asked.transport.code()))?;
    server.enabled = asked.enabled;
    // The engine-signed fallback, when asked for (and the server isn't the catalog's keyed one).
    if asked.auth.is_engine() && !matches!(server.auth, McpAuth::Key { .. }) {
        server.auth = McpAuth::Engine;
    }
    Ok(server)
}

/// How a member's engine is given a server at launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// The engine signs in itself and keeps its own token: its address.
    Engine,
    /// equipe's proxy, which adds the credential in the daemon.
    Proxy,
    /// Its key isn't saved yet (`secret`): the server is left out, and the key card asked for.
    MissingKey { secret: String },
    /// equipe isn't signed in to it: left out, and the sign-in card asked for.
    NeedsSignIn,
    /// A local server: the engine runs its command, nothing to hold.
    Local,
}

/// How `server` reaches a member: `kept`, whether a keyed server's key is
/// saved; `state`, where it was last seen to stand.
pub fn credential(server: &McpServer, kept: bool, state: Option<McpState>) -> Credential {
    match &server.auth {
        McpAuth::Local => Credential::Local,
        McpAuth::Engine => Credential::Engine,
        McpAuth::Key { secret, .. } if !kept => Credential::MissingKey { secret: secret.clone() },
        McpAuth::Key { .. } => Credential::Proxy,
        McpAuth::Oauth if state == Some(McpState::NeedsSignIn) => Credential::NeedsSignIn,
        McpAuth::Oauth => Credential::Proxy,
    }
}

/// What a member's proxy entry runs: equipe's own binary, for this squad,
/// member and home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyCommand {
    pub binary: PathBuf,
    pub squad: String,
    pub member: String,
    pub home: PathBuf,
}

impl ProxyCommand {
    /// `mcp proxy --squad … --server … --member … --home …`: names only, never a credential.
    pub fn args(&self, server: &str) -> Vec<String> {
        let mut args = strs(&["mcp", "proxy", "--squad"]);
        args.extend([self.squad.clone(), "--server".into(), server.to_owned(), "--member".into(), self.member.clone(), "--home".into(), self.home.to_string_lossy().into_owned()]);
        args
    }
}

/// Where a keyed server ([`McpAuth::Key`]) stands: on whether its key is
/// kept — connected, or waiting for its key. Never "needs sign-in": no
/// engine's sign-in can give it a key.
pub fn key_state(kept: bool) -> McpState {
    if kept { McpState::Connected } else { McpState::NeedsKey }
}

/// The catalog service a server is, for its words and links.
pub fn service_of(server: &McpServer) -> Option<&'static Service> {
    server.catalog.as_deref().and_then(Service::by_id).filter(|s| s.is_mcp())
}

/// The catalog service a server is for display (its logo, its row): the one
/// it was added from, else the one whose official server it is by address —
/// a server added by URL (`https://mcp.figma.com/mcp`) is still Figma's.
pub fn catalog_service(server: &McpServer) -> Option<&'static Service> {
    service_of(server).or_else(|| {
        let host = crate::vault::site_of(&server.url);
        crate::connections::CATALOG.iter().find(|s| !host.is_empty() && s.mcp.is_some_and(|m| crate::vault::site_of(m.url) == host))
    })
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

/// The servers a member of `role` on `engine` gets: on, for its role (or,
/// for the PM, a design library's search to show references with:
/// [`for_references`]), over a transport its engine speaks.
pub fn for_member<'a>(servers: &'a [McpServer], role: &str, engine: Engine) -> Vec<&'a McpServer> {
    servers.iter().filter(|s| s.enabled && (s.serves(role) || for_references(s, role)) && s.reaches(engine)).collect()
}

/// The design libraries from the Connections catalog whose search the PM
/// may use to show the person references (`show_references`).
pub const REFERENCE_LIBRARIES: [&str; 2] = ["mobbin", "21st"];

/// Whether a member of `role` gets `server` only to show references: the PM,
/// a catalog design library not connected for the PM's role, reached through
/// equipe's proxy — which lets through its tools that only read
/// ([`crate::mcp_gateway::ReadOnly`]) and nothing else.
pub fn for_references(server: &McpServer, role: &str) -> bool {
    role == crate::roles::PM && !server.serves(role) && server.auth.proxied() && server.catalog.as_deref().is_some_and(|c| REFERENCE_LIBRARIES.contains(&c))
}

/// What the PM is told of the design libraries it may search to show
/// references; `None` when it has none — nobody else's charter changes.
pub fn reference_note(servers: &[&McpServer], role: &str) -> Option<String> {
    let names: Vec<String> = servers.iter().filter(|s| for_references(s, role)).map(|s| format!("`mcp__{}__*`", s.name)).collect();
    (!names.is_empty()).then(|| {
        format!(
            "You may search {} read-only, only to show the person references: when they ask to see components, screens or \
examples, search there and show the best with `show_references` in the same turn. Their other tools are off for you; the \
design work itself stays the designer's.",
            names.join(" and ")
        )
    })
}

/// A server's entry in a Claude Code member's `--mcp-config`: equipe's
/// proxy over stdio; for the engine-signed fallback, its type and URL — the
/// same name, type and URL (and no headers) the sign-in was made for, so
/// Claude Code finds the token in its own store.
pub fn claude_entry(server: &McpServer, credential: &Credential, proxy: &ProxyCommand) -> Value {
    match credential {
        Credential::Engine => json!({ "type": server.transport.code(), "url": server.url }),
        // Its own command, and no `env`: nothing added to the member's environment, nothing taken out.
        Credential::Local => json!({ "type": "stdio", "command": server.command.first(), "args": server.command.get(1..).unwrap_or_default() }),
        _ => json!({ "type": "stdio", "command": proxy.binary, "args": proxy.args(&server.name) }),
    }
}

/// A server's entry in a Codex member's thread config: equipe's proxy; for
/// the engine-signed fallback, its URL (Codex finds its own sign-in by name
/// and URL). `approved`: its tools run without asking.
pub fn codex_entry(server: &McpServer, approved: bool, credential: &Credential, proxy: &ProxyCommand) -> Value {
    match credential {
        Credential::Engine => equipe_providers::codex_app::mcp_http_server(&server.url, approved),
        Credential::Local => equipe_providers::codex_app::mcp_server(Path::new(server.command.first().map_or("", String::as_str)), server.command.get(1..).unwrap_or_default(), approved),
        _ => equipe_providers::codex_app::mcp_server(&proxy.binary, &proxy.args(&server.name), approved),
    }
}

/// Where an earlier equipe wrote a member's MCP config when it carried a key
/// (a key now stays in the daemon): removed at the member's next launch.
pub fn private_config_path(home: &Path, session: u64) -> PathBuf {
    home.join("credentials").join("mcp").join(format!("{session}.json"))
}

// ───────────────────────── equipe's own connection ─────────────────────────

/// How long equipe's own look at a server may take.
const PROBE_WAIT: Duration = Duration::from_secs(20);

/// Where a server stands as equipe's own connection finds it: `header`, the
/// credential (none: not signed in yet, or no key). An `initialize` it
/// answers is connected; a refusal means a sign-in (or, `keyed`, its key).
pub async fn probe(http: &reqwest::Client, server: &McpServer, header: Option<(&str, &str)>, keyed: bool) -> (McpState, Option<String>) {
    let request = match server.transport {
        McpTransport::Stdio => return probe_local(server).await,
        McpTransport::Http => http.post(&server.url).header("content-type", "application/json").header("accept", "application/json, text/event-stream").body(crate::mcp_oauth::initialize_message().to_string()),
        McpTransport::Sse => http.get(&server.url).header("accept", "text/event-stream"),
    };
    let request = match header {
        Some((name, value)) => request.header(name, value),
        None => request,
    };
    let response = match tokio::time::timeout(PROBE_WAIT, request.send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) if e.is_connect() => return (McpState::Failing, Some("it couldn't be reached".to_owned())),
        Ok(Err(_)) => return (McpState::Failing, Some("the connection to it failed".to_owned())),
        Err(_) => return (McpState::Failing, Some(format!("it didn't answer in {} s", PROBE_WAIT.as_secs()))),
    };
    let status = response.status().as_u16();
    // A session it opened for the look is closed again.
    if let Some(id) = response.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(str::to_owned) {
        let mut close = http.delete(&server.url).header("mcp-session-id", id);
        if let Some((name, value)) = header {
            close = close.header(name, value);
        }
        drop(response);
        let _ = tokio::time::timeout(Duration::from_secs(5), close.send()).await;
    }
    match status {
        200..=299 => (McpState::Connected, None),
        401 | 403 if keyed => (McpState::NeedsKey, header.map(|_| "the saved key was refused".to_owned())),
        401 | 403 => (McpState::NeedsSignIn, header.map(|_| "the sign-in was refused".to_owned())),
        other => (McpState::Failing, Some(format!("it answered {other}"))),
    }
}

/// Where a local server stands: its command started (in the home folder,
/// the daemon's environment, input and output piped) and asked to
/// `initialize`. Any answer is connected; the process is stopped after.
pub async fn probe_local(server: &McpServer) -> (McpState, Option<String>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let Some(program) = server.command.first() else {
        return (McpState::Failing, Some("it has no command".to_owned()));
    };
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&server.command[1..]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    if let Some(home) = std::env::var_os("HOME") {
        cmd.current_dir(home);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (McpState::Failing, Some(format!("{program} isn't on this machine"))),
        Err(e) => return (McpState::Failing, Some(short(&format!("{program} couldn't start: {e}")))),
    };
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return (McpState::Failing, Some("its standard streams couldn't be opened".to_owned()));
    };
    let look = async {
        let mut message = crate::mcp_oauth::initialize_message().to_string();
        message.push('\n');
        input.write_all(message.as_bytes()).await.map_err(|_| "it closed its input".to_owned())?;
        input.flush().await.map_err(|_| "it closed its input".to_owned())?;
        let mut lines = tokio::io::BufReader::new(output).lines();
        while let Some(line) = lines.next_line().await.map_err(|_| "its output couldn't be read".to_owned())? {
            let Ok(reply) = serde_json::from_str::<Value>(line.trim()) else { continue };
            if reply.get("id").is_some() && reply.get("result").is_some() {
                return Ok(());
            }
            if let Some(error) = reply.get("error") {
                return Err(short(&format!("it refused to initialize: {}", error["message"].as_str().unwrap_or("no reason"))));
            }
        }
        Err("it stopped without answering".to_owned())
    };
    let seen = match tokio::time::timeout(PROBE_WAIT, look).await {
        Ok(Ok(())) => (McpState::Connected, None),
        Ok(Err(why)) => (McpState::Failing, Some(why)),
        Err(_) => (McpState::Failing, Some(format!("it didn't answer in {} s", PROBE_WAIT.as_secs()))),
    };
    let _ = child.kill().await;
    seen
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
references instead of browsing its site. When one says it needs a sign-in or a key, say so in a line and carry on \
without it: equipe has already put one card for it in front of the person.",
        each.join("; ")
    ))
}

/// What a catalog MCP service is for, in English, for a member's charter.
fn purpose(id: &str) -> &'static str {
    match id {
        "mobbin" => "search real product screens and flows for design references",
        "21st" => "proven React, Tailwind and shadcn/ui components to adapt",
        "figma" => "read and draw design files",
        "framer" => "build and edit Framer sites",
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

/// One change at a time to Claude Code's settings for a squad's sign-in
/// folder, across processes (the daemon and `equipe mcp …` alike): two
/// `claude mcp add`s of one server at once leave the second "already exists
/// in local config". Held while the lock file's descriptor is open.
struct ConfigLock(#[allow(dead_code)] std::fs::File);

impl ConfigLock {
    async fn take(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join(".equipe-lock")).map_err(|e| format!("{}: {e}", dir.display()))?;
        let deadline = tokio::time::Instant::now() + CHECK_WAIT * 2;
        loop {
            // An exclusive lock, not waited for: `flock` on Unix, `LockFileEx` on Windows.
            if file.try_lock().is_ok() {
                return Ok(Self(file));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("another change to Claude Code's MCP settings didn't finish".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn first_line(said: &str, otherwise: &str) -> String {
    short(said.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(otherwise))
}

/// Make sure `server` is configured, as members name it, in `dir` for
/// Claude Code (`claude mcp add`, in local scope): what `claude mcp login`
/// and `claude mcp get` need. Where it stands, as `claude mcp get` says.
///
/// Idempotent: one already there at the same address is reused as it is
/// (its sign-in with it); one at another address is replaced. Never fails
/// because the server "already exists".
pub async fn prepare_claude(binary: &str, dir: &Path, server: &McpServer) -> Result<(McpState, Option<String>), String> {
    let _lock = ConfigLock::take(dir).await?;
    let get_args = claude_get_args(&server.name);
    let get = || run(binary, &get_args, Some(dir));
    let (_, said) = get().await?;
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
        if !said.contains("already exists") {
            return Err(first_line(&said, "claude mcp add failed"));
        }
        // Kept meanwhile, or by an equipe from before this lock: the same
        // server is reused; anything else under the name is replaced.
        let (_, again) = get().await?;
        if let ClaudeGet::Status(state, detail) = parse_claude_get(&again, server) {
            return Ok((state, detail));
        }
        let _ = run(binary, &claude_remove_args(&server.name), Some(dir)).await?;
        let (added, said) = run(binary, &claude_add_args(server), Some(dir)).await?;
        if !added {
            return Err(first_line(&said, "claude mcp add failed"));
        }
    }
    let (_, said) = get().await?;
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
        Engine::Claude if dir.is_dir() => {
            let Ok(_lock) = ConfigLock::take(dir).await else { return };
            run(binary, &claude_remove_args(&server.name), Some(dir)).await
        }
        Engine::Claude => return,
        Engine::Codex => run(binary, &codex_logout_args(server), None).await,
    };
}

/// The sign-in a card runs for an engine-signed `server` on `engine`: the
/// engine's own interactive login, in a terminal of its own, its page printed for the card.
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
    fn a_server_reaches_only_its_roles_when_on_through_the_proxy_on_any_engine() {
        let mut servers = vec![mobbin(), server("docs", Some("https://docs.example.com/mcp"), None, Some("sse")).unwrap()];
        let names = |role: &str, engine: Engine, s: &[McpServer]| for_member(s, role, engine).iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("designer", Engine::Claude, &servers), ["mobbin", "docs"]);
        assert_eq!(names("designer", Engine::Codex, &servers), ["mobbin", "docs"], "through the proxy, Codex too, over SSE as well");
        servers[1].auth = McpAuth::Engine;
        assert_eq!(names("designer", Engine::Codex, &servers), ["mobbin"], "signed in by the engine: Codex speaks streamable HTTP only");
        servers[1].auth = McpAuth::Oauth;
        assert_eq!(names("backend", Engine::Claude, &servers), ["docs"], "no roles named: everyone");
        servers[0].enabled = false;
        assert_eq!(names("frontend", Engine::Claude, &servers), ["docs"], "off: no one");
        // Replaced in place by name; at most MOST_SERVERS.
        let replaced = with(&servers, mobbin()).unwrap();
        assert_eq!((replaced.len(), replaced[0].enabled), (2, true));
        let full: Vec<McpServer> = (0..MOST_SERVERS).map(|i| server(&format!("s{i}"), Some("https://x.example.com/mcp"), None, None).unwrap()).collect();
        assert!(with(&full, mobbin()).is_err());
    }

    fn proxy() -> ProxyCommand {
        ProxyCommand { binary: PathBuf::from("/opt/equipe"), squad: "acme".into(), member: "dana".into(), home: PathBuf::from("/h") }
    }

    #[test]
    fn every_engine_gets_equipes_proxy_and_never_a_token() {
        let m = mobbin();
        assert_eq!(m.auth, McpAuth::Oauth, "equipe signs in itself");
        assert_eq!(credential(&m, false, None), Credential::Proxy, "not looked at yet: given, the proxy says where it stands");
        assert_eq!(credential(&m, false, Some(McpState::NeedsSignIn)), Credential::NeedsSignIn);
        let claude = claude_entry(&m, &Credential::Proxy, &proxy());
        assert_eq!(claude, json!({ "type": "stdio", "command": "/opt/equipe", "args": ["mcp", "proxy", "--squad", "acme", "--server", "mobbin", "--member", "dana", "--home", "/h"] }));
        let codex = codex_entry(&m, false, &Credential::Proxy, &proxy());
        assert_eq!((codex["command"].as_str(), codex["args"][1].as_str(), codex["default_tools_approval_mode"].as_str()), (Some("/opt/equipe"), Some("proxy"), Some("prompt")));
        // The keyed server: the key stays in the daemon too.
        let ui = server("21st", None, None, None).unwrap();
        assert_eq!(credential(&ui, false, None), Credential::MissingKey { secret: "21st-api-key".into() });
        assert_eq!(credential(&ui, true, None), Credential::Proxy);
        for entry in [claude, codex, claude_entry(&ui, &Credential::Proxy, &proxy()), codex_entry(&ui, true, &Credential::Proxy, &proxy())] {
            let text = entry.to_string().to_ascii_lowercase();
            for word in ["header", "token", "authorization", "bearer", "x-api-key", "env", "https://"] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
        assert_eq!(allowed_rule(&m), "mcp__mobbin");
    }

    #[test]
    fn the_engine_signed_fallback_gets_the_address_as_before() {
        let m = McpServer { auth: McpAuth::Engine, ..mobbin() };
        assert_eq!(credential(&m, false, Some(McpState::NeedsSignIn)), Credential::Engine);
        assert_eq!(claude_entry(&m, &Credential::Engine, &proxy()), json!({ "type": "http", "url": "https://api.mobbin.com/mcp" }));
        assert_eq!(codex_entry(&m, false, &Credential::Engine, &proxy())["url"], "https://api.mobbin.com/mcp");
    }

    #[test]
    fn a_codex_members_thread_has_the_server_over_the_persons_own_of_that_name() {
        use equipe_providers::codex_app::{others_off, thread_config};
        let m = mobbin();
        let ours = json!({ "equipe": { "command": "/opt/equipe" }, "mobbin": codex_entry(&m, true, &Credential::Proxy, &proxy()) });
        // The person's own Codex servers: one named the same (theirs is replaced, not switched off), one other (off).
        let listing = json!({ "data": [{ "name": "mobbin" }, { "name": "shadcn" }] });
        let config = thread_config(Some(&ours), Some(&others_off(&listing, &ours)));
        assert_eq!(config["mcp_servers"]["mobbin"]["args"][1], "proxy");
        assert!(config["mcp_servers"]["mobbin"].get("enabled").is_none(), "ours, not switched off");
        assert_eq!(config["mcp_servers"]["shadcn"], json!({ "enabled": false }));
        assert!(!config.to_string().to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn a_keyed_server_is_the_catalogs_at_its_own_address_only() {
        let ui = server("21st", None, None, None).unwrap();
        assert_eq!(ui.auth, McpAuth::Key { secret: "21st-api-key".into(), header: "x-api-key".into() });
        assert_eq!((ui.url.as_str(), ui.roles.clone()), ("https://21st.dev/api/mcp", vec!["designer".to_owned(), "frontend".to_owned()]));
        // At another address, someone's own server: no key of the catalog's goes to it.
        assert_eq!(server("21st", Some("https://mcp.example.com/x"), None, None).unwrap().auth, McpAuth::Oauth);
        assert_eq!(key_state(false), McpState::NeedsKey, "never needs sign-in");
        assert_eq!(key_state(true), McpState::Connected);
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
    fn the_pm_searches_the_design_libraries_only_to_show_references() {
        let m = mobbin();
        let ui = server("21st", None, None, None).unwrap();
        let own = server("docs", Some("https://docs.example.com/mcp"), Some(&["designer".to_owned()]), None).unwrap();
        let servers = vec![m.clone(), ui.clone(), own];
        let names = |role: &str| for_member(&servers, role, Engine::Claude).iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(crate::roles::PM), ["mobbin", "21st"], "the libraries, not the squad's own server for designers");
        assert!(for_references(&m, crate::roles::PM) && !for_references(&m, "designer") && !for_references(&m, "backend"));
        assert!(names("backend").is_empty());
        // Connected for the PM by the person: all of it, as for anyone it serves.
        let mine = McpServer { roles: vec![crate::roles::PM.to_owned()], ..m.clone() };
        assert!(!for_references(&mine, crate::roles::PM));
        // Signed in by each engine itself: equipe can't keep it to reading, so not given.
        let engine_signed = McpServer { auth: McpAuth::Engine, ..m.clone() };
        assert!(!for_references(&engine_signed, crate::roles::PM));
        let note = reference_note(&[&m, &ui], crate::roles::PM).unwrap();
        assert!(note.contains("`mcp__mobbin__*` and `mcp__21st__*` read-only") && note.contains("`show_references` in the same turn"), "{note}");
        assert_eq!(reference_note(&[&m], "designer"), None);
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

    fn words(w: &[&str]) -> Vec<String> {
        strs(w)
    }

    #[test]
    fn a_local_server_is_its_command_given_to_every_engine_as_it_is_with_nothing_added() {
        let local = stdio_server("Equipe-Platform", &words(&[" node ", "/home/x/server.mjs", "--quiet"]), None).unwrap();
        assert_eq!((local.name.as_str(), local.url.as_str(), local.transport, local.auth.clone(), local.roles.is_empty()), ("equipe-platform", "", McpTransport::Stdio, McpAuth::Local, true));
        assert_eq!(local.command, words(&["node", "/home/x/server.mjs", "--quiet"]));
        assert_eq!(credential(&local, false, Some(McpState::Failing)), Credential::Local, "always given: the member's engine starts it");
        let proxy = ProxyCommand { binary: "/opt/equipe".into(), squad: "acme".into(), member: "backend".into(), home: "/h".into() };
        // Claude Code: its own command, no `env` (nothing injected, nothing stripped), no proxy.
        assert_eq!(claude_entry(&local, &Credential::Local, &proxy), json!({ "type": "stdio", "command": "node", "args": ["/home/x/server.mjs", "--quiet"] }));
        // Codex: the same command.
        let codex = codex_entry(&local, true, &Credential::Local, &proxy);
        assert_eq!((codex["command"].clone(), codex["args"].clone(), codex.get("env")), (json!("node"), json!(["/home/x/server.mjs", "--quiet"]), None));
        assert!(local.reaches(Engine::Codex) && for_member(std::slice::from_ref(&local), "qa", Engine::Codex).len() == 1);
        assert!(charter_note(&[&local]).unwrap().contains("`equipe-platform` (tools `mcp__equipe-platform__*`)"));
        assert_eq!(allowed_rule(&local), "mcp__equipe-platform");
        // Only by the person's own command: a settings change (the PM's) takes addresses only.
        assert!(server("tools", None, None, Some("stdio")).unwrap_err().contains("--stdio"));
        // Checked again in the daemon whatever a screen sent.
        let sent = McpServer { name: "Tools".into(), command: words(&["node", "a.mjs"]), url: "ignored".into(), catalog: Some("mobbin".into()), ..local.clone() };
        let kept = checked(&sent).unwrap();
        assert_eq!((kept.name.as_str(), kept.url.as_str(), kept.catalog.clone(), kept.auth.clone()), ("tools", "", None, McpAuth::Local));
        assert!(checked(&McpServer { command: Vec::new(), ..local.clone() }).is_err());
        for bad in [&[][..], &[""][..], &["--help"][..], &["node", "a\nb"][..]] {
            assert!(stdio_server("tools", &words(bad), None).is_err(), "{bad:?}");
        }
        assert!(stdio_server("equipe", &words(&["node"]), None).is_err(), "equipe's own names stay its own");
        assert!(stdio_server("tools", &vec!["x".to_owned(); MOST_WORDS + 1], None).is_err());
    }

    #[tokio::test]
    async fn a_local_server_stands_on_its_command_answering_initialize() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("equipe-local-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_string_lossy().into_owned()
        };
        let answers = script("answers.sh", r#"read line; echo 'not json'; echo '{"jsonrpc":"2.0","id":0,"result":{"capabilities":{}}}'; cat >/dev/null"#);
        let refuses = script("refuses.sh", r#"read line; echo '{"jsonrpc":"2.0","id":0,"error":{"code":-1,"message":"no config"}}'"#);
        let quits = script("quits.sh", "exit 3");
        let at = |cmd: &str| stdio_server("tools", &words(&[cmd]), None).unwrap();
        assert_eq!(probe_local(&at(&answers)).await, (McpState::Connected, None));
        let (state, why) = probe_local(&at(&refuses)).await;
        assert_eq!(state, McpState::Failing);
        assert!(why.unwrap().contains("no config"));
        assert_eq!(probe_local(&at(&quits)).await.0, McpState::Failing);
        let (state, why) = probe_local(&at("/no/such/tool")).await;
        assert_eq!((state, why.as_deref()), (McpState::Failing, Some("/no/such/tool isn't on this machine")));
        // Through `probe`, the same look.
        assert_eq!(probe(&reqwest::Client::new(), &at(&answers), None, false).await.0, McpState::Connected);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
