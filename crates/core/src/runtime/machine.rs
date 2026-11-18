//! The machine's load, as the daemon watches it ([`crate::machine_load`]):
//! read on each minute tick; while it stays overloaded, equipe starts no new
//! work of its own — ticket runs, routines, the PM's idle wakes and reminders,
//! signals, daily design reviews, design prechecks' browsers, repacks — and the
//! person is told once, then again when it calms down and the held work starts.
//! Turns already running, and what the person asks for themselves, go on.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::machine_load::{self, Change, Reading, Why};

/// How long work started off the loop waits for the machine to calm down
/// before going ahead anyway: held work shouldn't wait forever on a Mac that
/// is always busy.
const WAIT_FOR_CALM_AT_MOST: Duration = Duration::from_secs(2 * 60 * 60);
/// How often such work looks again.
const WAIT_FOR_CALM_EVERY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct MachineWatch {
    pub watch: machine_load::Watch,
    /// Whether new work waits now, for work off the loop ([`wait_for_calm`]).
    pub busy: Arc<AtomicBool>,
    /// When members' orphans were last looked for ([`Runtime::reap_orphans`]).
    reaped_at: Option<std::time::Instant>,
}

/// How often members' orphans are looked for.
const REAP_EVERY: Duration = Duration::from_secs(5 * 60);

impl Runtime {
    /// The machine read now, on the minute tick: only on a daemon watching the
    /// real machine (`Config::watch_disk`), never a test's.
    pub(super) fn look_at_load(&mut self) {
        if !self.config.watch_disk {
            return;
        }
        if let Some(reading) = machine_load::read() {
            self.machine_read(reading);
        }
    }

    /// What members' commands left running once the shell that started them
    /// was gone — a dev server, a Playwright script — stopped once its ticket
    /// is closed (`leftovers::orphans`, `leftovers::verdict`): only processes
    /// a member started, never one whose ticket is open or can't be told.
    /// Every few minutes, off the loop; on a daemon watching the real machine.
    pub(super) fn reap_orphans(&mut self) {
        if !self.config.watch_disk || self.machine.reaped_at.is_some_and(|at| at.elapsed() < REAP_EVERY) {
            return;
        }
        self.machine.reaped_at = Some(std::time::Instant::now());
        let squads: Vec<String> = self.journals.keys().cloned().collect();
        let mut statuses: HashMap<(String, u64), equipe_protocol::TicketStatus> = HashMap::new();
        let mut projects: Vec<(String, Vec<String>)> = Vec::new();
        for squad in squads {
            statuses.extend(self.tickets(&squad).iter().map(|t| ((squad.clone(), u64::from(t.id)), t.status)));
            projects.push((squad.clone(), self.projects_cached(&squad).iter().map(|p| board::project_name(p)).collect()));
        }
        let home = self.config.home.clone();
        tokio::task::spawn_blocking(move || {
            use crate::leftovers::{Judge, orphans, over_the_cap, reap, ticket_place, verdict};
            let place = |p: &Path| ticket_place(p, &home, &projects);
            let status = |squad: &str, ticket: u64| statuses.get(&(squad.to_owned(), ticket)).copied();
            let judge = Judge { status: &status, place: &place };
            let (mut going, mut staying) = (Vec::new(), Vec::new());
            for orphan in orphans() {
                match verdict(&orphan, &judge) {
                    Some(why) => going.push((orphan, why)),
                    None => staying.push(orphan),
                }
            }
            going.extend(over_the_cap(&staying, &home));
            if !going.is_empty() {
                for line in reap(&going) {
                    crate::daemon_log!("equipe: {line}");
                }
            }
        });
    }

    /// equipe's own folders of copies — worktrees, temporary folders, build
    /// caches, the design kit — kept out of Spotlight with its marker file:
    /// indexing them (node_modules by the hundred thousand files) took most
    /// of a core. An app's marker in its own folders; no system setting.
    pub(super) fn keep_out_of_spotlight(&self) {
        if self.config.persist && cfg!(target_os = "macos") {
            never_index(&self.config.home);
        }
    }

    /// Whether equipe holds its own new work back now: the machine stays overloaded.
    pub(super) fn overloaded(&self) -> bool {
        self.machine.watch.overloaded()
    }

    /// A reading of the machine: the person told when the level changes, and
    /// the work held back started once it calms down.
    pub(super) fn machine_read(&mut self, reading: Reading) {
        let Some(change) = self.machine.watch.observe(reading) else { return };
        self.machine.busy.store(self.overloaded(), Ordering::Relaxed);
        let memory = reading.memory_left.map_or_else(|| "unknown".to_owned(), |m| format!("{m}%"));
        let text = match change {
            Change::Overloaded(r, why) => {
                crate::daemon_log!("equipe: this machine is overloaded (load {:.1} on {} cores, memory left {memory}): new work waits; running turns go on", r.load, r.cores);
                match why {
                    Why::Load => say!(self, "notice.machine_overloaded", load = r.load_shown(), cores = r.cores.to_string()),
                    Why::Memory => say!(self, "notice.machine_low_memory", left = r.memory_left.unwrap_or_default().to_string()),
                }
            }
            Change::Calm(r) => {
                crate::daemon_log!("equipe: this machine calmed down (load {:.1} on {} cores, memory left {memory}): the work held back starts", r.load, r.cores);
                let _ = self.internal.send(Internal::Dispatch);
                say!(self, "notice.machine_calm")
            }
        };
        for squad in self.followed_squads() {
            self.squad_error(&squad, text.clone());
        }
        // Runs waiting on their workspace say whether the machine holds them up.
        self.tell_waiting(false);
    }
}

