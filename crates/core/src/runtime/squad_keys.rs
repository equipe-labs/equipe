//! The keys, tokens and passwords a squad's members obtain and keep
//! (`crate::squad_secrets`): the tools (`secret_save`, `secret_list`,
//! `secret_use`, `secret_write`), the person's first-use question, the
//! screens' list, and the masking of every kept value in what leaves a
//! member — events, tool arguments, the log.
//!
//! Values come from the daemon machine's secret store and sit in memory only
//! as the masking list ([`crate::squad_secrets::watch`]); no event, journal
//! row or tool answer carries one.

use equipe_protocol::decide::{self, Choice, Question, QuestionSet};

use super::*;
use crate::protocol::secrets::{NO_PROJECT, SecretValue, SquadSecret};
use crate::squad_secrets as ks;

/// What a gate decided about a use.
enum Gate {
    Go(Box<SquadSecret>),
    /// The person is asked (or was): what to tell the member.
    Told(String),
}

impl Runtime {
    /// What conversation `id` is on: its ticket, the project's name (empty:
    /// none), and where it works.
    fn key_context(&mut self, squad: &str, id: SessionId) -> (Option<TicketId>, String, PathBuf) {
        // The squad's journal open, so its projects are known.
        let _ = self.journal(squad);
        self.key_context_of(squad, id)
    }

    fn key_context_of(&self, squad: &str, id: SessionId) -> (Option<TicketId>, String, PathBuf) {
        let names: Vec<String> = self.projects_cached(squad).iter().map(|p| board::project_name(p)).collect();
        let Some(session) = self.sessions.get(&id) else { return (None, NO_PROJECT.to_owned(), PathBuf::new()) };
        match &session.work {
            Some(work) => {
                let name = board::project_name(&work.project);
                (Some(work.ticket), if names.contains(&name) { name } else { NO_PROJECT.to_owned() }, work.cwd.clone())
            }
            None => (None, session.lane.as_ref().and_then(|(_, p)| p.clone()).filter(|p| names.contains(p)).unwrap_or_default(), session.cwd.clone()),
        }
    }

    /// Every squad's kept keys, as facts.
    pub(super) fn all_squad_secrets(&mut self) -> Vec<SquadSecret> {
        let squads: Vec<String> = crate::squads::list(&self.config.home).into_iter().map(|s| s.name).collect();
        squads.iter().flat_map(|s| self.journal(s).map(ks::read).unwrap_or_default()).collect()
    }

    pub(super) fn publish_squad_secrets(&mut self) {
        let secrets = self.all_squad_secrets();
        self.publish(Event::SquadSecrets { secrets });
    }

    /// A kept value: the masking list first (read once, at start), else the secret store.
    fn key_value(&self, squad: &str, name: &str) -> Result<String, String> {
        if let Some(value) = ks::watched(&ks::watch_key(squad, name)) {
            return Ok(value);
        }
        let value = self.vault.lock().map_err(|_| "The secret store is unreadable.".to_owned())?.squad_secret(squad, name, false)?;
        ks::watch(&ks::watch_key(squad, name), &value);
        Ok(value)
    }

