//! The squad's settings from the conversation (`crate::controls`): the PM
//! reads them, and changes one the person asked for — at once when it only
//! narrows what the squad does, else on an approval card the daemon writes
//! (never the member's words), done by the daemon on the person's yes.

use super::*;
use crate::controls::{self, Change, ModelFit, Mover, engine_name};

impl Runtime {
    /// The squad's parallel work, kept and told; more room on max starts
    /// what is ready at once, and less lets the runs at work finish.
    pub(super) async fn set_parallel(&mut self, squad: &str, parallel: crate::protocol::lanes::Parallel) {
        let parallel = parallel.clamped();
        match self.journal(squad).map(|j| j.set_parallel(parallel)) {
            Some(Ok(())) => {}
            Some(Err(err)) => return self.squad_error(squad, format!("{err:#}")),
            None => return,
        }
        crate::daemon_log!("equipe: {squad}'s parallel work is now {} ({} per member, {} per engine)", parallel.mode.code(), parallel.per_member, parallel.per_engine);
        self.publish(Event::ParallelChanged { squad: squad.to_owned(), parallel });
        self.dispatch(squad).await;
    }

    /// `settings`: what the person's screen shows of the squad's settings.
    pub(super) fn settings_text(&mut self, squad: &str) -> String {
        let projects = self.projects(squad);
        let Some(journal) = self.journal(squad) else { return format!("No squad {squad}.") };
        let (mode, pm_accepts, idle, switching, hq, subagents_at_once) = (
            journal.mode(),
            journal.pm_accepts(),
            journal.idle_wake_hours(),
            journal.engine_switching(),
            journal.hq(),
            journal.subagents_at_once(),
        );
        let reviewer = journal.design_reviewer();
        let parallel = journal.parallel();
        let model_choice = journal.model_choice();
        let (routines, signals, allowed) = (journal.routines(), journal.signals(), journal.allowed().unwrap_or_default());
        let resting = journal.resting();
        let on = |b: bool| if b { "on" } else { "off" };
        let mut out = vec![format!("Squad {squad}.")];
        let names: Vec<String> = projects.iter().map(|p| board::project_name(p)).collect();
        out.push(format!("Projects: {}.", if names.is_empty() { "none".to_owned() } else { names.join(", ") }));
        out.push(format!("HQ: {}.", hq.as_deref().map_or_else(|| "none".to_owned(), board::project_name)));
        out.push(format!("Mode (how much members do on their own): {} for the squad.", mode_word(mode)));
        let mut members: Vec<_> = self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad) && s.is_member_itself() && s.member.is_some()).collect();
        members.sort_by_key(|s| s.name().map(str::to_owned));
        for s in members {
            let name = s.name().unwrap_or_default();
            let own = s.mode.map_or_else(|| "the squad's".to_owned(), |m| mode_word(m).to_owned());
            let effort = s.effort.as_deref().unwrap_or("default");
            // An alias the engine resolved: what it runs as, too.
            let ran = s.ran.as_deref().filter(|r| s.model.as_deref() != Some(*r)).map(|r| format!(" (runs as {r})")).unwrap_or_default();
            // Rested by the person: no new work until woken.
            let rests = if resting.iter().any(|r| r == name) { ", resting (takes no new work)" } else { "" };
            out.push(format!("- {name}: engine {:?}, model {}{ran}, effort {effort}, mode {own}{rests}.", s.engine, s.model.as_deref().unwrap_or("default")));
        }
        out.push(format!("You accept the team's handed-in work yourself (pm_accepts): {}.", on(pm_accepts)));
        out.push(format!("An empty board wakes you after (idle_wake_hours): {}.", if idle == 0 { "off".to_owned() } else { format!("{idle} h") }));
        out.push(match subagents_at_once {
            Some(at_once) => format!("Subagents at once (subagents_at_once): {at_once}."),
            None => format!(
                "Subagents at once (subagents_at_once): default — Claude {}, Codex {}.",
                crate::protocol::CLAUDE_SUBAGENTS_AT_ONCE,
                crate::protocol::CODEX_SUBAGENTS_AT_ONCE
            ),
        });
        out.push(format!("Engine switching: {}.", format!("{switching:?}").to_lowercase()));
        out.push(if parallel.is_max() {
            format!("Parallel work (parallel): max — up to {} tickets per member and {} per engine at once, whatever the plans' usage.", parallel.per_member, parallel.per_engine)
        } else {
            format!("Parallel work (parallel): auto — fewer tickets at once as a plan runs low (max's caps: {} per member, {} per engine).", parallel.per_member, parallel.per_engine)
        });
        out.push(match model_choice {
            crate::protocol::tiers::ModelChoice::QualityFirst => {
                "Model choice (model_choice): quality — each ticket's lane starts on the cheapest model and effort that does its work well (an everyday model for routine changes, a strong one for features, the hardest thinking for real risk), never below a model the person chose for its member; work sent back for correctness goes up a step.".to_owned()
            }
            crate::protocol::tiers::ModelChoice::Off => "Model choice (model_choice): off — every ticket runs on its member's own model and effort.".to_owned(),
        });
        out.push(format!(
            "Who reviews the designer's work (design_reviewer): {}.",
            match reviewer {
                crate::protocol::engines::Reviewer::Auto => "auto — the other engine when it is signed in with room, else the member's own in a fresh session",
                crate::protocol::engines::Reviewer::Same => "same — the member's own engine, in a fresh session",
                crate::protocol::engines::Reviewer::Other => "other — always the other engine, the member's own only when it can't run",
            }
        ));
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
        let change = match Change::parse(&call.arguments) {
            Ok(change) => change,
            Err(why) => {
                let _ = reply.send(ToolResult::error(why));
                return;
            }
        };
        let made = match self.vet(&squad, &change).await {
            Ok(()) => self.command_for(&squad, &change),
            Err(why) => Err(why),
        };
        let command = match made {
            Ok(command) => command,
            Err(why) => {
                let _ = reply.send(ToolResult::error(why));
                return;
            }
        };
        let now = self.mode_now(&squad, &change);
        let action = self.describe_change(&squad, &change);
        let needs_yes = match change {
            Change::SubagentsAtOnce(at_once) => {
                // With no squad override, an increase for either engine widens
                // its parallel work. Codex has the smaller built-in default.
                let current = self.subagents_at_once(Some(&squad)).unwrap_or(crate::protocol::CODEX_SUBAGENTS_AT_ONCE);
                at_once > current
            }
            _ => change.needs_yes(now),
        };
        if !needs_yes {
            crate::daemon_log!("equipe: {squad}'s PM changed a setting the person asked for: {action}");
            Box::pin(self.handle(command)).await;
            self.audit(&squad, None, &call.member, "changed", &action);
            let _ = reply.send(ToolResult::ok(format!("Done: {action}. Tell the person in one line.")));
            return;
        }
        let why = board::arg_str(&call.arguments, "why").map(str::to_owned);
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let tool = if matches!(change, Change::Engine { .. }) { controls::ENGINE_TOOL } else { controls::SETTING_TOOL };
        let approval = Approval { id: approval_id, session: id, member: call.member.clone(), ticket: None, tool: tool.to_owned(), action, reason: why, preview: None };
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
        // The card's first line says what it does; the rest, when each member moves.
        let what = approval.action.lines().next().unwrap_or_default();
        let text = if approval.tool == controls::ENGINE_TOOL {
            format!("The person said yes: {what}. Each one moves as the card said, and you are told as it does. Tell them in one line.")
        } else {
            format!("The person said yes: {what} — done. Tell them in one line.")
        };
        let _ = reply.send(ToolResult::ok(text));
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
            Change::SubagentsAtOnce(at_once) => Command::SetSubagentsAtOnce { squad: squad_name, at_once: Some(at_once) },
            Change::Model { member, model } => Command::SetModel { session: session(&member)?, model },
            Change::Engine { members, engine, model, effort, now } => {
                for name in &members {
                    session(name)?;
                }
                Command::MoveEngine { squad: squad_name, members, engine, model, effort, now }
            }
            Change::Effort { member, effort } => Command::SetEffort { session: session(&member)?, effort },
            Change::EngineSwitching(mode) => Command::SetEngineSwitching { squad: squad_name, mode },
            Change::Parallel { .. } => Command::SetParallel { squad: squad_name, parallel: self.parallel_for(squad, change) },
            Change::ModelChoice(choice) => Command::SetModelChoice { squad: squad_name, choice },
            Change::DesignReviewer(reviewer) => Command::SetDesignReviewer { squad: squad_name, reviewer },
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
            Change::Resting { member, resting } => {
                let id = session(&member)?;
                if resting && self.sessions.get(&id).is_some_and(Session::is_pm) {
                    return Err("You can't rest yourself: the PM stays on duty.".to_owned());
                }
                Command::SetResting { squad: squad_name, member, resting, stop: false }
            }
        })
    }

    /// The squad's parallel work as `change` leaves it: the caps it names,
    /// else those the squad has.
    fn parallel_for(&self, squad: &str, change: &Change) -> crate::protocol::lanes::Parallel {
        let now = self.journals.get(squad).map(Journal::parallel).unwrap_or_default();
        match change {
            Change::Parallel { mode, per_member, per_engine } => {
                crate::protocol::lanes::Parallel { mode: *mode, per_member: per_member.unwrap_or(now.per_member), per_engine: per_engine.unwrap_or(now.per_engine) }.clamped()
            }
            _ => now,
        }
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

    /// A model, an effort or a move checked against what the engine takes —
    /// the member's own, or the one it moves to — before anything is done or
    /// asked: another engine's model in a member's would break it.
    async fn vet(&mut self, squad: &str, change: &Change) -> Result<(), String> {
        match change {
            Change::Model { member, model } => {
                let engine = self.engine_of(squad, member)?;
                let codex = if engine == Engine::Codex || controls::model_fit(engine, model, None) != ModelFit::Fits { Some(self.codex_catalog().await) } else { None };
                match controls::model_fit(engine, model, listed(codex.as_ref())) {
                    ModelFit::Fits => Ok(()),
                    ModelFit::Other(there) => Err(format!(
                        "{model} is a {there_name} model and {member} runs on {here}: `model` only changes a model within the member's engine, and this would break {member}. Nothing changed. To move {member} to {there_name}, use setting `engine` (member {member}, value {code}, model {model}) — the person is asked first.",
                        there_name = engine_name(there),
                        here = engine_name(engine),
                        code = engine_code(there),
                    )),
                    ModelFit::Unknown => Err(format!("{member} runs on {}, which has no model {model:?}. {} Nothing changed.", engine_name(engine), models_line(engine, codex.as_ref()))),
                }
            }
            Change::Effort { member, effort: Some(effort) } => {
                let engine = self.engine_of(squad, member)?;
                let model = self.member_id(squad, member).and_then(|id| self.sessions.get(&id)).and_then(|s| s.model.clone());
                let takes = self.efforts_on(engine, model.as_deref()).await;
                if takes.iter().any(|e| e == effort) {
                    return Ok(());
                }
                Err(match takes.as_slice() {
                    [] => format!("{member}'s model takes no effort setting; nothing changed."),
                    takes => format!("{member}'s model on {} takes effort {}, or default — not {effort:?}. Nothing changed.", engine_name(engine), takes.join(" | ")),
                })
            }
            Change::Engine { members, engine, model, effort, .. } => {
                let mut already = Vec::new();
                for name in members {
                    if self.engine_of(squad, name)? == *engine {
                        already.push(name.as_str());
                    }
                }
                if !already.is_empty() {
                    return Err(format!(
                        "{} already on {}: leave them out of `members` (within an engine, a model is `model` and an effort `effort`). Nothing was asked.",
                        already.join(", "),
                        engine_name(*engine)
                    ));
                }
                if let Some(model) = model {
                    let codex = if *engine == Engine::Codex { Some(self.codex_catalog().await) } else { None };
                    match controls::model_fit(*engine, model, listed(codex.as_ref())) {
                        ModelFit::Fits => {}
                        ModelFit::Other(there) => {
                            return Err(format!(
                                "{model} is a {} model, not {}'s. {} Or leave `model` out for what {} recommends for each one's role. Nothing was asked.",
                                engine_name(there),
                                engine_name(*engine),
                                models_line(*engine, codex.as_ref()),
                                engine_name(*engine)
                            ));
                        }
                        ModelFit::Unknown => return Err(format!("{} has no model {model:?}. {} Nothing was asked.", engine_name(*engine), models_line(*engine, codex.as_ref()))),
                    }
                }
                if let Some(effort) = effort {
                    let takes = self.efforts_on(*engine, model.as_deref()).await;
                    if !takes.iter().any(|e| e == effort) {
                        let takes = if takes.is_empty() { "none".to_owned() } else { takes.join(" | ") };
                        return Err(format!("On {} that model takes effort {takes} — not {effort:?}; or leave `effort` out for its default. Nothing was asked.", engine_name(*engine)));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The engine member `name` of `squad` runs on.
    fn engine_of(&self, squad: &str, name: &str) -> Result<Engine, String> {
        self.member_id(squad, name).and_then(|id| self.sessions.get(&id)).map(|s| s.engine).ok_or_else(|| format!("No member called {name} on the squad."))
    }

    /// The efforts `model` takes on `engine` (`None`: the engine's default
    /// model): Claude Code's by the catalog, Codex's as its own list says.
    async fn efforts_on(&mut self, engine: Engine, model: Option<&str>) -> Vec<String> {
        let ours = || crate::models::efforts(engine, model).iter().map(|e| (*e).to_owned()).collect();
        if engine == Engine::Claude {
            return ours();
        }
        let _ = self.codex_catalog().await;
        let listed = self.codex_models.as_ref().map(|(_, l)| l.as_slice()).unwrap_or_default();
        let info = match model {
            Some(model) => listed.iter().find(|m| m.id == model),
            None => listed.iter().find(|m| m.default).or(listed.first()),
        };
        info.map(|m| m.efforts.clone()).filter(|e| !e.is_empty()).unwrap_or_else(ours)
    }

    /// A model as people name it: Claude's family and version, Codex's own
    /// name for it, a provider's model without the provider.
    fn model_label(&self, model: &str) -> String {
        if crate::model_providers::split_ref(model).is_some() {
            return crate::model_providers::display(model).to_owned();
        }
        if crate::models::is_alias(model)
            && let Some(family) = crate::models::family(model)
        {
            return family.name.to_owned();
        }
        if let Some(name) = crate::models::name(model) {
            return name;
        }
        let listed = self.codex_models.as_ref().and_then(|(_, l)| l.iter().find(|m| m.id == model));
        listed.and_then(|m| m.name.clone()).unwrap_or_else(|| model.to_owned())
    }

    /// The members an engine move takes, as they stand now: for when each one moves.
    fn movers(&self, squad: &str, members: &[String], now: bool) -> Vec<Mover> {
        members
            .iter()
            .filter_map(|name| {
                let id = self.member_id(squad, name)?;
                let s = self.sessions.get(&id)?;
                Some(Mover { name: name.clone(), from: s.engine, working: s.is_working(), on_ticket: !self.runs_of(id).is_empty(), now })
            })
            .collect()
    }

    /// What a change does, in the person's language: the words on its card.
    fn describe_change(&self, squad: &str, change: &Change) -> String {
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
            Change::SubagentsAtOnce(at_once) => say!(self, "control.subagents_at_once", at_once = at_once),
            Change::Model { member, model } => say!(self, "control.model", member = member, model = model),
            Change::Engine { members, engine, model, effort, now } => {
                let model = model.as_deref().map(|m| self.model_label(m));
                controls::move_words(lang, &self.movers(squad, members, *now), *engine, model.as_deref(), effort.as_deref())
            }
            Change::Effort { member, effort } => say!(self, "control.effort", member = member, effort = effort.as_deref().unwrap_or("default")),
            Change::EngineSwitching(m) => say!(self, "control.engine_switching", mode = format!("{m:?}").to_lowercase()),
            Change::ModelChoice(crate::protocol::tiers::ModelChoice::QualityFirst) => say!(self, "control.model_choice_quality"),
            Change::ModelChoice(crate::protocol::tiers::ModelChoice::Off) => say!(self, "control.model_choice_off"),
            Change::Parallel { .. } => {
                let parallel = self.parallel_for(squad, change);
                if parallel.is_max() {
                    say!(self, "control.parallel_max", per_member = parallel.per_member, per_engine = parallel.per_engine)
                } else {
                    say!(self, "control.parallel_auto")
                }
            }
            Change::DesignReviewer(crate::protocol::engines::Reviewer::Auto) => say!(self, "control.design_reviewer_auto"),
            Change::DesignReviewer(crate::protocol::engines::Reviewer::Same) => say!(self, "control.design_reviewer_same"),
            Change::DesignReviewer(crate::protocol::engines::Reviewer::Other) => say!(self, "control.design_reviewer_other"),
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
            Change::Resting { member, resting: true } => say!(self, "control.resting_on", member = member),
            Change::Resting { member, resting: false } => say!(self, "control.resting_off", member = member),
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

/// An engine as `change_setting` takes it.
fn engine_code(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// Codex's list of models, when it was read.
fn listed(codex: Option<&Result<Vec<String>, String>>) -> Option<&[String]> {
    codex.and_then(|c| c.as_ref().ok()).map(Vec::as_slice)
}

/// The models `engine` takes, as one line for the PM: Claude Code's aliases
/// and ids, Codex's own list — or why that list couldn't be read.
fn models_line(engine: Engine, codex: Option<&Result<Vec<String>, String>>) -> String {
    match engine {
        Engine::Claude => {
            let aliases = crate::models::CLAUDE_FAMILIES.iter().map(|f| f.alias);
            let ids = crate::models::CLAUDE_MODELS.iter().map(|p| p.id);
            format!("Claude's models: {}.", aliases.chain(ids).collect::<Vec<_>>().join(", "))
        }
        Engine::Codex => match codex {
            Some(Ok(list)) if !list.is_empty() => format!("Codex's models: {}.", list.join(", ")),
            Some(Err(why)) => format!("Codex's list of models couldn't be read just now ({why}), so it can't be checked; try again in a moment."),
            _ => "Codex lists no models just now; try again in a moment.".to_owned(),
        },
    }
}
