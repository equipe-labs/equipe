//! Side chats, as the daemon runs them ([`crate::side_chat`]).
//!
//! `StartSideChat` keeps the passage and says whether it searches the web;
//! each `SideChatSubmit` runs one turn on the member's engine and model, in
//! its working folder, read-only. The first turn forks the member's own
//! conversation (Claude Code's session, Codex's thread); the next ones go on
//! in the fork. What the turn does is published inside
//! [`Event::SideChat`] — never folded into the member's conversation nor
//! journalled — and what it used counts toward the member's tokens. Closing
//! the side chat stops its turn; one nobody wrote in for
//! [`equipe_protocol::side_chat::IDLE_SECS`] is let go on the minute tick,
//! and the conversation closing takes its side chats with it.

use super::*;
use crate::side_chat as rules;
use equipe_protocol::SideChatId;

/// One side chat.
pub(super) struct SideChat {
    /// The conversation it is about.
    session: SessionId,
    squad: Option<String>,
    passage: String,
    search: bool,
    /// The engine's own conversation for it once its first turn ran: Claude
    /// Code's forked session, Codex's forked thread.
    thread: Option<String>,
    /// Its own history, for Codex over the API (which keeps none).
    history: Vec<responses::Message>,
    /// Turns it ran.
    turns: usize,
    turn: Option<JoinHandle<()>>,
    /// Stops the turn running now through its engine (Codex's interrupt).
    stop: Option<oneshot::Sender<()>>,
    last: std::time::Instant,
}

/// What a side chat's turn hands back to the loop.
pub(super) struct SideDone {
    pub side: SideChatId,
    pub prompt: String,
    pub reply: Option<String>,
    pub thread: Option<String>,
}

impl Runtime {
    /// Whether a side chat about `session` searches the web, on its engine as it runs now.
    fn side_searches(&self, session: &Session) -> bool {
        let other = session.engine == Engine::Codex && session.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
        rules::searches(session.engine, session.codex.is_none(), other)
    }

    /// `StartSideChat`: kept, and said open. One side chat a conversation:
    /// another about it is let go.
    pub(super) fn start_side_chat(&mut self, id: SessionId, side: SideChatId, passage: String) {
        let Some(session) = self.sessions.get(&id) else {
            return self.error(None, say!(self, "error.no_session", id = id), None);
        };
        let (squad, search) = (session.squad.clone(), self.side_searches(session));
        let others: Vec<SideChatId> = self.side_chats.iter().filter(|(s, c)| c.session == id && **s != side).map(|(s, _)| *s).collect();
        for other in others {
            self.close_side_chat(other);
        }
        let kept = self.side_chats.entry(side).or_insert_with(|| SideChat {
            session: id,
            squad: squad.clone(),
            passage: passage.clone(),
            search,
            thread: None,
            history: Vec::new(),
            turns: 0,
            turn: None,
            stop: None,
            last: std::time::Instant::now(),
        });
        kept.last = std::time::Instant::now();
        self.publish(Event::SideChatOpened { squad, session: id, side, search });
    }

    /// `CloseSideChat`: its turn stopped, and let go.
    pub(super) fn close_side_chat(&mut self, side: SideChatId) {
        let Some(chat) = self.side_chats.remove(&side) else { return };
        stop_side_turn(chat.turn, chat.stop);
        self.publish(Event::SideChatClosed { squad: chat.squad, side });
    }

    /// What happened in a side chat: published inside it; a turn's usage
    /// counts toward the member's.
    pub(super) fn side_event(&mut self, side: SideChatId, event: Event) {
        let Some(chat) = self.side_chats.get(&side) else { return };
        let (session, squad) = (chat.session, chat.squad.clone());
        if let Event::TurnEnded { usage: Some(u), .. } = &event
            && let Some(s) = self.sessions.get_mut(&session)
        {
            s.tokens += u.total();
        }
        self.publish(Event::SideChat { squad, session, side, event: Box::new(event) });
    }

