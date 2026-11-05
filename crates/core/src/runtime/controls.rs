//! The squad's settings from the conversation (`crate::controls`): the PM
//! reads them, and changes one the person asked for — at once when it only
//! narrows what the squad does, else on an approval card the daemon writes
//! (never the member's words), done by the daemon on the person's yes.

use super::*;
use crate::controls::{self, Change};

impl Runtime {
    /// `settings`: what the person's screen shows of the squad's settings.
    pub(super) fn settings_text(&mut self, squad: &str) -> String {
        let projects = self.projects(squad);
        let Some(journal) = self.journal(squad) else { return format!("No squad {squad}.") };
        let (mode, pm_accepts, idle, switching, hq) = (journal.mode(), journal.pm_accepts(), journal.idle_wake_hours(), journal.engine_switching(), journal.hq());
        let (routines, signals, allowed) = (journal.routines(), journal.signals(), journal.allowed().unwrap_or_default());
        let on = |b: bool| if b { "on" } else { "off" };
        let mut out = vec![format!("Squad {squad}.")];
        let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
        out.push(format!("Projects: {}.", if names.is_empty() { "none".to_owned() } else { names.join(", ") }));
        out.push(format!("HQ: {}.", hq.as_deref().map_or_else(|| "none".to_owned(), board::project_name)));
        out.push(format!("Mode (how much members do on their own): {} for the squad.", mode_word(mode)));
        let mut members: Vec<_> = self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad) && s.lane.is_none() && s.member.is_some()).collect();
        members.sort_by_key(|s| s.name().map(str::to_owned));
        for s in members {
            let name = s.name().unwrap_or_default();
            let own = s.mode.map_or_else(|| "the squad's".to_owned(), |m| mode_word(m).to_owned());
            let effort = s.effort.as_deref().unwrap_or("default");
            // An alias the engine resolved: what it runs as, too.
            let ran = s.ran.as_deref().filter(|r| s.model.as_deref() != Some(*r)).map(|r| format!(" (runs as {r})")).unwrap_or_default();
            out.push(format!("- {name}: engine {:?}, model {}{ran}, effort {effort}, mode {own}.", s.engine, s.model.as_deref().unwrap_or("default")));
        }
        out.push(format!("You accept the team's handed-in work yourself (pm_accepts): {}.", on(pm_accepts)));
        out.push(format!("An empty board wakes you after (idle_wake_hours): {}.", if idle == 0 { "off".to_owned() } else { format!("{idle} h") }));
        out.push(format!("Engine switching: {}.", format!("{switching:?}").to_lowercase()));
        if routines.is_empty() {
            out.push("Routines: none.".into());
        }
        for r in &routines {
            out.push(format!("- routine {} \"{}\": {:?}, {}.", r.id, r.title, r.when, on(r.on)));
        }
        if signals.is_empty() {
            out.push("Signals: none.".into());
        }
        for s in &signals {
            out.push(format!("- signal {} in {}: {}.", s.name, s.project, on(s.on)));
        }
        out.push(format!("\"Always allow\" rules: {}.", if allowed.is_empty() { "none".to_owned() } else { allowed.join(", ") }));
        out.join("\n")
    }

    /// `change_setting`: done at once, or put to the person and done by the
    /// daemon on their yes. Only the PM's.
    pub(super) async fn change_setting(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let Some(id) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(ToolResult::error(format!("{} isn't on the squad.", call.member)));
            return;
        };
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            let _ = reply.send(ToolResult::error("Only the PM changes the squad's settings: tell the PM what the person wants changed."));
            return;
        }
        let squad = call.squad.clone();
        let made = Change::parse(&call.arguments).and_then(|change| Ok((self.command_for(&squad, &change)?, change)));
        let (command, change) = match made {
            Ok(made) => made,
            Err(why) => {
                let _ = reply.send(ToolResult::error(why));
                return;
            }
        };
        let now = self.mode_now(&squad, &change);
        let action = self.describe_change(&change);
        if !change.needs_yes(now) {
            crate::daemon_log!("equipe: {squad}'s PM changed a setting the person asked for: {action}");
            Box::pin(self.handle(command)).await;
            self.audit(&squad, None, &call.member, "changed", &action);
            let _ = reply.send(ToolResult::ok(format!("Done: {action}. Tell the person in one line.")));
            return;
        }
        let why = board::arg_str(&call.arguments, "why").map(str::to_owned);
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval { id: approval_id, session: id, member: call.member.clone(), ticket: None, tool: controls::SETTING_TOOL.to_owned(), action, reason: why, preview: None };
        let input = serde_json::to_value(&command).unwrap_or_default();
        self.pending.insert(approval_id, Pending { squad: squad.clone(), approval: approval.clone(), input, reply });
        self.publish(Event::ApprovalRequested { squad, approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// The person answered a change's card: on their yes the daemon makes it.
    pub(super) fn setting_answered(&mut self, squad: String, approval: Approval, input: Value, decision: Decision, reply: oneshot::Sender<ToolResult>) {
        if decision == Decision::Deny {
            self.audit(&squad, None, "you", "declined", &approval.action);
            let _ = reply.send(ToolResult::ok("The person said no: it stays as it is. Don't ask again unless they bring it up."));
            return;
        }
        let Ok(command) = serde_json::from_value::<Command>(input) else {
            let _ = reply.send(ToolResult::error("The change couldn't be read back; nothing changed."));
            return;
        };
        self.audit(&squad, None, "you", "allowed", &approval.action);
        let _ = self.internal.send(Internal::Apply(Box::new(command)));
        let _ = reply.send(ToolResult::ok(format!("The person said yes: {} — done. Tell them in one line.", approval.action)));
    }

    /// The command the person's screen would send for `change`.
    fn command_for(&self, squad: &str, change: &Change) -> Result<Command, String> {
        let squad_name = squad.to_owned();
        let session = |name: &str| self.member_id(squad, name).ok_or_else(|| format!("No member called {name} on the squad."));
        let project = |name: &str| {
            let projects = self.projects_known(squad);
            board::resolve_project(name, &projects).ok_or_else(|| {
                let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
                format!("No project called {name}. The squad's projects: {}.", names.join(", "))
            })
        };
        Ok(match change.clone() {
            Change::Mode { member, mode } => {
                if let Some(name) = &member {
                    session(name)?;
                }
                Command::SetMode { squad: squad_name, member, mode }
            }
            Change::PmAccepts(on) => Command::SetPmAccepts { squad: squad_name, on },
            Change::IdleWake(hours) => Command::SetIdleWake { squad: squad_name, hours },
            Change::Model { member, model } => Command::SetModel { session: session(&member)?, model },
            Change::Effort { member, effort } => Command::SetEffort { session: session(&member)?, effort },
            Change::EngineSwitching(mode) => Command::SetEngineSwitching { squad: squad_name, mode },
            Change::RemoveProject(name) => Command::RemoveProject { squad: squad_name, path: project(&name)? },
            Change::Hq(name) => Command::SetHq { squad: squad_name, path: name.as_deref().map(project).transpose()? },
            Change::RunRoutine(id) | Change::DeleteRoutine(id) => {
                let known = self.journals.get(squad).map(Journal::routines).unwrap_or_default().iter().any(|r| r.id == id);
                if !known {
                    return Err(format!("No routine {id}; `settings` lists them."));
                }
                if matches!(change, Change::RunRoutine(_)) { Command::RunRoutine { squad: squad_name, id } } else { Command::DeleteRoutine { squad: squad_name, id } }
            }
            Change::Signal { project, name, on } => Command::SetSignalOn { squad: squad_name, project, name, on },
            Change::Disallow(rule) => Command::Disallow { squad: squad_name, rule },
            Change::Merge { ticket, push } => Command::Merge { squad: squad_name, ticket, push },
            Change::KeepOnBranch(ticket) => Command::KeepOnBranch { squad: squad_name, ticket },
        })
    }

    /// The squad's projects, as far as is known without opening its journal.
    fn projects_known(&self, squad: &str) -> Vec<PathBuf> {
        self.journals.get(squad).and_then(|j| j.projects().ok()).unwrap_or_default()
    }

    /// The mode a change of mode would replace: the member's own, else the squad's.
    fn mode_now(&mut self, squad: &str, change: &Change) -> Mode {
        let squads = self.journal(squad).map(Journal::mode).unwrap_or_default();
        match change {
            Change::Mode { member: Some(name), .. } => self.member_id(squad, name).and_then(|id| self.sessions.get(&id)).and_then(|s| s.mode).unwrap_or(squads),
            _ => squads,
        }
    }

    /// What a change does, in the person's language: the words on its card.
    fn describe_change(&self, change: &Change) -> String {
        let lang = self.config.lang;
        let mode = |m: Mode| crate::i18n::label::mode(lang, m).to_owned();
        match change {
            Change::Mode { member: None, mode: Some(m) } => say!(self, "control.mode_squad", mode = mode(*m)),
            Change::Mode { member: Some(name), mode: Some(m) } => say!(self, "control.mode_member", member = name, mode = mode(*m)),
            Change::Mode { member, mode: None } => say!(self, "control.mode_default", member = member.as_deref().unwrap_or_default()),
            Change::PmAccepts(true) => say!(self, "control.pm_accepts_on"),
            Change::PmAccepts(false) => say!(self, "control.pm_accepts_off"),
            Change::IdleWake(0) => say!(self, "control.idle_wake_off"),
            Change::IdleWake(hours) => say!(self, "control.idle_wake", hours = hours),
            Change::Model { member, model } => say!(self, "control.model", member = member, model = model),
            Change::Effort { member, effort } => say!(self, "control.effort", member = member, effort = effort.as_deref().unwrap_or("default")),
            Change::EngineSwitching(m) => say!(self, "control.engine_switching", mode = format!("{m:?}").to_lowercase()),
            Change::RemoveProject(name) => say!(self, "control.remove_project", project = name),
            Change::Hq(Some(name)) => say!(self, "control.hq", project = name),
            Change::Hq(None) => say!(self, "control.hq_none"),
            Change::RunRoutine(id) => say!(self, "control.run_routine", id = id),
            Change::DeleteRoutine(id) => say!(self, "control.delete_routine", id = id),
            Change::Signal { project, name, on: true } => say!(self, "control.signal_on", project = project, name = name),
            Change::Signal { project, name, on: false } => say!(self, "control.signal_off", project = project, name = name),
            Change::Disallow(rule) => say!(self, "control.disallow", rule = rule),
            Change::Merge { ticket, push: true } => say!(self, "control.merge_push", ticket = ticket),
            Change::Merge { ticket, push: false } => say!(self, "control.merge", ticket = ticket),
            Change::KeepOnBranch(ticket) => say!(self, "control.keep_on_branch", ticket = ticket),
        }
    }
}

/// A mode as `settings` and `change_setting` name it.
fn mode_word(mode: Mode) -> &'static str {
    match mode {
        Mode::ReadOnly => "read_only",
        Mode::Ask => "ask",
        Mode::Auto => "auto",
        Mode::Full => "full",
    }
}
