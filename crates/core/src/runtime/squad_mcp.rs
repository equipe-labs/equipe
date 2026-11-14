//! A squad's outside MCP servers on the daemon (`crate::squad_mcp`): kept in
//! its journal, checked through the engines' own tools off the loop, given
//! to members as their turns start, and signed in on a sign-in card
//! (`super::cli_signin`) — one card per server and engine, opened when a
//! member who gets the server finds it needs a sign-in, or when the person
//! asks. A card put off ("Not now") isn't opened again for that server until
//! the person asks for its sign-in.

use std::collections::HashSet;

use crate::cli_signin::Kind;
use crate::squad_mcp::{self as sm, Credential, McpAuth, McpEngineStatus, McpServer, McpServerStatus, McpState};

use super::*;

/// What the runtime keeps of the squads' outside servers besides their settings.
#[derive(Default)]
pub(super) struct McpBook {
    /// Where each server stands, by squad and server name, one entry per engine checked.
    status: HashMap<String, HashMap<String, Vec<McpEngineStatus>>>,
    /// Checks under way, by squad and server name, with who the sign-in
    /// card is for where one turns out needed: a member (its session, name
    /// and ticket), or the person (`Some(None)`); `None`: no card.
    checking: HashMap<(String, String), Option<CardFor>>,
    /// Sign-ins the person put off, by squad, server name and engine.
    put_off: HashSet<(String, String, Engine)>,
    /// Keyed servers whose key card a member's turn has already put up, by
    /// squad and server name: asked once, then only when the person asks.
    key_asked: HashSet<(String, String)>,
}

/// A member a sign-in card comes up for (its session, name and ticket); `None`: the person asked.
type CardFor = Option<(SessionId, String, Option<TicketId>)>;