    /// A side chat's turn ended: the fork it runs in kept for the next.
    pub(super) fn side_done(&mut self, done: SideDone) {
        let Some(chat) = self.side_chats.get_mut(&done.side) else { return };
        chat.turn = None;
        chat.stop = None;
        chat.last = std::time::Instant::now();
        if done.thread.is_some() {
            chat.thread = done.thread;
        }
        if let Some(reply) = done.reply {
            chat.history.push(responses::Message { role: responses::Role::User, text: done.prompt });
            chat.history.push(responses::Message { role: responses::Role::Assistant, text: reply });
        }
    }

    /// The minute tick: side chats nobody wrote in for long are let go, and
    /// those whose conversation closed.
    pub(super) fn let_side_chats_go(&mut self) {
        let idle = Duration::from_secs(equipe_protocol::side_chat::IDLE_SECS);
        let gone: Vec<SideChatId> = self
            .side_chats
            .iter()
            .filter(|(_, c)| !self.sessions.contains_key(&c.session) || (c.turn.as_ref().is_none_or(JoinHandle::is_finished) && c.last.elapsed() >= idle))
            .map(|(s, _)| *s)
            .collect();
        for side in gone {
            self.close_side_chat(side);
        }
    }

    /// A conversation closed: its side chats with it.
    pub(super) fn close_side_chats_of(&mut self, id: SessionId) {
        let of: Vec<SideChatId> = self.side_chats.iter().filter(|(_, c)| c.session == id).map(|(s, _)| *s).collect();
        for side in of {
            self.close_side_chat(side);
        }
    }

    /// `SideChatSubmit`: the person's message, and the side chat's turn on it.
    pub(super) fn side_chat_submit(&mut self, id: SessionId, side: SideChatId, passage: String, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if !self.side_chats.contains_key(&side) {
            self.start_side_chat(id, side, passage);
        }
        let Some(chat) = self.side_chats.get(&side) else { return };
        if chat.turn.as_ref().is_some_and(|t| !t.is_finished()) {
            return self.side_event(side, Event::Error { session: Some(side), message: say!(self, "error.side_chat_busy"), next: None });
        }
        self.side_event(side, Event::UserMessage { session: side, text: text.clone(), kept: Vec::new() });
        if let Err(message) = self.start_side_turn(side, text) {
            self.side_event(side, Event::Error { session: Some(side), message, next: None });
            self.side_event(side, Event::TurnEnded { session: side, outcome: TurnOutcome::Failed, usage: None });
        }
    }

