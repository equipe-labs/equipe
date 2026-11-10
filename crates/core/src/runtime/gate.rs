//! Accepted code checked before it lands: when the PM accepts a code ticket
//! (`accept_work`) and its project declares checks (`crate::checks`), the
//! branch merged with main is worked out, the checks run on exactly that
//! merge off the loop, and only a pass accepts the ticket and lands that
//! commit on main. A failure sends the work back to its member with the
//! failing command's output; main is never touched. One run at a time per
//! repository: each lands on the main the next one is worked out on.

use super::*;
use crate::checks::{self, Outcome};

/// The build folder (`CARGO_TARGET_DIR`) the checks of a squad's work use,
/// beside its members' (`journal::build_dir`), kept between runs.
pub(super) const CHECKS_BUILDS: &str = "_checks";

/// How many times checks run again because main moved on meanwhile.
const TRIES: u32 = 3;

/// Accepted work waiting for, or under, its project's checks.
#[derive(Debug, Clone)]
pub(super) struct Gate {
    pub squad: String,
    pub ticket: TicketId,
    pub title: String,
    /// The PM accepting it, and its conversation.
    pub by: String,
    pub session: SessionId,
    /// What the PM said it checked.
    pub checked: String,
    pub push: bool,
    pub repo: PathBuf,
    pub branch: String,
    /// Runs so far that main moved on under.
    pub tries: u32,
}

impl Runtime {
    /// Whether `ticket`'s checks are waiting or running.
    pub(super) fn gating(&self, squad: &str, ticket: TicketId) -> bool {
        self.gates.values().flatten().any(|g| g.squad == squad && g.ticket == ticket)
    }

    /// The build folders checks are building in now, to be left alone.
    pub(super) fn gate_builds(&self) -> Vec<PathBuf> {
        self.gates.values().filter_map(|q| q.front()).map(|g| journal::build_dir(&self.config.home, &g.squad, CHECKS_BUILDS)).collect()
    }

    /// Put `gate` in line for its repository's checks; run now if nothing is ahead.
    pub(super) fn queue_gate(&mut self, gate: Gate) {
        let repo = gate.repo.clone();
        let queue = self.gates.entry(repo.clone()).or_default();
        queue.push_back(gate);
        if queue.len() == 1 {
            self.run_gate(&repo);
        }
    }

