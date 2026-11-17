//! A squad's outside MCP servers on the daemon (`crate::squad_mcp`): kept in
//! its journal, looked at off the loop, given to members as their turns
//! start, and signed in on a sign-in card (`super::cli_signin`).
//!
//! A server equipe connects itself ([`McpAuth::Oauth`], [`McpAuth::Key`]) is
//! looked at through equipe's own connection, the same for every engine; its
//! members reach it through equipe's proxy, whose credential this hands out
//! ([`Runtime::mcp_upstream`]) and whose calls say where it stands
//! ([`Runtime::mcp_seen`]). One card per server, opened when a member who
//! gets the server finds it needs a sign-in (or its key), or when the person
//! asks. A card put off ("Not now", or left to run out after a day) is
//! offered once more when a member needs that server again — saying so — and
//! after that only when the person asks for it.
//!
//! Staying connected: every [`HEALTH_EVERY`] the daemon looks at each server
//! itself — renewing an OAuth token well before it runs out
//! ([`mcp_oauth::access_ahead`]) — so a member's call never meets a stale
//! token. Only when that fails for good (the refresh token refused, the key
//! refused) does one card go to the person, saying what to do; members are
//! told the server is unavailable and why ([`unavailable_note`]).
//!
//! The engine-signed fallback ([`McpAuth::Engine`]) is looked at through each
//! engine's own tool, and signed in with that tool, one card per engine.

use std::collections::HashSet;
use std::sync::Arc;

use crate::cli_signin::Kind;
use crate::mcp_gateway::{Seen, Upstream};
use crate::mcp_oauth::{self, NoToken};
use crate::squad_mcp::{self as sm, Credential, McpAuth, McpEngineStatus, McpServer, McpServerStatus, McpState};
use equipe_protocol::rpc::McpProxyOpen;

use super::*;

/// What the runtime keeps of the squads' outside servers besides their settings.
#[derive(Default)]
pub(super) struct McpBook {
    /// Where each server stands, by squad and server name, one entry per engine.
    status: HashMap<String, HashMap<String, Vec<McpEngineStatus>>>,
    /// Checks under way, by squad and server name, with who the sign-in
    /// card is for where one turns out needed: a member (its session, name
    /// and ticket), or the person (`Some(None)`); `None`: no card.
    checking: HashMap<(String, String), Option<CardFor>>,
    /// Sign-ins the person put off, or whose card already ended unanswered
    /// or refused, by squad, server name and engine (a server equipe signs in
    /// to itself: under Claude Code's). Kept on disk ([`put_off_file`]): a
    /// member's turn offers that card once more (saying why), then never
    /// again until the person asks, after a restart either — Connections
    /// shows it needs a sign-in meanwhile.
    put_off: HashMap<(String, String, Engine), PutOff>,
    /// Squads whose put-off sign-ins were read from disk.
    put_off_read: HashSet<String>,
    /// Keyed servers whose key card a member's turn has already put up, by
    /// squad and server name: asked once, then only when the person asks.
    key_asked: HashSet<(String, String)>,
    /// When the daemon last looked at every server itself ([`HEALTH_EVERY`]).
    health_at: u64,
}

/// A sign-in put off: since when, and whether a member's need has offered it once more since.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct PutOff {
    pub at: u64,
    pub reoffered: bool,
}

/// One put-off sign-in as kept on disk.
#[derive(serde::Serialize, serde::Deserialize)]
struct KeptPutOff {
    server: String,
    engine: Engine,
    #[serde(default)]
    at: u64,
    #[serde(default)]
    reoffered: bool,
}

/// A squad's put-off sign-ins from the file's text: this equipe's records,
/// or an earlier one's `[server, engine]` pairs (not offered again yet).
pub(super) fn parse_put_off(text: &str) -> Vec<(String, Engine, PutOff)> {
    if let Ok(kept) = serde_json::from_str::<Vec<KeptPutOff>>(text) {
        return kept.into_iter().map(|k| (k.server, k.engine, PutOff { at: k.at, reoffered: k.reoffered })).collect();
    }
    serde_json::from_str::<Vec<(String, Engine)>>(text).unwrap_or_default().into_iter().map(|(server, engine)| (server, engine, PutOff { at: 0, reoffered: false })).collect()
}

/// The servers a member gets this turn, with how; and those it doesn't, with why.
pub(super) type ForMember = (Vec<(McpServer, Credential)>, Vec<(McpServer, String)>);

/// How often the daemon looks at every server itself (and renews tokens ahead).
pub(super) const HEALTH_EVERY: u64 = 10 * 60;

/// Where a squad's put-off sign-ins are kept.
fn put_off_file(home: &Path, squad: &str) -> PathBuf {
    crate::journal::squad_dir(home, squad).join("mcp-put-off.json")
}

/// A member a sign-in card comes up for (its session, name and ticket); `None`: the person asked.
type CardFor = Option<(SessionId, String, Option<TicketId>)>;

/// How long what was seen of a server is taken as still true.
const FRESH_FOR: u64 = 6 * 3600;
/// …and that it needs a sign-in, which may have happened elsewhere meanwhile.
const RECHECK_SIGN_IN: u64 = 10 * 60;

/// A check's answer, from off the loop.
#[derive(Debug)]
pub(super) struct Checked {
    squad: String,
    server: McpServer,
    statuses: Vec<McpEngineStatus>,
}

fn kind_of(engine: Engine) -> Kind {
    match engine {
        Engine::Claude => Kind::ClaudeMcp,
        Engine::Codex => Kind::CodexMcp,
    }
}

/// The engine a put-off sign-in of a server equipe signs in to itself is kept under.
const OWN: Engine = Engine::Claude;

