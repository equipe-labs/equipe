//! A daemon a test or a check started in a temporary home leaves on its own:
//! detached (`setsid`), it outlives whatever started it, and before this they
//! piled up — sixteen at once, each holding its squads, sockets and disk.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_equipe");

/// `equipe daemon run` in a fresh home under /tmp, as a check runs it; ended
/// when dropped, should it still be there.
struct Trial {
    dir: PathBuf,
    daemon: Child,
}

impl Trial {
    fn start(name: &str, idle_secs: u64) -> Self {
        let dir = PathBuf::from("/tmp").join(format!("eqleave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("person")).unwrap();
        let daemon = Command::new(BIN)
            .args(["daemon", "run"])
            .env("HOME", dir.join("person"))
            .env("EQUIPE_HOME", dir.join("home"))
            .env("EQUIPE_LANG", "en")
            .env("EQUIPE_TRIAL_IDLE_SECS", idle_secs.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let trial = Self { dir, daemon };
        let socket = trial.dir.join("home/equipe.sock");
        let started = Instant::now();
        while !socket.exists() && started.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(socket.exists(), "the daemon came up");
        trial
    }

    /// Whether the daemon has exited within `wait`.
    fn exits_within(&mut self, wait: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < wait {
            if self.daemon.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for Trial {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_daemon_in_a_temporary_home_with_no_screen_leaves_on_its_own() {
    let mut trial = Trial::start("idle", 1);
    assert!(trial.exits_within(Duration::from_secs(30)), "no screen, no one at work: it left");
    assert!(!trial.dir.join("home/equipe.sock").exists(), "and took its socket with it");
}

#[test]
fn a_daemon_whose_temporary_home_is_removed_leaves() {
    let mut trial = Trial::start("gone", 3600);
    assert!(!trial.exits_within(Duration::from_secs(2)), "its home is there, and an hour to wait");
    std::fs::remove_dir_all(trial.dir.join("home")).unwrap();
    assert!(trial.exits_within(Duration::from_secs(30)), "its home gone: it left");
}
