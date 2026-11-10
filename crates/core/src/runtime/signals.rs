//! The squad's signals (`crate::signals`): each minute, those due run off
//! the loop inside their fence, in their project's folder; each reading is
//! kept, and what it means told to the PM — a first failure wakes it, a
//! recovery or a big move goes with its next message. The PM proposes a
//! signal on a card (like `ask_person`, its exact command shown); only the
//! person's yes keeps it.

use std::collections::BTreeMap;

use super::*;
use crate::signals::{self as rules, Alert, Change, Ran, Reading, Signal, State};

impl Runtime {
    fn signals(&mut self, squad: &str) -> Vec<Signal> {
        self.journal(squad).map(Journal::signals).unwrap_or_default()
    }

    fn signal_states(&mut self, squad: &str) -> BTreeMap<String, State> {
        self.journal(squad).map(Journal::signal_states).unwrap_or_default()
    }

    /// The squad's signals as screens show them.
    pub(super) fn signal_views(&mut self, squad: &str) -> Vec<equipe_protocol::signals::SignalView> {
        let states = self.signal_states(squad);
        self.signals(squad).iter().map(|s| s.view(states.get(&s.key()).unwrap_or(&State::default()))).collect()
    }

    fn publish_signals(&mut self, squad: &str) {
        let signals = self.signal_views(squad);
        self.publish(Event::SignalsChanged { squad: squad.to_owned(), signals });
    }

