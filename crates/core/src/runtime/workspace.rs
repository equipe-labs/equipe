//! A run's workspace, made off the loop. Starting a run on a ticket that
//! edits needs its worktree: `git worktree add`, a sparse checkout in a squad
//! home, the finished work it builds on merged in. That takes seconds on a
//! busy machine, and on the daemon's core loop it held up every screen
//! (docs/INCIDENT-2026-10-16-daemon-hang.md). So a start goes in two steps:
//!
//! - on the loop, [`Runtime::begin_work`] reserves the ticket — its run's
//!   generation and lease, the ticket in progress for the member, marked as
//!   still starting ([`journal::STARTING`]) — and opens the run's session,
//!   which screens show preparing its workspace and the lane policy counts;
//! - off the loop, a [`Job`] makes the worktree; [`Internal::Workspace`]
//!   brings it back, and [`Runtime::workspace_ready`] starts the run's first
//!   turn — or, when it failed, lets the reservation go and puts the ticket
//!   back as a start that failed always did.
//!
//! Jobs from one folder (a project, a squad home) go one at a time, in the
//! order they came: git takes locks on a repository's config and worktrees,
//! and the board's order is the order runs start in. A completion is fenced
//! by its run's generation: one for a run that ended meanwhile (cancelled,
//! reassigned, its member gone) starts nothing, and a worktree it made for
//! nothing goes. A ticket has one job at a time; it isn't started again
//! until its job is back. A daemon stopped mid-way puts the ticket back on
//! the board when it starts again ([`Journal::release_starting_executions`]).

use std::collections::{HashSet, VecDeque};
use std::time::Instant;

use super::*;

/// What a run's checkout is to be, worked out on the loop as it is reserved.
pub(super) enum Checkout {
    /// In review and sent back: its worktree is still there.
    Kept { path: PathBuf, branch: String, base: String },
    /// Its worktree went away: its branch checked out again.
    Resume { branch: String, base: String },
    /// New: from HEAD, with the finished work it depends on (`(T-n, branch)`).
    New { label: String, after: Vec<(String, String)> },
}

/// A run reserved on the loop, waiting for its workspace: what its start
/// goes on with once that is there.
#[derive(Clone)]
pub(super) struct Start {
    pub squad: String,
    pub name: String,
    pub ticket: TicketId,
    /// The run's session, which is its runtime session.
    pub run: SessionId,
    pub generation: u64,
    pub project: PathBuf,
    /// The finished work its branch starts from.
    pub after: Vec<(String, String)>,
    /// Its branch is cut now (not checked out again).
    pub fresh: bool,
    /// A recovery the person approved.
    pub recovered: bool,
    /// The reopen envelope's note, when it was sent back.
    pub reopened: Option<String>,
    /// Why it starts in a fresh conversation, when its model changed.
    pub handoff: Option<String>,
    pub board_tools: bool,
    /// The ticket's own engine conversation, to carry on.
    pub engine_session: Option<String>,
    /// Who the ticket named before the reservation: given back when the start fails.
    pub assignee_before: Option<String>,
    pub began: Instant,
}

/// Making one run's workspace, off the loop.
pub(super) struct Job {
    pub start: Start,
    /// Where its worktree comes from: the ticket's project, or the squad home.
    pub workplace: PathBuf,
    /// `None`: it doesn't edit, and has no worktree.
    pub checkout: Option<Checkout>,
    /// The PM, on a code ticket of its own, edits only when the project is a
    /// repository: on a plain folder it reads, with no worktree.
    pub only_in_repo: bool,
    /// Sparse-checkout patterns ([`worktree::home_patterns`]).
    pub only: Vec<String>,
    pub home: PathBuf,
    /// The worktree's folder and branch suffix: `t<ticket>-<time>-<generation>`.
    pub id_part: String,
    /// A relative plan's repository, whose main branch its prompt names.
    pub plan_repo: Option<PathBuf>,
}

/// What a [`Job`] made.
pub(super) struct Prepared {
    pub start: Start,
    /// Its checkout, `None` where it has none, or why it couldn't be made.
    pub sandbox: Result<Option<Sandbox>, String>,
    /// What the job made, for a completion that comes too late: `Some(true)`
    /// a new branch and its worktree, `Some(false)` a worktree for a branch
    /// that was there already; `None` nothing.
    pub made: Option<bool>,
    /// The plan's repository's main branch.
    pub main: Option<String>,
}

