//! Members' commands, as the daemon watches them run ([`crate::command_watch`]).
//!
//! Each turn's engine says its process as it starts; each command step
//! (`Bash`, Claude Code's or Codex's) is followed from its start to its
//! answer. While one is open, every few seconds (the desktop tick) one `ps`
//! runs off the loop for every member at once, and each member's screens are
//! told what runs for its command ([`Event::CommandWatch`]) — and, once the
//! step is answered or the turn ends, that nothing does any more. Stopping a
//! command ([`Command::StopCommand`]) looks again first, off the loop, and
//! ends only what runs for it under that member's engine.

use super::*;
use crate::command_watch::{self, Proc, Seen, Watcher};

/// How long a look may take before it is given up.
const PS_WAIT: Duration = Duration::from_secs(10);
/// How long a stopped command has to end before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// The commands the daemon watches, by session.
#[derive(Default)]
pub(super) struct Commands {
    by: HashMap<SessionId, Watched>,
    /// A look under way, so they never pile up.
    looking: bool,
}

/// One session's turn, as watched.
struct Watched {
    engine: Engine,
    /// The engine's process, once it said it.
    pid: Option<u32>,
    /// When the engine last said anything.
    heard: std::time::Instant,
    /// Its command steps not answered yet: each command's first line, and
    /// since when it has run.
    open: Vec<(Option<String>, std::time::Instant)>,
    watcher: Option<Watcher>,
    /// The screens were told of a command under way, and not yet that it ended.
    told: bool,
}

impl Watched {
    fn new(engine: Engine) -> Self {
        Self { engine, pid: None, heard: std::time::Instant::now(), open: Vec::new(), watcher: None, told: false }
    }

    /// The commands' lines, and how long the oldest has run.
    fn steps(&self, now: std::time::Instant) -> (Vec<String>, Duration) {
        let targets = self.open.iter().filter_map(|(t, _)| t.clone()).collect();
        let age = self.open.iter().map(|(_, since)| now.saturating_duration_since(*since)).max().unwrap_or_default();
        (targets, age)
    }
}

/// Whether a step is a command the watch follows: a shell command, run to
/// its answer (one left in the background runs on by design).
fn is_command(tool: &str) -> bool {
    tool == "Bash"
}

impl Runtime {
    /// A turn's engine started, as process `pid`.
    pub(super) fn engine_started(&mut self, id: SessionId, pid: u32) {
        let Some(engine) = self.sessions.get(&id).map(|s| s.engine) else { return };
        let watched = self.commands.by.entry(id).or_insert_with(|| Watched::new(engine));
        watched.engine = engine;
        watched.pid = Some(pid);
        watched.heard = std::time::Instant::now();
    }

    /// The engine of `id` said something: it isn't silent.
    pub(super) fn engine_heard(&mut self, id: SessionId) {
        if let Some(watched) = self.commands.by.get_mut(&id) {
            watched.heard = std::time::Instant::now();
        }
    }

    /// Follow command steps as they start and end, and turns as they end.
    pub(super) fn follow_commands(&mut self, event: &Event) {
        match event {
            Event::ToolUse { session, tool, target, background: false, .. } if is_command(tool) => {
                if let Some(watched) = self.commands.by.get_mut(session) {
                    if watched.open.is_empty() {
                        watched.watcher = Some(Watcher::new(watched.engine));
                    }
                    watched.open.push((target.clone(), std::time::Instant::now()));
                }
            }
            Event::ToolResult { session, tool, target, .. } if is_command(tool) => {
                let ended = self.commands.by.get_mut(session).is_some_and(|watched| {
                    if let Some(at) = watched.open.iter().position(|(t, _)| t == target) {
                        watched.open.remove(at);
                    }
                    watched.open.is_empty()
                });
                if ended {
                    self.command_ended(*session);
                }
            }
            Event::TurnEnded { session, .. } | Event::SessionClosed { session } => {
                self.command_ended(*session);
                self.commands.by.remove(session);
            }
            _ => {}
        }
    }

