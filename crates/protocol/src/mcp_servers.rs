//! Outside MCP servers a squad connects for its members (a design library's
//! official server, or any server added by its address): what is kept about
//! each, and how each engine on the daemon's machine reaches it.
//!
//! Nothing here is a credential. Who holds a server's credential is its
//! [`McpAuth`]: equipe itself (its own OAuth sign-in, the tokens in the
//! squad's secret store), an API key the person saved there, or — a
//! fallback for a server that won't let other apps sign in — the engine's
//! own command-line tool. Keys are referred to by name, never by value.

use serde::{Deserialize, Serialize};

use crate::Engine;

/// How a server is reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Streamable HTTP: both engines.
    #[default]
    Http,
    /// Server-sent events, the older transport: Claude Code only.
    Sse,
    /// A command on this machine speaking MCP on its standard streams (a
    /// local server: [`McpServer::command`]). Every engine runs it itself.
    Stdio,
}

impl McpTransport {
    pub fn code(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Sse => "sse",
            Self::Stdio => "stdio",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "" | "http" | "streamablehttp" => Some(Self::Http),
            "sse" => Some(Self::Sse),
            "stdio" => Some(Self::Stdio),
            _ => None,
        }
    }

    /// Whether `engine` can reach a server over it.
    pub fn on(self, engine: Engine) -> bool {
        self != Self::Sse || engine == Engine::Claude
    }
}

/// Who holds a server's credential, and so how a member's engine is given it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpAuth {
    /// equipe's own connection (the default, and what a record from before
    /// it reads as): no credential when the server asks for none, else
    /// equipe's own OAuth sign-in — the person signs in once, to equipe,
    /// which keeps and refreshes the tokens in the squad's secret store.
    /// Every member, whatever its engine, gets the server through equipe's
    /// proxy (`equipe mcp proxy`), which never holds the token.
    #[default]
    Oauth,
    /// An API key the person saved in the squad's secret store under
    /// `secret`, sent to the server as the header `header` by equipe's proxy.
    Key { secret: String, header: String },
    /// The fallback for a server that won't let other apps sign in: each
    /// engine's own command-line tool signs in (`claude mcp login`, `codex
    /// mcp login`) and keeps the token; members get the server's address.
    Engine,
    /// A local server ([`McpTransport::Stdio`]): no credential at all. Each
    /// member's engine runs its command itself, in the member's own
    /// environment — equipe adds no value to it and takes none away.
    Local,
}

impl McpAuth {
    /// The default: said by saying nothing.
    pub fn is_default(&self) -> bool {
        *self == Self::Oauth
    }

    pub fn is_engine(&self) -> bool {
        *self == Self::Engine
    }

    /// Reached through equipe's proxy (its own sign-in, or a kept key).
    pub fn proxied(&self) -> bool {
        matches!(self, Self::Oauth | Self::Key { .. })
    }

    /// Where it stands is one word for every engine (all but the engine-signed fallback).
    pub fn one_for_all(&self) -> bool {
        !self.is_engine()
    }
}

/// One outside server, as the squad keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// Its name, which members' tools are named after (`mcp__<name>__…`).
    pub name: String,
    /// Its address (https); empty for a local server.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub transport: McpTransport,
    /// A local server's command line ([`McpTransport::Stdio`]): the program,
    /// then its arguments. Empty for a server reached by its address.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// The roles whose members get it; empty: every member.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default = "on")]
    pub enabled: bool,
    /// The Connections catalog entry it came from, when it did (`mobbin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    /// Who holds its credential.
    #[serde(default, skip_serializing_if = "McpAuth::is_default")]
    pub auth: McpAuth,
}

fn on() -> bool {
    true
}

impl McpServer {
    /// Whether a member of `role` gets it (when it is on).
    pub fn serves(&self, role: &str) -> bool {
        self.roles.is_empty() || self.roles.iter().any(|r| r.eq_ignore_ascii_case(role))
    }

    /// Whether a member on `engine` can be given it: through equipe's proxy
    /// every engine can; the engine-signed fallback only over a transport the engine speaks.
    pub fn reaches(&self, engine: Engine) -> bool {
        self.auth.proxied() || self.transport.on(engine)
    }

    /// A command this machine runs rather than a server at an address.
    pub fn is_local(&self) -> bool {
        self.transport == McpTransport::Stdio
    }

    /// Where it is, for the person: its address, or a local server's command line.
    pub fn place(&self) -> String {
        if self.is_local() { self.command.join(" ") } else { self.url.clone() }
    }

    /// The same server as `other` is: the same name, address and command.
    pub fn same_place(&self, other: &Self) -> bool {
        self.name == other.name && self.url == other.url && self.command == other.command
    }
}

/// Where a server stands on one engine, as that engine's own tool said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpState {
    /// Not looked at yet.
    #[default]
    Unchecked,
    /// Reached, its tools listed (or no sign-in needed).
    Connected,
    /// It wants the person signed in first (the engine's own OAuth sign-in).
    NeedsSignIn,
    /// Its API key isn't saved yet ([`McpAuth::Key`]): the key card, never a sign-in.
    NeedsKey,
    /// It could not be reached (`detail` says why).
    #[serde(other)]
    Failing,
}

/// One engine's word on a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEngineStatus {
    pub engine: Engine,
    pub state: McpState,
    /// What the engine's tool said went wrong, short; never a token or an address with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When it was looked at, unix seconds.
    #[serde(default)]
    pub checked_at: u64,
}

/// A server's standing on each engine its members run on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerStatus {
    pub name: String,
    #[serde(default)]
    pub engines: Vec<McpEngineStatus>,
}