    /// The person pauses a signal or starts it again; started, it runs now.
    pub(super) fn set_signal_on(&mut self, squad: &str, project: &str, name: &str, on: bool) {
        let mut kept = self.signals(squad);
        if !rules::set_on(&mut kept, project, name, on) {
            let text = say!(self, "error.signal_missing", project = project, name = name);
            return self.squad_error(squad, text);
        }
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_signals(&kept)) {
            crate::daemon_log!("equipe: {squad}: the signals weren't kept: {err}");
        }
        crate::daemon_log!("equipe: {squad}: signal {project}/{name} {}", if on { "started again" } else { "paused" });
        if on && let Some(signal) = kept.into_iter().find(|s| s.project == project && s.name == name) {
            self.run_signal(squad, signal);
        }
        self.publish_signals(squad);
    }

    /// Each minute: every squad's signals that are due, run.
    pub(super) fn run_due_signals(&mut self) {
        let now = now_secs();
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        for squad in squads {
            let states = self.signal_states(&squad);
            for signal in self.signals(&squad).into_iter().filter(|s| s.on) {
                let last = states.get(&signal.key()).and_then(|s| s.readings.last()).map(|r| r.at);
                if rules::due(&signal, last, now) {
                    self.run_signal(&squad, signal);
                }
            }
        }
    }

    /// Run one signal off the loop, unless it runs already; its reading comes
    /// back as [`Internal::SignalRead`].
    fn run_signal(&mut self, squad: &str, signal: Signal) {
        let key = signal.key();
        if !self.signals_running.insert((squad.to_owned(), key.clone())) {
            return;
        }
        let project = self.projects(squad).into_iter().find(|p| board::project_name(p) == signal.project);
        // What no member's fenced command reads, nor this one.
        let private: Vec<PathBuf> = [self.config.home.join("browser"), self.config.home.join("credentials"), equipe_desktop_control::helper::dir(&self.config.home)]
            .into_iter()
            .chain(squad_home::others(&self.config.home, squad))
            .collect();
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        let (client, home) = (self.client.clone(), self.person_home());
        tokio::spawn(async move {
            let ran = match (project, &signal.source, home) {
                (None, ..) => Ran { failed: Some(format!("the squad has no project \"{}\" any more", signal.project)), ..Ran::default() },
                (Some(dir), rules::Source::Command, _) => run_fenced(&signal.command, &dir, &private).await,
                (Some(_), source, Some(home)) => crate::signal_sources::read(&client, source, &home, now_secs()).await,
                (Some(_), _, None) => Ran { failed: Some("no home folder to read the keys under".into()), ..Ran::default() },
            };
            let reading = rules::judge(&signal.expect, &ran, now_secs());
            let _ = internal.send(Internal::SignalRead { squad, key, reading });
        });
    }

    /// A reading back: kept, and what it means told to the PM.
    pub(super) async fn signal_read(&mut self, squad: &str, key: &str, reading: Reading) {
        self.signals_running.remove(&(squad.to_owned(), key.to_owned()));
        // Turned off while it ran: nothing to keep.
        let Some(signal) = self.signals(squad).into_iter().find(|s| s.key() == key) else { return };
        let mut states = self.signal_states(squad);
        let state = states.entry(key.to_owned()).or_default();
        let passed_before = state.readings.iter().rev().find(|r| r.ok).map(|r| r.at);
        let pm = self.pm_of(squad);
        let alert = rules::step(&signal, state, reading.clone(), pm.is_some());
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_signal_states(&states)) {
            crate::daemon_log!("equipe: {squad}: signal {key}'s reading wasn't kept: {err}");
        }
        self.publish_signals(squad);
        let (name, project) = (signal.name.clone(), signal.project.clone());
        match alert {
            Alert::Quiet => {}
            Alert::Failing => {
                let Some(pm) = pm else { return };
                crate::daemon_log!("equipe: {squad}: signal {key} fails; the PM is woken");
                let note = reading.note.clone().unwrap_or_default();
                let shown = say!(self, "news.signal_failing", name = name, project = project, note = note);
                self.show_news(pm, shown);
                let text = rules::failing_message(&signal, &reading, passed_before);
                self.said_about = Some(Some(project));
                match self.sessions.get(&pm) {
                    Some(s) if s.is_working() => self.leave_memo(pm, text),
                    _ => self.submit(pm, text).await,
                }
                self.said_about = None;
            }
            Alert::Recovered => {
                let shown = say!(self, "news.signal_recovered", name = name, project = project);
                self.brief_pm(squad, rules::recovered_line(&signal, &reading), Some(shown));
            }
            Alert::Changed { pct, from } => {
                let to = reading.value.clone().unwrap_or_default();
                let shown = say!(self, "news.signal_changed", name = name, project = project, from = from, to = to, pct = rules::pct_text(pct));
                self.brief_pm(squad, rules::changed_line(&signal, &from, &reading, pct), Some(shown));
            }
        }
    }

    /// The `signals` tool: the PM reads them, or runs them now.
    pub(super) fn read_signals(&mut self, squad: &str, id: SessionId, args: &Value) -> Result<String, String> {
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            return Err("Only the PM reads the signals. Ask the PM what they say.".into());
        }
        let only = board::arg_str(args, "name").map(str::trim).filter(|n| !n.is_empty());
        let signals = self.signals(squad);
        if args.get("check").and_then(Value::as_bool) == Some(true) {
            let run: Vec<Signal> = signals.into_iter().filter(|s| only.is_none_or(|n| s.name == n)).collect();
            if run.is_empty() {
                return Err(only.map_or_else(|| "The squad has no signals to run.".into(), |n| format!("No signal is called \"{n}\".")));
            }
            let names: Vec<String> = run.iter().map(|s| s.name.clone()).collect();
            for signal in run {
                self.run_signal(squad, signal);
            }
            return Ok(format!(
                "Running {} now. What fails or moves reaches you as a briefing; call `signals` again in a minute for the values.",
                names.join(", ")
            ));
        }
        let states = self.signal_states(squad);
        Ok(rules::report(&signals, |s| states.get(&s.key()).cloned().unwrap_or_default(), only))
    }

    /// `propose_signal`: checked, then put to the person on a card that waits
    /// for their answer, as `ask_person` does.
    pub(super) fn propose_signal(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let refuse = |text: String| ToolResult { text, is_error: true, image: None };
        let Some(id) = self.member_id(&call.squad, &call.member) else {
            let _ = reply.send(refuse("You're not on the squad.".into()));
            return;
        };
        if !self.sessions.get(&id).is_some_and(Session::is_pm) {
            let _ = reply.send(refuse("Only the PM proposes signals. Tell the PM what should be watched.".into()));
            return;
        }
        let kept = self.signals(&call.squad);
        let projects: Vec<String> = self.projects(&call.squad).iter().map(|p| board::project_name(p)).collect();
        let home = self.person_home().unwrap_or_default();
        let (change, reason) = match rules::check(&call.arguments, &kept, &projects, &home, now_secs()) {
            Ok(checked) => checked,
            Err(why) => {
                let _ = reply.send(refuse(why));
                return;
            }
        };
        let action = match &change {
            Change::On { signal, replaces } => {
                let every = rules::every_text(signal.every);
                let (name, project, command, expect) = (&signal.name, &signal.project, &signal.what_with_keys(), signal.expectation());
                if *replaces {
                    say!(self, "card.signal_change", name = name, project = project, command = command, expect = expect, every = every)
                } else {
                    say!(self, "card.signal_on", name = name, project = project, command = command, expect = expect, every = every)
                }
            }
            Change::Off { project, name } => say!(self, "card.signal_off", name = name, project = project),
        };
        let ticket = self.sessions.get(&id).and_then(|s| s.work.as_ref().map(|w| w.ticket));
        let approval_id = self.next_approval;
        self.next_approval += 1;
        let approval = Approval {
            id: approval_id,
            session: id,
            member: call.member.clone(),
            ticket,
            tool: CONFIRM.to_owned(),
            action,
            reason: (!reason.is_empty()).then_some(reason),
            preview: None,
        };
        let input = rules::card_input(&change);
        self.pending.insert(approval_id, Pending { squad: call.squad.clone(), approval: approval.clone(), input, reply });
        self.publish(Event::ApprovalRequested { squad: call.squad.clone(), approval });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_WAIT).await;
            let _ = internal.send(Internal::ApprovalTimedOut(approval_id));
        });
    }

    /// The person's answer to a signal's card: on a yes it is kept (and run
    /// at once). `None`: the card wasn't a signal's.
    pub(super) fn signal_answered(&mut self, squad: &str, input: &Value, yes: bool) -> Option<String> {
        let change = rules::card_change(input)?;
        if !yes {
            return Some("The person said no: the signal is not changed. Don't propose it again unless they ask.".into());
        }
        let mut kept = self.signals(squad);
        rules::apply(&mut kept, &change);
        if let Some(Err(err)) = self.journal(squad).map(|j| j.set_signals(&kept)) {
            crate::daemon_log!("equipe: {squad}: the signals weren't kept: {err}");
            return Some(format!("The person said yes, but equipe couldn't keep it: {err}."));
        }
        let mut states = self.signal_states(squad);
        Some(match change {
            Change::On { signal, .. } => {
                // A changed signal starts its readings afresh: the old ones measured something else.
                states.remove(&signal.key());
                let _ = self.journal(squad).map(|j| j.set_signal_states(&states));
                let text = format!(
                    "The person turned on the signal \"{}\" of {}: it runs every {}, first now. A failure wakes you; \
`signals` shows its readings.",
                    signal.name,
                    signal.project,
                    rules::every_text(signal.every)
                );
                self.run_signal(squad, signal);
                self.publish_signals(squad);
                text
            }
            Change::Off { project, name } => {
                states.remove(&rules::key(&project, &name));
                let _ = self.journal(squad).map(|j| j.set_signal_states(&states));
                self.publish_signals(squad);
                format!("The person turned off the signal \"{name}\" of {project}.")
            }
        })
    }
}