/// How long what an engine said of a server is taken as still true.
const FRESH_FOR: u64 = 6 * 3600;
/// …and that it needs a sign-in, which the person may have done in a terminal.
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
            if server.serves(&m.role) && server.transport.on(engine) && !engines.contains(&engine) {
                engines.push(engine);
            }
        }
        if engines.is_empty() {
            engines.push(Engine::Claude);
        }
        engines.sort_by_key(|e| *e != Engine::Claude);
        engines
    }

    fn binary_of(&self, engine: Engine) -> String {
        match engine {
            Engine::Claude => self.config.claude_binary.clone(),
            Engine::Codex => self.config.codex_binary.clone(),
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
    /// isn't trusted), shown, and checked; `sign_in`: a card where it needs one.
    pub(super) fn add_mcp_server(&mut self, squad: &str, asked: McpServer, sign_in: bool) {
        let server = match sm::server(&asked.name, Some(&asked.url), Some(&asked.roles), Some(asked.transport.code())) {
            Ok(mut server) => {
                server.enabled = asked.enabled;
                server
            }
            Err(why) => return self.squad_error(squad, why),
        };
        let now = self.journal(squad).map(Journal::mcp_servers).unwrap_or_default();
        let servers = match sm::with(&now, server.clone()) {
            Ok(servers) => servers,
            Err(why) => return self.squad_error(squad, why),
        };
        if !self.keep_mcp(squad, &servers) {
            return;
        }
        // A server changed under its name: what was known of it no longer holds.
        if let Some(known) = self.mcp.status.get_mut(squad) {
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

    /// `RemoveMcpServer`: off the squad, its cards closed, and the engines'
    /// own sign-in for it cleared where no other squad uses the same server.
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
        let shared = self.journals.iter().any(|(other, j)| other != squad && j.mcp_servers().iter().any(|s| s.name == gone.name && s.url == gone.url));
        if !shared {
            let dir = sm::signin_dir(&self.config.home, squad);
            let jobs: Vec<(Engine, String)> = [Engine::Claude, Engine::Codex].into_iter().map(|e| (e, self.binary_of(e))).collect();
            tokio::spawn(async move {
                for (engine, binary) in jobs {
                    sm::forget(engine, &binary, &dir, &gone).await;
                }
            });
        }
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

    /// `SignInMcpServer`: the person asks. Put-off sign-ins are asked again;
    /// each engine its members run on gets a card unless it is connected —
    /// known after a fresh look, so a check runs first.
    pub(super) fn sign_in_mcp_server(&mut self, squad: &str, name: &str) {
        self.mcp.put_off.retain(|(s, n, _)| !(s == squad && n == name));
        self.mcp.key_asked.remove(&(squad.to_owned(), name.to_owned()));
        self.check_mcp_servers(squad, Some(name), Some(None));
    }

    /// Look at the squad's servers (or only `only`) through each engine's own
    /// tool, off the loop. `card_for`: where one needs a sign-in, a card for that member or the person.
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
            // A keyed server stands on whether its key is kept: known here, no engine asked.
            if let McpAuth::Key { secret, .. } | McpAuth::Gateway { secret } = &server.auth {
                let kept = self.key_kept(squad, secret);
                let gateway = matches!(server.auth, McpAuth::Gateway { .. });
                let statuses = self
                    .mcp_engines(squad, &server)
                    .into_iter()
                    .map(|engine| match (gateway, kept) {
                        (true, _) => McpEngineStatus { engine, state: McpState::Failing, detail: Some("equipe-managed sign-in isn't available yet".to_owned()), checked_at: now_secs() },
                        (false, true) => McpEngineStatus { engine, state: McpState::Connected, detail: None, checked_at: now_secs() },
                        (false, false) => McpEngineStatus { engine, state: McpState::NeedsSignIn, detail: None, checked_at: now_secs() },
                    })
                    .collect();
                self.mcp_checked(Checked { squad: squad.to_owned(), server, statuses });
                continue;
            }
            let engines: Vec<(Engine, String)> = self.mcp_engines(squad, &server).into_iter().map(|e| (e, self.binary_of(e))).collect();
            let dir = sm::signin_dir(&self.config.home, squad);
            let (internal, squad) = (self.internal.clone(), squad.to_owned());
            tokio::spawn(async move {
                let mut statuses = Vec::new();
                for (engine, binary) in engines {
                    statuses.push(sm::check(engine, &binary, &dir, &server, now_secs()).await);
                }
                let _ = internal.send(Internal::McpChecked(Checked { squad, server, statuses }));
            });
        }
    }

    /// A check came back: kept, shown, and — where it was for a card and an
    /// engine needs a sign-in — the card.
    pub(super) fn mcp_checked(&mut self, checked: Checked) {
        let Checked { squad, server, statuses } = checked;
        let card_for = self.mcp.checking.remove(&(squad.clone(), server.name.clone())).flatten();
        // Gone or changed meanwhile: this answer is about something no longer kept.
        if !self.mcp_servers_of(&squad).iter().any(|s| s.name == server.name && s.url == server.url && s.enabled) {
            return;
        }
        for s in &statuses {
            crate::daemon_log!("equipe: {squad}'s outside MCP server {} on {}: {:?}{}", server.name, s.engine.label(), s.state, s.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default());
        }
        self.mcp.status.entry(squad.clone()).or_default().insert(server.name.clone(), statuses.clone());
        self.publish_mcp(&squad);
        if let Some(by) = card_for {
            for status in statuses.iter().filter(|s| s.state == McpState::NeedsSignIn || (by.is_none() && s.state == McpState::Failing)) {
                self.open_mcp_card(&squad, &server, status.engine, by.clone());
            }
        }
    }

    /// The person put off the sign-in to `server` on `kind`'s engine.
    pub(super) fn mcp_put_off(&mut self, squad: &str, server: &str, kind: Kind) {
        let engine = if kind == Kind::CodexMcp { Engine::Codex } else { Engine::Claude };
        self.mcp.put_off.insert((squad.to_owned(), server.to_owned(), engine));
    }

    /// One sign-in card for `server` on `engine` — unless one is up, or the
    /// person put it off and didn't ask (`by` a member).
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
        if matches!(server.auth, McpAuth::Gateway { .. }) {
            return;
        }
        let kind = kind_of(engine);
        if self.mcp_card_up(squad, &server.name, kind) {
            return;
        }
        if by.is_some() && self.mcp.put_off.contains(&(squad.to_owned(), server.name.clone(), engine)) {
            return;
        }
        let plan = sm::signin_plan(engine, &self.binary_of(engine), &sm::signin_dir(&self.config.home, squad), server);
        let name = format!("{} ({})", sm::label(server), kind.name());
        self.start_mcp_card(squad, kind, plan, &server.name, name, sm::signin_why(server), by);
    }

    /// The outside servers a member of `role` on `engine` gets this turn.
    /// One it gets that needs a sign-in on its engine puts one card in front
    /// of the person (once); one not looked at yet is looked at, the card
    /// following if it needs one.
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
        for server in &mine {
            let known = self.mcp.status.get(squad).and_then(|k| k.get(&server.name)).and_then(|all| all.iter().find(|s| s.engine == engine)).map(|s| (s.state, s.checked_at));
            let by = Some((id, member.0.clone(), ticket));
            // A keyed server stands on its key being kept: looked at again when that changed.
            if let McpAuth::Key { secret, .. } = &server.auth {
                let kept = self.key_kept(squad, secret);
                let shown = known.map(|(state, _)| state);
                if shown != Some(if kept { McpState::Connected } else { McpState::NeedsSignIn }) {
                    self.check_mcp_servers(squad, Some(&server.name), Some(by));
                } else if !kept {
                    self.open_mcp_card(squad, server, engine, by);
                }
                continue;
            }
            match known {
                // Signed in elsewhere meanwhile (`equipe mcp login`)? Looked at again after a while.
                Some((McpState::NeedsSignIn, at)) if now.saturating_sub(at) > RECHECK_SIGN_IN => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((McpState::NeedsSignIn, _)) => self.open_mcp_card(squad, server, engine, by),
                // Not looked at on this engine yet, or not lately (a sign-in
                // can run out): looked at now, a card following if needed.
                None | Some((McpState::Unchecked, _)) => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                Some((_, at)) if now.saturating_sub(at) > FRESH_FOR => self.check_mcp_servers(squad, Some(&server.name), Some(by)),
                _ => {}
            }
        }
        // Each one's credential, read now: a keyed server without its key (or
        // one equipe would have to sign in to itself) isn't given.
        mine.into_iter()
            .filter_map(|server| {
                let credential = sm::credential(&server, |secret| self.key_kept(squad, secret).then(|| self.key_value(squad, secret).ok()).flatten());
                match credential {
                    Credential::Engine | Credential::Header { .. } => Some((server, credential)),
                    Credential::MissingKey { .. } | Credential::Unavailable => None,
                }
            })
            .collect()
    }

    /// Whether the squad keeps a key called `name` (its facts; no value read).
    fn key_kept(&self, squad: &str, name: &str) -> bool {
        self.journals.get(squad).map(crate::squad_secrets::read).unwrap_or_default().iter().any(|k| k.name == name)
    }
}
