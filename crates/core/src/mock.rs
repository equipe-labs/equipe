//! A stand-in for a coding agent: streams realistic-looking output into the
//! store, so the UI can be measured under load without spending tokens.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::store::{LogKind, Store};

pub struct MockAgent {
    pub name: String,
    /// Lines committed so far; the UI polls this instead of being pushed to.
    pub committed: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockAgent {
    /// Stream `lines_per_sec` lines, committed every `flush` interval.
    pub fn spawn(db: PathBuf, name: &str, lines_per_sec: u32, flush: Duration) -> Self {
        let mut store = Store::open(&db).expect("open store");
        let start = store.next_seq(name).expect("next seq");
        let committed = Arc::new(AtomicU64::new(start));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let (name, committed, stop) = (name.to_string(), committed.clone(), stop.clone());
            thread::Builder::new()
                .name(format!("mock-{name}"))
                .spawn(move || {
                    let per_flush =
                        ((lines_per_sec as f64) * flush.as_secs_f64()).ceil().max(1.0) as u64;
                    let mut seq = start;
                    let mut batch = Vec::with_capacity(per_flush as usize);
                    while !stop.load(Ordering::Relaxed) {
                        let t0 = Instant::now();
                        if lines_per_sec == 0 {
                            thread::sleep(flush);
                            continue;
                        }
                        batch.clear();
                        for i in 0..per_flush {
                            batch.push(line(&name, seq + i));
                        }
                        store.append(&name, seq, &batch).expect("append");
                        seq += per_flush;
                        committed.store(seq, Ordering::Release);
                        thread::sleep(flush.saturating_sub(t0.elapsed()));
                    }
                })
                .expect("spawn mock agent")
        };

        Self { name: name.to_string(), committed, stop, handle: Some(handle) }
    }
}

impl Drop for MockAgent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn line(agent: &str, seq: u64) -> (LogKind, String) {
    const FILES: [&str; 4] = ["src/board.rs", "src/acp/client.rs", "src/store.rs", "ui/log_pane.rs"];
    const TEXT: [&str; 6] = [
        "Reading the ticket spec and acceptance criteria",
        "The board query needs an index on (status, updated_at)",
        "Running tests: 42 passed, 0 failed",
        "Moving the ticket to review and handing it to QA",
        "Refactoring the paging logic so only visible rows are loaded",
        "Checking memory budget: footprint within limits",
    ];
    let file = FILES[(seq as usize / 7) % FILES.len()];
    match seq % 11 {
        0 => (LogKind::Tool, format!("[{agent}] tool: read_file {file}")),
        3 => (LogKind::Tool, format!("[{agent}] tool: run `cargo test -p core` (#{seq})")),
        5 => (LogKind::DiffDel, format!("-    let rows = store.all_lines()?; // #{seq}")),
        6 => (LogKind::DiffAdd, format!("+    let rows = store.lines(agent, visible)?; // #{seq}")),
        _ => (LogKind::Text, format!("{} (#{seq})", TEXT[(seq as usize) % TEXT.len()])),
    }
}