    /// At start: every kept value into the masking list, off the loop (the
    /// secret store may ask the person once).
    pub(super) fn load_squad_secrets(&mut self) {
        let squads: Vec<String> = crate::squads::list(&self.config.home).into_iter().map(|s| s.name).collect();
        let wanted: Vec<(String, String)> = squads.iter().flat_map(|s| self.journal(s).map(ks::read).unwrap_or_default()).map(|k| (k.squad, k.name)).collect();
        if wanted.is_empty() {
            return;
        }
        let vault = self.vault.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(vault) = vault.lock() else { return };
            for (squad, name) in wanted {
                if let Ok(value) = vault.squad_secret(&squad, &name, false) {
                    ks::watch(&ks::watch_key(&squad, &name), &value);
                }
            }
        });
    }

    /// `secret_save`: keep what a member obtained.
    pub(super) fn secret_save(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let draft = ks::check_save(args)?;
        let (ticket, current, _) = self.key_context(squad, id);
        let squad_projects = self.projects(squad);
        let names: Vec<String> = squad_projects.iter().map(|p| board::project_name(p)).collect();
        let existing = self.journal(squad).map(ks::read).unwrap_or_default().into_iter().find(|s| s.name == draft.name);
        let projects = match (&draft.projects, &existing) {
            (Some(asked), _) => {
                let mut out: Vec<String> = Vec::new();
                for arg in asked {
                    let project = board::resolve_project(arg, &squad_projects).ok_or_else(|| format!("No project called {arg}. The squad's projects: {}.", names.join(", ")))?;
                    let name = board::project_name(&project);
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
                out
            }
            (None, Some(kept)) => kept.projects.clone(),
            (None, None) if !current.is_empty() => vec![current],
            (None, None) => Vec::new(),
        };
        self.vault
            .lock()
            .map_err(|_| "The secret store is unreadable.".to_owned())?
            .set_squad_secret(squad, &draft.name, &draft.value)
            .map_err(|why| format!("The secret store refused it ({why}); nothing was kept."))?;
        ks::watch(&ks::watch_key(squad, &draft.name), &draft.value);
        let replaced = existing.is_some();
        let secret = SquadSecret {
            squad: squad.to_owned(),
            name: draft.name.clone(),
            kind: draft.kind,
            service: draft.service,
            projects,
            any_project: existing.as_ref().is_some_and(|e| e.any_project),
            created_by: member.to_owned(),
            created_ticket: ticket,
            created_at: now_secs(),
            note: draft.note,
            last_used: existing.and_then(|e| e.last_used),
        };
        let journal = self.journal(squad).ok_or("The squad's journal can't be opened.")?;
        ks::put(journal, secret.clone())?;
        if let Some(ticket) = ticket {
            let _ = journal.ticket_event(ticket, member, "secret_saved", &secret.name);
        }
        let text = say!(self, "notice.squad_secret_saved", member = member, name = secret.name, service = secret.service);
        self.publish(Event::Notice { undo: None, session: id, text });
        self.publish_squad_secrets();
        Ok(ks::saved_text(&secret, replaced))
    }

    /// `secret_list`: facts only.
    pub(super) fn secret_list(&mut self, squad: &str) -> Result<String, String> {
        let all = self.journal(squad).map(ks::read).unwrap_or_default();
        Ok(ks::list_text(&all))
    }

    /// May `member` use `name` on `project`: the key, or the card put to the
    /// person (once) for a project it wasn't allowed on.
    fn key_gate(&mut self, squad: &str, member: &str, id: SessionId, name: &str, project: &str, ticket: Option<TicketId>) -> Result<Gate, String> {
        let secret = self
            .journal(squad)
            .map(ks::read)
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("No key called {name} is kept for this squad. `secret_list` shows what is."))?;
        Ok(match ks::access(&secret, project) {
            ks::Access::Allowed => Gate::Go(Box::new(secret)),
            ks::Access::Ask => Gate::Told(self.ask_first_use(squad, member, id, &secret, project, ticket)),
        })
    }

    /// A key's first use on a project: the person is asked once, on a card.
    fn ask_first_use(&mut self, squad: &str, member: &str, id: SessionId, secret: &SquadSecret, project: &str, ticket: Option<TicketId>) -> String {
        let shown = if project.is_empty() { say!(self, "secrets.no_project") } else { project.to_owned() };
        let grants = self.journal(squad).map(ks::grants).unwrap_or_default();
        let waiting: Vec<u64> = self.question_sets(squad).iter().map(|s| s.id).collect();
        if grants.iter().any(|g| g.name == secret.name && g.project == project && waiting.contains(&g.set)) {
            return format!("The person was already asked whether you may use {} on {shown}; their answer comes to you as your next message. Don't ask again until then.", secret.name);
        }
        let choice = |label: String, description: String| Choice { label, description, ..Choice::default() };
        let question = Question {
            question: say!(self, "secrets.grant.question", member = member, name = secret.name, service = secret.service, project = shown),
            header: say!(self, "secrets.grant.header"),
            options: vec![
                choice(say!(self, "secrets.grant.here", project = shown), say!(self, "secrets.grant.here_desc")),
                choice(say!(self, "secrets.grant.any"), say!(self, "secrets.grant.any_desc")),
                choice(say!(self, "secrets.grant.no"), say!(self, "secrets.grant.no_desc")),
            ],
            ..Question::default()
        };
        let set = QuestionSet { id: equipe_protocol::new_session_id(), session: id, member: member.to_owned(), ticket, questions: vec![question], asked_at: now_secs(), review: false, retro: None };
        let _ = decide::add(self.question_sets(squad), set);
        let set = self.question_sets(squad).last().cloned().expect("just added");
        self.save_questions(squad);
        if let Some(journal) = self.journal(squad) {
            let mut all = ks::grants(journal);
            all.push(ks::Grant { set: set.id, name: secret.name.clone(), project: project.to_owned(), member: member.to_owned() });
            ks::keep_grants(journal, &all);
            if let Some(ticket) = ticket {
                let _ = journal.ticket_event(ticket, member, "secret_asked", &format!("{} → {}", secret.name, if project.is_empty() { "(no project)" } else { project }));
            }
        }
        self.publish(Event::QuestionsAsked { squad: squad.to_owned(), questions: set });
        format!(
            "The person is asked on a card whether you may use {} on {shown}; their answer comes to you as your next message. \
Don't wait for it in this turn: end it with a short line, or carry on with work that doesn't need the key — then call this again.",
            secret.name
        )
    }

    /// `secret_use`: the key as an environment variable of this session's next turns.
    pub(super) fn secret_use(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let name = board::arg_str(args, "name").ok_or("Which key? Give its `name` (`secret_list`).")?.to_owned();
        let var = board::arg_str(args, "as_env").ok_or("Say in `as_env` which environment variable it is given as (`POSTHOG_KEY`).")?.to_owned();
        if !ks::valid_env(&var) {
            return Err(format!("{var:?}: an environment variable is uppercase letters, digits and _, starting with a letter (POSTHOG_KEY)."));
        }
        let (ticket, project, _) = self.key_context(squad, id);
        let secret = match self.key_gate(squad, member, id, &name, &project, ticket)? {
            Gate::Told(text) => return Ok(text),
            Gate::Go(secret) => secret,
        };
        self.key_value(squad, &secret.name).map_err(|why| format!("The key's value can't be read: {why}"))?;
        let bound = self.secret_env.entry(id).or_default();
        bound.retain(|(v, _)| *v != var);
        bound.push((var.clone(), secret.name.clone()));
        self.record_key_use(squad, member, &secret.name, ticket, &project, "env");
        Ok(format!(
            "Every command you run from your next turn on has ${var} set to {} — the value is never shown to you or written \
anywhere. Your commands in this turn don't have it yet: end the turn and carry on, or use `secret_write` to put it in a git-ignored \
file now. Never print, echo or copy the value.",
            secret.name
        ))
    }

    /// `secret_write`: the key into a git-ignored file of the worktree.
    pub(super) fn secret_write(&mut self, squad: &str, member: &str, id: SessionId, args: &Value) -> Result<String, String> {
        let name = board::arg_str(args, "name").ok_or("Which key? Give its `name` (`secret_list`).")?.to_owned();
        let rel = board::arg_str(args, "path").ok_or("Say in `path` which file, relative to your worktree: `.env.local`.")?.to_owned();
        let format = ks::Format::parse(board::arg_str(args, "format"))?;
        let (ticket, project, cwd) = self.key_context(squad, id);
        // Where, before whether: a file that may not hold a key is refused first.
        let path = ks::place(&cwd, &rel)?;
        let secret = match self.key_gate(squad, member, id, &name, &project, ticket)? {
            Gate::Told(text) => return Ok(text),
            Gate::Go(secret) => secret,
        };
        let key = board::arg_str(args, "key").map(str::to_owned).unwrap_or_else(|| ks::default_key(&secret.name, format));
        let value = self.key_value(squad, &secret.name).map_err(|why| format!("The key's value can't be read: {why}"))?;
        ks::write_into(&path, format, &key, &value).map_err(|why| format!("{rel}: {why}"))?;
        self.record_key_use(squad, member, &secret.name, ticket, &project, "file");
        let what = match format {
            ks::Format::Env => format!("as {key}"),
            ks::Format::Json => format!("as the field {key}"),
            ks::Format::Raw => "as the whole file".to_owned(),
        };
        Ok(format!(
            "Wrote {} {what} to {rel}: git-ignored and untracked, readable only by the person. The value is not shown to you. Don't \
commit it, print it or copy it elsewhere; to hand it to a service, run the CLI you already have against that file.",
            secret.name
        ))
    }

    fn record_key_use(&mut self, squad: &str, member: &str, name: &str, ticket: Option<TicketId>, project: &str, how: &str) {
        let usage = ks::usage(member, ticket, project, how, now_secs());
        if let Some(journal) = self.journal(squad)
            && let Err(why) = ks::record(journal, name, usage)
        {
            crate::daemon_log!("equipe: a use of {squad}'s key {name} wasn't recorded: {why}");
        }
        self.publish_squad_secrets();
    }

    /// A first-use card was answered, or closed: the person's word applied.
    pub(super) fn secret_card_settled(&mut self, squad: &str, set: &QuestionSet, answers: Option<&[decide::Answer]>) {
        let Some(journal) = self.journal(squad) else { return };
        let all = ks::grants(journal);
        let Some(grant) = all.iter().find(|g| g.set == set.id).cloned() else { return };
        let rest: Vec<ks::Grant> = all.into_iter().filter(|g| g.set != set.id).collect();
        ks::keep_grants(journal, &rest);
        let answer = answers.and_then(<[_]>::first).map_or(ks::Answer::No, |a| ks::answer_of(&a.picked));
        if let Err(why) = ks::apply_answer(journal, &grant, answer) {
            crate::daemon_log!("equipe: the answer about {}'s use of {} wasn't kept: {why}", grant.member, grant.name);
        }
        if let Some(ticket) = set.ticket {
            let said = match answer {
                ks::Answer::Here => "allowed on this project",
                ks::Answer::Any => "allowed on any project",
                ks::Answer::No => "not allowed",
            };
            let _ = journal.ticket_event(ticket, "you", "secret_answered", &format!("{} → {}: {said}", grant.name, if grant.project.is_empty() { "(no project)" } else { grant.project.as_str() }));
        }
        self.publish_squad_secrets();
    }

    /// The environment `secret_use` asked for, for a turn of `id` starting now:
    /// each key still kept, still allowed on the project it is working on.
    pub(super) fn secret_env_for(&self, squad: &str, id: SessionId) -> Vec<(String, String)> {
        let Some(bound) = self.secret_env.get(&id).cloned() else { return Vec::new() };
        let (_, project, _) = self.key_context_of(squad, id);
        let all = self.journals.get(squad).map(ks::read).unwrap_or_default();
        bound
            .into_iter()
            .filter_map(|(var, name)| {
                let secret = all.iter().find(|s| s.name == name)?;
                (ks::access(secret, &project) == ks::Access::Allowed).then_some(())?;
                Some((var, self.key_value(squad, &name).ok()?))
            })
            .collect()
    }

    /// The person's commands about kept keys.
    pub(super) fn squad_secret_command(&mut self, command: Command) {
        match command {
            Command::SquadSecrets => {}
            Command::SetSquadSecretProjects { squad, name, projects, any_project } => {
                let done = self.journal(&squad).ok_or_else(|| "the squad's journal can't be opened".to_owned()).and_then(|j| ks::set_scope(j, &name, &projects, any_project));
                if let Err(why) = done {
                    self.error(None, why, None);
                }
            }
            Command::DeleteSquadSecret { squad, name } => {
                let gone = match self.vault.lock() {
                    Ok(mut vault) => vault.delete_squad_secret(&squad, &name),
                    Err(_) => Err("the secret store is unreadable".to_owned()),
                };
                match gone {
                    Ok(()) => {
                        ks::unwatch(&ks::watch_key(&squad, &name));
                        for bound in self.secret_env.values_mut() {
                            bound.retain(|(_, n)| *n != name);
                        }
                        if let Some(journal) = self.journal(&squad) {
                            let _ = ks::remove(journal, &name);
                        }
                    }
                    Err(why) => self.error(None, why, None),
                }
            }
            Command::RevealSquadSecret { squad, name, ask } => {
                // Never to a phone: the value is shown on the machine the person sits at.
                if self.on_phone {
                    return;
                }
                let found = self.journal(&squad).map(ks::read).unwrap_or_default().iter().any(|s| s.name == name);
                let value = if found { self.vault.lock().map_err(|_| "the secret store is unreadable".to_owned()).and_then(|v| v.squad_secret(&squad, &name, true)) } else { Err("no such key".to_owned()) };
                match value {
                    Ok(value) => self.publish(Event::SquadSecretRevealed { squad, name, ask, value: SecretValue(value) }),
                    Err(why) => self.error(None, why, None),
                }
                return;
            }
            _ => return,
        }
        self.publish_squad_secrets();
    }

    // ── masking ──

    /// What leaves a member through an event, with every kept value cut out of
    /// it — and what looks like a key that wasn't kept cut out of a command's
    /// output, its member told to keep it. A held-back end of a streaming
    /// answer comes back to be published first, when the event is another
    /// kind; a fragment may come back empty, to be dropped.
    pub(super) fn guard_event(&mut self, event: &mut Event) -> Option<Event> {
        let longest = ks::longest();
        let mut flush = None;
        if !matches!(event, Event::TextDelta { .. })
            && let Some(id) = event.session()
            && let Some(tail) = self.delta_tail.remove(&id)
            && !tail.is_empty()
        {
            flush = Some(Event::TextDelta { session: id, text: tail });
        }
        let mask = |text: &mut String| {
            if let std::borrow::Cow::Owned(masked) = ks::mask(text) {
                *text = masked;
            }
        };
        match event {
            Event::TextDelta { session, text } => {
                // Held back while kept values exist: one split across fragments is cut whole.
                let mut joined = self.delta_tail.remove(session).unwrap_or_default();
                joined.push_str(text);
                mask(&mut joined);
                let mut keep = longest.saturating_sub(1).min(joined.len());
                while keep > 0 && !joined.is_char_boundary(joined.len() - keep) {
                    keep -= 1;
                }
                let tail = joined.split_off(joined.len() - keep);
                if !tail.is_empty() {
                    self.delta_tail.insert(*session, tail);
                }
                *text = joined;
            }
            Event::Thinking { text, .. } | Event::UserMessage { text, .. } | Event::MessageQueued { text, .. } | Event::Steered { text, .. } | Event::Notice { text, .. } => mask(text),
            Event::Plan { steps, .. } => steps.iter_mut().for_each(|step| mask(&mut step.text)),
            Event::ToolUse { tool, target, label, .. } => {
                // `secret_save`'s arguments are the value itself.
                if ks::carries_value(tool) {
                    *target = None;
                    *label = None;
                }
                target.iter_mut().chain(label.iter_mut()).for_each(mask);
            }
            Event::ToolResult { session, outcome, .. } => {
                mask(&mut outcome.output);
                if let Some(diff) = outcome.diff.as_mut() {
                    mask(diff);
                }
                // A key a command printed that nobody kept: cut out, and its member told to keep it.
                let (clean, found) = ks::scrub(&outcome.output);
                if found > 0 {
                    outcome.output = clean;
                    if self.sessions.get(session).is_some_and(|s| s.member.is_some()) {
                        self.secret_nudge.insert(*session);
                    }
                }
            }
            Event::SessionClosed { session } => {
                self.secret_env.remove(session);
                self.secret_nudge.remove(session);
                self.delta_tail.remove(session);
            }
            Event::TicketChanged { ticket, .. } => {
                mask(&mut ticket.title);
                mask(&mut ticket.body);
                ticket.note.iter_mut().for_each(mask);
            }
            _ => {}
        }
        flush
    }

    /// A member's tool call, before anything reads it: every kept value cut
    /// out of its arguments (ticket text, a comment, a hand-in, a memory), and
    /// what looks like a key cut out too. Whether the member should be told to
    /// keep one — now, or because a command printed one.
    pub(super) fn guard_call(&mut self, call: &mut ToolCall) -> bool {
        // The value is `secret_save`'s to keep; a command's approval card is the person's to see whole.
        if ks::carries_value(&call.name) || call.name == permissions::APPROVE_TOOL {
            return false;
        }
        let nudged = self.caller_id(call).is_some_and(|id| self.secret_nudge.remove(&id));
        ks::mask_json(&mut call.arguments);
        ks::scrub_json(&mut call.arguments) > 0 || nudged
    }
}
