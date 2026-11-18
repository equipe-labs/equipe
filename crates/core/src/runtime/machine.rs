//! The machine's load, as the daemon watches it ([`crate::machine_load`]):
//! read on each minute tick; while it stays overloaded, equipe starts no new
//! work of its own — ticket runs, routines, the PM's idle wakes and reminders,
//! signals, daily design reviews, design prechecks' browsers, repacks — and the
//! person is told once, then again when it calms down and the held work starts.
//! Turns already running, and what the person asks for themselves, go on.

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
}

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