    /// The first in line for `repo`'s checks, run off the loop.
    fn run_gate(&mut self, repo: &Path) {
        let Some(gate) = self.gates.get(repo).and_then(|q| q.front()).cloned() else {
            return;
        };
        let scratch = checks_scratch(&self.config.home);
        let builds = journal::build_dir(&self.config.home, &gate.squad, CHECKS_BUILDS);
        let mut env = vec![("CARGO_TARGET_DIR".to_owned(), builds.to_string_lossy().into_owned()), ("CARGO_INCREMENTAL".to_owned(), "0".to_owned())];
        if let Some(debug) = self.journals.get(&gate.squad).map(Journal::build_debug).and_then(|d| d.cargo_value()) {
            env.push(("CARGO_PROFILE_DEV_DEBUG".to_owned(), debug.to_owned()));
        }
        let message = format!("equipe: merge T-{} {}\n\nAccepted by the PM; merged from {}.", gate.ticket, gate.title, gate.branch);
        let internal = self.internal.clone();
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::create_dir_all(&builds);
            let label = format!("t{}", gate.ticket);
            let outcome = checks::Run { repo: &gate.repo, branch: &gate.branch, message: &message, scratch: &scratch, label: &label, env: &env }.run();
            let _ = internal.send(Internal::Checked(Box::new(gate), outcome));
        });
    }

    /// A run of checks ended: the next in line starts, and the work is
    /// accepted and landed, sent back, or left in review, as it went.
    pub(super) async fn on_checked(&mut self, gate: Gate, outcome: Outcome) {
        if let Some(queue) = self.gates.get_mut(&gate.repo) {
            queue.pop_front();
            if queue.is_empty() {
                self.gates.remove(&gate.repo);
            } else {
                let repo = gate.repo.clone();
                self.run_gate(&repo);
            }
        }
        let (squad, ticket) = (gate.squad.clone(), gate.ticket);
        let Some(t) = self.find_ticket(&squad, ticket) else { return };
        if t.status != TicketStatus::Review || self.worker_on(&squad, ticket).is_some() {
            let line = format!("T-{ticket} changed while its checks ran (it is {} now): the checks merged nothing.", t.status.label());
            return self.brief_pm(&squad, line, None);
        }
        let note = |rt: &mut Self, kind: &str, text: &str| {
            if let Some(journal) = rt.journal(&squad) {
                let _ = journal.ticket_event(ticket, "equipe", kind, text);
            }
        };
        match outcome {
            Outcome::Nothing { .. } => self.accept_checked(&gate, None, None).await,
            Outcome::Passed { merge, ran } => self.accept_checked(&gate, Some(merge), Some(ran)).await,
            // Its checks went from main meanwhile: merged as any project without them.
            Outcome::Undeclared { merge } => self.accept_checked(&gate, Some(merge), None).await,
            Outcome::Failed { ref command, .. } | Outcome::TimedOut { ref command, .. } => {
                let command = command.clone();
                let reason = checks::failed_reason(&outcome).unwrap_or_default();
                note(self, "checks failed", &command);
                match self.send_back(&squad, ticket, "equipe", &reason, equipe_protocol::tiers::SendBackReason::Correctness, false) {
                    Ok(who) => {
                        let shown = say!(self, "news.checks_sent_back", id = ticket, title = t.title, name = who, command = command);
                        let line = format!(
                            "equipe ran the project's checks on T-{ticket} merged with main before merging it: `{command}` failed. It went back to {who} \
with the output and what to do; main is untouched. Nothing to do about it but carry on."
                        );
                        self.brief_pm(&squad, line, Some(shown));
                    }
                    Err(why) => {
                        let line = format!("T-{ticket} failed the project's checks (`{command}`) and wasn't merged, but it couldn't go back to its member: {why}");
                        self.brief_pm(&squad, line, None);
                    }
                }
            }
            Outcome::MainFails { command, tail } => {
                note(self, "not merged", &format!("main fails `{command}` without it"));
                let message = say!(self, "error.checks_main_fails", id = ticket, command = command);
                self.squad_error(&squad, message);
                let line = format!(
                    "equipe ran the project's checks on T-{ticket} merged with main before merging it: `{command}` failed — and fails on main \
alone too, so main is broken already, not by T-{ticket}. T-{ticket} stays in review, not merged. Main's output ended:\n```\n{tail}\n```\n\
Get main fixed first (put it on the board), then accept T-{ticket} again."
                );
                self.brief_pm(&squad, line, None);
            }
            Outcome::Moved if gate.tries + 1 < TRIES => self.queue_gate(Gate { tries: gate.tries + 1, ..gate }),
            Outcome::Moved => {
                let line = format!("Main or its branch kept moving while T-{ticket}'s checks ran, so what passed was never what would land. T-{ticket} stays in review: accept it again.");
                self.brief_pm(&squad, line, None);
            }
            Outcome::Error(err) => {
                note(self, "not merged", &err);
                let message = say!(self, "error.checks_not_run", id = ticket, err = err);
                self.squad_error(&squad, message);
                let line = format!("T-{ticket}'s checks couldn't run ({err}), so it wasn't merged; it stays in review. Accept it again once that is fixed.");
                self.brief_pm(&squad, line, None);
            }
        }
    }

    /// Checked work accepted, and `merge` — the very commit checked — landed
    /// on main; with no merge, nothing was left to bring (it is merged as before).
    async fn accept_checked(&mut self, gate: &Gate, merge: Option<worktree::MergeCommit>, ran: Option<Vec<String>>) {
        let (squad, ticket) = (gate.squad.as_str(), gate.ticket);
        if let Some(journal) = self.journal(squad) {
            let _ = journal.ticket_event(ticket, &gate.by, "accepted by the PM", &gate.checked);
            if let Some(ran) = &ran {
                let _ = journal.ticket_event(ticket, "equipe", "checks passed", &ran.join("; "));
            }
        }
        Box::pin(self.accept(squad, ticket, crate::telemetry::Accepter::Pm)).await;
        let checked_merge = merge.is_some();
        match merge {
            Some(merge) => self.land_checked(squad, ticket, gate.repo.clone(), merge, gate.push),
            None => self.merge(squad, ticket, gate.push),
        }
        let ids = format!("T-{ticket}");
        let text = match (&ran, checked_merge) {
            (Some(_), _) => say!(self, "notice.pm_accepted_checked", ids = ids, checked = gate.checked),
            (None, true) => say!(self, "notice.pm_accepted_unchecked", ids = ids, checked = gate.checked, file = checks::FILE),
            (None, false) => say!(self, "notice.pm_accepted", ids = ids, checked = gate.checked),
        };
        self.publish(Event::Notice { undo: None, session: gate.session, text });
    }

    /// Land `merge` on main off the loop — only while main is where it was
    /// worked out — and push when asked; [`Runtime::on_merged`] takes it from there.
    fn land_checked(&mut self, squad: &str, ticket: TicketId, repo: PathBuf, merge: worktree::MergeCommit, push: bool) {
        if !self.merging.insert((squad.to_owned(), ticket)) {
            return;
        }
        let (internal, squad) = (self.internal.clone(), squad.to_owned());
        tokio::task::spawn_blocking(move || {
            let result = worktree::land(&repo, &merge).map(|main| (main, push.then(|| worktree::push_main(&repo).map(|_| ()))));
            let _ = internal.send(Internal::Merged { squad, ticket, result });
        });
    }
}

/// Where checks' temporary worktrees go, under equipe's home.
pub(crate) fn checks_scratch(home: &Path) -> PathBuf {
    home.join("checks")
}