/// What starts of `assigned` (ticket, member) while the machine is
/// overloaded: every ticket the person asked for (`person`), and one more
/// for each member with nothing running (`idle`) — never a member left idle
/// on an always-busy Mac, never a pile of new runs on top of it.
pub(super) fn paced(assigned: Vec<(TicketId, String)>, person: &HashSet<TicketId>, idle: &HashSet<&str>) -> Vec<(TicketId, String)> {
    // A member starting the person's ticket has its one already.
    let mut started: HashSet<String> = assigned.iter().filter(|(t, _)| person.contains(t)).map(|(_, n)| n.clone()).collect();
    assigned.into_iter().filter(|(ticket, name)| person.contains(ticket) || (idle.contains(name.as_str()) && started.insert(name.clone()))).collect()
}

/// The file that keeps Spotlight out of the folder it is in.
pub(super) const NEVER_INDEX: &str = ".metadata_never_index";

/// [`NEVER_INDEX`] in each of equipe's folders of copies under `home` that
/// exists and has none yet.
pub(super) fn never_index(home: &Path) {
    for dir in ["worktrees", "tmp", "cache", "design-kit"].map(|d| home.join(d)) {
        let marker = dir.join(NEVER_INDEX);
        if dir.is_dir() && !marker.exists() {
            let _ = std::fs::write(&marker, b"");
        }
    }
}

/// Until the machine calms down ([`MachineWatch::busy`]), or at most
/// [`WAIT_FOR_CALM_AT_MOST`]: for heavy work started off the loop (a design
/// precheck's browsers) that shouldn't add to an overloaded Mac.
pub(super) async fn wait_for_calm(busy: &AtomicBool) {
    wait_for_calm_within(busy, WAIT_FOR_CALM_EVERY, WAIT_FOR_CALM_AT_MOST).await;
}

async fn wait_for_calm_within(busy: &AtomicBool, every: Duration, at_most: Duration) {
    let began = std::time::Instant::now();
    while busy.load(Ordering::Relaxed) && began.elapsed() < at_most {
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipes_folders_of_copies_are_kept_out_of_spotlight() {
        let home = std::env::temp_dir().join(format!("equipe-never-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["worktrees", "tmp", "squads"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        never_index(&home);
        assert!(home.join("worktrees").join(NEVER_INDEX).is_file() && home.join("tmp").join(NEVER_INDEX).is_file());
        assert!(!home.join("squads").join(NEVER_INDEX).exists(), "the squads' own files stay findable");
        assert!(!home.join("cache").exists(), "a folder not there isn't made");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn overloaded_the_persons_work_starts_and_each_idle_member_gets_one() {
        let assigned: Vec<(TicketId, String)> = [(1, "designer"), (2, "designer"), (3, "backend"), (4, "backend"), (5, "frontend"), (6, "frontend")].into_iter().map(|(t, n)| (t, n.to_owned())).collect();
        let person: HashSet<TicketId> = [4, 6].into_iter().collect();
        let idle: HashSet<&str> = ["designer", "backend"].into_iter().collect();
        let started = paced(assigned, &person, &idle);
        // The designer, idle: one. The backend, idle, gets the person's ticket (and nothing more of its own).
        // The frontend, busy: only the person's.
        assert_eq!(started.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [1, 4, 6]);
    }

    #[tokio::test]
    async fn work_off_the_loop_waits_for_calm_and_goes_at_once_when_calm() {
        let (every, at_most) = (Duration::from_millis(10), Duration::from_secs(5));
        let busy = Arc::new(AtomicBool::new(false));
        // Calm: no wait at all.
        tokio::time::timeout(Duration::from_millis(5), wait_for_calm_within(&busy, every, at_most)).await.expect("no wait when calm");
        // Overloaded: it waits until calm.
        busy.store(true, Ordering::Relaxed);
        let waiting = tokio::spawn({
            let busy = busy.clone();
            async move { wait_for_calm_within(&busy, every, at_most).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiting.is_finished(), "held while overloaded");
        busy.store(false, Ordering::Relaxed);
        tokio::time::timeout(Duration::from_secs(2), waiting).await.expect("goes once calm").unwrap();
        // Never forever.
        tokio::time::timeout(Duration::from_secs(2), wait_for_calm_within(&AtomicBool::new(true), every, Duration::from_millis(50))).await.expect("not past its limit");
    }
}
