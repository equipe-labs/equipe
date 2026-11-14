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
//! asks. A card put off ("Not now") isn't opened again for that server until
//! the person asks for it.
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
    /// Sign-ins the person put off, by squad, server name and engine (a
    /// server equipe signs in to itself: under Claude Code's).
    put_off: HashSet<(String, String, Engine)>,
    /// Keyed servers whose key card a member's turn has already put up, by
    /// squad and server name: asked once, then only when the person asks.
    key_asked: HashSet<(String, String)>,
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

    /// Where equipe keeps a server's sign-in: the squad's secret store.
    pub(super) fn mcp_store(&self) -> Arc<dyn mcp_oauth::Store> {
        Arc::new(mcp_oauth::VaultStore(self.vault.clone()))
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
        let same = now.iter().any(|s| *s == server);
        let servers = match sm::with(&now, server.clone()) {
            Ok(servers) => servers,
            Err(why) => return self.squad_error(squad, why),
        };
        if !self.keep_mcp(squad, &servers) {
            return;
        }
        // A server changed under its name: what was known of it no longer holds.
        if !same && let Some(known) = self.mcp.status.get_mut(squad) {
            known.remove(&server.name);
        }
        self.mcp.put_off.retain(|(s, n, _)| !(s == squad && *n == server.name));
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
        self.mcp.put_off.retain(|(s, n, _)| !(s == squad && n == name));
        self.mcp.key_asked.remove(&(squad.to_owned(), name.to_owned()));
        let Some(server) = self.mcp_servers_of(squad).into_iter().find(|s| s.name == name && s.enabled) else {
            return self.squad_error(squad, format!("{name} isn't connected to this squad, or it is off."));
        };
        match server.auth {
            McpAuth::Engine => self.check_mcp_servers(squad, Some(name), Some(None)),
            _ => self.open_mcp_card(squad, &server, OWN, None),
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
                    tokio::spawn(async move {
                        let http = crate::mcp_gateway::client();
                        let (state, detail) = match mcp_oauth::access(&http, store.clone(), &squad_s, &server.name, None, now_secs()).await {
                            Ok(token) => match sm::probe(&http, &server, Some(("authorization", &format!("Bearer {token}"))), false).await {
                                // Refused before its time: renewed once, and looked at again.
                                (McpState::NeedsSignIn, _) => match mcp_oauth::access(&http, store, &squad_s, &server.name, Some(&token), now_secs()).await {
                                    Ok(fresh) => sm::probe(&http, &server, Some(("authorization", &format!("Bearer {fresh}"))), false).await,
                                    Err(NoToken::Failing(why)) => (McpState::Failing, Some(why)),
                                    Err(_) => (McpState::NeedsSignIn, Some("the sign-in ran out".to_owned())),
                                },
                                seen => seen,
                            },
                            // Never signed in: a server that asks for none is connected as it is.
                            Err(NoToken::Never) => sm::probe(&http, &server, None, false).await,
                            Err(NoToken::SignIn(_)) => (McpState::NeedsSignIn, Some("the sign-in ran out".to_owned())),
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
                crate::daemon_log!("equipe: {squad}'s outside MCP server {} on {}: {:?}{}", server.name, s.engine.label(), s.state, s.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default());
            }
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
        self.set_mcp_status(&squad, &server, statuses.clone());
        let Some(by) = card_for else { return };
        let wanting: Vec<Engine> = statuses.iter().filter(|s| matches!(s.state, McpState::NeedsSignIn | McpState::NeedsKey) || (by.is_none() && s.state == McpState::Failing)).map(|s| s.engine).collect();
        if server.auth.proxied() {
            // One card for all engines: equipe's own sign-in, or the key.
            if !wanting.is_empty() {
                self.open_mcp_card(&squad, &server, OWN, by);
            }
        } else {
            for engine in wanting {
                self.open_mcp_card(&squad, &server, engine, by.clone());
            }
        }
    }

    /// The person put off the sign-in to `server` on `kind`'s engine.
    pub(super) fn mcp_put_off(&mut self, squad: &str, server: &str, kind: Kind) {
        let engine = if kind == Kind::CodexMcp { Engine::Codex } else { OWN };
        self.mcp.put_off.insert((squad.to_owned(), server.to_owned(), engine));
    }

    /// One sign-in card for `server` (on `engine`, for the fallback) — unless
    /// one is up, or the person put it off and didn't ask (`by` a member).
    fn open_mcp_card(&mut self, squad: &str, server: &McpServer, engine: Engine, by: CardFor) {
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
            }
            return;
        }
        let kind = if server.auth.is_engine() { kind_of(engine) } else { Kind::EquipeMcp };
        if self.mcp_card_up(squad, &server.name, kind) {
            // Asked again by the person: the card already up comes back in
            // front of them (a fresh link if its last one ran out).
            if by.is_none() {
                self.reshow_mcp_card(squad, &server.name, kind);
            }
            return;
        }
        let engine = if server.auth.is_engine() { engine } else { OWN };
        if by.is_some() && self.mcp.put_off.contains(&(squad.to_owned(), server.name.clone(), engine)) {
            return;
        }
        let (plan, name) = if server.auth.is_engine() {
            (sm::signin_plan(engine, &self.binary_of(engine), &sm::signin_dir(&self.config.home, squad), server), format!("{} ({})", sm::label(server), kind.name()))
        } else {
            (crate::cli_signin::Plan { program: String::new(), args: Vec::new(), env: Vec::new(), whoami: (String::new(), Vec::new()), cwd: None }, sm::label(server))
        };
        self.start_mcp_card(squad, kind, plan, &server.name, name, sm::signin_why(server), by);
    }

    /// The address of squad `squad`'s server `name`, for its sign-in card.
    pub(super) fn mcp_url_of(&self, squad: &str, name: &str) -> Option<(String, bool)> {
        self.mcp_servers_of(squad).into_iter().find(|s| s.name == name).map(|s| (s.url, s.transport == crate::squad_mcp::McpTransport::Sse))
    }

    /// The outside servers a member of `role` on `engine` gets this turn.
    /// One it gets that needs a sign-in (or its key) puts one card in front
    /// of the person (once) and isn't given; one not looked at yet is given
    /// and looked at, the card following if it needs one.
    pub(super) fn mcp_for_member(&mut self, squad: &str, id: SessionId) -> Vec<(McpServer, Credential)> {
        let Some(session) = self.sessions.get(&id) else { return Vec::new() };
        let Some(member) = session.member.as_ref().map(|m| (m.name.clone(), m.role.clone())) else { return Vec::new() };
        let engine = match session.engine {
            Engine::Codex if session.codex.is_some() => return Vec::new(),
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
                    self.open_mcp_card(squad, server, engine, by);
                }
                continue;
            }
            match known {
                // Signed in elsewhere meanwhile? Looked at again after a while.
                Some((McpState::NeedsSignIn, at)) if now.saturating_sub(at) > RECHECK_SIGN_IN => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((McpState::NeedsSignIn, _)) => self.open_mcp_card(squad, server, engine, by),
                // Not looked at yet, or not lately (a sign-in can run out):
                // looked at now, a card following if needed.
                None | Some((McpState::Unchecked, _)) => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((_, at)) if now.saturating_sub(at) > FRESH_FOR => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                _ => {}
            }
        }
        // How each is given: equipe's proxy (no credential in it), or the
        // fallback's address; one that needs a sign-in or its key isn't.
        mine.into_iter()
            .filter_map(|server| {
                let kept = matches!(&server.auth, McpAuth::Key { secret, .. } if self.key_kept(squad, secret));
                let state = known_of(self, &server).map(|(s, _)| s);
                match sm::credential(&server, kept, state) {
                    credential @ (Credential::Engine | Credential::Proxy) => Some((server, credential)),
                    Credential::MissingKey { .. } | Credential::NeedsSignIn => None,
                }
            })
            .collect()
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
        if !server.serves(&role) {
            let _ = reply.send(Err(format!("{name} isn't connected for {role}s")));
            return;
        }
        let by = Some((sid, member.clone(), ticket));
        let (url, transport) = (server.url.clone(), server.transport);
        let up = move |header| Upstream { url: url.clone(), transport, header };
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
                    self.open_mcp_card(&squad, &server, OWN, by);
                    let _ = reply.send(Err(format!("{name}: {why} — the person was asked for it")));
                }
            },
            McpAuth::Oauth => {
                let (store, internal) = (self.mcp_store(), self.internal.clone());
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
                        Err(NoToken::Failing(why)) => {
                            let _ = internal.send(Internal::McpSeen(open, Seen::Failing(why.clone())));
                            Err(why)
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
                self.open_mcp_card(&open.squad, &server, OWN, by);
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