    /// Run side chat `side`'s turn on `text`.
    fn start_side_turn(&mut self, side: SideChatId, text: String) -> Result<(), String> {
        let chat = self.side_chats.get(&side).ok_or_else(String::new)?;
        let id = chat.session;
        let session = self.sessions.get(&id).ok_or_else(|| say!(self, "error.no_session", id = id))?;
        let first = chat.turns == 0 || chat.thread.is_none();
        // The member's own conversation, to fork from: its ticket's, else its desk's.
        let member_thread = session.work.as_ref().map_or_else(|| session.claude_session.clone(), |w| w.claude_session.clone());
        let (resume, fork) = match (&chat.thread, first) {
            (Some(own), false) => (Some(own.clone()), false),
            _ => (member_thread, true),
        };
        let search = chat.search;
        let prompt = if first { rules::first_prompt(&chat.passage, &text, search, None) } else { text.clone() };
        // For an engine that can't fork: the conversation in short.
        let fresh = rules::first_prompt(&chat.passage, &text, search, Some(&rules::summary(session.transcript.entries())));
        let cwd = session.work.as_ref().map_or_else(|| session.cwd.clone(), |w| w.cwd.clone());
        let squad = session.squad.clone();
        // It reads what the member reads: the squad's projects and home.
        let mut add_dirs: Vec<PathBuf> = squad.as_deref().map(|s| self.projects_cached(s)).unwrap_or_default();
        add_dirs.extend(squad.as_deref().map(|s| squad_home::path(&self.config.home, s)));
        add_dirs.retain(|d| *d != cwd);
        add_dirs.dedup();
        let other_homes = squad.as_deref().map(|s| squad_home::others(&self.config.home, s)).unwrap_or_default();
        let mut env: Vec<(String, String)> = Vec::new();
        if let Some(name) = session.name() {
            env.push((guard::MEMBER_ENV.to_owned(), name.to_owned()));
        }
        let model = session.model.clone();
        let effort = effort_of(session);
        let internal = self.internal.clone();
        let lang = self.config.lang;
        let emit = {
            let internal = internal.clone();
            move |event: Event| {
                let _ = internal.send(Internal::SideChat { side, event: Box::new(event) });
            }
        };
        // What a turn does, as the side chat shows it: its words and its steps;
        // the plan's usage, as any turn's.
        let forward = {
            let internal = internal.clone();
            let emit = emit.clone();
            move |activity: Activity| match activity {
                Activity::Text(_) | Activity::Tool { .. } | Activity::ToolDone(_) => {
                    if let Some(event) = activity_event(side, activity) {
                        emit(event);
                    }
                }
                Activity::Plan { .. } => {
                    if let Some(event) = activity_event(side, activity) {
                        let _ = internal.send(Internal::Event(Box::new(event)));
                    }
                }
                Activity::Limits(limits) => {
                    let _ = internal.send(Internal::CodexLimits(Some(limits)));
                }
                _ => {}
            }
        };
        let mut stop = None;
        let turn = match session.engine {
            Engine::Claude => {
                let binary = self.config.claude_binary.clone();
                let memory = session.memory.clone();
                let hooks = self.config.equipe_binary.as_deref().map(guard::side_chat_hooks);
                let private: Vec<PathBuf> = [self.config.home.join("browser"), self.config.home.join("credentials"), equipe_desktop_control::helper::dir(&self.config.home)]
                    .into_iter()
                    .chain(other_homes)
                    .collect();
                let claude = rules::claude_rules(&private);
                tokio::spawn(async move {
                    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
                    let said = AtomicBool::new(false);
                    let mut on = |activity: Activity| {
                        if matches!(activity, Activity::Text(_)) {
                            said.store(true, Relaxed);
                        }
                        forward(activity);
                    };
                    let asked = if fork && resume.is_none() { fresh.clone() } else { prompt.clone() };
                    let request = claude_code::TurnRequest {
                        prompt: &asked,
                        model: model.as_deref(),
                        access: claude_code::Access::ReadOnly,
                        resume: resume.as_deref(),
                        fork,
                        tools: &claude.tools,
                        memory: memory.as_deref(),
                        add_dirs: &add_dirs,
                        mcp_config: Some(rules::NO_MCP),
                        allowed_tools: &claude.allowed,
                        disallowed_tools: &claude.disallowed,
                        deny: &claude.deny,
                        hooks: hooks.as_ref(),
                        env: &env,
                        effort: effort.as_deref(),
                        ..Default::default()
                    };
                    let mut outcome = claude_code::run_turn(&binary, &cwd, request, None, &mut on).await;
                    // The member's session can't be forked (gone, or never begun): a
                    // fresh conversation, told the conversation in short.
                    if outcome.is_err() && fork && resume.is_some() && !said.load(Relaxed) {
                        outcome = claude_code::run_turn(&binary, &cwd, claude_code::TurnRequest { prompt: &fresh, resume: None, ..request }, None, &mut on).await;
                    }
                    let thread = outcome.as_ref().ok().and_then(|turn| turn.session_id.clone());
                    let result = outcome.map(|turn| turn.result).map_err(claude_failure);
                    let reply = finish_turn(&emit, side, result, said.load(Relaxed));
                    let _ = internal.send(Internal::SideDone(SideDone { side, prompt: asked, reply, thread }));
                })
            }
            Engine::Codex if session.codex.is_none() => {
                let binary = self.config.codex_binary.clone();
                let nebius = session.model.as_deref().is_some_and(crate::nemotron::is_agent_model);
                let provider = if nebius {
                    let cfg = self.open_config(session.model.as_deref())?.ok_or_else(|| say!(self, "error.nebius_no_key"))?;
                    env.push(cfg.codex_env());
                    Some(cfg.codex_provider())
                } else {
                    None
                };
                let effort = if nebius { crate::nemotron::agent_effort(effort.as_deref()).map(str::to_owned) } else { effort };
                let model = model.as_deref().map(|m| crate::model_providers::bare_model(m).to_owned());
                let (stop_tx, stop_rx) = oneshot::channel::<()>();
                stop = Some(stop_tx);
                tokio::spawn(async move {
                    let mut streamed = false;
                    let none = json!({});
                    let request = codex_app::TurnRequest {
                        prompt: if fork && resume.is_none() { &fresh } else { &prompt },
                        fresh_prompt: Some(&fresh),
                        model: model.as_deref(),
                        effort: effort.as_deref(),
                        access: claude_code::Access::ReadOnly,
                        resume: resume.as_deref(),
                        fork,
                        web_search: search,
                        mcp_servers: Some(&none),
                        env: &env,
                        provider: provider.as_ref(),
                        ..Default::default()
                    };
                    let outcome = codex_app::run_turn(&binary, &cwd, request, None, None, Some(stop_rx), |activity| {
                        streamed |= matches!(activity, Activity::Text(_));
                        forward(activity);
                    })
                    .await;
                    if outcome.as_ref().is_ok_and(|turn| turn.result.status == "interrupted") {
                        return;
                    }
                    let thread = outcome.as_ref().ok().and_then(|turn| turn.thread_id.clone());
                    let result = outcome.map(|turn| turn.result).map_err(|err| codex_cli_failure(err, lang));
                    let reply = finish_turn(&emit, side, result, streamed);
                    let _ = internal.send(Internal::SideDone(SideDone { side, prompt, reply, thread }));
                })
            }
            // Codex over the API: no tools; the recent conversation goes with each turn.
            Engine::Codex => {
                let cred = session.codex.clone().ok_or_else(|| say!(self, "error.codex_signed_out"))?;
                let client = self.client.clone();
                let version = self.config.client_version.clone();
                let mut history = recent_history(&session.history);
                history.extend(chat.history.iter().cloned());
                let model = model.unwrap_or_default();
                let prompt = if first { rules::first_prompt(&chat.passage, &text, false, None) } else { text.clone() };
                tokio::spawn(async move {
                    let mut streamed = false;
                    let request = responses::TurnRequest { prompt: &prompt, model: &model, instructions: None, history: &history, effort: effort.as_deref() };
                    let result = responses::run_turn(&client, &cred, &version, request, |delta| {
                        streamed = true;
                        emit(Event::TextDelta { session: side, text: delta.to_owned() });
                    })
                    .await;
                    let reply = finish_turn(&emit, side, result.map_err(codex_failure), streamed);
                    let _ = internal.send(Internal::SideDone(SideDone { side, prompt, reply, thread: None }));
                })
            }
        };
        if let Some(chat) = self.side_chats.get_mut(&side) {
            chat.turns += 1;
            chat.turn = Some(turn);
            chat.stop = stop;
            chat.last = std::time::Instant::now();
        }
        Ok(())
    }
}

/// A side chat's turn stopped: Codex winds it down, else it is cut (its
/// engine's process ends with it).
fn stop_side_turn(turn: Option<JoinHandle<()>>, stop: Option<oneshot::Sender<()>>) {
    let Some(turn) = turn.filter(|t| !t.is_finished()) else { return };
    if stop.is_some_and(|stop| stop.send(()).is_ok()) {
        tokio::spawn(async move {
            tokio::time::sleep(STOP_GRACE).await;
            turn.abort();
        });
    } else {
        turn.abort();
    }
}