impl Runtime {
    /// The squad's outside servers, as kept.
    pub(super) fn mcp_servers_of(&self, squad: &str) -> Vec<McpServer> {
        self.journals.get(squad).map(Journal::mcp_servers).unwrap_or_default()
    }

    /// Where each of the squad's servers stands, in their order.
    pub(super) fn mcp_status_of(&self, squad: &str) -> Vec<McpServerStatus> {
        let known = self.mcp.status.get(squad);
        self.mcp_servers_of(squad)
            .into_iter()
            .map(|s| McpServerStatus { engines: known.and_then(|k| k.get(&s.name)).cloned().unwrap_or_default(), name: s.name })
            .collect()
    }

    fn publish_mcp(&mut self, squad: &str) {
        let (servers, status) = (self.mcp_servers_of(squad), self.mcp_status_of(squad));
        self.publish(Event::McpServers { squad: squad.to_owned(), servers, status });
    }

    /// The engines the members who get `server` run on (Codex over its CLI
    /// only); none yet: Claude Code, where most squads start.
    fn mcp_engines(&self, squad: &str, server: &McpServer) -> Vec<Engine> {
        let mut engines: Vec<Engine> = Vec::new();
        for s in self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad) && s.is_member_itself()) {
            let Some(m) = s.member.as_ref() else { continue };
            let engine = match s.engine {
                Engine::Codex if s.codex.is_some() => continue,
                e => e,
            };
            if server.serves(&m.role) && server.reaches(engine) && !engines.contains(&engine) {
                engines.push(engine);
            }
        }
        if engines.is_empty() {
            engines.push(Engine::Claude);
        }
        engines.sort_by_key(|e| *e != Engine::Claude);
        engines
    }

    /// The same word for each engine: equipe's own connection is one for all.
    fn mcp_each(&self, squad: &str, server: &McpServer, state: McpState, detail: Option<String>) -> Vec<McpEngineStatus> {
        self.mcp_engines(squad, server).into_iter().map(|engine| McpEngineStatus { engine, state, detail: detail.clone(), checked_at: now_secs() }).collect()
    }

    fn binary_of(&self, engine: Engine) -> String {
        match engine {
            Engine::Claude => self.config.claude_binary.clone(),
            Engine::Codex => self.config.codex_binary.clone(),
        }
    }

    /// Where equipe keeps a server's sign-in: its own MCP credentials file,
    /// read with nobody at the Mac ([`crate::mcp_keys`]).
    pub(super) fn mcp_store(&self) -> Arc<dyn mcp_oauth::Store> {
        Arc::new(mcp_oauth::KeptStore { home: self.config.home.clone(), vault: self.vault.clone() })
    }

    /// The daemon's own look at every squad's servers, every [`HEALTH_EVERY`]:
    /// tokens renewed ahead, keys moved out of the Keychain, statuses fresh —
    /// before a member meets a failure. One that stops working puts one card
    /// up for the person ([`Self::mcp_checked`]).
    pub(super) fn mcp_health_due(&mut self, now: u64) {
        if now.saturating_sub(self.mcp.health_at) >= HEALTH_EVERY {
            self.mcp.health_at = now;
            self.mcp_health();
        }
    }

    pub(super) fn mcp_health(&mut self) {
        let squads: Vec<String> = crate::squads::list(&self.config.home).into_iter().map(|s| s.name).collect();
        for squad in squads {
            if self.journal(&squad).is_none() {
                continue;
            }
            let names: Vec<String> = self.mcp_servers_of(&squad).into_iter().filter(|s| s.enabled && s.auth.proxied()).map(|s| s.name).collect();
            for name in names {
                self.check_mcp_servers(&squad, Some(&name), None);
            }
        }
    }

    fn keep_mcp(&mut self, squad: &str, servers: &[McpServer]) -> bool {
        match self.journal(squad).map(|j| j.set_mcp_servers(servers)) {
            Some(Ok(())) => true,
            Some(Err(err)) => {
                self.squad_error(squad, format!("{err:#}"));
                false
            }
            None => false,
        }
    }

    /// `AddMcpServer`: the server kept (checked again here: a screen's word
    /// isn't trusted), shown, and looked at; `sign_in`: a card where it needs one.
    /// Adding one already there is the same: idempotent.
    pub(super) fn add_mcp_server(&mut self, squad: &str, asked: McpServer, sign_in: bool) {
        let server = match sm::server(&asked.name, Some(&asked.url), Some(&asked.roles), Some(asked.transport.code())) {
            Ok(mut server) => {
                server.enabled = asked.enabled;
                // The engine-signed fallback, when asked for (and the server isn't the catalog's keyed one).
                if asked.auth.is_engine() && !matches!(server.auth, McpAuth::Key { .. }) {
                    server.auth = McpAuth::Engine;
                }
                server
            }
            Err(why) => return self.squad_error(squad, why),
        };
        let now = self.journal(squad).map(Journal::mcp_servers).unwrap_or_default();
        let same = now.contains(&server);
        let servers = match sm::with(&now, server.clone()) {
            Ok(servers) => servers,
            Err(why) => return self.squad_error(squad, why),
        };
        if !self.keep_mcp(squad, &servers) {
            return;
        }
        // A server changed under its name: what was known of it no longer
        // holds — nor do tokens a sign-in to what stood there before left.
        if !same {
            if let Some(known) = self.mcp.status.get_mut(squad) {
                known.remove(&server.name);
            }
            if server.auth == McpAuth::Oauth && !now.iter().any(|s| s.name == server.name && s.url == server.url) {
                mcp_oauth::forget(self.mcp_store().as_ref(), squad, &server.name);
            }
        }
        self.clear_put_off(squad, &server.name);
        self.mcp.key_asked.remove(&(squad.to_owned(), server.name.clone()));
        crate::daemon_log!("equipe: {squad} connected the outside MCP server {} ({})", server.name, server.url);
        self.publish_mcp(squad);
        self.status_changed(squad);
        if server.enabled {
            self.check_mcp_servers(squad, Some(&server.name), sign_in.then_some(None));
        }
    }

    /// `RemoveMcpServer`: off the squad, its cards closed, and its sign-in
    /// cleared — equipe's own tokens for it, and the engines' own entry
    /// where no other squad uses the same server that way.
    pub(super) fn remove_mcp_server(&mut self, squad: &str, name: &str) {
        let now = self.journal(squad).map(Journal::mcp_servers).unwrap_or_default();
        let Some(gone) = now.iter().find(|s| s.name == name).cloned() else { return };
        let servers: Vec<McpServer> = now.into_iter().filter(|s| s.name != name).collect();
        if !self.keep_mcp(squad, &servers) {
            return;
        }
        if let Some(known) = self.mcp.status.get_mut(squad) {
            known.remove(name);
        }
        self.close_mcp_cards(squad, name);
        let shared = self.journals.iter().any(|(other, j)| other != squad && j.mcp_servers().iter().any(|s| s.name == gone.name && s.url == gone.url && s.auth.is_engine()));
        let dir = sm::signin_dir(&self.config.home, squad);
        let jobs: Vec<(Engine, String)> = if shared { Vec::new() } else { [Engine::Claude, Engine::Codex].into_iter().map(|e| (e, self.binary_of(e))).collect() };
        let store = self.mcp_store();
        let (squad_s, engine_signed) = (squad.to_owned(), gone.auth.is_engine());
        tokio::spawn(async move {
            if !engine_signed {
                let (squad, name) = (squad_s.clone(), gone.name.clone());
                let _ = tokio::task::spawn_blocking(move || mcp_oauth::forget(store.as_ref(), &squad, &name)).await;
            }
            // The engines' own: the fallback's sign-ins, and Claude Code's
            // local entry in equipe's folder if an earlier equipe made one
            // (so adding it again never meets "already exists").
            for (engine, binary) in jobs {
                if engine_signed || (engine == Engine::Claude && dir.is_dir()) {
                    sm::forget(engine, &binary, &dir, &gone).await;
                }
            }
        });
        crate::daemon_log!("equipe: {squad} removed the outside MCP server {name}");
        self.publish_mcp(squad);
        self.status_changed(squad);
    }

    /// `SetMcpServerEnabled`: off, it is given to no one (kept, and its sign-in with it).
    pub(super) fn set_mcp_server_enabled(&mut self, squad: &str, name: &str, on: bool) {
        let mut servers = self.journal(squad).map(Journal::mcp_servers).unwrap_or_default();
        let Some(server) = servers.iter_mut().find(|s| s.name == name) else { return };
        server.enabled = on;
        if !self.keep_mcp(squad, &servers) {
            return;
        }
        if !on {
            self.close_mcp_cards(squad, name);
        }
        self.publish_mcp(squad);
        if on {
            self.check_mcp_servers(squad, Some(name), None);
        }
    }

    /// `SignInMcpServer`: the person asks. Put-off sign-ins are asked again.
    /// A server equipe signs in to gets its card at once (a "Sign in again"
    /// too); a keyed one its key card; the engine-signed fallback a card for
    /// each engine not connected, after a fresh look.
    pub(super) fn sign_in_mcp_server(&mut self, squad: &str, name: &str) {
        self.clear_put_off(squad, name);
        self.mcp.key_asked.remove(&(squad.to_owned(), name.to_owned()));
        let Some(server) = self.mcp_servers_of(squad).into_iter().find(|s| s.name == name && s.enabled) else {
            return self.squad_error(squad, format!("{name} isn't connected to this squad, or it is off."));
        };
        match server.auth {
            McpAuth::Engine => self.check_mcp_servers(squad, Some(name), Some(None)),
            _ => self.open_mcp_card(squad, &server, OWN, None, None),
        }
    }

    /// Look at the squad's servers (or only `only`), off the loop: through
    /// equipe's own connection, or each engine's own tool for the fallback.
    /// `card_for`: where one needs a sign-in, a card for that member or the person.
    pub(super) fn check_mcp_servers(&mut self, squad: &str, only: Option<&str>, card_for: Option<CardFor>) {
        if self.journal(squad).is_none() {
            return;
        }
        let servers: Vec<McpServer> = self.mcp_servers_of(squad).into_iter().filter(|s| s.enabled && only.is_none_or(|n| n == s.name)).collect();
        for server in servers {
            let key = (squad.to_owned(), server.name.clone());
            // A look under way answers this one too: its card goes to the
            // person if they asked, else to the first member it came up for.
            if let Some(waiting) = self.mcp.checking.get_mut(&key) {
                match (&waiting, &card_for) {
                    (_, Some(None)) | (None, Some(_)) => *waiting = card_for.clone(),
                    _ => {}
                }
                continue;
            }
            self.mcp.checking.insert(key, card_for.clone());
            let engines = self.mcp_engines(squad, &server);
            let (internal, squad_s) = (self.internal.clone(), squad.to_owned());
            match server.auth.clone() {
                // A keyed server: no key, no look; with one, equipe's own look with it.
                McpAuth::Key { secret, header } => {
                    if !self.key_kept(squad, &secret) {
                        let statuses = self.mcp_each(squad, &server, McpState::NeedsKey, None);
                        self.mcp_checked(Checked { squad: squad.to_owned(), server, statuses });
                        continue;
                    }
                    let value = match self.key_value(squad, &secret) {
                        Ok(value) => value,
                        // Behind a Keychain dialog nobody answered: one thing for the person to do.
                        Err(why) if crate::mcp_keys::keychain_blocked(&why) => {
                            let statuses = self.mcp_each(squad, &server, McpState::NeedsKey, Some(crate::mcp_keys::blocked_detail(&super::window::this_mac(), true)));
                            self.mcp_checked(Checked { squad: squad.to_owned(), server, statuses });
                            continue;
                        }
                        Err(why) => {
                            let statuses = self.mcp_each(squad, &server, McpState::Failing, Some(why));
                            self.mcp_checked(Checked { squad: squad.to_owned(), server, statuses });
                            continue;
                        }
                    };
                    tokio::spawn(async move {
                        let (state, detail) = sm::probe(&crate::mcp_gateway::client(), &server, Some((&header, &value)), true).await;
                        let statuses = engines.into_iter().map(|engine| McpEngineStatus { engine, state, detail: detail.clone(), checked_at: now_secs() }).collect();
                        let _ = internal.send(Internal::McpChecked(Checked { squad: squad_s, server, statuses }));
                    });
                }
                // equipe's own sign-in: its token (renewed when it runs out), then a look with it.
                McpAuth::Oauth => {
                    let store = self.mcp_store();
                    let mac = super::window::this_mac();
                    tokio::spawn(async move {
                        let http = crate::mcp_gateway::client();
                        // Renewed ahead: the look every few minutes keeps the token far from running out.
                        let (state, detail) = match mcp_oauth::access_ahead(&http, store.clone(), &squad_s, &server.name, now_secs()).await {
                            Ok(token) => match sm::probe(&http, &server, Some(("authorization", &format!("Bearer {token}"))), false).await {
                                // Refused before its time: renewed once, and looked at again.
                                (McpState::NeedsSignIn, _) => match mcp_oauth::access(&http, store, &squad_s, &server.name, Some(&token), now_secs()).await {
                                    Ok(fresh) => sm::probe(&http, &server, Some(("authorization", &format!("Bearer {fresh}"))), false).await,
                                    Err(NoToken::Failing(why)) => (McpState::Failing, Some(why)),
                                    Err(NoToken::Blocked(_)) => (McpState::NeedsSignIn, Some(crate::mcp_keys::blocked_detail(&mac, false))),
                                    Err(NoToken::SignIn(why)) => (McpState::NeedsSignIn, Some(format!("the sign-in ran out and couldn't be renewed ({why})"))),
                                    Err(NoToken::Never) => (McpState::NeedsSignIn, Some("the sign-in ran out".to_owned())),
                                },
                                seen => seen,
                            },
                            // Never signed in: a server that asks for none is connected as it is.
                            Err(NoToken::Never) => sm::probe(&http, &server, None, false).await,
                            Err(NoToken::SignIn(why)) => (McpState::NeedsSignIn, Some(format!("the sign-in ran out and couldn't be renewed ({why})"))),
                            Err(NoToken::Blocked(_)) => (McpState::NeedsSignIn, Some(crate::mcp_keys::blocked_detail(&mac, false))),
                            Err(NoToken::Failing(why)) => (McpState::Failing, Some(why)),
                        };
                        let statuses = engines.into_iter().map(|engine| McpEngineStatus { engine, state, detail: detail.clone(), checked_at: now_secs() }).collect();
                        let _ = internal.send(Internal::McpChecked(Checked { squad: squad_s, server, statuses }));
                    });
                }
                McpAuth::Engine => {
                    let engines: Vec<(Engine, String)> = engines.into_iter().map(|e| (e, self.binary_of(e))).collect();
                    let dir = sm::signin_dir(&self.config.home, squad);
                    tokio::spawn(async move {
                        let mut statuses = Vec::new();
                        for (engine, binary) in engines {
                            statuses.push(sm::check(engine, &binary, &dir, &server, now_secs()).await);
                        }
                        let _ = internal.send(Internal::McpChecked(Checked { squad: squad_s, server, statuses }));
                    });
                }
            }
        }
    }

    /// Where a server stands now: kept, shown, and logged when it changed.
    fn set_mcp_status(&mut self, squad: &str, server: &McpServer, statuses: Vec<McpEngineStatus>) {
        // The same answer again is no news.
        let before = self.mcp.status.get(squad).and_then(|k| k.get(&server.name)).cloned().unwrap_or_default();
        for s in &statuses {
            if !before.iter().any(|b| b.engine == s.engine && b.state == s.state && b.detail == s.detail) {
                // equipe's own connection is one for every engine: said once.
                let on = if server.auth.proxied() { "through equipe".to_owned() } else { format!("on {}", s.engine.label()) };
                crate::daemon_log!("equipe: {squad}'s outside MCP server {} {on}: {:?}{}", server.name, s.state, s.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default());
                if server.auth.proxied() {
                    break;
                }
            }
        }
        // Signed in after all (elsewhere, or by the person): nothing put off any more.
        if statuses.iter().any(|s| s.state == McpState::Connected) && self.mcp.put_off.keys().any(|(q, n, _)| q == squad && *n == server.name) {
            self.clear_put_off(squad, &server.name);
        }
        self.mcp.status.entry(squad.to_owned()).or_default().insert(server.name.clone(), statuses);
        self.publish_mcp(squad);
    }

    /// A check came back: kept, shown, and — where it was for a card and it
    /// needs a sign-in (or its key) — the card.
    pub(super) fn mcp_checked(&mut self, checked: Checked) {
        let Checked { squad, server, statuses } = checked;
        let card_for = self.mcp.checking.remove(&(squad.clone(), server.name.clone())).flatten();
        // Gone or changed meanwhile: this answer is about something no longer kept.
        if !self.mcp_servers_of(&squad).iter().any(|s| s.name == server.name && s.url == server.url && s.enabled) {
            return;
        }
        let was_connected = self.mcp.status.get(&squad).and_then(|k| k.get(&server.name)).is_some_and(|all| all.iter().any(|s| s.state == McpState::Connected));
        self.set_mcp_status(&squad, &server, statuses.clone());
        // Working until now, and equipe couldn't heal it itself (the renewal
        // refused, the key refused): one card for the person, saying what
        // happened — unless they put it off.
        let lost = statuses.iter().find(|s| matches!(s.state, McpState::NeedsSignIn | McpState::NeedsKey));
        if card_for.is_none() && was_connected && server.auth.proxied()
            && let Some(lost) = lost
        {
            self.read_put_off(&squad);
            if !self.mcp.put_off.contains_key(&(squad.clone(), server.name.clone(), OWN)) {
                let why = format!("{} stopped working for the squad: {}.", sm::label(&server), lost.detail.clone().unwrap_or_else(|| "it asks for a new sign-in".to_owned()));
                self.open_mcp_card(&squad, &server, OWN, None, Some(why));
            }
            return;
        }
        let Some(by) = card_for else { return };
        let wanting: Vec<Engine> = statuses.iter().filter(|s| matches!(s.state, McpState::NeedsSignIn | McpState::NeedsKey) || (by.is_none() && s.state == McpState::Failing)).map(|s| s.engine).collect();
        if server.auth.proxied() {
            // One card for all engines: equipe's own sign-in, or the key.
            if !wanting.is_empty() {
                self.open_mcp_card(&squad, &server, OWN, by, None);
            }
        } else {
            for engine in wanting {
                self.open_mcp_card(&squad, &server, engine, by.clone(), None);
            }
        }
    }

    /// The person put off the sign-in to `server` on `kind`'s engine — or
    /// its card ended without one (refused, or left to run out): not put up
    /// again by a member's turn until the person asks.
    pub(super) fn mcp_put_off(&mut self, squad: &str, server: &str, kind: Kind) {
        let engine = if kind == Kind::CodexMcp { Engine::Codex } else { OWN };
        self.read_put_off(squad);
        // Put off again after it was offered once more: it stays offered once.
        let entry = self.mcp.put_off.entry((squad.to_owned(), server.to_owned(), engine)).or_insert(PutOff { at: 0, reoffered: false });
        entry.at = now_secs();
        self.keep_put_off(squad);
    }

    /// A squad's put-off sign-ins from disk, once.
    fn read_put_off(&mut self, squad: &str) {
        if !self.mcp.put_off_read.insert(squad.to_owned()) {
            return;
        }
        let kept = std::fs::read_to_string(put_off_file(&self.config.home, squad)).map(|t| parse_put_off(&t)).unwrap_or_default();
        for (server, engine, put_off) in kept {
            self.mcp.put_off.insert((squad.to_owned(), server, engine), put_off);
        }
    }

    /// A squad's put-off sign-ins, written as they are now.
    fn keep_put_off(&self, squad: &str) {
        let mut kept: Vec<KeptPutOff> = self.mcp.put_off.iter().filter(|((q, _, _), _)| q == squad).map(|((_, n, e), p)| KeptPutOff { server: n.clone(), engine: *e, at: p.at, reoffered: p.reoffered }).collect();
        kept.sort_by(|a, b| a.server.cmp(&b.server).then_with(|| format!("{:?}", a.engine).cmp(&format!("{:?}", b.engine))));
        let file = put_off_file(&self.config.home, squad);
        if kept.is_empty() {
            let _ = std::fs::remove_file(file);
        } else if let Some(dir) = file.parent()
            && std::fs::create_dir_all(dir).is_ok()
        {
            let _ = std::fs::write(file, serde_json::to_string(&kept).unwrap_or_default());
        }
    }

    /// The person asks for `name`'s sign-in again (or reconnected it): no longer put off.
    fn clear_put_off(&mut self, squad: &str, name: &str) {
        self.read_put_off(squad);
        let before = self.mcp.put_off.len();
        self.mcp.put_off.retain(|(s, n, _), _| !(s == squad && n == name));
        if before != self.mcp.put_off.len() {
            self.keep_put_off(squad);
        }
    }

    /// One sign-in card for `server` (on `engine`, for the fallback) — unless
    /// one is up, or the person put it off and it was already offered once
    /// more (`by` a member). The person asking (`by` none) always gets a fresh
    /// one. `why`: what the card says instead of the server's own reason.
    fn open_mcp_card(&mut self, squad: &str, server: &McpServer, engine: Engine, by: CardFor, why: Option<String>) {
        // A keyed server: the catalog's key card (steps and the page to create
        // the key), once for a member's turns; again whenever the person asks.
        if let McpAuth::Key { secret, .. } = &server.auth {
            let asked = (squad.to_owned(), server.name.clone());
            let up = self.secret_asks(squad).iter().any(|a| a.store.as_ref().is_some_and(|st| st.name == *secret));
            if up || (by.is_some() && self.mcp.key_asked.contains(&asked)) {
                return;
            }
            if let Some(service) = sm::service_of(server).filter(|s| s.takes_key()) {
                self.mcp.key_asked.insert(asked);
                self.request_service_key(squad, service.id);
                self.tell_member_of_card(squad, server, by.as_ref());
            }
            return;
        }
        let kind = if server.auth.is_engine() { kind_of(engine) } else { Kind::EquipeMcp };
        if self.mcp_card_up(squad, &server.name, kind) {
            match by {
                // A member's turn while one is up: that one stands.
                Some(_) => return,
                // The person asks: a fresh sign-in, whatever the one up came to.
                None => self.close_mcp_cards_of(squad, &server.name, kind),
            }
        }
        let engine = if server.auth.is_engine() { engine } else { OWN };
        self.read_put_off(squad);
        let mut why = why.unwrap_or_else(|| sm::signin_why(server));
        let key = (squad.to_owned(), server.name.clone(), engine);
        if let (Some((_, member, ticket)), Some(put_off)) = (by.as_ref(), self.mcp.put_off.get(&key).copied()) {
            // Offered once more, saying why; after that only the person brings it back.
            if put_off.reoffered {
                return;
            }
            self.mcp.put_off.insert(key, PutOff { reoffered: true, ..put_off });
            self.keep_put_off(squad);
            let on = ticket.map(|t| format!(" for T-{t}")).unwrap_or_default();
            let seen = self.mcp.status.get(squad).and_then(|k| k.get(&server.name)).and_then(|all| all.iter().find_map(|s| s.detail.clone())).map(|d| format!(" Where it stands: {d}.")).unwrap_or_default();
            why = format!("{member} needs {} again{on}. Its sign-in was put off earlier; sign in so members can use it — or Not now, and it isn't offered again until you ask from Connections.{seen}", sm::label(server));
        }
        let (plan, name) = if server.auth.is_engine() {
            (sm::signin_plan(engine, &self.binary_of(engine), &sm::signin_dir(&self.config.home, squad), server), format!("{} ({})", sm::label(server), kind.name()))
        } else {
            (crate::cli_signin::Plan { program: String::new(), args: Vec::new(), env: Vec::new(), whoami: (String::new(), Vec::new()), cwd: None }, sm::label(server))
        };
        self.start_mcp_card(squad, kind, plan, &server.name, name, why, by.clone());
        self.tell_member_of_card(squad, server, by.as_ref());
    }

    /// The member a card came up for hears, in its conversation, that the
    /// server waits on the person — the remote person sees it there too.
    fn tell_member_of_card(&mut self, squad: &str, server: &McpServer, by: Option<&(SessionId, String, Option<TicketId>)>) {
        let Some((session, member, _)) = by else { return };
        let text = say!(self, "notice.mcp_card_up", server = sm::label(server), member = member);
        let _ = squad;
        self.publish(Event::Notice { undo: None, session: *session, text });
    }

    /// The address of squad `squad`'s server `name`, for its sign-in card.
    pub(super) fn mcp_url_of(&self, squad: &str, name: &str) -> Option<(String, bool)> {
        self.mcp_servers_of(squad).into_iter().find(|s| s.name == name).map(|s| (s.url, s.transport == crate::squad_mcp::McpTransport::Sse))
    }

    /// The outside servers a member of `role` on `engine` gets this turn.
    /// One it gets that needs a sign-in (or its key) puts one card in front
    /// of the person (once) and isn't given; one not looked at yet is given
    /// and looked at, the card following if it needs one.
    /// Those it doesn't get this turn come back too, each with why, for its charter ([`unavailable_note`]).
    pub(super) fn mcp_for_member(&mut self, squad: &str, id: SessionId) -> ForMember {
        let Some(session) = self.sessions.get(&id) else { return Default::default() };
        let Some(member) = session.member.as_ref().map(|m| (m.name.clone(), m.role.clone())) else { return Default::default() };
        let engine = match session.engine {
            Engine::Codex if session.codex.is_some() => return Default::default(),
            e => e,
        };
        let ticket = session.work.as_ref().map(|w| w.ticket);
        let servers = self.mcp_servers_of(squad);
        let mine: Vec<McpServer> = sm::for_member(&servers, &member.1, engine).into_iter().cloned().collect();
        let now = now_secs();
        let known_of = |this: &Self, server: &McpServer| this.mcp.status.get(squad).and_then(|k| k.get(&server.name)).and_then(|all| all.iter().find(|s| s.engine == engine || server.auth.proxied())).map(|s| (s.state, s.checked_at));
        for server in &mine {
            let known = known_of(self, server);
            let by = Some((id, member.0.clone(), ticket));
            // A keyed server stands on its key being kept: looked at again when that changed.
            if let McpAuth::Key { secret, .. } = &server.auth {
                let kept = self.key_kept(squad, secret);
                let shown = known.map(|(state, _)| state);
                let stale = match shown {
                    None => true,
                    Some(McpState::NeedsKey) => kept,
                    Some(_) => !kept,
                };
                if stale {
                    self.check_mcp_servers(squad, Some(&server.name), Some(by));
                } else if !kept {
                    self.open_mcp_card(squad, server, engine, by, None);
                }
                continue;
            }
            match known {
                // Signed in elsewhere meanwhile? Looked at again after a while.
                Some((McpState::NeedsSignIn, at)) if now.saturating_sub(at) > RECHECK_SIGN_IN => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((McpState::NeedsSignIn, _)) => self.open_mcp_card(squad, server, engine, by, None),
                // Not looked at yet, or not lately (a sign-in can run out):
                // looked at now, a card following if needed.
                None | Some((McpState::Unchecked, _)) => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((_, at)) if now.saturating_sub(at) > FRESH_FOR => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                _ => {}
            }
        }
        // How each is given: equipe's proxy (no credential in it), or the
        // fallback's address; one that needs a sign-in or its key isn't — and
        // the member is told why, and what the person has been asked.
        self.read_put_off(squad);
        let (mut given, mut unavailable) = (Vec::new(), Vec::new());
        for server in mine {
            let kept = matches!(&server.auth, McpAuth::Key { secret, .. } if self.key_kept(squad, secret));
            let known = self.mcp.status.get(squad).and_then(|k| k.get(&server.name)).and_then(|all| all.iter().find(|s| s.engine == engine || server.auth.proxied())).cloned();
            match sm::credential(&server, kept, known.as_ref().map(|s| s.state)) {
                credential @ (Credential::Engine | Credential::Proxy) => given.push((server, credential)),
                missing => {
                    let card_up = self.mcp_card_up(squad, &server.name, if server.auth.is_engine() { kind_of(engine) } else { Kind::EquipeMcp });
                    let put_off = self.mcp.put_off.get(&(squad.to_owned(), server.name.clone(), if server.auth.is_engine() { engine } else { OWN })).is_some_and(|p| p.reoffered);
                    let why = unavailable_reason(&missing, known.and_then(|s| s.detail).as_deref(), card_up, put_off);
                    unavailable.push((server, why));
                }
            }
        }
        (given, unavailable)
    }

    /// A proxy asks for its server's address and credential, for member
    /// `open.member`: refused unless that member gets the server; a kept key
    /// or equipe's access token (renewed when it runs out, or when the server
    /// just refused `rejected`) — never anything else, and never to a screen.
    pub(super) fn mcp_upstream(&mut self, open: McpProxyOpen, rejected: Option<String>, reply: oneshot::Sender<Result<Upstream, String>>) {
        let McpProxyOpen { squad, server: name, member } = open.clone();
        let Some(server) = self.mcp_servers_of(&squad).into_iter().find(|s| s.name == name && s.enabled) else {
            let _ = reply.send(Err(format!("{name} isn't connected to the squad, or it is off")));
            return;
        };
        let session = self.sessions.iter().find(|(_, s)| s.squad.as_deref() == Some(squad.as_str()) && s.is_member_itself() && s.name() == Some(member.as_str())).map(|(id, s)| (*id, s.member.as_ref().map(|m| m.role.clone()).unwrap_or_default(), s.work.as_ref().map(|w| w.ticket)));
        let Some((sid, role, ticket)) = session else {
            let _ = reply.send(Err(format!("{member} isn't one of {squad}'s members")));
            return;
        };
        // The PM's design-library search: its tools that read, and no others.
        let read_only = !server.serves(&role);
        if read_only && !sm::for_references(&server, &role) {
            let _ = reply.send(Err(format!("{name} isn't connected for {role}s")));
            return;
        }
        let by = Some((sid, member.clone(), ticket));
        let (url, transport) = (server.url.clone(), server.transport);
        let up = move |header| Upstream { url: url.clone(), transport, header, read_only };
        match server.auth.clone() {
            McpAuth::Engine => {
                let _ = reply.send(Err(format!("{name} is signed in by each engine itself")));
            }
            McpAuth::Key { secret, header } => match (self.key_kept(&squad, &secret), rejected.is_some()) {
                (true, false) => {
                    let _ = reply.send(self.key_value(&squad, &secret).map(|value| up(Some((header, value)))));
                }
                (kept, _) => {
                    let why = if kept { "the saved key was refused" } else { "it needs its API key" };
                    let statuses = self.mcp_each(&squad, &server, McpState::NeedsKey, kept.then(|| why.to_owned()));
                    self.set_mcp_status(&squad, &server, statuses);
                    self.open_mcp_card(&squad, &server, OWN, by, None);
                    let _ = reply.send(Err(format!("{name}: {why} — the person was asked for it")));
                }
            },
            McpAuth::Oauth => {
                let (store, internal, mac) = (self.mcp_store(), self.internal.clone(), super::window::this_mac());
                let rejected = rejected.map(|r| r.strip_prefix("Bearer ").unwrap_or(&r).to_owned());
                tokio::spawn(async move {
                    let http = crate::mcp_gateway::client();
                    let answer = match mcp_oauth::access(&http, store, &squad, &name, rejected.as_deref().filter(|r| !r.is_empty()), now_secs()).await {
                        Ok(token) => Ok(up(Some(("authorization".to_owned(), format!("Bearer {token}"))))),
                        // Never signed in: tried as it is, in case it asks for none.
                        Err(NoToken::Never) if rejected.is_none() => Ok(up(None)),
                        Err(NoToken::Never) | Err(NoToken::SignIn(_)) => {
                            let why = format!("{name} needs the person signed in to it through equipe: they were asked");
                            let _ = internal.send(Internal::McpSeen(open, Seen::NeedsSignIn(why.clone())));
                            Err(why)
                        }
                        Err(NoToken::Blocked(_)) => {
                            let why = format!("{name} is unavailable: {}", crate::mcp_keys::blocked_detail(&mac, false));
                            let _ = internal.send(Internal::McpSeen(open, Seen::NeedsSignIn(why.clone())));
                            Err(why)
                        }
                        Err(NoToken::Failing(why)) => {
                            let _ = internal.send(Internal::McpSeen(open, Seen::Failing(why.clone())));
                            Err(format!("{name} is failing for now ({why}); equipe looks again on its own"))
                        }
                    };
                    let _ = reply.send(answer);
                });
            }
        }
    }

    /// What a proxy's message showed of its server: its status, the same for
    /// every engine; one that wants a sign-in puts its card up for the member.
    pub(super) fn mcp_seen(&mut self, open: McpProxyOpen, seen: Seen) {
        let Some(server) = self.mcp_servers_of(&open.squad).into_iter().find(|s| s.name == open.server && s.enabled) else { return };
        let keyed = matches!(server.auth, McpAuth::Key { .. });
        let (state, detail) = match seen {
            Seen::Connected => (McpState::Connected, None),
            Seen::NeedsSignIn(_) if keyed => (McpState::NeedsKey, Some("the saved key was refused".to_owned())),
            Seen::NeedsSignIn(_) => (McpState::NeedsSignIn, None),
            Seen::Failing(why) => (McpState::Failing, Some(why)),
        };
        let shown = self.mcp.status.get(&open.squad).and_then(|k| k.get(&server.name)).is_some_and(|all| !all.is_empty() && all.iter().all(|s| s.state == state && s.detail == detail));
        if !shown {
            let statuses = self.mcp_each(&open.squad, &server, state, detail);
            self.set_mcp_status(&open.squad, &server, statuses);
        }
        if matches!(state, McpState::NeedsSignIn | McpState::NeedsKey) {
            let by = self.sessions.iter().find(|(_, s)| s.squad.as_deref() == Some(open.squad.as_str()) && s.is_member_itself() && s.name() == Some(open.member.as_str())).map(|(id, s)| (*id, open.member.clone(), s.work.as_ref().map(|w| w.ticket)));
            if by.is_some() {
                self.open_mcp_card(&open.squad, &server, OWN, by, None);
            }
        }
    }

    /// The squads' kept keys changed (one saved, deleted): each keyed server
    /// whose key that was stands anew at once — no engine asked, nothing polled.
    pub(super) fn mcp_keys_changed(&mut self) {
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            for server in self.mcp_servers_of(&squad).into_iter().filter(|s| s.enabled) {
                let McpAuth::Key { secret, .. } = &server.auth else { continue };
                let kept = self.key_kept(&squad, secret);
                let shown = self.mcp.status.get(&squad).and_then(|k| k.get(&server.name)).and_then(|all| all.first().map(|s| s.state));
                let stale = match shown {
                    None => kept,
                    Some(McpState::NeedsKey) => kept,
                    Some(_) => !kept,
                };
                if stale {
                    self.check_mcp_servers(&squad, Some(&server.name), None);
                }
            }
        }
    }

    /// Whether the squad keeps a key called `name` (its facts; no value read).
    fn key_kept(&self, squad: &str, name: &str) -> bool {
        self.journals.get(squad).map(crate::squad_secrets::read).unwrap_or_default().iter().any(|k| k.name == name)
    }
}