/// One thing done with a folder's worktrees, off the loop.
pub(super) enum Task {
    Prepare(Box<Job>),
    /// A worktree made for a run that ended before it was ready: removed,
    /// with its branch when the job cut it.
    Discard { sandbox: Sandbox, branch_too: bool },
}

/// The workspaces being made: the tickets waiting on one, and, per folder,
/// the task under way and those queued behind it.
#[derive(Default)]
pub(super) struct Workspaces {
    busy: HashSet<(String, TicketId)>,
    running: HashSet<PathBuf>,
    queued: HashMap<PathBuf, VecDeque<Task>>,
}

impl Workspaces {
    /// Whether `ticket`'s workspace is still being made (or a job for it is
    /// still out): it isn't started again until that is back.
    pub(super) fn busy(&self, squad: &str, ticket: TicketId) -> bool {
        self.busy.contains(&(squad.to_owned(), ticket))
    }
}

/// Make `job`'s workspace. Runs git: off the loop. A git that hangs is
/// stopped at its time ([`worktree::WORKTREE_ADD_TIMEOUT`],
/// [`worktree::GIT_TIMEOUT`]), which fails the start.
fn prepare(job: Job) -> Prepared {
    let Job { start, workplace, checkout, only_in_repo, only, home, id_part, plan_repo } = job;
    let mut made = None;
    let sandbox = match checkout {
        None => Ok(None),
        Some(checkout) => match worktree::repo_root(&workplace) {
            Err(_) if only_in_repo => Ok(None),
            Err(err) => Err(err),
            Ok(repo) => match checkout {
                Checkout::Kept { path, branch, base } => Ok(Some(Sandbox { repo, path, branch, base })),
                Checkout::Resume { branch, base } => {
                    made = Some(false);
                    worktree::resume(&repo, &home, &id_part, &branch, &base, &only).map(Some)
                }
                Checkout::New { label, after } => {
                    made = Some(true);
                    worktree::create_after(&repo, &home, &id_part, &label, &after, &only).map(Some)
                }
            },
        },
    };
    let made = made.filter(|_| matches!(sandbox, Ok(Some(_))));
    let main = plan_repo.and_then(|repo| worktree::main_branch(&repo).ok());
    Prepared { start, sandbox, made, main }
}

impl Runtime {
    /// Make `job`'s workspace off the loop, after what its folder has under way.
    pub(super) fn prepare_workspace(&mut self, job: Job) {
        self.workspaces.busy.insert((job.start.squad.clone(), job.start.ticket));
        let folder = job.workplace.clone();
        self.workspaces.queued.entry(folder.clone()).or_default().push_back(Task::Prepare(Box::new(job)));
        self.next_workspace_task(&folder);
    }

