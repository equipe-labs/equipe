//! Tests' scratch folders in the system's temporary folder: each test run
//! made its own and none took them away — thousands of them, gigabytes a
//! day. A run sweeps away those its earlier runs left (older than an hour,
//! so a run going on beside it keeps its own) the first time it makes one.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Older than this, a scratch folder is a run's that has ended.
const LEFT_AFTER: Duration = Duration::from_secs(60 * 60);

/// Sweep the temporary folder's `prefix…` folders left from earlier runs, once
/// per `done` (a `std::sync::Once` of the caller's).
#[doc(hidden)]
pub fn sweep_once(done: &std::sync::Once, prefix: &str) {
    done.call_once(|| sweep(&std::env::temp_dir(), prefix, SystemTime::now()));
}

fn sweep(dir: &Path, prefix: &str, now: SystemTime) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(prefix) {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| now.duration_since(at).unwrap_or_default() > LEFT_AFTER);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_sweeps_what_earlier_runs_left_and_keeps_what_runs_now() {
        let dir = std::env::temp_dir().join(format!("equipe-sweep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["equipe-x-old", "equipe-x-new", "other-old"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let later = SystemTime::now() + LEFT_AFTER + Duration::from_secs(60);
        std::fs::File::open(dir.join("equipe-x-new")).unwrap().set_modified(later).unwrap();
        sweep(&dir, "equipe-x-", later);
        assert!(!dir.join("equipe-x-old").exists(), "left by an earlier run");
        assert!(dir.join("equipe-x-new").exists(), "a run going on now");
        assert!(dir.join("other-old").exists(), "not ours");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