    /// No command of `id`'s runs any more: its screens hear so, once.
    fn command_ended(&mut self, id: SessionId) {
        let Some(watched) = self.commands.by.get_mut(&id) else { return };
        watched.open.clear();
        watched.watcher = None;
        if std::mem::take(&mut watched.told) {
            self.publish(Event::CommandWatch { session: id, watch: None });
        }
    }

    /// Look at what runs under the engines with a command open: one `ps`, off the loop.
    pub(super) fn look_at_commands(&mut self) {
        if self.commands.looking || !self.commands.by.values().any(|w| w.pid.is_some() && !w.open.is_empty()) {
            return;
        }
        self.commands.looking = true;
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let procs = list_processes().await;
            let _ = internal.send(Internal::Processes(procs));
        });
    }

    /// What a look found: each command's screens told what runs for it.
    pub(super) fn on_processes(&mut self, procs: Option<Vec<Proc>>) {
        self.commands.looking = false;
        let Some(procs) = procs else { return };
        let now = std::time::Instant::now();
        let ids: Vec<SessionId> = self.commands.by.keys().copied().collect();
        for id in ids {
            let asking = self.pending.values().any(|p| p.approval.session == id);
            let Some(watched) = self.commands.by.get_mut(&id) else { continue };
            let (Some(pid), false) = (watched.pid, watched.open.is_empty()) else { continue };
            let (targets, step_age) = watched.steps(now);
            let quiet = now.saturating_duration_since(watched.heard);
            let seen = Seen { procs: &procs, engine_pid: pid, step_age, targets: &targets, quiet, asking };
            let engine = watched.engine;
            let watch = watched.watcher.get_or_insert_with(|| Watcher::new(engine)).look(&seen, now);
            watched.told = true;
            self.publish(Event::CommandWatch { session: id, watch: Some(watch) });
        }
    }

    /// Whether `id`'s engine runs a command of the member's now.
    pub(super) fn command_running(&self, id: SessionId) -> bool {
        self.commands.by.get(&id).is_some_and(|w| w.pid.is_some() && !w.open.is_empty())
    }

    /// Stop the command `id`'s engine runs now: what runs for it ends, off
    /// the loop — asked nicely first, then killed what didn't. A process of it
    /// that leads a group of its own ends with its group: what it started
    /// and let go of is the command's too.
    pub(super) fn stop_command(&mut self, id: SessionId) {
        let now = std::time::Instant::now();
        let Some(watched) = self.commands.by.get(&id) else { return };
        let (Some(pid), false) = (watched.pid, watched.open.is_empty()) else { return };
        let (targets, step_age) = watched.steps(now);
        let who = self.sessions.get(&id).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
        tokio::spawn(async move {
            let Some(procs) = list_processes().await else { return };
            let stopping = command_watch::to_stop(&procs, pid, step_age, &targets);
            if stopping.is_empty() {
                return;
            }
            let names: Vec<String> = stopping.iter().map(Proc::name).collect();
            crate::daemon_log!("stopping {who}'s command at the person's word: {}", names.join(", "));
            for p in &stopping {
                crate::group::end(p.pid, libc::SIGTERM);
            }
            tokio::time::sleep(STOP_GRACE).await;
            let Some(after) = list_processes().await else { return };
            for pid in command_watch::survivors(&stopping, &after) {
                crate::group::end(pid, libc::SIGKILL);
            }
        });
    }
}

/// Every process, as `ps` lists it; `None` when it couldn't be read.
async fn list_processes() -> Option<Vec<Proc>> {
    let output = tokio::process::Command::new("ps")
        .args(command_watch::PS_ARGS)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(PS_WAIT, output).await.ok()?.ok()?;
    output.status.success().then(|| command_watch::parse_ps(&String::from_utf8_lossy(&output.stdout)))
}