    /// Start `folder`'s next task, unless one is under way.
    fn next_workspace_task(&mut self, folder: &Path) {
        if self.workspaces.running.contains(folder) {
            return;
        }
        let Some(task) = self.workspaces.queued.get_mut(folder).and_then(VecDeque::pop_front) else {
            self.workspaces.queued.remove(folder);
            return;
        };
        self.workspaces.running.insert(folder.to_owned());
        let (internal, folder) = (self.internal.clone(), folder.to_owned());
        tokio::task::spawn_blocking(move || {
            let prepared = match task {
                Task::Prepare(job) => {
                    // A panic must not leave the run preparing for ever, nor the folder's queue stuck.
                    let start = job.start.clone();
                    Some(Box::new(std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || prepare(*job))).unwrap_or_else(|_| Prepared {
                        start,
                        sandbox: Err("making its worktree failed unexpectedly".to_owned()),
                        made: None,
                        main: None,
                    })))
                }
                Task::Discard { sandbox, branch_too } => {
                    let gone = if branch_too { sandbox.discard() } else { sandbox.remove_worktree() };
                    if let Err(err) = gone {
                        crate::daemon_log!("equipe: {} wasn't removed: {err}", sandbox.path.display());
                    }
                    None
                }
            };
            let _ = internal.send(Internal::Workspace { folder, prepared });
        });
    }

    /// A task on `folder`'s worktrees ended: the run it prepared starts, and
    /// the folder's next task goes.
    pub(super) async fn workspace_done(&mut self, folder: PathBuf, prepared: Option<Box<Prepared>>) {
        self.workspaces.running.remove(&folder);
        if let Some(prepared) = prepared {
            Box::pin(self.workspace_ready(&folder, *prepared)).await;
        }
        self.next_workspace_task(&folder);
    }

    /// A run's workspace is ready, or couldn't be made. Only the run that
    /// still holds its ticket's current generation, as reserved, goes on:
    /// it starts its first turn, or — the workspace failed — lets go of its
    /// reservation and its ticket goes back as a start that failed always
    /// did. Anything else came too late: the run ended meanwhile, and a
    /// worktree made for it goes.
    async fn workspace_ready(&mut self, folder: &Path, prepared: Prepared) {
        let Prepared { start, sandbox, made, main } = prepared;
        let squad = start.squad.clone();
        self.workspaces.busy.remove(&(squad.clone(), start.ticket));
        let took = start.began.elapsed().as_secs_f64();
        let held = self.sessions.get(&start.run).and_then(|s| s.work.as_ref()).is_some_and(|w| w.preparing && w.ticket == start.ticket && w.execution_generation == start.generation);
        let current = self.journal(&squad).and_then(|j| j.execution(start.ticket).ok().flatten()).is_some_and(|e| {
            e.state == ExecutionState::Active && e.generation == start.generation && e.session == start.run
        });
        if !held || !current {
            crate::daemon_log!(
                "equipe: {squad}: T-{}'s workspace for {} came after {took:.1}s, for a run (generation {}) that ended meanwhile; nothing starts",
                start.ticket,
                start.name,
                start.generation
            );
            if let (Ok(Some(sandbox)), Some(branch_too)) = (sandbox, made) {
                self.workspaces.queued.entry(folder.to_owned()).or_default().push_back(Task::Discard { sandbox, branch_too });
                self.next_workspace_task(folder);
            }
            // What waited for this ticket's job may go now.
            Box::pin(self.dispatch(&squad)).await;
            return;
        }
        match sandbox {
            Ok(sandbox) => {
                let what = match (&sandbox, made) {
                    (None, _) => "no worktree",
                    (Some(_), Some(true)) => "a new worktree",
                    (Some(_), Some(false)) => "its branch checked out again",
                    (Some(_), None) => "its worktree as it was",
                };
                crate::daemon_log!("equipe: {squad}: T-{}'s workspace for {} ready in {took:.1}s ({what})", start.ticket, start.name);
                // Its dependencies cloned from a checkout that has them installed, off the loop.
                if let (Some(sandbox), Some(_)) = (&sandbox, made) {
                    worktree::seed_in_background(sandbox.path.clone());
                }
                self.start_run(start, sandbox, main);
            }
            Err(reason) => {
                crate::daemon_log!("equipe: {squad}: T-{}'s workspace for {} failed after {took:.1}s: {reason}", start.ticket, start.name);
                self.start_failed(start, &reason);
                Box::pin(self.dispatch(&squad)).await;
            }
        }
    }

    /// The reserved run couldn't get its workspace: its reservation goes —
    /// the run stopped, its session closed — and the ticket goes back as a
    /// start that failed always did ([`Runtime::cannot_start`]).
    fn start_failed(&mut self, start: Start, reason: &str) {
        let Start { squad, name, ticket, run, generation, assignee_before, .. } = start;
        if let Some(journal) = self.journal(&squad) {
            let _ = journal.finish_execution(ticket, generation, ExecutionState::Stopped, Some(COULD_NOT_START_REASON));
        }
        if let Some(session) = self.sessions.get_mut(&run) {
            session.work = None;
        }
        let of = self.sessions.get(&run).and_then(|s| s.run_of);
        self.publish(Event::WorkEnded { session: run, ticket, of });
        self.close_run(run);
        let Some(mut ticket) = self.journal(&squad).and_then(|j| j.ticket(ticket).ok().flatten()) else { return };
        ticket.assignee = assignee_before;
        self.cannot_start(&squad, ticket, &name, reason);
    }
}

/// The reason a run that couldn't get its workspace stopped with.
pub(super) const COULD_NOT_START_REASON: &str = "could_not_start";