impl McpServerStatus {
    /// What a screen shows in one word: needs its key, else needs sign-in where any engine does,
    /// else failing where any does, else connected once one said so.
    pub fn overall(&self) -> McpState {
        let any = |s: McpState| self.engines.iter().any(|e| e.state == s);
        if any(McpState::NeedsKey) {
            McpState::NeedsKey
        } else if any(McpState::NeedsSignIn) {
            McpState::NeedsSignIn
        } else if any(McpState::Failing) {
            McpState::Failing
        } else if any(McpState::Connected) {
            McpState::Connected
        } else {
            McpState::Unchecked
        }
    }

    /// The first thing an engine said went wrong.
    pub fn detail(&self) -> Option<&str> {
        self.engines.iter().filter(|e| e.state != McpState::Connected).find_map(|e| e.detail.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_round_trips_and_old_or_short_records_read_with_defaults() {
        let server = McpServer { name: "mobbin".into(), url: "https://api.mobbin.com/mcp".into(), transport: McpTransport::Http, command: Vec::new(), roles: vec!["designer".into(), "frontend".into()], enabled: true, catalog: Some("mobbin".into()), auth: McpAuth::Oauth };
        assert!(!serde_json::to_string(&server).unwrap().contains("auth"), "equipe's own connection: said by saying nothing");
        // A record from before equipe signed in itself reads as equipe's own connection: it moves to the proxy.
        let old: McpServer = serde_json::from_str(r#"{"name":"mobbin","url":"https://api.mobbin.com/mcp"}"#).unwrap();
        assert_eq!(old.auth, McpAuth::Oauth);
        let engine = McpServer { auth: McpAuth::Engine, ..server.clone() };
        assert!(serde_json::to_string(&engine).unwrap().contains(r#""auth":{"kind":"engine"}"#));
        assert!(server.reaches(Engine::Codex) && McpServer { transport: McpTransport::Sse, ..server.clone() }.reaches(Engine::Codex), "through the proxy, every engine");
        assert!(!McpServer { transport: McpTransport::Sse, ..engine.clone() }.reaches(Engine::Codex));
        let keyed = McpServer { auth: McpAuth::Key { secret: "21st-api-key".into(), header: "x-api-key".into() }, ..server.clone() };
        let text = serde_json::to_string(&keyed).unwrap();
        assert!(text.contains(r#""auth":{"kind":"key","secret":"21st-api-key","header":"x-api-key"}"#), "{text}");
        assert_eq!(serde_json::from_str::<McpServer>(&text).unwrap(), keyed);
        let text = serde_json::to_string(&server).unwrap();
        assert_eq!(serde_json::from_str::<McpServer>(&text).unwrap(), server);
        let short: McpServer = serde_json::from_str(r#"{"name":"x","url":"https://x.example.com/mcp"}"#).unwrap();
        assert_eq!((short.transport, short.enabled, short.roles.is_empty(), short.catalog.as_deref()), (McpTransport::Http, true, true, None));
        assert!(short.serves("qa"), "no roles named: everyone");
        assert!(server.serves("Designer") && !server.serves("qa"));
    }

    #[test]
    fn sse_reaches_claude_code_only_and_states_read_forward() {
        assert!(McpTransport::Http.on(Engine::Codex) && McpTransport::Sse.on(Engine::Claude) && !McpTransport::Sse.on(Engine::Codex));
        assert_eq!(McpTransport::parse("streamable-http"), Some(McpTransport::Http));
        assert_eq!(McpTransport::parse("websocket"), None);
        assert_eq!(serde_json::from_str::<McpState>("\"something_new\"").unwrap(), McpState::Failing);
        let status = |states: &[McpState]| McpServerStatus { name: "m".into(), engines: states.iter().map(|s| McpEngineStatus { engine: Engine::Claude, state: *s, detail: None, checked_at: 0 }).collect() };
        assert_eq!(status(&[McpState::Connected, McpState::NeedsSignIn]).overall(), McpState::NeedsSignIn);
        assert_eq!(status(&[McpState::Connected, McpState::Failing]).overall(), McpState::Failing);
        assert_eq!(status(&[McpState::Connected]).overall(), McpState::Connected);
        assert_eq!(status(&[]).overall(), McpState::Unchecked);
        assert_eq!(status(&[McpState::NeedsKey]).overall(), McpState::NeedsKey);
        assert_eq!(serde_json::to_string(&McpState::NeedsKey).unwrap(), "\"needs_key\"");
    }

    #[test]
    fn a_local_server_round_trips_reaches_every_engine_and_holds_no_credential() {
        let local = McpServer { name: "platform".into(), url: String::new(), transport: McpTransport::Stdio, command: vec!["node".into(), "/home/x/server.mjs".into()], roles: Vec::new(), enabled: true, catalog: None, auth: McpAuth::Local };
        let text = serde_json::to_string(&local).unwrap();
        assert!(text.contains(r#""transport":"stdio""#) && text.contains(r#""command":["node","/home/x/server.mjs"]"#) && text.contains(r#""auth":{"kind":"local"}"#), "{text}");
        assert_eq!(serde_json::from_str::<McpServer>(&text).unwrap(), local);
        assert!(local.reaches(Engine::Claude) && local.reaches(Engine::Codex));
        assert!(!local.auth.proxied() && local.auth.one_for_all() && local.is_local());
        assert_eq!(local.place(), "node /home/x/server.mjs");
        assert_eq!(McpTransport::parse("stdio"), Some(McpTransport::Stdio));
        // A server at an address writes no command.
        let remote: McpServer = serde_json::from_str(r#"{"name":"x","url":"https://x.example.com/mcp"}"#).unwrap();
        assert!(!serde_json::to_string(&remote).unwrap().contains("command") && !remote.is_local());
        assert!(!remote.same_place(&local) && local.same_place(&local.clone()));
    }
}