/// Why a member doesn't get a server this turn, in a line: what is missing,
/// what equipe saw, and what the person has (or hasn't) been asked.
pub(super) fn unavailable_reason(missing: &Credential, detail: Option<&str>, card_up: bool, put_off: bool) -> String {
    let what = match missing {
        Credential::MissingKey { .. } => "its API key isn't available to equipe",
        _ => "equipe isn't signed in to it",
    };
    let seen = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    let asked = if card_up {
        "the person has a card for it in Needs you"
    } else if put_off {
        "the person put its sign-in off; only they can turn it back on, from Connections"
    } else {
        "the person is asked on a card"
    };
    format!("{what}{seen}; {asked}")
}

/// What a member is told of the servers it would have but doesn't this
/// turn; `None` when it has them all.
pub(super) fn unavailable_note(unavailable: &[(McpServer, String)]) -> Option<String> {
    if unavailable.is_empty() {
        return None;
    }
    let each: Vec<String> = unavailable.iter().map(|(s, why)| format!("`{}` ({}): {why}", s.name, sm::label(s))).collect();
    Some(format!(
        "Not available this turn: {}. Don't look for another way in or guess at the cause: if your work needs it, say in a line \
that it is unavailable and why (as above), then carry on with what you can do without it.",
        each.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_off_sign_ins_from_an_earlier_equipe_are_offered_once_more() {
        // What 0.103 wrote: pairs, never offered again.
        let old = parse_put_off(r#"[["mobbin","claude"],["figma","claude"]]"#);
        assert_eq!(old.len(), 2);
        assert!(old.iter().all(|(_, e, p)| *e == Engine::Claude && !p.reoffered), "{old:?}");
        // What this one writes: when, and whether it was offered once more.
        let kept = vec![KeptPutOff { server: "mobbin".into(), engine: Engine::Claude, at: 7, reoffered: true }];
        let back = parse_put_off(&serde_json::to_string(&kept).unwrap());
        assert_eq!(back, vec![("mobbin".to_owned(), Engine::Claude, PutOff { at: 7, reoffered: true })]);
        assert!(parse_put_off("not json").is_empty());
    }

    #[test]
    fn a_member_is_told_why_a_server_is_missing_and_what_the_person_was_asked() {
        let key = unavailable_reason(&Credential::MissingKey { secret: "21st-api-key".into() }, Some("its saved credential is behind a Keychain dialog on mini"), false, false);
        assert!(key.starts_with("its API key isn't available to equipe (its saved credential is behind a Keychain dialog on mini)"), "{key}");
        assert!(unavailable_reason(&Credential::NeedsSignIn, None, true, false).ends_with("the person has a card for it in Needs you"));
        assert!(unavailable_reason(&Credential::NeedsSignIn, None, false, true).contains("only they can turn it back on, from Connections"));
        let mobbin = sm::server("mobbin", None, None, None).unwrap();
        let note = unavailable_note(&[(mobbin, "equipe isn't signed in to it; the person is asked on a card".into())]).unwrap();
        assert!(note.starts_with("Not available this turn: `mobbin` (Mobbin): equipe isn't signed in to it"), "{note}");
        assert!(note.contains("Don't look for another way in or guess"));
        assert_eq!(unavailable_note(&[]), None);
    }
}