/// Run `command` with `/bin/sh -c` in `dir`, fenced: writing only to a
/// scratch folder of its own, reading none of `private`, stopped after
/// [`rules::TIMEOUT_SECS`]. Only where the fence exists (macOS).
async fn run_fenced(command: &str, dir: &Path, private: &[PathBuf]) -> Ran {
    if !cfg!(target_os = "macos") || !Path::new(SANDBOX_EXEC).exists() {
        return Ran { failed: Some("signals run only on macOS, inside its sandbox".into()), ..Ran::default() };
    }
    let scratch = std::env::temp_dir().join(format!("equipe-signal-{}", equipe_protocol::new_session_id()));
    if let Err(err) = std::fs::create_dir_all(&scratch) {
        return Ran { failed: Some(format!("no scratch folder: {err}")), ..Ran::default() };
    }
    let scratch = scratch.canonicalize().unwrap_or(scratch);
    let profile = rules::fence_profile(&scratch, private);
    let mut cmd = tokio::process::Command::new(SANDBOX_EXEC);
    cmd.args(["-p", &profile, "/bin/sh", "-c", command])
        .current_dir(dir)
        .env("TMPDIR", &scratch)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let run = tokio::time::timeout(Duration::from_secs(rules::TIMEOUT_SECS), cmd.output()).await;
    let _ = std::fs::remove_dir_all(&scratch);
    match run {
        Ok(Ok(out)) => Ran {
            exit: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            failed: None,
        },
        Ok(Err(err)) => Ran { failed: Some(format!("it couldn't start: {err}")), ..Ran::default() },
        Err(_) => Ran { failed: Some(format!("timed out after {}s", rules::TIMEOUT_SECS)), ..Ran::default() },
    }
}

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
